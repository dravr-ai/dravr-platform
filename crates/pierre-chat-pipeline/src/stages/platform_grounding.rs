// ABOUTME: Whether a reply the platform composed on its own initiative may be posted, and what it ends on
// ABOUTME: Posted only when grounded and not a canonical refusal; a stored post closes on ranked catalogue use cases
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The rule a [`TurnOrigin::Platform`](pierre_core::models::TurnOrigin) turn's
//! reply must pass before it is stored.
//!
//! A turn the athlete started is answered whatever the answer is: they asked,
//! and "I can't help with that" is an honest reply to a question. A turn the
//! platform started has no question behind it — a completed backfill re-asking
//! the athlete's earlier question, the coach speaking first after their data
//! lands (carnet#831). Posting unprompted is only worth it when the coach has
//! something of the athlete's own to say, so the reply goes out only when the
//! turn grounded in their data and did not refuse. Otherwise nothing is
//! written at all: the caller falls back to what it would have sent without a
//! model, or sends nothing.
//!
//! A post that is stored ends on what the athlete could do next: at most
//! three use cases from the catalogue, ranked from the athlete's state the
//! way a welcome's starters are ([`closing_steps`]), so the coach speaking
//! first leaves a tap, not a blank composer.

use chrono::Utc;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_CAPABILITY_REFUSAL, KEY_SCOPE_REFUSAL,
};
use pierre_core::narration::IdentityLeakMatch;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_providers::ai_scope;
use pierre_services::use_case_rank::{rank_for_athlete, rotation_for};
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;
use tracing::{info, warn};
use uuid::Uuid;

use crate::agent_welcome::guided_walk_owns;
use crate::envelope::{build_envelope, TurnEnvelope, TurnState, TurnTelemetry};
use crate::next_steps::NextSteps;
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::ChatPipelineContext;

/// What a turn put in front of the model from the athlete's own data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grounding {
    /// The tool loop captured a formatted activity list: the model asked for
    /// the athlete's activities and got them.
    pub activity_list_captured: bool,
    /// The platform prefetched a non-empty window of the athlete's activities
    /// into the prompt, and the prompt contract told the agent to answer from
    /// it without fetching again.
    pub activities_prefetched: bool,
    /// A tool call served the athlete's provider items — sleep, recovery,
    /// load, goals — to the model, as the dispatch chokepoint records on the
    /// turn's provenance.
    pub provider_data_served: bool,
}

/// Whether a platform-composed reply may be posted.
///
/// Grounded three ways. A list the tool loop captured is trusted whatever the
/// prose: the list is folded into what a channel shows, so the athlete
/// receives their data even when the analysis around it is thin. A prefetched
/// window and a provider read put the data in front of the model without
/// making it use the data, so a reply grounded only that way is posted unless
/// it is a refusal — the model had the data and declined, and a refusal sent
/// unprompted reads as the coach volunteering that it cannot help.
///
/// Refusals are canonical: the system prompt interpolates the exact sentence
/// per locale, so a reply carrying one is recognised rather than guessed at.
///
/// Deliberately reads no tool names: the prefetch leaves `get_activities`
/// behind in the called-tools list, and so does a model call that failed, so
/// the name alone says nothing about whether any data reached the model.
#[must_use]
pub fn admits_platform_reply(
    grounding: &Grounding,
    reply: &str,
    canonical_refusals: &[String],
) -> bool {
    if grounding.activity_list_captured {
        return true;
    }
    (grounding.activities_prefetched || grounding.provider_data_served)
        && !carries_canonical_refusal(reply, canonical_refusals)
}

/// The platform's canonical refusal sentences in `locale`.
///
/// They are the ones the system prompt tells the agent to answer with when a
/// request is out of scope or beyond its capabilities.
#[must_use]
pub fn canonical_refusals(registry: &MessagingStringsRegistry, locale: &str) -> [String; 2] {
    [KEY_SCOPE_REFUSAL, KEY_CAPABILITY_REFUSAL].map(|key| registry.get(key, locale))
}

/// Whether `reply` contains one of the platform's canonical refusal sentences.
///
/// Compared case-insensitively with whitespace collapsed, because a channel
/// fold or the model's own line breaks can re-wrap the sentence without
/// changing it. An empty sentence — a locale the catalogue does not carry —
/// is skipped: it would otherwise be contained in every reply.
fn carries_canonical_refusal(reply: &str, canonical_refusals: &[String]) -> bool {
    let normalize = |text: &str| {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let reply = normalize(reply);
    canonical_refusals
        .iter()
        .map(|sentence| normalize(sentence))
        .any(|sentence| !sentence.is_empty() && reply.contains(&sentence))
}

/// A platform-composed turn's reply, as it stands just before it would be
/// stored.
pub(crate) struct PlatformReplyDraft<'a> {
    pub(crate) input: &'a TurnInput,
    pub(crate) user_message: &'a MessageRecord,
    pub(crate) conv: &'a ConversationRecord,
    /// The reply text that would be stored.
    pub(crate) content: &'a str,
    pub(crate) result: &'a ToolLoopResult,
    pub(crate) active_model: &'a str,
    pub(crate) provider_name: &'a str,
    pub(crate) activities_prefetched: bool,
    pub(crate) identity_leak: Option<IdentityLeakMatch>,
    pub(crate) answered_by: Option<String>,
}

