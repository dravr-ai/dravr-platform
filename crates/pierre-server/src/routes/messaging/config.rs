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
use pierre_database::backends::{MessagingRepository, UpsertChannelConfigParams};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

use crate::errors::AppError;
use crate::mcp::resources::ServerResources;
use crate::middleware::extract_auth_from_headers;
use pierre_auth::auth::AuthResult;

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

/// Resolve tenant ID from auth result, using `active_tenant_id` or falling back to `user_id`
fn resolve_tenant_id(auth: &AuthResult) -> TenantId {
    auth.active_tenant_id
        .map_or_else(|| TenantId::from(auth.user_id), TenantId::from)
}

/// List all channel configurations for the authenticated tenant
pub async fn list_channel_configs(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth);
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();

    let configs = db.list_channel_configs(tenant_id).await?;

    Ok((
        StatusCode::OK,
        Json(json!({
            "tenant_id": tenant_id,
            "channels": configs
        })),
    ))
}

/// Get a specific channel configuration
pub async fn get_channel_config(
    State(resources): State<Arc<ServerResources>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth);

    let channel_type = ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
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
    State(resources): State<Arc<ServerResources>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    Json(body): Json<UpsertChannelConfigBody>,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth);

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

    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
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
    State(resources): State<Arc<ServerResources>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth);

    let channel_type = ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
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
