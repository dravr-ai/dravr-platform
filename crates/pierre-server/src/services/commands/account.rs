// ABOUTME: Handler for /logout command unlinking the messaging account
// ABOUTME: Reuses existing logout logic from webhooks with confirmation flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;

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
        let text = format!(
            "This will unlink your {} account from Pierre.\n\
             You will need to re-link to use messaging again.\n\n\
             Type \"logout\" to confirm.",
            ctx.channel_type
        );

        Ok(CommandResponse::with_confirmation(text))
    }
}
