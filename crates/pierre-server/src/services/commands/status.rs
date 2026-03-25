// ABOUTME: Handlers for /status and /me commands showing account overview
// ABOUTME: Returns connected providers, active coach, and group membership
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write;

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;

use super::{CommandHandler, PlatformCommandContext};

/// Handler for `/status` — account summary
pub struct StatusHandler;

#[async_trait]
impl CommandHandler for StatusHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let mut text = String::with_capacity(256);
        text.push_str("Your Pierre Status:\n");

        // Connected providers
        let connections = ctx
            .resources
            .repos
            .provider_connections
            .get_for_user(ctx.user_id, None)
            .await
            .unwrap_or_default();

        if connections.is_empty() {
            text.push_str("\nProviders: None connected");
        } else {
            let names: Vec<&str> = connections.iter().map(|c| c.provider.as_str()).collect();
            let _ = write!(text, "\nProviders: {}", names.join(", "));
        }

        // Groups
        #[cfg(feature = "tools-groups")]
        {
            let groups = ctx
                .resources
                .repos
                .groups
                .list_groups_for_user(ctx.user_id, ctx.tenant_id)
                .await
                .unwrap_or_default();
            let _ = write!(text, "\nGroups: {}", groups.len());
        }

        text.push_str("\nChannel: ");
        text.push_str(&ctx.channel_type);

        Ok(CommandResponse::text(text))
    }
}
