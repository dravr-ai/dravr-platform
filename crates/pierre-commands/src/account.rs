// ABOUTME: Handler for /logout command unlinking the messaging account
// ABOUTME: Reuses existing logout logic from webhooks with confirmation flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;
use tracing::{error, info};

use pierre_contremaitre::messaging_strings::KEY_LINK_LOGOUT_COMPLETE;

use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/logout` — unlink the caller's messaging account.
///
/// Unlinks immediately (like the bare-word `logout` path in the messaging
/// ingress) by deleting the channel link for this sender, then confirms.
/// The earlier confirmation-prompt stub never unlinked — there was no
/// `YES`-reply handler — so the user believed they had logged out while the
/// channel stayed linked.
pub struct LogoutHandler;

#[async_trait]
impl CommandHandler for LogoutHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        // Only messaging surfaces carry a channel sender to unlink; web/mobile
        // sessions log out through the app, not a slash command.
        if let Some(sender_id) = ctx.sender_id.as_deref() {
            if let Err(e) = ctx
                .ctx
                .repos()
                .messaging
                .logout_channel_sender(ctx.tenant_id, &ctx.channel_type, sender_id)
                .await
            {
                error!(error = %e, "Failed to unlink channel sender on /logout");
                return Err(AppError::internal("Failed to unlink messaging account"));
            }
            info!(
                channel = %ctx.channel_type,
                "User unlinked messaging channel via /logout"
            );
        }

        let text = ctx.ctx.messaging_strings_registry().render(
            KEY_LINK_LOGOUT_COMPLETE,
            ctx.locale.as_str(),
            &[],
        );
        Ok(CommandResponse::text(text))
    }
}
