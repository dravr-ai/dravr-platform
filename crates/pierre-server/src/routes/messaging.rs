// ABOUTME: REST routes for Slack webhook events and messaging connection management
// ABOUTME: Handles Slack Events API webhooks, connection CRUD, and channel binding operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Json;
use axum::routing::{delete, get, post};
use axum::Router;
use pierre_auth::auth::AuthResult;
use pierre_core::models::messaging::{
    CreateChannelBindingParams, CreateMessagingConnectionParams, MessagingConnectionRecord,
};
use pierre_database::plugins::{MessagingRepository, TenantRepository};
use pierre_messaging::slack::types::{SlackEvent, SlackMessageEvent, UrlVerificationResponse};
use pierre_messaging::slack::SlackProvider;
use pierre_messaging::types::IncomingMessage;
use pierre_messaging::MessagingProvider;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::mcp::resources::ServerResources;
use crate::middleware::{extract_auth_from_headers, require_admin};
use crate::models::TenantId;
use crate::services::messaging_bridge;

/// Fallback sender ID when a Slack message has no user field
const UNKNOWN_SENDER: &str = "unknown";

/// Supported messaging providers for connection creation
const SUPPORTED_PROVIDERS: &[&str] = &["slack"];

/// Build the messaging routes router
pub fn messaging_routes(resources: Arc<ServerResources>) -> Router {
    Router::new()
        // Slack webhook endpoint (public, verified via signing secret)
        .route("/webhooks/slack", post(slack_webhook_handler))
        // Connection management (admin-authenticated for writes)
        .route("/connections", get(list_connections))
        .route("/connections", post(create_connection))
        .route("/connections/:id", delete(delete_connection))
        // Channel binding management (admin-authenticated for writes)
        .route("/bindings", get(list_bindings))
        .route("/bindings", post(create_binding))
        .route("/bindings/:id", delete(delete_binding))
        .with_state(resources)
}

// ============================================================================
// Auth Helpers
// ============================================================================

/// Authenticate the request and resolve the tenant ID (any authenticated user)
async fn authenticate(
    headers: &HeaderMap,
    resources: &Arc<ServerResources>,
) -> Result<(Uuid, TenantId), AppError> {
    let auth = extract_auth_from_headers(headers, resources).await?;
    let tenant_id = resolve_tenant_id(auth.user_id, &auth, resources).await?;
    Ok((auth.user_id, tenant_id))
}

/// Authenticate the request, verify admin privileges, and resolve tenant ID
async fn authenticate_admin(
    headers: &HeaderMap,
    resources: &Arc<ServerResources>,
) -> Result<(Uuid, TenantId), AppError> {
    let auth = extract_auth_from_headers(headers, resources).await?;
    require_admin(auth.user_id, &resources.database).await?;
    let tenant_id = resolve_tenant_id(auth.user_id, &auth, resources).await?;
    Ok((auth.user_id, tenant_id))
}

/// Resolve the tenant ID from auth result, falling back to user's first tenant
async fn resolve_tenant_id(
    user_id: Uuid,
    auth: &AuthResult,
    resources: &Arc<ServerResources>,
) -> Result<TenantId, AppError> {
    if let Some(tid) = auth.active_tenant_id {
        return Ok(TenantId::from(tid));
    }
    let tenants = resources.database.list_for_user(user_id).await?;
    Ok(tenants
        .first()
        .map_or_else(|| TenantId::from(user_id), |t| t.id))
}

// ============================================================================
// Slack Webhook Handler
// ============================================================================

