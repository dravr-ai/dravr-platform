// ABOUTME: The one-time onboarding agent proposal a messaging turn sends ahead of its first coached reply
// ABOUTME: Builds the inferred-profile proposal, renders it as channel text, sends it, stamps the link once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write as _;

use dravr_canot::turn::ConversationTurnId as CanotTurnId;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_AGENT_PROPOSAL_FOOTER, KEY_AGENT_PROPOSAL_WELCOME,
    KEY_AGENT_PROPOSAL_WELCOME_GENERIC,
};
use pierre_core::errors::AppResult;
use pierre_core::models::messaging::{ChannelConfig, MessageContent, OutgoingMessage};
use pierre_database::backends::MessagingRepository;
use pierre_database::repositories::OnboardingStepRecord;
use pierre_routes_agents::agents::{build_agent_proposal, ProposedAgent, SportProfileSummary};
use pierre_services::activity_sports::sport_label;
use pierre_services::analytics::hash_id;
use pierre_services::intake;
use tracing::{info, warn};

use super::PendingDispatch;

/// Auto-send the one-time onboarding agent proposal for this user, if it hasn't
/// been sent on this channel link yet.
///
/// Fires on the user's first provider-connected messaging turn: builds the
/// inferred-profile proposal (shared with the REST route via
/// [`build_agent_proposal`]), renders it to text, sends it through the channel
/// adapter, and stamps the link so it never re-sends. Entirely best-effort —
/// any failure is logged and the turn proceeds normally. When no agents are
/// eligible yet (cold start, activities not synced) it returns *without*
/// stamping, so a later turn can propose once data lands.
pub(super) async fn maybe_send_agent_proposal(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
) {
    let Some((outgoing, offered_ids)) = build_agent_proposal_message(dispatch).await else {
        return;
    };

    if let Err(e) = dispatch.adapter.send(&outgoing, channel_config).await {
        warn!(error = %e, "coach proposal: send failed; will retry next turn");
        return;
    }

    stamp_agent_proposal_sent(dispatch, &offered_ids).await;

    info!(
        hashed_user = %hash_id(&dispatch.session.user_id),
        "coach proposal auto-sent"
    );
}

/// Stamp the channel link so the proposal is never re-sent. Best-effort: a
/// failure here only risks a duplicate proposal on a later turn, never the turn.
async fn stamp_agent_proposal_sent(dispatch: &PendingDispatch, offered_ids: &[String]) {
    let db: &dyn MessagingRepository = dispatch.resources.common.repos.messaging.as_ref();
    if let Err(e) = db
        .mark_agent_proposal_sent(
            dispatch.channel_tenant_id,
            &dispatch.channel,
            &dispatch.sender_id,
            offered_ids,
        )
        .await
    {
        warn!(error = %e, "coach proposal: sent but failed to stamp link; may re-send next turn");
    }
}

/// Whether the user's onboarding steps rule the proposal out.
///
/// A coach who does not train has no agent of their own to pick: the
/// proposal ranks agents on the reader's own activities, and their group's
/// agent is chosen when the group is created. Fails closed like the other
/// checks in [`build_agent_proposal_message`] — a read error must not offer an
/// athlete's agent to a coach.
fn withheld_for_onboarding_steps(steps: &AppResult<Vec<OnboardingStepRecord>>) -> bool {
    steps
        .as_ref()
        .map_or(true, |steps| intake::athlete_steps_waived(steps))
}

