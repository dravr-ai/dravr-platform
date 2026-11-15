// ABOUTME: The messaging identities an account is linked to, as the admin user read lists them
// ABOUTME: Channel, channel user id, display name and link time, so an admin sees whom a Slack tap acts as

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppResult;
use pierre_database::RepositoryRegistry;
use serde_json::{json, Value};
use uuid::Uuid;

/// Every Slack, Discord or other messaging identity linked to `user_id`,
/// oldest first: which account a tap from that identity acts as, readable
/// without a SQL join (carnet#1576). Only what an admin needs to tell one
/// link from another; the link's row id and tenant stay out.
///
/// # Errors
///
/// Returns the repository's error when the listing fails.
pub(crate) async fn for_user(repos: &RepositoryRegistry, user_id: Uuid) -> AppResult<Vec<Value>> {
    let links = repos
        .messaging
        .list_channel_links_for_user(&user_id.to_string())
        .await?;
    Ok(links
        .iter()
        .map(|link| {
            json!({
                "channel": link["channel_type"],
                "channel_user_id": link["channel_user_id"],
                "display_name": link["display_name"],
                "linked_at": link["linked_at"],
            })
        })
        .collect())
}
