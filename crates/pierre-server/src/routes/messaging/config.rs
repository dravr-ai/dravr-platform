// ABOUTME: Channel configuration CRUD handlers for messaging gateway
// ABOUTME: Manages per-tenant channel configs with JWT-authenticated endpoints
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::TenantId;
use pierre_database::backends::{MessagingRepository, TenantRepository, UpsertChannelConfigParams};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_middleware::extract_auth_from_headers;
use pierre_runtime_context::{resolve_tenant, tenant::require, TenantMode};

/// Request body for upserting a channel configuration
#[derive(Debug, Deserialize)]
pub struct UpsertChannelConfigBody {
    /// Whether this channel is enabled for the tenant
    pub enabled: bool,
    /// Channel-specific credentials (API key, bot token, signing secret, etc.)
    pub credentials: serde_json::Value,
    /// Webhook URL override (if different from default)
    pub webhook_url: Option<String>,
}

/// Response body for a channel configuration
#[derive(Debug, Serialize)]
pub struct ChannelConfigResponse {
    /// Channel type identifier
    pub channel: String,
    /// Whether this channel is enabled
    pub enabled: bool,
    /// Whether the channel has credentials configured
    pub has_credentials: bool,
    /// Webhook URL for this channel
    pub webhook_url: Option<String>,
}

/// Resolve tenant via the canonical resolver. Verifies membership when
/// `active_tenant_id` is claimed; errors if the user has no tenants.
/// No user-id fallback.
async fn resolve_tenant_id(
    auth: &AuthResult,
    resources: &Arc<ServerContext>,
) -> Result<TenantId, AppError> {
    require(resolve_tenant(resources, auth, TenantMode::Required).await?)
}

/// Gate tenant messaging-credential access to the tenant owner or an admin.
///
/// Channel configs hold per-tenant secrets (bot tokens, API secrets, webhook /
/// verify tokens). Reading, writing, or deleting them is an operator action, so
/// a plain tenant member must be denied. Mirrors the tenant-scoped owner/admin
/// gate on LLM credential writes (`routes-admin/llm_settings.rs`) and the
/// `require_admin` gate on fitness tenant-config save/delete
/// (`config/routes/fitness.rs`).
///
/// In single-tenant mode the tenant id equals the user id, so ownership is
/// implicit and no role lookup is needed.
///
/// # Errors
///
/// Returns `AppError` when the tenant-role lookup fails, or `PermissionDenied`
/// when the user is neither the tenant owner nor an admin.
async fn require_tenant_owner_or_admin(
    user_id: Uuid,
    tenant_id: TenantId,
    resources: &Arc<ServerContext>,
) -> Result<(), AppError> {
    // Single-tenant mode: tenant_id == user_id, the user is the implicit owner.
    if tenant_id.as_uuid() == user_id {
        return Ok(());
    }

    let tenants: &dyn TenantRepository = resources.common.repos.tenants.as_ref();
    let role = tenants
        .get_user_role(user_id, tenant_id)
        .await
        .map_err(|e| AppError::database(format!("Failed to check tenant role: {e}")))?;

    let is_owner_or_admin = role
        .as_deref()
        .is_some_and(|r| r == "owner" || r == "admin");

    if !is_owner_or_admin {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Only tenant owners or administrators can manage messaging channel configuration",
        ));
    }

    Ok(())
}

/// List all channel configurations for the authenticated tenant
pub async fn list_channel_configs(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth, &resources).await?;
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    let configs = db.list_channel_configs(tenant_id).await?;

    Ok((
        StatusCode::OK,
        Json(json!({
            "tenant_id": tenant_id,
            "channels": configs
        })),
    ))
}

/// A messaging channel the tenant has configured and a user can connect to — with
/// no credentials exposed. Backs the onboarding channel-picker (step 3).
#[derive(Debug, Serialize)]
pub struct AvailableChannel {
    /// Channel id (`telegram`, `whatsapp`, `slack`, `discord`, `messenger`).
    pub channel: String,
    /// User-facing name.
    pub display_name: String,
    /// Linking method: `deep_link` (QR / tap) or `oauth` (redirect).
    pub method: String,
    /// Whether to pre-select this channel (Telegram is the most complete).
    pub recommended: bool,
}

/// Every messaging channel, in picker order (recommended first).
const ALL_CHANNELS: [ChannelType; 5] = [
    ChannelType::Telegram,
    ChannelType::WhatsApp,
    ChannelType::Slack,
    ChannelType::Discord,
    ChannelType::Messenger,
];

/// User-facing display name for a channel.
const fn channel_display_name(channel: ChannelType) -> &'static str {
    match channel {
        ChannelType::Telegram => "Telegram",
        ChannelType::WhatsApp => "WhatsApp",
        ChannelType::Slack => "Slack",
        ChannelType::Discord => "Discord",
        ChannelType::Messenger => "Messenger",
    }
}