/// Handle incoming Slack Events API webhooks
///
/// Security model:
/// 1. URL verification: no signature check (Slack handshake, lightweight parse only)
/// 2. Event callbacks: lightweight JSON parse to extract `type` and `team_id`,
///    then HMAC signature verification BEFORE full deserialization into typed structs
async fn slack_webhook_handler(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    body: String,
) -> Result<Json<serde_json::Value>, AppError> {
    let body_bytes = body.as_bytes();

    // Lightweight parse: extract only the top-level "type" field to route the request
    let envelope: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| AppError::invalid_input(format!("Invalid Slack event payload: {e}")))?;

    let event_type = envelope["type"]
        .as_str()
        .ok_or_else(|| AppError::invalid_input("Slack payload missing 'type' field"))?;

    match event_type {
        // URL verification: echo back the challenge token (no signature needed per Slack spec)
        "url_verification" => {
            let challenge = envelope["challenge"]
                .as_str()
                .ok_or_else(|| {
                    AppError::invalid_input("url_verification missing 'challenge' field")
                })?
                .to_owned();

            info!("Slack URL verification challenge received");
            let response = UrlVerificationResponse { challenge };
            Ok(Json(serde_json::to_value(response).map_err(|e| {
                AppError::internal(format!("Failed to serialize response: {e}"))
            })?))
        }

        // Event callback: verify HMAC signature before full deserialization
        "event_callback" => {
            let team_id = envelope["team_id"]
                .as_str()
                .ok_or_else(|| AppError::invalid_input("event_callback missing 'team_id' field"))?;

            // Look up the connection for this team to get credentials
            let connection = resources
                .database
                .get_messaging_connection_by_team("slack", team_id)
                .await?
                .ok_or_else(|| {
                    AppError::not_found(format!("No Slack connection for team {team_id}"))
                })?;

            // Create a provider instance with the connection's credentials
            let provider = SlackProvider::new(
                connection.bot_token.clone(),
                connection.signing_secret.clone(),
            );

            // Verify the request signature BEFORE deserializing into typed structs
            if !provider.verify_request(&headers, body_bytes)? {
                warn!(team_id = %team_id, "Slack webhook signature verification failed");
                return Err(AppError::auth_invalid("Invalid Slack request signature"));
            }

            // Signature verified — now safe to deserialize the nested event
            let event: SlackEvent =
                serde_json::from_value(envelope["event"].clone()).map_err(|e| {
                    AppError::invalid_input(format!("Invalid Slack event structure: {e}"))
                })?;

            match event {
                SlackEvent::Message(msg) | SlackEvent::AppMention(msg) => {
                    handle_slack_message(resources, &connection, &msg)?;
                }
            }

            Ok(Json(serde_json::json!({"ok": true})))
        }

        other => Err(AppError::invalid_input(format!(
            "Unsupported Slack event type: {other}"
        ))),
    }
}

/// Process a Slack message event by bridging it to the Dravr chat system
fn handle_slack_message(
    resources: Arc<ServerResources>,
    connection: &MessagingConnectionRecord,
    msg: &SlackMessageEvent,
) -> AppResult<()> {
    // Ignore bot messages to prevent infinite loops
    if msg.is_bot_message() {
        debug!("Ignoring bot message from Slack");
        return Ok(());
    }

    // Extract required fields
    let text = match msg.text.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => {
            debug!("Ignoring Slack message with empty text");
            return Ok(());
        }
    };

    let channel_id = msg
        .channel
        .as_deref()
        .ok_or_else(|| AppError::invalid_input("Slack message missing channel field"))?;

    let sender_id = msg.user.as_deref().unwrap_or(UNKNOWN_SENDER);
    let timestamp = msg.timestamp.as_deref().unwrap_or("0");

    // Build a normalized incoming message
    let incoming = IncomingMessage {
        channel_id: channel_id.to_owned(),
        sender_id: sender_id.to_owned(),
        sender_name: None,
        text: text.to_owned(),
        message_id: timestamp.to_owned(),
        thread_id: msg.thread_ts.clone(),
        team_id: connection.team_id.clone(),
        timestamp: chrono::Utc::now(),
    };

    // Spawn the bridge processing as a background task so we respond to Slack quickly
    // (Slack requires a 200 response within 3 seconds)
    let connection_id = connection.id.clone();
    let bot_token = connection.bot_token.clone();
    let signing_secret = connection.signing_secret.clone();

    tokio::spawn(async move {
        let bg_provider = SlackProvider::new(bot_token, signing_secret);
        if let Err(e) = messaging_bridge::process_incoming_message(
            &resources,
            &bg_provider,
            &incoming,
            &connection_id,
        )
        .await
        {
            warn!(error = %e, "Failed to process bridged Slack message");
        }
    });

    Ok(())
}

// ============================================================================
// Connection Management Routes
// ============================================================================

/// Request body for creating a messaging connection
#[derive(Debug, Deserialize)]
struct CreateConnectionRequest {
    /// Provider name (e.g., "slack")
    provider: String,
    /// Provider-specific workspace/team identifier
    team_id: String,
    /// Human-readable workspace name
    team_name: Option<String>,
    /// Bot token for API calls
    bot_token: String,
    /// Webhook signing secret
    signing_secret: String,
}

/// Response for a messaging connection
#[derive(Debug, Serialize)]
struct ConnectionResponse {
    id: String,
    provider: String,
    team_id: String,
    team_name: Option<String>,
    created_at: String,
}