/// Decide whether to auto-send and, if so, build the outbound proposal message.
///
/// Returns `None` when the proposal was already sent (or the idempotency read
/// errored — fail closed), when the user already has an active agent (they are
/// past onboarding), when they coach others and do not train, when the build
/// fails, or when no agents are eligible yet (cold start). In the cold-start
/// case the link is intentionally left un-stamped so a later turn can propose
/// once activities sync.
async fn build_agent_proposal_message(
    dispatch: &PendingDispatch,
) -> Option<(OutgoingMessage, Vec<String>)> {
    let db: &dyn MessagingRepository = dispatch.resources.common.repos.messaging.as_ref();

    let already_sent = db
        .agent_proposal_sent(
            dispatch.channel_tenant_id,
            &dispatch.channel,
            &dispatch.sender_id,
        )
        .await
        .unwrap_or(true); // fail closed: never risk double-sending on a read error
    if already_sent {
        return None;
    }

    // Never onboard a user who already has an active agent. The idempotency
    // flag alone is insufficient: a user can acquire an agent on the web before
    // ever receiving a messaging proposal, leaving the flag NULL — and the
    // "Welcome!" lead-in is jarring for someone mid-plan. Re-checked each turn
    // (cheap, indexed); left un-stamped so a transient read error can't
    // permanently suppress a genuinely new user's proposal.
    let has_active_agent = dispatch
        .resources
        .common
        .repos
        .agents
        .get_active_agent(dispatch.auth_result.user_id, dispatch.user_tenant_id)
        .await
        .map_or(true, |agent| agent.is_some()); // fail closed: never onboard a possibly-coached user
    if has_active_agent {
        return None;
    }

    // Left un-stamped, so a later turn re-reads the steps.
    let steps = dispatch
        .resources
        .common
        .repos
        .user_onboarding
        .get_onboarding_steps(&dispatch.auth_result.user_id.to_string())
        .await;
    if withheld_for_onboarding_steps(&steps) {
        return None;
    }

    let (profile, agents) = build_agent_proposal(
        &dispatch.resources,
        dispatch.auth_result.user_id,
        dispatch.user_tenant_id,
        &dispatch.locale,
    )
    .await
    .inspect_err(|e| warn!(error = %e, "coach proposal: build failed; skipping"))
    .ok()?;

    if agents.is_empty() {
        return None;
    }

    let body = render_agent_proposal_text(
        &profile,
        &agents,
        &dispatch.resources.mcp.messaging_strings_registry,
        &dispatch.locale,
    );
    // Captured in the SAME order the user reads, because that ordering is what a
    // numeric reply indexes into.
    let offered_ids: Vec<String> = agents.iter().map(|c| c.agent.id.clone()).collect();
    Some((
        OutgoingMessage {
            channel_type: dispatch.channel_type,
            recipient_id: dispatch.sender_id.clone(),
            content: MessageContent::Text { body },
            // A fresh turn id: the proposal is a proactive message, not a reply to
            // the user's inbound turn.
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: dispatch.thread_id.clone(),
        },
        offered_ids,
    ))
}

/// Render the onboarding agent proposal as a channel text message: a short
/// profile-aware lead-in, then a numbered list of `title — reason` lines.
///
/// The lead-in and footer are resolved from the messaging-strings `registry`
/// for `locale`; the numbered list is locale-neutral formatting and the
/// per-agent reasons arrive already localized from [`build_agent_proposal`].
fn render_agent_proposal_text(
    profile: &SportProfileSummary,
    agents: &[ProposedAgent],
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> String {
    let count = agents.len().to_string();
    let mut body = profile
        .primary_sport
        .as_deref()
        .filter(|_| profile.has_profile)
        .map_or_else(
            || registry.render(KEY_AGENT_PROPOSAL_WELCOME_GENERIC, locale, &[&count]),
            |primary| {
                // The wire sport names itself in the athlete's locale when the
                // shared vocabulary knows it; an unknown spelling keeps its
                // wire text rather than inventing one.
                let sport = sport_label(registry, primary, locale);
                registry.render(KEY_AGENT_PROPOSAL_WELCOME, locale, &[&sport, &count])
            },
        );
    for (index, proposed) in agents.iter().enumerate() {
        let reason = if proposed.reason.is_empty() {
            String::new()
        } else {
            format!(" — {}", proposed.reason)
        };
        let _ = writeln!(
            body,
            "{number}. {title}{reason}",
            number = index + 1,
            title = proposed.agent.title,
        );
    }
    body.push_str(&registry.get(KEY_AGENT_PROPOSAL_FOOTER, locale));
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::errors::AppError;

    fn step(step_id: &str, status: &str) -> OnboardingStepRecord {
        OnboardingStepRecord {
            step_id: step_id.to_owned(),
            status: status.to_owned(),
            chosen_channel: None,
        }
    }

    #[test]
    fn a_coach_who_does_not_train_is_not_offered_an_agent() {
        let steps = Ok(vec![
            step(intake::STEP_PROFILE_TYPE, intake::STATUS_COMPLETE),
            step(intake::STEP_PARQ, intake::STATUS_NOT_APPLICABLE),
        ]);
        assert!(withheld_for_onboarding_steps(&steps));
    }

    #[test]
    fn an_athlete_is_still_offered_one() {
        assert!(!withheld_for_onboarding_steps(&Ok(vec![])));
        let skipped_parq = Ok(vec![step(intake::STEP_PARQ, intake::STATUS_SKIPPED)]);
        assert!(!withheld_for_onboarding_steps(&skipped_parq));
    }

    #[test]
    fn an_unreadable_record_withholds_the_proposal() {
        let steps = Err(AppError::internal("read failed"));
        assert!(withheld_for_onboarding_steps(&steps));
    }
}
