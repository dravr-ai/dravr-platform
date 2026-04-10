// ABOUTME: Handlers for /privacy slash commands managing analytics consent
// ABOUTME: Allows messaging-only users to view, enable, or disable anonymous analytics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;
use tracing::info;

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

        let status = if user.analytics_consent {
            "enabled"
        } else {
            "disabled"
        };

        let text = format!(
            "Analytics consent is currently <b>{status}</b>.\n\n\
             Use <code>/privacy on</code> to enable or <code>/privacy off</code> to disable anonymous analytics."
        );

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

        let text = "Analytics consent has been <b>enabled</b>. \
                    Thank you for helping us improve Pierre!\n\n\
                    Use <code>/privacy off</code> to opt out at any time.";

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

        let text = "Analytics consent has been <b>disabled</b>. \
                    No anonymous usage data will be collected.\n\n\
                    Use <code>/privacy on</code> to opt back in at any time.";

        Ok(CommandResponse::text(text))
    }
}
