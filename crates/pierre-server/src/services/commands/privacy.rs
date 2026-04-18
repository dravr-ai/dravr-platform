// ABOUTME: Handlers for /privacy slash commands managing analytics consent
// ABOUTME: Allows messaging-only users to view, enable, or disable anonymous analytics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;
use tracing::info;

use crate::contremaitre::messaging_strings::{
    KEY_PRIVACY_OFF_CONFIRMATION, KEY_PRIVACY_ON_CONFIRMATION, KEY_PRIVACY_STATUS_DISABLED,
    KEY_PRIVACY_STATUS_ENABLED, KEY_PRIVACY_STATUS_LINE,
};
use crate::services::analytics::{analytics, hash_id};

use super::{CommandHandler, PlatformCommandContext};

/// Handler for `/privacy` — display current analytics consent status
pub struct PrivacyStatusHandler;

#[async_trait]
impl CommandHandler for PrivacyStatusHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let user = ctx
            .resources
            .repos
            .users
            .get_global(ctx.user_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("User {}", ctx.user_id)))?;

        let reg = &ctx.resources.messaging_strings_registry;
        let locale = ctx.locale.as_str();
        let status_word_key = if user.analytics_consent {
            KEY_PRIVACY_STATUS_ENABLED
        } else {
            KEY_PRIVACY_STATUS_DISABLED
        };
        let status_word = reg.render(status_word_key, locale, &[]);
        let text = reg.render(KEY_PRIVACY_STATUS_LINE, locale, &[&status_word]);

        Ok(CommandResponse::text(text))
    }
}

/// Handler for `/privacy on` — enable analytics consent
pub struct PrivacyOnHandler;

#[async_trait]
impl CommandHandler for PrivacyOnHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        ctx.resources
            .repos
            .users
            .update_analytics_consent(ctx.user_id, true)
            .await?;

        let hashed_user = hash_id(&ctx.user_id.to_string());
        analytics().set_consent(&hashed_user, true);

        info!(user_id = %ctx.user_id, "Analytics consent enabled via /privacy on");

        let text = ctx.resources.messaging_strings_registry.render(
            KEY_PRIVACY_ON_CONFIRMATION,
            ctx.locale.as_str(),
            &[],
        );

        Ok(CommandResponse::text(text))
    }
}

/// Handler for `/privacy off` — disable analytics consent
pub struct PrivacyOffHandler;

#[async_trait]
impl CommandHandler for PrivacyOffHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        ctx.resources
            .repos
            .users
            .update_analytics_consent(ctx.user_id, false)
            .await?;

        let hashed_user = hash_id(&ctx.user_id.to_string());
        analytics().set_consent(&hashed_user, false);

        info!(user_id = %ctx.user_id, "Analytics consent disabled via /privacy off");

        let text = ctx.resources.messaging_strings_registry.render(
            KEY_PRIVACY_OFF_CONFIRMATION,
            ctx.locale.as_str(),
            &[],
        );

        Ok(CommandResponse::text(text))
    }
}
