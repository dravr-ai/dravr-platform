// ABOUTME: The dated title a new conversation falls back to when nothing else names it
// ABOUTME: Rendered in the athlete's stored language and timezone, for the web chat, A2A and messaging alike

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_contremaitre::messaging_strings::KEY_NEW_CONVERSATION_TITLE_PREFIX;
use pierre_services::conversation_forge::dated_title;
use pierre_services::locale::user_locale;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// The dated stamp a forged thread falls back to when it has neither a room
/// nor an agent to be named after, in the athlete's stored language and
/// timezone.
pub(crate) async fn fresh_title_fallback(resources: &ServerContext, user_id: &str) -> String {
    let user = match Uuid::parse_str(user_id) {
        Ok(id) => resources
            .common
            .repos
            .users
            .get_global(id)
            .await
            .ok()
            .flatten(),
        Err(_) => None,
    };
    let locale = user_locale(user.as_ref());
    let prefix = resources.mcp.messaging_strings_registry.render(
        KEY_NEW_CONVERSATION_TITLE_PREFIX,
        &locale,
        &[],
    );
    dated_title(
        &prefix,
        &locale,
        user.as_ref().and_then(|u| u.timezone.as_deref()),
        Utc::now(),
    )
}
