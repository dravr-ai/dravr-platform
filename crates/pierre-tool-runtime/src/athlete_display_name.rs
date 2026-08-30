// ABOUTME: The one display-name rule a roster, a peer fetch and a coach-scope refusal render a member by
// ABOUTME: users.display_name, else the email local part, else "Unknown" — so a room reads the name it already knows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Member display names, resolved once and the same way everywhere.
//!
//! `GroupMember.display_name` is populated by the membership query with the
//! member's full e-mail address, so matching or printing it would leak an
//! address into a tool error — and from there into a room. Every path that
//! names a member resolves the name here instead: the group roster the coach
//! reads, the peer-activity fetch that matches a roster name, and the plan
//! tools' `athlete=` resolution.

use pierre_runtime_context::DataContext;
use tracing::info;
use uuid::Uuid;

/// Fetch display name for a user from the global user database.
///
/// Returns the user's display name if set, email prefix if not, or "Unknown"
/// if the user cannot be fetched.
pub async fn fetch_user_display_name(data: &DataContext, user_id: Uuid) -> String {
    match data.repos().users.get_global(user_id).await {
        Ok(Some(user)) => user
            .display_name
            .unwrap_or_else(|| user.email.split('@').next().unwrap_or("Unknown").to_owned()),
        Ok(None) => {
            info!(
                user_id = %user_id,
                "Snapshot: user record not found, display_name falls back to 'Unknown'"
            );
            "Unknown".to_owned()
        }
        Err(e) => {
            info!(user_id = %user_id, error = %e, "Snapshot: failed to fetch user; display_name falls back to 'Unknown'");
            "Unknown".to_owned()
        }
    }
}
