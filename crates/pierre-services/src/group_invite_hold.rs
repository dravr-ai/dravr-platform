// ABOUTME: Holding a group invite for an account until it can join — taken at sign-up, after a sign-in, or from the waiting screen
// ABOUTME: The server keeps the code a /groups/join link carried, so no tab, reload or device change loses it (carnet#1648)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A group invite held for an account.
//!
//! A `/groups/join/<code>` link is often opened by someone with no usable
//! account yet: they sign up and confirm their address from a mail that opens
//! another tab, or sign in and land on a page the app reloads. The code used
//! to live in the opening tab and was lost on the way. Now it reaches the
//! server as soon as an account exists (the sign-up body, the call right
//! after a sign-in, or the waiting screen's code field) and is held here
//! against the account. The account joins the group once it is active, the
//! way `/group join` joins it ([`pierre_groups::GroupService::join_group_from_held_invite`]).
//!
//! Both kinds of group invite are held: a member invite joins the account as
//! an athlete, a coach invite attaches it as the group's human coach, as
//! `/group join` would. A held invite is checked when it is taken and again
//! when it is redeemed, except for a member invite's expiry: an athlete who
//! opened the link in time is not turned away because their address took a
//! day to confirm. A hold an account never got to use lapses after
//! `HOLD_LAPSES_AFTER_DAYS`. Nothing here logs a code.

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::GroupInvite;
use pierre_core::models::{GroupInviteHold, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_groups::{GroupService, SIGNUP_INVITE_MAX_DAYS};
use tracing::info;
use uuid::Uuid;

use crate::signup_codes;

/// Days a held invite waits for its account to become active before it
/// lapses: the longest a coach's signup code stays redeemable.
const HOLD_LAPSES_AFTER_DAYS: i64 = SIGNUP_INVITE_MAX_DAYS;

/// The live group invite `raw` names (active, not expired, under its use
/// limit), its group still open; `None` for a code no one could hold.
async fn live_group_invite(
    repos: &RepositoryRegistry,
    raw: &str,
) -> AppResult<Option<GroupInvite>> {
    let normalized = signup_codes::normalize_code(raw);
    if normalized.is_empty() {
        return Ok(None);
    }
    let Some(invite) = repos
        .groups
        .get_invite_by_code(&normalized)
        .await?
        .filter(|invite| GroupService::check_invite_usable(invite).is_ok())
    else {
        return Ok(None);
    };
    let tenant_id = TenantId::parse_str(&invite.tenant_id)
        .map_err(|e| AppError::internal(format!("Invite tenant id is not a uuid: {e}")))?;
    let group_open = repos
        .groups
        .get_group(&invite.group_id.to_string(), tenant_id)
        .await?
        .is_some_and(|group| group.is_active);
    Ok(group_open.then_some(invite))
}

/// Whether `raw` names a live group invite that does not approve a sign-up:
/// the waiting screen's code field holds such a code, and redeems every other
/// code as a signup code.
///
/// # Errors
///
/// Returns the repository's error when a lookup fails.
pub async fn names_plain_group_invite(repos: &RepositoryRegistry, raw: &str) -> AppResult<bool> {
    Ok(live_group_invite(repos, raw)
        .await?
        .is_some_and(|invite| !invite.approves_signup))
}

/// Hold the group invite `raw` names for `user_id`, replacing the one it
/// held. Returns the invite.
///
/// # Errors
///
/// Returns the uniform invalid-code error for a code no one could hold
/// (unknown, deactivated, expired, used up, or its group archived), and the
/// repository's error when a write fails.
pub async fn hold(repos: &RepositoryRegistry, user_id: Uuid, raw: &str) -> AppResult<GroupInvite> {
    let invite = live_group_invite(repos, raw)
        .await?
        .ok_or_else(signup_codes::invalid_signup_code)?;
    repos
        .signup_codes
        .hold_group_invite(user_id, invite.id, Utc::now())
        .await?;
    info!(user_id = %user_id, group_id = %invite.group_id, invite_id = %invite.id, kind = ?invite.kind, "Group invite held until the account joins");
    Ok(invite)
}

/// Whether `hold` was taken longer ago than a hold waits.
#[must_use]
pub fn lapsed(hold: &GroupInviteHold, now: DateTime<Utc>) -> bool {
    now - hold.held_at > Duration::days(HOLD_LAPSES_AFTER_DAYS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_lapses_after_its_window() {
        let now = Utc::now();
        let hold = |days| GroupInviteHold {
            user_id: Uuid::new_v4(),
            invite_id: Uuid::new_v4(),
            held_at: now - Duration::days(days),
        };
        assert!(
            !lapsed(&hold(HOLD_LAPSES_AFTER_DAYS), now),
            "bug: a hold exactly as old as its window lapsed"
        );
        assert!(
            lapsed(&hold(HOLD_LAPSES_AFTER_DAYS + 1), now),
            "bug: a hold a day past its window was kept"
        );
    }
}
