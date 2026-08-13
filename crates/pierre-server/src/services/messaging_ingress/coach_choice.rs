// ABOUTME: Resolves a bare numeric reply to the coach the proposal offered at that position
// ABOUTME: Implements "Reply with a number to start" — the instruction the proposal has always given

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Numeric coach selection.
//!
//! The onboarding coach proposal ends with "Reply with a number to start", and
//! nothing parsed that number. Typing `1` reached the model as ordinary
//! conversation and bound nothing — the first proactive message we ever send
//! taught the user that the bot does not do what it says.
//!
//! Resolution indexes the ids stamped alongside the proposal rather than
//! rebuilding it: the proposal is LLM-re-ranked, so a rebuild can come back in a
//! different order and bind a coach the user did not pick.

use crate::services::outgoing::proactive_text;
use pierre_contremaitre::messaging_strings::KEY_COACH_USER_UPDATED;
use pierre_core::models::messaging::{ChannelType, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::repositories::MessagingRepository;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// Parse a reply that is *only* a number.
///
/// `pub` so the integration suite can pin the strictness directly — the
/// distinction between "2" and "I run 3 times a week" is the whole safety
/// property here, and it deserves assertions of its own.
///
/// Deliberately strict: "2" selects, "2 please" and "I'll take 2" do not. A
/// loose parse would hijack ordinary conversation — someone answering "I run 3
/// times a week" must not silently rebind their coach.
#[must_use]
pub fn parse_choice(text: &str) -> Option<usize> {
    let trimmed = text.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    trimmed.parse::<usize>().ok().filter(|n| *n > 0)
}

/// Handle a numeric reply to the coach proposal, if that is what this is.
///
/// Returns `None` when the message is not a bare number, when no proposal is
/// outstanding, or when the number is out of range — in every case the turn
/// continues to the model untouched, which is exactly the previous behaviour.
/// Everything [`try_handle_coach_choice`] needs to resolve a numeric reply.
///
/// A struct rather than eight positional parameters: the four `&str`-ish fields
/// are trivially swappable at a call site, and binding the wrong coach to the
/// wrong sender is the failure this whole path exists to avoid.
pub(super) struct CoachChoiceParams<'a> {
    pub resources: &'a ServerContext,
    pub tenant_id: TenantId,
    pub channel: &'a str,
    pub channel_type: ChannelType,
    pub sender_id: &'a str,
    pub user_id: Uuid,
    pub locale: &'a str,
    pub text: &'a str,
}

pub(super) async fn try_handle_coach_choice(
    params: CoachChoiceParams<'_>,
) -> Option<OutgoingMessage> {
    let CoachChoiceParams {
        resources,
        tenant_id,
        channel,
        channel_type,
        sender_id,
        user_id,
        locale,
        text,
    } = params;
    let choice = parse_choice(text)?;

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let offered = db
        .proposed_coach_ids(tenant_id, channel, sender_id)
        .await
        .unwrap_or_default();
    if offered.is_empty() {
        return None;
    }

    // Out of range is not a selection. Falling through to the model beats
    // guessing, and beats an error for someone who just happened to type "9".
    let coach_id = offered.get(choice - 1)?;

    let coach = resources
        .common
        .repos
        .coaches
        .get_by_id(coach_id, user_id, tenant_id)
        .await
        .ok()
        .flatten()?;

    // The one selection pointer — same write `/coach select` performs, so a
    // user ends up with a coach exactly one way regardless of how they said so.
    if let Err(e) = resources
        .common
        .repos
        .tenants
        .set_selected_coach(tenant_id, user_id, Some(coach_id))
        .await
    {
        warn!(error = %e, coach_id = %coach_id, "failed to bind coach from numeric reply");
        return None;
    }

    info!(
        coach_id = %coach_id,
        choice,
        "coach bound from a numeric reply to the proposal"
    );

    let body = resources.mcp.messaging_strings_registry.render(
        KEY_COACH_USER_UPDATED,
        locale,
        &[&coach.title],
    );

    Some(proactive_text(channel_type, sender_id.to_owned(), body))
}
