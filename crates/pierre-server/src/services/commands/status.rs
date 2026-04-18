// ABOUTME: Handlers for /status and /me commands showing account overview
// ABOUTME: Returns connected providers, active coach, and group membership
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;

use crate::contremaitre::messaging_strings::{
    KEY_STATUS_CHANNEL_LABEL, KEY_STATUS_GROUPS_LABEL, KEY_STATUS_HEADER,
    KEY_STATUS_PROVIDERS_LABEL, KEY_STATUS_PROVIDERS_NONE,
};

use super::{CommandHandler, PlatformCommandContext};

/// Handler for `/status` — account summary
pub struct StatusHandler;

#[async_trait]
impl CommandHandler for StatusHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = &ctx.resources.messaging_strings_registry;
        let locale = ctx.locale.as_str();
        let mut text = String::with_capacity(256);
        text.push_str(&reg.render(KEY_STATUS_HEADER, locale, &[]));

        // Connected providers
        let connections = ctx
            .resources
            .repos
            .provider_connections
            .get_for_user(ctx.user_id, None)
            .await
            .unwrap_or_default();

        if connections.is_empty() {
            text.push_str(&reg.render(KEY_STATUS_PROVIDERS_NONE, locale, &[]));
        } else {
            let names: Vec<&str> = connections.iter().map(|c| c.provider.as_str()).collect();
            let joined = names.join(", ");
            text.push_str(&reg.render(KEY_STATUS_PROVIDERS_LABEL, locale, &[&joined]));
        }

        // Groups
        #[cfg(feature = "tools-groups")]
        {
            let groups = ctx
                .resources
                .repos
                .groups
                .list_groups_for_user(ctx.user_id)
                .await
                .unwrap_or_default();
            let count = groups.len().to_string();
            text.push_str(&reg.render(KEY_STATUS_GROUPS_LABEL, locale, &[&count]));
        }

        text.push_str(&reg.render(KEY_STATUS_CHANNEL_LABEL, locale, &[&ctx.channel_type]));

        Ok(CommandResponse::text(text))
    }
}
