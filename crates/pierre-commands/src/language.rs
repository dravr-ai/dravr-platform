// ABOUTME: Handler for /language pinning the language a messaging channel reads in
// ABOUTME: Writes the channel link's locale override, which every channel reply resolves first
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::SUPPORTED_LOCALES;
use pierre_messaging::commands::CommandResponse;
use tracing::info;

use pierre_contremaitre::messaging_strings::{
    KEY_LANGUAGE_INVALID, KEY_LANGUAGE_NO_CHANNEL, KEY_LANGUAGE_SET,
};

use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/language <code>` — pin the language this channel reads in.
///
/// The override lives on the athlete's channel link
/// (`messaging_channel_links.locale`), the first step of the chain every
/// channel reply resolves its locale through, so Telegram can read English
/// while the app stays French. The code is checked against the supported
/// locales before anything is written. A conversation with no channel link
/// (the web and mobile apps) has nothing to pin: the app's own language is
/// set in Settings, and the reply says so.
pub struct LanguageHandler;

#[async_trait]
impl CommandHandler for LanguageHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();

        let requested = ctx
            .args
            .first()
            .map(|code| code.trim().to_lowercase())
            .unwrap_or_default();
        let Some(locale) = SUPPORTED_LOCALES
            .iter()
            .find(|supported| **supported == requested)
        else {
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_LANGUAGE_INVALID,
                ctx.locale.as_str(),
                &[&SUPPORTED_LOCALES.join(", ")],
            )));
        };

        let written = ctx
            .ctx
            .repos()
            .messaging
            .set_channel_link_locale(
                ctx.tenant_id,
                &ctx.user_id.to_string(),
                &ctx.channel_type,
                Some(locale),
            )
            .await;
        match written {
            Ok(()) => {}
            Err(e) if e.code == ErrorCode::ResourceNotFound => {
                return Ok(CommandResponse::rich_text(reg.render(
                    KEY_LANGUAGE_NO_CHANNEL,
                    ctx.locale.as_str(),
                    &[],
                )));
            }
            Err(e) => return Err(e),
        }

        info!(
            user_id = %ctx.user_id,
            channel = %ctx.channel_type,
            locale = %locale,
            "Channel language set via /language"
        );

        // The confirmation reads in the language just chosen.
        Ok(CommandResponse::rich_text(reg.render(
            KEY_LANGUAGE_SET,
            locale,
            &[],
        )))
    }
}
