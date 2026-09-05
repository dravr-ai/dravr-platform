// ABOUTME: The one-time onboarding coach proposal a messaging turn sends ahead of its first coached reply
// ABOUTME: Builds the inferred-profile proposal, renders it as channel text, sends it, stamps the link once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write as _;

use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_COACH_PROPOSAL_FOOTER, KEY_COACH_PROPOSAL_WELCOME,
    KEY_COACH_PROPOSAL_WELCOME_GENERIC,
};
use pierre_core::models::messaging::{ChannelConfig, MessageContent, OutgoingMessage};
use pierre_database::backends::MessagingRepository;
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use pierre_routes_coaches::coaches::{build_coach_proposal, ProposedCoach, SportProfileSummary};
use pierre_services::activity_sports::sport_label;
use pierre_services::analytics::hash_id;
use tracing::{info, warn};

use super::PendingDispatch;

/// Auto-send the one-time onboarding coach proposal for this user, if it hasn't
/// been sent on this channel link yet.
///
/// Fires on the user's first provider-connected messaging turn: builds the
/// inferred-profile proposal (shared with the REST route via
/// [`build_coach_proposal`]), renders it to text, sends it through the channel
/// adapter, and stamps the link so it never re-sends. Entirely best-effort —
/// any failure is logged and the turn proceeds normally. When no coaches are
/// eligible yet (cold start, activities not synced) it returns *without*
/// stamping, so a later turn can propose once data lands.
pub(super) async fn maybe_send_coach_proposal(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
) {
    let Some((outgoing, offered_ids)) = build_coach_proposal_message(dispatch).await else {
        return;
    };

    if let Err(e) = dispatch.adapter.send(&outgoing, channel_config).await {
        warn!(error = %e, "coach proposal: send failed; will retry next turn");
        return;
    }

    stamp_coach_proposal_sent(dispatch, &offered_ids).await;

    info!(
        hashed_user = %hash_id(&dispatch.session.user_id),
        "coach proposal auto-sent"
    );
}

/// Stamp the channel link so the proposal is never re-sent. Best-effort: a
/// failure here only risks a duplicate proposal on a later turn, never the turn.
async fn stamp_coach_proposal_sent(dispatch: &PendingDispatch, offered_ids: &[String]) {
    let db: &dyn MessagingRepository = dispatch.resources.common.repos.messaging.as_ref();
    if let Err(e) = db
        .mark_coach_proposal_sent(
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

/// Decide whether to auto-send and, if so, build the outbound proposal message.
///
/// Returns `None` when the proposal was already sent (or the idempotency read
/// errored — fail closed), when the user already has an active coach (they are
/// past onboarding), when the build fails, or when no coaches are eligible yet
/// (cold start). In the cold-start case the link is intentionally left
/// un-stamped so a later turn can propose once activities sync.
async fn build_coach_proposal_message(
    dispatch: &PendingDispatch,
) -> Option<(OutgoingMessage, Vec<String>)> {
    let db: &dyn MessagingRepository = dispatch.resources.common.repos.messaging.as_ref();

    let already_sent = db
        .coach_proposal_sent(
            dispatch.channel_tenant_id,
            &dispatch.channel,
            &dispatch.sender_id,
        )
        .await
        .unwrap_or(true); // fail closed: never risk double-sending on a read error
    if already_sent {
        return None;
    }

    // Never onboard a user who already has an active coach. The idempotency
    // flag alone is insufficient: a user can acquire a coach on the web before
    // ever receiving a messaging proposal, leaving the flag NULL — and the
    // "Welcome!" lead-in is jarring for someone mid-plan. Re-checked each turn
    // (cheap, indexed); left un-stamped so a transient read error can't
    // permanently suppress a genuinely new user's proposal.
    let has_active_coach = dispatch
        .resources
        .common
        .repos
        .coaches
        .get_active_coach(dispatch.auth_result.user_id, dispatch.user_tenant_id)
        .await
        .map_or(true, |coach| coach.is_some()); // fail closed: never onboard a possibly-coached user
    if has_active_coach {
        return None;
    }

    let (profile, coaches) = build_coach_proposal(
        &dispatch.resources,
        dispatch.auth_result.user_id,
        dispatch.user_tenant_id,
        &dispatch.locale,
    )
    .await
    .inspect_err(|e| warn!(error = %e, "coach proposal: build failed; skipping"))
    .ok()?;

    if coaches.is_empty() {
        return None;
    }

    let body = render_coach_proposal_text(
        &profile,
        &coaches,
        &dispatch.resources.mcp.messaging_strings_registry,
        &dispatch.locale,
    );
    // Captured in the SAME order the user reads, because that ordering is what a
    // numeric reply indexes into.
    let offered_ids: Vec<String> = coaches.iter().map(|c| c.coach.id.clone()).collect();
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

/// Render the onboarding coach proposal as a channel text message: a short
/// profile-aware lead-in, then a numbered list of `title — reason` lines.
///
/// The lead-in and footer are resolved from the messaging-strings `registry`
/// for `locale`; the numbered list is locale-neutral formatting and the
/// per-coach reasons arrive already localized from [`build_coach_proposal`].
fn render_coach_proposal_text(
    profile: &SportProfileSummary,
    coaches: &[ProposedCoach],
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> String {
    let count = coaches.len().to_string();
    let mut body = profile
        .primary_sport
        .as_deref()
        .filter(|_| profile.has_profile)
        .map_or_else(
            || registry.render(KEY_COACH_PROPOSAL_WELCOME_GENERIC, locale, &[&count]),
            |primary| {
                // The wire sport names itself in the athlete's locale when the
                // shared vocabulary knows it; an unknown spelling keeps its
                // wire text rather than inventing one.
                let sport = sport_label(registry, primary, locale);
                registry.render(KEY_COACH_PROPOSAL_WELCOME, locale, &[&sport, &count])
            },
        );
    for (index, proposed) in coaches.iter().enumerate() {
        let reason = if proposed.reason.is_empty() {
            String::new()
        } else {
            format!(" — {}", proposed.reason)
        };
        let _ = writeln!(
            body,
            "{number}. {title}{reason}",
            number = index + 1,
            title = proposed.coach.title,
        );
    }
    body.push_str(&registry.get(KEY_COACH_PROPOSAL_FOOTER, locale));
    body
}
