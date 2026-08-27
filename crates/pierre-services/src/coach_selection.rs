// ABOUTME: The one place a coach is bound to a conversation and coach.selected is emitted
// ABOUTME: Shared by the REST usage endpoint, web chat, /coach add, and messaging ingress

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach-selection recording.
//!
//! Four surfaces bind a coach to a conversation — `POST
//! /api/coaches/{id}/usage` (what the web Coaches UI and onboarding
//! proposal call when the athlete picks one), web chat conversation
//! creation, the `/coach add` slash command, and messaging session
//! creation. All four bump the same `coach_assignments.use_count`, so all
//! four are the same product event.
//!
//! Only the REST route used to emit `coach.selected`, which made the metric
//! read as "nobody picks coaches" while every chat user picked one — the
//! event belongs to the domain operation, not to whichever transport
//! happened to trigger it.
//!
//! The surfaces are not equally meaningful, though, which is why every
//! emission carries a [`CoachSelectionSource`]: a REST bump and a
//! `/coach add` are an athlete actively choosing, while a conversation
//! create re-reports the choice they already made. Counting them together
//! answers "how much is coaching used", counting
//! [`CoachSelectionSource::Rest`] and [`CoachSelectionSource::SlashCommand`]
//! alone answers "how many people choose a coach" — the question that
//! motivated moving this off the REST route in the first place.

use std::fmt;

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::repositories::CoachesRepository;
use tracing::{info, warn};
use uuid::Uuid;

/// Which surface bound the coach, carried on `coach.selected` as `source`.
///
/// An additive field — the catalogue's `required_fields` for the event are
/// `user_id`, `tenant_id` and `coach_slug`, so this narrows the metric
/// without changing its contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoachSelectionSource {
    /// `POST /api/coaches/{id}/usage` — the web Coaches UI and the
    /// onboarding proposal. An explicit pick.
    Rest,
    /// The `/coach add` (or `/coach assign`) slash command on any chat
    /// surface. An explicit pick.
    SlashCommand,
    /// Web chat conversation creation, binding the already-selected coach.
    ChatConversation,
    /// Messaging session creation, binding the already-selected coach. Fires
    /// again whenever a session rolls over, so it counts conversations rather
    /// than choices.
    MessagingSession,
}

impl CoachSelectionSource {
    /// Stable wire value for the event field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::SlashCommand => "slash_command",
            Self::ChatConversation => "chat_conversation",
            Self::MessagingSession => "messaging_session",
        }
    }
}

impl fmt::Display for CoachSelectionSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Record that `coach_id` was selected for a conversation and emit the
/// catalogued `coach.selected` event.
///
/// Returns whether the usage bump landed. `Ok(false)` means the coach is not
/// visible to this tenant (a caller passing an id they cannot see); no event
/// is emitted in that case, because nothing was selected.
///
/// `user_id` and `tenant_id` ride on the event inline rather than being left
/// to the enclosing span: the messaging ingress span carries neither, so a
/// span-only event would be dropped by the `PostHog` sink, which keys
/// `distinct_id` off `user_id`.
///
/// `source` records which surface bound the coach, so an explicit pick can be
/// told apart from a conversation re-reporting one.
///
/// # Errors
///
/// Returns the database error if the usage bump fails.
pub async fn record_coach_selection(
    coaches: &dyn CoachesRepository,
    coach_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    source: CoachSelectionSource,
) -> AppResult<bool> {
    let recorded = coaches.record_usage(coach_id, user_id, tenant_id).await?;
    if !recorded {
        warn!(
            coach_id,
            %tenant_id,
            source = source.as_str(),
            "skipping coach usage bump — coach not visible to caller's tenant"
        );
        return Ok(false);
    }

    // `coach_slug` is the catalogue's field name for the coach identifier the
    // routing rule keys on — the same value every surface passes as
    // `coach_id`.
    info!(
        target: "notify",
        event = "coach.selected",
        user_id = %user_id,
        tenant_id = %tenant_id,
        coach_slug = %coach_id,
        source = source.as_str(),
        "user selected coach"
    );
    Ok(true)
}
