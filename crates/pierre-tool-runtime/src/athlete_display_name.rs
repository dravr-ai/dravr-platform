// ABOUTME: The one identity rule a roster, a peer fetch and a coach-scope refusal render a member by
// ABOUTME: users.display_name, else the email local part, else "Unknown" — plus the zone their days are counted in
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Member identity, resolved once and the same way everywhere.
//!
//! `GroupMember.display_name` is populated by the membership query with the
//! member's full e-mail address, so matching or printing it would leak an
//! address into a tool error — and from there into a room. Every path that
//! names a member resolves the name here instead: the group roster the coach
//! reads, the peer-activity fetch that matches a roster name, and the plan
//! tools' `athlete=` resolution.
//!
//! The timezone rides along because it comes off the same row. A roster card
//! that names a weekday needs the zone the member actually trained in, and
//! fetching it separately would be a second round trip for a column already in
//! hand.

use pierre_runtime_context::DataContext;
use tracing::info;
use uuid::Uuid;

/// How a member is named and which civil clock their days are counted on.
#[derive(Debug, Clone)]
pub struct AthleteIdentity {
    /// Display name, resolved by the rule in [`fetch_athlete_identity`].
    pub display_name: String,
    /// IANA timezone from the member's own user row, `None` when unset or the
    /// row could not be read. Callers resolve it through
    /// [`pierre_core::civil_time::resolve_zone`], which falls back to UTC.
    pub timezone: Option<String>,
}

/// Fetch a user's display name and timezone from the global user database.
///
/// The name is the user's `display_name` if set, the e-mail local part if not,
/// or `"Unknown"` when the user cannot be fetched. The timezone is `None`
/// whenever the row is missing, unreadable, or carries no zone.
pub async fn fetch_athlete_identity(data: &DataContext, user_id: Uuid) -> AthleteIdentity {
    match data.repos().users.get_global(user_id).await {
        Ok(Some(user)) => AthleteIdentity {
            display_name: user
                .display_name
                .unwrap_or_else(|| user.email.split('@').next().unwrap_or("Unknown").to_owned()),
            timezone: user.timezone,
        },
        Ok(None) => {
            info!(
                user_id = %user_id,
                "Snapshot: user record not found, display_name falls back to 'Unknown'"
            );
            AthleteIdentity {
                display_name: "Unknown".to_owned(),
                timezone: None,
            }
        }
        Err(e) => {
            info!(user_id = %user_id, error = %e, "Snapshot: failed to fetch user; display_name falls back to 'Unknown'");
            AthleteIdentity {
                display_name: "Unknown".to_owned(),
                timezone: None,
            }
        }
    }
}

/// Fetch just the display name, for the paths that name a member and never
/// print one of their dates.
pub async fn fetch_user_display_name(data: &DataContext, user_id: Uuid) -> String {
    fetch_athlete_identity(data, user_id).await.display_name
}
