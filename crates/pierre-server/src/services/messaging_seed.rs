// ABOUTME: Seed messaging channel configs from environment variables on server startup
// ABOUTME: Reads Slack, Telegram, Meta WhatsApp, Meta Messenger, and Discord env vars and upserts DB channel configs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;

use pierre_core::models::TenantId;
use pierre_database::plugins::UpsertChannelConfigParams;
use pierre_database::repositories::MessagingRepository;
use pierre_database::RepositoryRegistry;
use tracing::{info, warn};
use uuid::Uuid;

/// Seed messaging channel configs from environment variables
///
/// Reads channel credentials from env vars and upserts them into the database.
/// Resolves the admin user's tenant as the owning tenant for all channel configs.
/// Skipped silently when no messaging env vars are set.
pub async fn seed_from_env(repos: &RepositoryRegistry) {
    let admin_email = env::var("ADMIN_EMAIL").unwrap_or_else(|_| "admin@example.com".to_owned());
    let Some(tenant_id) = resolve_admin_tenant(repos, &admin_email).await else {
        // No admin user yet — skip seeding (server may be starting fresh before bootstrap)
        return;
    };

    let mut seeded = 0;

    if let Some(count) = seed_slack(&*repos.messaging, tenant_id).await {
        seeded += count;
    }
    if let Some(count) = seed_telegram(&*repos.messaging, tenant_id).await {
        seeded += count;
    }
    if let Some(count) = seed_whatsapp(&*repos.messaging, tenant_id).await {
        seeded += count;
    }
    if let Some(count) = seed_messenger(&*repos.messaging, tenant_id).await {
        seeded += count;
    }
    if let Some(count) = seed_discord(&*repos.messaging, tenant_id).await {
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
    let tenants = repos.tenants.list_for_user(user.id).await.ok()?;
    let tenant = tenants.first()?;
    Some(tenant.id)
}

/// Seed Slack channel config from `SLACK_BOT_TOKEN` + `SLACK_SIGNING_SECRET`
async fn seed_slack(database: &(dyn MessagingRepository + '_), tenant_id: TenantId) -> Option<u32> {
    let bot_token = env::var("SLACK_BOT_TOKEN").ok()?;
    let signing_secret = env::var("SLACK_SIGNING_SECRET").ok()?;

    if bot_token.is_empty() || signing_secret.is_empty() {
        return None;
    }

    let id = Uuid::new_v4().to_string();
    let params = UpsertChannelConfigParams {
        id: &id,
        tenant_id,
        channel_type: "slack",
        api_key: Some(&bot_token),
        api_secret: None,
        webhook_secret: Some(&signing_secret),
        verify_token: None,
        account_id: None,
        phone_number: None,
        bot_token: None,
        is_active: true,
    };

    match database.upsert_channel_config(&params).await {
        Ok(()) => {
            info!("Seeded Slack channel config from env vars");
            Some(1)
        }
        Err(e) => {
            warn!(error = %e, "Failed to seed Slack channel config");
            None
        }
    }
}

/// Seed Telegram channel config from `TELEGRAM_BOT_TOKEN` + `TELEGRAM_WEBHOOK_SECRET`
async fn seed_telegram(
    database: &(dyn MessagingRepository + '_),
    tenant_id: TenantId,
) -> Option<u32> {
    let bot_token = env::var("TELEGRAM_BOT_TOKEN").ok()?;
    let webhook_secret = env::var("TELEGRAM_WEBHOOK_SECRET").ok()?;

    if bot_token.is_empty() || webhook_secret.is_empty() {
        return None;
    }

    let id = Uuid::new_v4().to_string();
    let params = UpsertChannelConfigParams {
        id: &id,
        tenant_id,
        channel_type: "telegram",
        api_key: None,
        api_secret: None,
        webhook_secret: Some(&webhook_secret),
        verify_token: None,
        account_id: None,
        phone_number: None,
        bot_token: Some(&bot_token),
        is_active: true,
    };

    match database.upsert_channel_config(&params).await {
        Ok(()) => {
            info!("Seeded Telegram channel config from env vars");
            Some(1)
        }
        Err(e) => {
            warn!(error = %e, "Failed to seed Telegram channel config");
            None
        }
    }
}

/// Seed `WhatsApp` channel config from `META_WHATSAPP_*` env vars
async fn seed_whatsapp(
    database: &(dyn MessagingRepository + '_),
    tenant_id: TenantId,
) -> Option<u32> {
    let app_secret = env::var("META_WHATSAPP_APP_SECRET").ok()?;
    let access_token = env::var("META_WHATSAPP_ACCESS_TOKEN").ok()?;

    if app_secret.is_empty() || access_token.is_empty() {
        return None;
    }

    let phone_number_id = env::var("META_WHATSAPP_PHONE_NUMBER_ID").ok();
    let verify_token = env::var("META_WHATSAPP_VERIFY_TOKEN").ok();

    let id = Uuid::new_v4().to_string();
    let params = UpsertChannelConfigParams {
        id: &id,
        tenant_id,
        channel_type: "whatsapp",
        api_key: Some(&access_token),
        api_secret: None,
        webhook_secret: Some(&app_secret),
        verify_token: verify_token.as_deref(),
        account_id: None,
        phone_number: phone_number_id.as_deref(),
        bot_token: None,
        is_active: true,
    };

    match database.upsert_channel_config(&params).await {
        Ok(()) => {
            info!("Seeded WhatsApp channel config from env vars");
            Some(1)
        }
        Err(e) => {
            warn!(error = %e, "Failed to seed WhatsApp channel config");
            None
        }
    }
}

/// Seed Messenger channel config from `META_MESSENGER_*` env vars
async fn seed_messenger(
    database: &(dyn MessagingRepository + '_),
    tenant_id: TenantId,
) -> Option<u32> {
    let app_secret = env::var("META_MESSENGER_APP_SECRET").ok()?;
    let page_access_token = env::var("META_MESSENGER_PAGE_ACCESS_TOKEN").ok()?;

    if app_secret.is_empty() || page_access_token.is_empty() {
        return None;
    }

    let verify_token = env::var("META_MESSENGER_VERIFY_TOKEN").ok();

    let id = Uuid::new_v4().to_string();
    let params = UpsertChannelConfigParams {
        id: &id,
        tenant_id,
        channel_type: "messenger",
        api_key: Some(&page_access_token),
        api_secret: None,
        webhook_secret: Some(&app_secret),
        verify_token: verify_token.as_deref(),
        account_id: None,
        phone_number: None,
        bot_token: None,
        is_active: true,
    };

    match database.upsert_channel_config(&params).await {
        Ok(()) => {
            info!("Seeded Messenger channel config from env vars");
            Some(1)
        }
        Err(e) => {
            warn!(error = %e, "Failed to seed Messenger channel config");
            None
        }
    }
}

/// Seed Discord channel config from `DISCORD_BOT_TOKEN` + `DISCORD_PUBLIC_KEY`
async fn seed_discord(
    database: &(dyn MessagingRepository + '_),
    tenant_id: TenantId,
) -> Option<u32> {
    let bot_token = env::var("DISCORD_BOT_TOKEN").ok()?;
    let public_key = env::var("DISCORD_PUBLIC_KEY").ok()?;

    if bot_token.is_empty() || public_key.is_empty() {
        return None;
    }

    let application_id = env::var("DISCORD_APPLICATION_ID").ok();

    let id = Uuid::new_v4().to_string();
    let params = UpsertChannelConfigParams {
        id: &id,
        tenant_id,
        channel_type: "discord",
        api_key: None,
        api_secret: None,
        webhook_secret: Some(&public_key),
        verify_token: None,
        account_id: application_id.as_deref(),
        phone_number: None,
        bot_token: Some(&bot_token),
        is_active: true,
    };

    match database.upsert_channel_config(&params).await {
        Ok(()) => {
            info!("Seeded Discord channel config from env vars");
            Some(1)
        }
        Err(e) => {
            warn!(error = %e, "Failed to seed Discord channel config");
            None
        }
    }
}
