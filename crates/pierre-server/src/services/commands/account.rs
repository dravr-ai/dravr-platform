// ABOUTME: Handler for /logout command unlinking the messaging account
// ABOUTME: Reuses existing logout logic from webhooks with confirmation flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;

use crate::contremaitre::messaging_strings::KEY_LOGOUT_CONFIRM_PROMPT;

use super::{CommandHandler, PlatformCommandContext};

/// Handler for `/logout` — unlink messaging account
///
/// Returns a confirmation prompt. The actual unlinking is handled
/// by the existing `handle_logout` function in webhooks.rs when
/// the user replies "YES" or types "logout".
pub struct LogoutHandler;

#[async_trait]
impl CommandHandler for LogoutHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let text = ctx.resources.messaging_strings_registry.render(
            KEY_LOGOUT_CONFIRM_PROMPT,
            ctx.locale.as_str(),
            &[&ctx.channel_type],
        );

        Ok(CommandResponse::with_confirmation(text))
    }
}
