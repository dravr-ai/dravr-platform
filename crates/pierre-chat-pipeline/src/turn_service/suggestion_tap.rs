// ABOUTME: A turn whose text is a tapped suggestion's postback: resolved to its words, or answered in one line
// ABOUTME: Starters, next steps, Getting started rows and add-to-team offer links all arrive here first

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The suggestion-tap step of the turn ladder (carnet#828, carnet#830,
//! carnet#1099): a postback becomes the words it stands for, or a one-line
//! platform answer when it no longer resolves or is an add-to-team link.

use pierre_contremaitre::messaging_strings::KEY_USE_CASES_UNAVAILABLE;
use pierre_core::errors::AppResult;
use pierre_core::models::{CarriedRead, ConversationRecord};
use pierre_database::repositories::UNATTRIBUTED_READ_KEY;
use pierre_services::first_run;
use tracing::warn;

use super::{CommandTurn, TurnRequest};
use crate::next_steps::StepsAfter;
use crate::suggestions::{self, Resolution, SuggestionEvent, SuggestionSource, TapContext};
use crate::surface_profile::SurfaceProfile;
use crate::team_offer::{self, OfferFor};
use crate::ChatPipelineContext;

/// What resolving a turn's text as a tapped suggestion concluded.
pub(super) enum TapOutcome {
    /// The turn goes on, with its text resolved or as it was sent.
    Proceed {
        /// The transition the turn completes, whose next steps its reply
        /// offers; `None` for every turn but the athlete's first tapped
        /// workout analysis.
        steps_after: Option<StepsAfter>,
    },
    /// A postback the platform answers in one line itself — one that no
    /// longer resolves, or a link of an add-to-team offer. No model runs and
    /// no transcript row is written: the athlete's line would be the
    /// postback, which means nothing to them.
    Answered(Box<CommandTurn>),
}

/// Rewrite `request` in place when its text is a suggestion's postback: the
/// words it stands for become the content, and the turn is reported as a
/// tapped starter or next step (carnet#828, carnet#830).
///
/// # Errors
///
/// Returns the repository error when the thread's agent cannot be read.
pub(super) async fn resolve_suggestion_tap(
    ctx: &ChatPipelineContext,
    request: &mut TurnRequest<'_>,
    conversation: Option<&ConversationRecord>,
    profile: &SurfaceProfile,
) -> AppResult<TapOutcome> {
    let tap = TapContext {
        conversation,
        user_id: request.user_id,
        agent_tenant_id: request.tool_tenant_id,
        locale: &profile.locale,
    };
    let resolution = suggestions::resolve(
        &ctx.repos,
        &ctx.messaging_strings_registry,
        &tap,
        &request.content,
    )
    .await?;
    if let Resolution::Resolved { postback, .. } = &resolution {
        // The model wrote a quick reply's words from a reply in this thread:
        // when the thread drew on another member's data, stored as the
        // sender's own no rule would withhold them, so the tap is answered as
        // one that no longer resolves (carnet#1511 call 21: dropped, not
        // stamped).
        if postback.source() == SuggestionSource::Model
            && Box::pin(thread_read_another_member(ctx, request, conversation)).await?
        {
            return Ok(TapOutcome::Answered(Box::new(unavailable_suggestion(
                ctx,
                &profile.locale,
            ))));
        }
    }
    match resolution {
        Resolution::NotAPostback => Ok(TapOutcome::Proceed { steps_after: None }),
        Resolution::Resolved { content, postback } => {
            // Read before this tap is recorded, which would make every tap
            // look like a repeat.
            let steps_after = match postback.catalogue_id() {
                Some(id) => match StepsAfter::on_first_tap(id) {
                    Some(after) if !ever_tapped(ctx, request, id).await => Some(after),
                    _ => None,
                },
                None => None,
            };
            let user_id = request.user_id.to_string();
            suggestions::record_tap(
                &ctx.repos,
                &SuggestionEvent {
                    user_id: &user_id,
                    tenant_id: request.conversation_tenant_id,
                    surface: postback.surface(),
                    channel: profile.surface.as_str(),
                },
                &postback,
                request.tool_tenant_id,
                request.user_id,
            )
            .await;
            if conversation.is_some_and(|thread| thread.personal) {
                first_run::after_personal_tap(
                    &ctx.repos,
                    request.user_id,
                    request.tool_tenant_id,
                    profile.surface.as_str(),
                )
                .await;
            }
            request.content = content;
            request.input_source = postback.input_source();
            Ok(TapOutcome::Proceed { steps_after })
        }
        Resolution::Unresolvable { postback } => {
            warn!(
                use_case = %postback.use_case(),
                conversation_id = %request.conversation_id,
                "a tapped suggestion no longer resolves; the athlete is told, no model runs"
            );
            Ok(TapOutcome::Answered(Box::new(unavailable_suggestion(
                ctx,
                &profile.locale,
            ))))
        }
        Resolution::TeamOffer {
            choice,
            pillar,
            postback,
        } => {
            let user_id = request.user_id.to_string();
            suggestions::record_tap(
                &ctx.repos,
                &SuggestionEvent {
                    user_id: &user_id,
                    tenant_id: request.conversation_tenant_id,
                    surface: postback.surface(),
                    channel: profile.surface.as_str(),
                },
                &postback,
                request.tool_tenant_id,
                request.user_id,
            )
            .await;
            let text = team_offer::answer(
                &ctx.repos,
                &ctx.messaging_strings_registry,
                &OfferFor {
                    user_id: request.user_id,
                    tenant_id: request.tool_tenant_id,
                    thread_agent: conversation.and_then(|thread| thread.agent_id.as_deref()),
                    locale: &profile.locale,
                },
                choice,
                pillar,
            )
            .await;
            Ok(TapOutcome::Answered(Box::new(platform_line(text))))
        }
    }
}

/// Whether a row of `conversation`'s current topic carries a read of anyone
/// but the turn's sender: what a reply there may restate, and the model's
/// quick replies with it. No conversation yet holds no row.
///
/// Read since the newest divider, as the offer decides by the reads the
/// turn's replay notes ([`crate::next_steps::for_reply`]): a quick reply
/// offered after `/reset` or "+" by a turn that drew on no one is taken:
/// no read stops it. Another participant's own words count as theirs. A legacy
/// row no read names ([`UNATTRIBUTED_READ_KEY`], carnet#1511 A8) names no
/// member, and the offer, which reads the turn's carried reads, counts none
/// for it: it is no read here either, so the offer and the tap agree.
async fn thread_read_another_member(
    ctx: &ChatPipelineContext,
    request: &TurnRequest<'_>,
    conversation: Option<&ConversationRecord>,
) -> AppResult<bool> {
    let Some(conversation) = conversation else {
        return Ok(false);
    };
    let reads = ctx
        .repos
        .chat
        .list_member_reads(
            &conversation.id,
            &request.user_id.to_string(),
            request.conversation_tenant_id,
        )
        .await?;
    Ok(reads
        .iter()
        .map(|read| &read.key)
        .filter(|key| key.as_str() != UNATTRIBUTED_READ_KEY)
        .any(|key| {
            let member = CarriedRead::from_key(key)
                .map(|read| read.member)
                .or_else(|| CarriedRead::legacy_about_member(key));
            member != Some(request.user_id)
        }))
}

/// Whether the athlete tapped the catalogue entry `id` before this tap. An
/// unreadable ledger answers yes, so a first-time step is never offered on a
/// guess.
async fn ever_tapped(ctx: &ChatPipelineContext, request: &TurnRequest<'_>, id: &str) -> bool {
    match ctx
        .repos
        .use_case_exposures
        .use_case_exposures(request.tool_tenant_id, request.user_id)
        .await
    {
        Ok(exposures) => exposures
            .iter()
            .any(|exposure| exposure.use_case_id == id && exposure.tapped_at.is_some()),
        Err(e) => {
            warn!(error = %e, use_case = %id, "use-case taps unreadable; the tap is not taken for a first one");
            true
        }
    }
}

/// The reply to a suggestion that no longer resolves: a notice in the
/// athlete's language, no model call. Not written to the transcript — the
/// athlete's line would be the postback, which means nothing to them.
fn unavailable_suggestion(ctx: &ChatPipelineContext, locale: &str) -> CommandTurn {
    platform_line(
        ctx.messaging_strings_registry
            .get(KEY_USE_CASES_UNAVAILABLE, locale),
    )
}

/// A one-line platform answer to a tap, with nothing to press.
fn platform_line(text: String) -> CommandTurn {
    CommandTurn {
        command_name: None,
        text,
        is_rich_text: false,
        card_title: None,
        actions: Vec::new(),
        rotated_to: None,
        opened_topic: None,
        persisted: None,
        welcome: None,
    }
}