/// The envelope of a platform-composed reply that is withheld, or `None` when
/// the reply may be stored — always `None` on a turn the athlete sent.
///
/// The rule is [`admits_platform_reply`]. A
/// withheld reply stores nothing at all: no assistant row, no tool rounds (the
/// turn held them for this decision), no verdicts, no memory pass, no session
/// finalize, because the athlete was told nothing. The
/// envelope still carries the turn's token usage, so the model calls it made
/// count against the athlete's budget like any other served turn.
pub(crate) fn withheld_platform_reply(
    ctx: &ChatPipelineContext,
    profile: &SurfaceProfile,
    draft: PlatformReplyDraft<'_>,
) -> Option<TurnEnvelope> {
    if draft.input.origin.persists_user_row() {
        return None;
    }
    let grounding = Grounding {
        activity_list_captured: draft.result.activity_list.is_some(),
        activities_prefetched: draft.activities_prefetched,
        provider_data_served: ai_scope::current_provenance()
            .is_some_and(|provenance| provenance.served_provider_data()),
    };
    let refusals = canonical_refusals(&ctx.messaging_strings_registry, &profile.locale);
    if admits_platform_reply(&grounding, draft.content, &refusals) {
        return None;
    }
    info!(
        conversation_id = %draft.input.conversation_id,
        activity_list_captured = grounding.activity_list_captured,
        activities_prefetched = grounding.activities_prefetched,
        provider_data_served = grounding.provider_data_served,
        "platform-composed reply withheld: not grounded in the athlete's data, or a refusal"
    );
    let unstored = MessageRecord {
        id: Uuid::new_v4().to_string(),
        conversation_id: draft.input.conversation_id.clone(),
        role: "assistant".to_owned(),
        content: draft.content.to_owned(),
        token_count: None,
        prompt_tokens: None,
        model: Some(draft.active_model.to_owned()),
        finish_reason: draft.result.finish_reason.clone(),
        content_blocks: None,
        transport_policy: ai_scope::derived_policy(),
        created_at: Utc::now().to_rfc3339(),
        channel_type: None,
    };
    let envelope = build_envelope(
        profile,
        TurnState {
            turn_id: draft.input.turn_id,
            user_message: draft.user_message.clone(),
            assistant_message: unstored,
            conversation: draft.conv.clone(),
            content: draft.content.to_owned(),
            finish_reason: draft.result.finish_reason.clone(),
            activity_list: None,
            introduction_end: None,
            telemetry: TurnTelemetry {
                model: draft.active_model.to_owned(),
                provider_name: draft.provider_name.to_owned(),
                tools_called: draft.result.tools_called.clone(),
                tool_calls_count: draft.result.tool_calls_count,
                activity_list_captured: grounding.activity_list_captured,
                activities_prefetched: grounding.activities_prefetched,
                usage: draft.result.usage.clone(),
                identity_leak: draft.identity_leak,
                provider_warnings: draft.result.provider_warnings.clone(),
            },
            quota: draft.input.quota.clone(),
            reconnect: None,
            verdict_chips: Vec::new(),
            scene_images: Vec::new(),
            actions: Vec::new(),
            actions_title: None,
            answered_by: draft.answered_by,
        },
    );
    Some(TurnEnvelope {
        posted: false,
        ..envelope
    })
}