/// List all messaging connections for the authenticated tenant
async fn list_connections(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Json<Vec<ConnectionResponse>>, AppError> {
    let (_, tenant_id) = authenticate(&headers, &resources).await?;

    let connections = resources
        .database
        .list_messaging_connections(tenant_id)
        .await?;

    let response: Vec<ConnectionResponse> = connections
        .into_iter()
        .map(|c| ConnectionResponse {
            id: c.id,
            provider: c.provider,
            team_id: c.team_id,
            team_name: c.team_name,
            created_at: c.created_at,
        })
        .collect();

    Ok(Json(response))
}

/// Create a new messaging connection (admin only)
async fn create_connection(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<CreateConnectionRequest>,
) -> Result<Json<ConnectionResponse>, AppError> {
    let (user_id, tenant_id) = authenticate_admin(&headers, &resources).await?;

    let tenant_id_str = tenant_id.to_string();
    let user_id_str = user_id.to_string();

    // Reject unsupported providers
    if !SUPPORTED_PROVIDERS.contains(&request.provider.as_str()) {
        return Err(AppError::invalid_input(format!(
            "Unsupported messaging provider '{}'. Supported: {}",
            request.provider,
            SUPPORTED_PROVIDERS.join(", ")
        )));
    }

    let params = CreateMessagingConnectionParams {
        tenant_id: &tenant_id_str,
        provider: &request.provider,
        team_id: &request.team_id,
        team_name: request.team_name.as_deref(),
        bot_token: &request.bot_token,
        signing_secret: &request.signing_secret,
        created_by: Some(&user_id_str),
    };

    let record = resources
        .database
        .create_messaging_connection(&params)
        .await?;

    info!(
        connection_id = %record.id,
        provider = %record.provider,
        team_id = %record.team_id,
        "Messaging connection created"
    );

    Ok(Json(ConnectionResponse {
        id: record.id,
        provider: record.provider,
        team_id: record.team_id,
        team_name: record.team_name,
        created_at: record.created_at,
    }))
}

/// Delete a messaging connection (admin only)
async fn delete_connection(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (_, tenant_id) = authenticate_admin(&headers, &resources).await?;

    let deleted = resources
        .database
        .delete_messaging_connection(&id, tenant_id)
        .await?;

    if !deleted {
        return Err(AppError::not_found(format!("Messaging connection {id}")));
    }

    info!(connection_id = %id, "Messaging connection deleted");
    Ok(Json(serde_json::json!({"deleted": true})))
}

// ============================================================================
// Channel Binding Routes
// ============================================================================

/// Request body for creating a channel binding
#[derive(Debug, Deserialize)]
struct CreateBindingRequest {
    /// Messaging connection ID
    messaging_connection_id: String,
    /// Provider-specific channel identifier
    channel_id: String,
    /// Human-readable channel name
    channel_name: Option<String>,
    /// Dravr conversation ID to bind to
    conversation_id: String,
}

/// Response for a channel binding
#[derive(Debug, Serialize)]
struct BindingResponse {
    id: String,
    messaging_connection_id: String,
    channel_id: String,
    channel_name: Option<String>,
    conversation_id: String,
    active: bool,
    created_at: String,
}

/// List all channel bindings for the authenticated tenant
async fn list_bindings(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Json<Vec<BindingResponse>>, AppError> {
    let (_, tenant_id) = authenticate(&headers, &resources).await?;

    let bindings = resources.database.list_channel_bindings(tenant_id).await?;

    let response: Vec<BindingResponse> = bindings
        .into_iter()
        .map(|b| BindingResponse {
            id: b.id,
            messaging_connection_id: b.messaging_connection_id,
            channel_id: b.channel_id,
            channel_name: b.channel_name,
            conversation_id: b.conversation_id,
            active: b.active,
            created_at: b.created_at,
        })
        .collect();

    Ok(Json(response))
}

/// Create a new channel binding (admin only)
async fn create_binding(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<CreateBindingRequest>,
) -> Result<Json<BindingResponse>, AppError> {
    let (user_id, tenant_id) = authenticate_admin(&headers, &resources).await?;

    // Verify the connection exists and belongs to this tenant
    resources
        .database
        .get_messaging_connection(&request.messaging_connection_id, tenant_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(format!(
                "Messaging connection {}",
                request.messaging_connection_id
            ))
        })?;

    let params = CreateChannelBindingParams {
        messaging_connection_id: &request.messaging_connection_id,
        tenant_id: &tenant_id.to_string(),
        channel_id: &request.channel_id,
        channel_name: request.channel_name.as_deref(),
        conversation_id: &request.conversation_id,
        user_id: &user_id.to_string(),
    };

    let record = resources.database.create_channel_binding(&params).await?;

    info!(
        binding_id = %record.id,
        channel = %record.channel_id,
        conversation = %record.conversation_id,
        "Channel binding created"
    );

    Ok(Json(BindingResponse {
        id: record.id,
        messaging_connection_id: record.messaging_connection_id,
        channel_id: record.channel_id,
        channel_name: record.channel_name,
        conversation_id: record.conversation_id,
        active: record.active,
        created_at: record.created_at,
    }))
}

/// Delete a channel binding (admin only)
async fn delete_binding(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (_, tenant_id) = authenticate_admin(&headers, &resources).await?;

    let deleted = resources
        .database
        .delete_channel_binding(&id, tenant_id)
        .await?;

    if !deleted {
        return Err(AppError::not_found(format!("Channel binding {id}")));
    }

    info!(binding_id = %id, "Channel binding deleted");
    Ok(Json(serde_json::json!({"deleted": true})))
}
