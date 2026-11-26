// ABOUTME: Seed messaging channel configs from the server configuration on startup
// ABOUTME: Upserts the Slack, Telegram, Meta WhatsApp, Meta Messenger and Discord channels the configuration names
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;

use pierre_config::environment::{ChannelCredentials, WebhookSecrets};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::backends::UpsertChannelConfigParams;
use pierre_database::repositories::MessagingRepository;
use pierre_database::RepositoryRegistry;
use secrecy::{ExposeSecret, SecretString};
use tracing::{info, warn};
use uuid::Uuid;

/// Seed the messaging channel configs the server configuration names
///
/// Upserts the credentials `channels` and `webhooks` captured from the
/// environment at startup into the database. The environment itself is not
/// read for them here: a configuration that names no channel seeds none,
/// whatever the process exports (carnet#1600).
/// Resolves the admin user's tenant as the owning tenant for all channel configs,
/// and marks every seeded config platform scope (see `seed_platform_config`).
/// Skipped silently when the configuration names no channel.
pub async fn seed_from_config(
    repos: &RepositoryRegistry,
    channels: &ChannelCredentials,
    webhooks: &WebhookSecrets,
) {
    let admin_email = env::var("ADMIN_EMAIL").unwrap_or_else(|_| "admin@example.com".to_owned());
    let Some(tenant_id) = resolve_admin_tenant(repos, &admin_email).await else {
        // No admin user yet — skip seeding (server may be starting fresh before bootstrap)
        return;
    };

    let mut seeded = 0;

    if let Some(count) = seed_slack(&*repos.messaging, tenant_id, webhooks).await {
        seeded += count;
    }
    if let Some(count) = seed_telegram(&*repos.messaging, tenant_id, channels).await {
        seeded += count;
    }
    if let Some(count) = seed_whatsapp(&*repos.messaging, tenant_id, channels).await {
        seeded += count;
    }
    if let Some(count) = seed_messenger(&*repos.messaging, tenant_id, channels).await {
        seeded += count;
    }
    if let Some(count) = seed_discord(&*repos.messaging, tenant_id, channels).await {
        seeded += count;
    }

    if seeded > 0 {
        info!(
            channels = seeded,
            "Messaging channel configs seeded from env vars"
        );
    }
}

/// Resolve the admin user's primary tenant
async fn resolve_admin_tenant(repos: &RepositoryRegistry, admin_email: &str) -> Option<TenantId> {
    let user = repos.users.get_by_email(admin_email).await.ok()??;
    repos.tenants.first_for_user(user.id).await.ok()?
}