/// The steps a platform-composed reply that is about to be stored ends on:
/// the catalogue's use cases ranked for the athlete and the thread's agent.
///
/// They are ranked as a welcome's starters are, and offered as next steps
/// labelled in the post's language.
///
/// None on a turn the athlete sent, in a room (one athlete's state is not
/// read back to everyone), in a thread no agent is bound to, or while a
/// guided walk owns the thread — its next question is the athlete's next
/// move. Read inside the post's turn, so the state the ranking reads is part
/// of what the stored reply is stamped from. Best-effort: an athlete whose
/// state cannot be read gets the reply without steps.
pub(crate) async fn closing_steps(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    profile: &SurfaceProfile,
    conv: &ConversationRecord,
) -> NextSteps {
    if input.origin.persists_user_row() || conv.group_id.is_some() || guided_walk_owns(conv) {
        return NextSteps::default();
    }
    let (Some(agent_id), Ok(user)) = (conv.agent_id.as_deref(), parse_uuid(&input.user_id)) else {
        return NextSteps::default();
    };
    // A platform post runs as the athlete, in their own tenant: the one their
    // agent, their state and their taps resolve in.
    let tenant = input.tool_tenant_id;
    let repos = &ctx.repos;
    let agent = match repos.agents.get_by_id(agent_id, user, tenant).await {
        Ok(Some(agent)) => agent,
        Ok(None) => return NextSteps::default(),
        Err(e) => {
            warn!(error = %e, conversation_id = %conv.id, "the post's agent is unreadable; it ends on no step");
            return NextSteps::default();
        }
    };
    let ranked = match rank_for_athlete(repos, tenant, user, agent.category, rotation_for(&conv.id))
        .await
    {
        // Steps ranked from first-party-only data carry its stamp onto the
        // stored reply, and are offered only where that data may be served.
        Ok(ranked) if ai_scope::admit_derived(ranked.policy) => ranked,
        Ok(_) => return NextSteps::default(),
        Err(e) => {
            warn!(error = %e, conversation_id = %conv.id, "athlete state unreadable; the post ends on no step");
            return NextSteps::default();
        }
    };
    // Reported shown with the reply's other steps, never counted toward
    // retiring a welcome's starter: that counts welcomes, and a daily
    // check-in would retire them within the week.
    NextSteps::labelled(
        &ranked.entries,
        &ctx.messaging_strings_registry,
        &profile.locale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical French refusals, read from the same registry the system
    /// prompt interpolates them from — so the tests break with the wording,
    /// not behind it.
    fn french_refusals() -> Vec<String> {
        canonical_refusals(&MessagingStringsRegistry::new(), "fr").to_vec()
    }

    const fn grounding(captured: bool, prefetched: bool, served: bool) -> Grounding {
        Grounding {
            activity_list_captured: captured,
            activities_prefetched: prefetched,
            provider_data_served: served,
        }
    }

    /// An answer built from the prefetched window is the athlete's answer.
    ///
    /// Observed 2026-09-21: "Compare mon dénivelé mensuel de mars à
    /// septembre". The backfill completed, the re-entry turn had the window
    /// prefetched into its prompt — and was told by the prompt contract not
    /// to re-fetch it — and it answered with the monthly totals. No tool call,
    /// so no captured list, and the push threw the answer away.
    #[test]
    fn an_answer_from_the_prefetched_window_is_posted() {
        assert!(
            admits_platform_reply(
                &grounding(false, true, false),
                "Septembre: 5 918 m de D+, ta plus grosse sortie c'est Montagne sacrée! (924 m). \
                 Août: 7 577 m. Juillet: 7 044 m.",
                &french_refusals(),
            ),
            "the model had every row in front of it and answered"
        );
    }

    /// The coach's first post after a WHOOP connect reads recovery and sleep,
    /// not activities: a provider read grounds it just the same.
    #[test]
    fn an_answer_from_a_provider_read_is_posted() {
        assert!(admits_platform_reply(
            &grounding(false, false, true),
            "Ta récupération est restée stable cette semaine: 71 % ce matin.",
            &french_refusals(),
        ));
    }

    /// A refusal is never posted unprompted, whatever put the data in front of
    /// the model.
    #[test]
    fn a_refusal_over_grounded_data_is_not_posted() {
        let refusals = french_refusals();
        for sentence in &refusals {
            assert!(
                !sentence.trim().is_empty(),
                "the catalogue must carry the French refusals this test stands on"
            );
            for grounded in [grounding(false, true, false), grounding(false, false, true)] {
                assert!(!admits_platform_reply(&grounded, sentence, &refusals));
                // Re-wrapped by a channel fold, re-cased, and wrapped in other
                // prose: still the same sentence.
                let rewrapped = format!(
                    "Désolé!\n\n{}\n",
                    sentence.to_uppercase().replace(' ', "\n  ")
                );
                assert!(
                    !admits_platform_reply(&grounded, &rewrapped, &refusals),
                    "a re-wrapped refusal is still a refusal: {rewrapped}"
                );
            }
        }
    }

    /// A reply with nothing of the athlete's in front of it is not posted,
    /// however plausible it reads.
    #[test]
    fn an_ungrounded_reply_is_not_posted() {
        assert!(!admits_platform_reply(
            &grounding(false, false, false),
            "Septembre: 5 918 m de D+.",
            &french_refusals(),
        ));
    }

    /// A list the tool loop captured is trusted as it always was — the list is
    /// folded into the body, so the athlete receives their data whatever the
    /// prose.
    #[test]
    fn a_captured_list_is_trusted_whatever_the_prose() {
        assert!(admits_platform_reply(
            &grounding(true, false, false),
            "Voici tes sorties.",
            &french_refusals(),
        ));
    }

    /// A locale the catalogue does not carry yields an empty sentence, and an
    /// empty sentence is contained in every reply. It must not turn every
    /// answer into a refusal.
    #[test]
    fn a_missing_refusal_string_does_not_reject_every_reply() {
        assert!(admits_platform_reply(
            &grounding(false, true, false),
            "Septembre: 5 918 m de D+.",
            &[String::new(), String::new()],
        ));
    }
}