/// GET /api/messaging/channels/available
///
/// Secret-free list of channels the tenant has configured and enabled, for the
/// onboarding channel-picker. Reads only each config's `is_active` flag and never
/// returns credentials — unlike `list_channel_configs`, which is the admin
/// surface. Requires a valid session.
///
/// # Errors
///
/// Returns `AppError` when authentication fails, no tenant can be resolved, or a
/// channel-config read errors.
pub async fn list_available_channels(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth, &resources).await?;
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    let mut available = Vec::new();
    for channel in ALL_CHANNELS {
        let channel_str = channel.to_string();
        // Connectable = the tenant has a config row that is not explicitly
        // disabled. `get_channel_config` serializes the stored flag as `is_active`
        // (the write side maps the request's `enabled` onto it); we read ONLY that
        // scalar flag — never any secret field.
        let Some(config) = db.get_channel_config(tenant_id, &channel_str).await? else {
            continue;
        };
        let is_active = config
            .get("is_active")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        if !is_active {
            continue;
        }
        available.push(AvailableChannel {
            channel: channel_str,
            display_name: channel_display_name(channel).to_owned(),
            method: channel.linking_method().to_string(),
            recommended: channel == ChannelType::Telegram,
        });
    }

    Ok((StatusCode::OK, Json(available)))
}

/// Get a specific channel configuration
pub async fn get_channel_config(
    State(resources): State<Arc<ServerContext>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth, &resources).await?;
    require_tenant_owner_or_admin(auth.user_id, tenant_id, &resources).await?;

    let channel_type = ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let channel_str = channel_type.to_string();
    let config = db.get_channel_config(tenant_id, &channel_str).await?;

    Ok((
        StatusCode::OK,
        Json(json!({
            "tenant_id": tenant_id,
            "config": config
        })),
    ))
}

/// Create or update a channel configuration
pub async fn upsert_channel_config(
    State(resources): State<Arc<ServerContext>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UpsertChannelConfigBody>,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth, &resources).await?;
    require_tenant_owner_or_admin(auth.user_id, tenant_id, &resources).await?;

    let channel_type = ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let id = Uuid::new_v4().to_string();
    let creds = &body.credentials;

    let api_key = creds.get("api_key").and_then(|v| v.as_str());
    let api_secret = creds.get("api_secret").and_then(|v| v.as_str());
    let webhook_secret = creds.get("webhook_secret").and_then(|v| v.as_str());
    let verify_token = creds.get("verify_token").and_then(|v| v.as_str());
    let account_id = creds.get("account_id").and_then(|v| v.as_str());
    let phone_number = creds.get("phone_number").and_then(|v| v.as_str());
    let bot_token = creds.get("bot_token").and_then(|v| v.as_str());

    let params = UpsertChannelConfigParams {
        id: &id,
        tenant_id,
        channel_type: &channel,
        api_key,
        api_secret,
        webhook_secret,
        verify_token,
        account_id,
        phone_number,
        bot_token,
        is_active: body.enabled,
    };

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    // Reject registering an external identity (phone number, page, or bot token)
    // already held by another tenant. Two tenants sharing the same identity makes
    // both configs verify the same inbound webhook signature, so tenant routing
    // would become order-dependent. Only enabled configs participate in webhook
    // signature matching, so the guard is scoped to enabled requests.
    if body.enabled
        && db
            .channel_identity_claimed_by_other_tenant(
                tenant_id,
                &channel,
                phone_number,
                account_id,
                bot_token,
            )
            .await?
    {
        return Err(AppError::new(
            ErrorCode::ResourceAlreadyExists,
            format!(
                "This {channel} identity is already configured under another workspace; \
                 a phone number / page / bot token can belong to only one workspace"
            ),
        ));
    }

    db.upsert_channel_config(&params).await?;

    let has_credentials = api_key.is_some()
        || api_secret.is_some()
        || webhook_secret.is_some()
        || bot_token.is_some();

    let response = ChannelConfigResponse {
        channel: channel_type.to_string(),
        enabled: body.enabled,
        has_credentials,
        webhook_url: body.webhook_url,
    };

    Ok((
        StatusCode::OK,
        Json(json!({
            "tenant_id": tenant_id,
            "config": response,
            "action": "upserted"
        })),
    ))
}

/// Delete a channel configuration
pub async fn delete_channel_config(
    State(resources): State<Arc<ServerContext>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth, &resources).await?;
    require_tenant_owner_or_admin(auth.user_id, tenant_id, &resources).await?;

    let channel_type = ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let deleted = db.delete_channel_config(tenant_id, &channel).await?;

    let status = if deleted {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };

    Ok((
        status,
        Json(json!({
            "tenant_id": tenant_id,
            "channel": channel_type.to_string(),
            "action": if deleted { "deleted" } else { "not_found" }
        })),
    ))
}
