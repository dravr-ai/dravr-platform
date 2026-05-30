// ABOUTME: Handler for /timezone setting the user's IANA timezone over messaging
// ABOUTME: Lets Telegram/Slack users populate users.timezone (web/mobile do it via /api/users/me/timezone)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;
use tracing::info;

use pierre_contremaitre::messaging_strings::{KEY_TIMEZONE_INVALID, KEY_TIMEZONE_SET};

use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/timezone <IANA>` — persist the user's IANA timezone.
///
/// Messaging surfaces have no `/api/users/me/timezone` equivalent, so a
/// Telegram-only user otherwise has a `NULL` `users.timezone` and the chat
/// prompt / activity start times render in UTC. This command lets them set it
/// once. The argument is validated as a known IANA name before any write so
/// junk never reaches the database the prompt resolver reads back.
pub struct TimezoneHandler;

#[async_trait]
impl CommandHandler for TimezoneHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let tz = ctx.args.first().map_or("", String::as_str).trim();
        if tz.is_empty() || tz.parse::<chrono_tz::Tz>().is_err() {
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_TIMEZONE_INVALID,
                locale,
                &[],
            )));
        }

        ctx.ctx.repos().users.set_timezone(ctx.user_id, tz).await?;

        info!(user_id = %ctx.user_id, timezone = %tz, "User timezone set via /timezone");

        Ok(CommandResponse::rich_text(reg.render(
            KEY_TIMEZONE_SET,
            locale,
            &[tz],
        )))
    }
}