/// Upsert one env-seeded config and mark it platform scope.
///
/// The environment is the deployment's, not the admin tenant's: the bot it
/// configures answers every athlete, whatever tenant they live in, so the row
/// is marked readable by all of them through `resolve_channel_config`. The
/// admin tenant only stores it.
async fn seed_platform_config(
    database: &(dyn MessagingRepository + '_),
    params: &UpsertChannelConfigParams<'_>,
    label: &str,
) -> Option<u32> {
    match upsert_as_platform(database, params).await {
        Ok(()) => {
            info!(
                channel = label,
                "Seeded platform channel config from env vars"
            );
            Some(1)
        }
        Err(e) => {
            warn!(error = %e, channel = label, "Failed to seed platform channel config");
            None
        }
    }
}

/// Write the config, then mark the row it landed in as platform scope.
async fn upsert_as_platform(
    database: &(dyn MessagingRepository + '_),
    params: &UpsertChannelConfigParams<'_>,
) -> AppResult<()> {
    database.upsert_channel_config(params).await?;
    if database
        .mark_channel_config_platform_scope(params.tenant_id, params.channel_type)
        .await?
    {
        Ok(())
    } else {
        Err(AppError::not_found(format!(
            "{} channel config for tenant {}",
            params.channel_type, params.tenant_id
        )))
    }
}

/// The secret a configured credential holds.
fn exposed(secret: Option<&SecretString>) -> Option<&str> {
    secret.map(ExposeSecret::expose_secret)
}

/// Seed Slack channel config from `SLACK_BOT_TOKEN` + `SLACK_SIGNING_SECRET`
///
/// Both are the values the interactive route reads, so a blank one
/// configures no Slack channel here either.
async fn seed_slack(
    database: &(dyn MessagingRepository + '_),
    tenant_id: TenantId,
    webhooks: &WebhookSecrets,
) -> Option<u32> {
    let bot_token = exposed(webhooks.slack_bot_token.as_ref())?;
    let signing_secret = exposed(webhooks.slack_signing_secret.as_ref())?;

    let id = Uuid::new_v4().to_string();
    let params = UpsertChannelConfigParams {
        id: &id,
        tenant_id,
        channel_type: "slack",
        api_key: Some(bot_token),
        api_secret: None,
        webhook_secret: Some(signing_secret),
        verify_token: None,
        account_id: None,
        phone_number: None,
        bot_token: None,
        is_active: true,
    };

    seed_platform_config(database, &params, "Slack").await
}

/// Seed Telegram channel config from `TELEGRAM_BOT_TOKEN` + `TELEGRAM_WEBHOOK_SECRET`
async fn seed_telegram(
    database: &(dyn MessagingRepository + '_),
    tenant_id: TenantId,
    channels: &ChannelCredentials,
) -> Option<u32> {
    let bot_token = exposed(channels.telegram_bot_token.as_ref())?;
    let webhook_secret = exposed(channels.telegram_webhook_secret.as_ref())?;

    let id = Uuid::new_v4().to_string();
    let params = UpsertChannelConfigParams {
        id: &id,
        tenant_id,
        channel_type: "telegram",
        api_key: None,
        api_secret: None,
        webhook_secret: Some(webhook_secret),
        verify_token: None,
        account_id: None,
        phone_number: None,
        bot_token: Some(bot_token),
        is_active: true,
    };

    seed_platform_config(database, &params, "Telegram").await
}

/// Seed `WhatsApp` channel config from `META_WHATSAPP_*` env vars
async fn seed_whatsapp(
    database: &(dyn MessagingRepository + '_),
    tenant_id: TenantId,
    channels: &ChannelCredentials,
) -> Option<u32> {
    let app_secret = exposed(channels.whatsapp_app_secret.as_ref())?;
    let access_token = exposed(channels.whatsapp_access_token.as_ref())?;
    let verify_token = exposed(channels.whatsapp_verify_token.as_ref());

    let id = Uuid::new_v4().to_string();
    let params = UpsertChannelConfigParams {
        id: &id,
        tenant_id,
        channel_type: "whatsapp",
        api_key: Some(access_token),
        api_secret: None,
        webhook_secret: Some(app_secret),
        verify_token,
        account_id: None,
        phone_number: channels.whatsapp_phone_number_id.as_deref(),
        bot_token: None,
        is_active: true,
    };

    seed_platform_config(database, &params, "WhatsApp").await
}

/// Seed Messenger channel config from `META_MESSENGER_*` env vars
async fn seed_messenger(
    database: &(dyn MessagingRepository + '_),
    tenant_id: TenantId,
    channels: &ChannelCredentials,
) -> Option<u32> {
    let app_secret = exposed(channels.messenger_app_secret.as_ref())?;
    let page_access_token = exposed(channels.messenger_page_access_token.as_ref())?;
    let verify_token = exposed(channels.messenger_verify_token.as_ref());

    let id = Uuid::new_v4().to_string();
    let params = UpsertChannelConfigParams {
        id: &id,
        tenant_id,
        channel_type: "messenger",
        api_key: Some(page_access_token),
        api_secret: None,
        webhook_secret: Some(app_secret),
        verify_token,
        account_id: channels.messenger_page_id.as_deref(),
        phone_number: None,
        bot_token: None,
        is_active: true,
    };

    seed_platform_config(database, &params, "Messenger").await
}

/// Seed Discord channel config from `DISCORD_BOT_TOKEN` + `DISCORD_PUBLIC_KEY`
async fn seed_discord(
    database: &(dyn MessagingRepository + '_),
    tenant_id: TenantId,
    channels: &ChannelCredentials,
) -> Option<u32> {
    let bot_token = exposed(channels.discord_bot_token.as_ref())?;
    let public_key = exposed(channels.discord_public_key.as_ref())?;

    let id = Uuid::new_v4().to_string();
    let params = UpsertChannelConfigParams {
        id: &id,
        tenant_id,
        channel_type: "discord",
        api_key: None,
        api_secret: None,
        webhook_secret: Some(public_key),
        verify_token: None,
        account_id: channels.discord_application_id.as_deref(),
        phone_number: None,
        bot_token: Some(bot_token),
        is_active: true,
    };

    seed_platform_config(database, &params, "Discord").await
}
