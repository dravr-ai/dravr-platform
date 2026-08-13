// ABOUTME: Handler for /pillars — (re)start the guided pillar-onboarding walk on a conversation
// ABOUTME: Activates onboarding mode; `full` / a pillar name re-screen by superseding prior onboarding facts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{
    KEY_PILLARS_DM_ONLY, KEY_PILLARS_OPENER, KEY_PILLARS_START_FAILED,
};
use pierre_core::errors::AppError;
use pierre_core::models::{AddMessageParams, GuidedFlow, OnboardingState, Pillar};
use pierre_messaging::commands::CommandResponse;
use tracing::{info, warn};

use crate::{CommandHandler, PlatformCommandContext};

/// Athlete-facing spellings of the six pillars, mapped to the canonical enum.
///
/// [`Pillar::parse`] stays closed to the DB slug set on purpose — it also parses
/// stored rows, where accepting a loose spelling would let a bad value round-trip
/// — so the short forms an athlete would actually type live here instead. The
/// canonical slugs keep working: [`parse_pillar_arg`] tries them first.
///
/// A bare `recovery` is deliberately absent. It reads equally as *Sleep &
/// Recovery* and *Recovery Optimisation*, and choosing one would silently expire
/// the wrong pillar's facts; [`unknown_pillar_error`] names both instead.
const PILLAR_ALIASES: [(&str, Pillar); 12] = [
    ("training", Pillar::TrainingAndMovement),
    ("movement", Pillar::TrainingAndMovement),
    ("fueling", Pillar::Fuelling),
    ("nutrition", Pillar::Fuelling),
    ("sleep", Pillar::SleepAndRecovery),
    ("rest", Pillar::SleepAndRecovery),
    ("mental", Pillar::MentalResilience),
    ("stress", Pillar::MentalResilience),
    ("community", Pillar::CommunityAndConnection),
    ("social", Pillar::CommunityAndConnection),
    ("substances", Pillar::RecoveryOptimisation),
    ("alcohol", Pillar::RecoveryOptimisation),
];

/// Resolve a `/pillars` argument to a pillar.
///
/// Accepts the canonical DB slug (`sleep_and_recovery`) and the short athlete
/// spelling (`sleep`) — the latter is what `/help` advertises. Callers pass the
/// already-trimmed, lowercased token.
#[must_use]
pub fn parse_pillar_arg(arg: &str) -> Option<Pillar> {
    Pillar::parse(arg).or_else(|| {
        PILLAR_ALIASES
            .iter()
            .find(|(alias, _)| *alias == arg)
            .map(|(_, pillar)| *pillar)
    })
}

/// The error for an argument that names no pillar, listing the spellings that
/// work so the athlete's next attempt succeeds.
fn unknown_pillar_error(arg: &str) -> AppError {
    AppError::invalid_input(format!(
        "unknown pillar '{arg}' — use one of: training, fuelling, sleep, mental, community, substances (or `full` for every pillar)"
    ))
}

/// Handler for `/pillars` — (re)build the user's pillar context conversationally.
///
/// - `/pillars` — enter onboarding mode; the next turns walk the North Star +
///   six pillars (coverage is re-derived from the Dossier each turn).
/// - `/pillars full` — re-screen everything: supersede prior onboarding facts
///   (they go stale) then walk all topics again.
/// - `/pillars <pillar>` — re-screen a single pillar. Takes the short athlete
///   spelling `/help` advertises (`sleep`) or the canonical slug
///   (`sleep_and_recovery`); see [`parse_pillar_arg`].
///
/// Superseding is done via `expire_onboarding_facts` (sets `valid_until=now`),
/// never deletion — the GDPR forget path stays separate.
///
/// Direct messages only. Slash replies from a shared room are already rerouted
/// to a private chat, so a group walk would ask the athlete about motivations,
/// sleep and stress in DM while the extraction worker stamped their answers
/// under the *channel* tenant — a fact space disjoint from their own dossier.
pub struct PillarsHandler;

#[async_trait]
impl CommandHandler for PillarsHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();

        if !ctx.is_direct_message {
            info!(
                user_id = %ctx.user_id,
                channel = %ctx.channel_type,
                "/pillars refused outside a direct message"
            );
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_PILLARS_DM_ONLY,
                &ctx.locale,
                &[],
            )));
        }

        let conversation_id = ctx.conversation_id.as_deref().ok_or_else(|| {
            AppError::invalid_input("/pillars needs an active conversation to onboard into")
        })?;
        let repos = ctx.ctx.repos();
        let user = ctx.user_id.to_string();

        // Re-screen scope: `full` expires all onboarding facts, a pillar name
        // expires that pillar's, bare `/pillars` supersedes nothing.
        let arg = ctx.args.first().map(|s| s.trim().to_lowercase());
        match arg.as_deref() {
            Some("full") => {
                let n = repos
                    .memory
                    .expire_onboarding_facts(ctx.tenant_id, &user, None, None)
                    .await?;
                info!(user_id = %ctx.user_id, superseded = n, "/pillars full re-screen");
            }
            Some(slug) => {
                let pillar = parse_pillar_arg(slug).ok_or_else(|| unknown_pillar_error(slug))?;
                let n = repos
                    .memory
                    .expire_onboarding_facts(ctx.tenant_id, &user, Some(pillar), None)
                    .await?;
                info!(user_id = %ctx.user_id, pillar = pillar.as_str(), superseded = n, "/pillars pillar re-screen");
            }
            None => {}
        }

        // Activate onboarding mode on this conversation. The returned `bool`
        // reports whether a row actually matched: a tenant mismatch updates
        // nothing, and answering with the opener anyway would start a walk that
        // no turn ever runs in.
        let json = OnboardingState::start_now_column(GuidedFlow::Pillars);
        let activated = repos
            .chat
            .set_conversation_onboarding_state(conversation_id, Some(&json), ctx.tenant_id)
            .await?;
        if !activated {
            warn!(
                user_id = %ctx.user_id,
                conversation_id,
                "/pillars matched no conversation row; onboarding not activated"
            );
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_PILLARS_START_FAILED,
                &ctx.locale,
                &[],
            )));
        }

        info!(user_id = %ctx.user_id, "onboarding mode activated via /pillars");

        let msg = reg.render(KEY_PILLARS_OPENER, &ctx.locale, &[]);

        // Persist the opener as the conversation's first assistant message — a
        // scoped exception to slash-command ephemerality, for flow-opening
        // commands only.
        //
        // Two things depend on it. The coach otherwise receives the athlete's
        // North Star answer with no question attached, and improvises a reply to
        // a question it cannot see; and with no history row, that answer is
        // message #1, which arms the first-turn coach startup prefetch — an
        // activity dump plus the coach's own "build a block" query injected as
        // if the athlete had asked for it (the 2026-07-24 derail).
        //
        // Written under the same tenant as the state above, so
        // `get_conversation_history` — which filters on the conversation
        // tenant — actually sees it.
        let opener = AddMessageParams {
            tenant_id: ctx.tenant_id,
            conversation_id,
            user_id: &user,
            role: "assistant",
            content: &msg,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            structured_content: None,
        };
        if let Err(e) = repos.chat.add_message(&opener).await {
            // The walk is already active and re-derives coverage from the
            // Dossier every turn, so a failed history write degrades the first
            // turn rather than the flow: ask the question anyway.
            warn!(error = %e, "failed to persist /pillars opener as assistant message");
        }

        Ok(CommandResponse::rich_text(msg))
    }
}
