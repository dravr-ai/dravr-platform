// ABOUTME: The one identity rule a roster, a peer fetch, a coach-scope refusal and an athlete's thread render a member by
// ABOUTME: users.display_name, else the email local part, else "Unknown" — plus the zone their days are counted in
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Member identity, resolved once and the same way everywhere.
//!
//! `GroupMember.display_name` is populated by the membership query with the
//! member's display name, else their full e-mail address, so matching or
//! printing it would leak an address into a tool error — and from there into
//! a room. Every path that names a member resolves the name here instead: the
//! group roster the agent reads, the peer-activity fetch that matches a roster
//! name, the plan tools' `athlete=` resolution, and the coach's thread bound
//! to one athlete. The naming rule itself is
//! [`roster_display_name`], which the conversation list applies to a joined
//! row as well.
//!
//! The timezone rides along because it comes off the same row. A roster card
//! that names a weekday needs the zone the member actually trained in, and
//! fetching it separately would be a second round trip for a column already in
//! hand.

use pierre_core::models::roster_display_name;
use pierre_database::RepositoryRegistry;
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
/// The name is [`roster_display_name`] of the user's row, or `"Unknown"` when
/// the user cannot be fetched. The timezone is `None` whenever the row is
/// missing, unreadable, or carries no zone.
pub async fn fetch_athlete_identity(repos: &RepositoryRegistry, user_id: Uuid) -> AthleteIdentity {
    match repos.users.get_global(user_id).await {
        Ok(Some(user)) => AthleteIdentity {
            display_name: roster_display_name(user.display_name.as_deref(), &user.email),
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
pub async fn fetch_user_display_name(repos: &RepositoryRegistry, user_id: Uuid) -> String {
    fetch_athlete_identity(repos, user_id).await.display_name
}
