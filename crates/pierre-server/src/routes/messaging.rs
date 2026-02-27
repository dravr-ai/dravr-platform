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
use pierre_core::models::messaging::{CreateChannelBindingParams, CreateMessagingConnectionParams};
use pierre_database::plugins::{MessagingRepository, TenantRepository};
use pierre_messaging::slack::types::{
    SlackEvent, SlackEventPayload, SlackMessageEvent, UrlVerificationResponse,
};
use pierre_messaging::slack::SlackProvider;
use pierre_messaging::types::IncomingMessage;
use pierre_messaging::MessagingProvider;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::mcp::resources::ServerResources;
use crate::middleware::extract_auth_from_headers;
use crate::models::TenantId;
use crate::services::messaging_bridge;

/// Build the messaging routes router
pub fn messaging_routes(resources: Arc<ServerResources>) -> Router {
    Router::new()
        // Slack webhook endpoint (public, verified via signing secret)
        .route("/webhooks/slack", post(slack_webhook_handler))
        // Connection management (admin-authenticated)
        .route("/connections", get(list_connections))
        .route("/connections", post(create_connection))
        .route("/connections/:id", delete(delete_connection))
        // Channel binding management (admin-authenticated)
        .route("/bindings", get(list_bindings))
        .route("/bindings", post(create_binding))
        .route("/bindings/:id", delete(delete_binding))
        .with_state(resources)
}

// ============================================================================
// Auth Helpers
// ============================================================================

/// Authenticate the request and resolve the tenant ID
async fn authenticate(
    headers: &HeaderMap,
    resources: &Arc<ServerResources>,
) -> Result<(Uuid, TenantId), AppError> {
    let auth = extract_auth_from_headers(headers, resources).await?;
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
/// This endpoint handles three types of Slack events:
/// 1. URL verification challenges (during webhook registration)
/// 2. Message events (forwarded to the bridge service)
/// 3. App mention events (forwarded to the bridge service)
///
/// All requests are verified using HMAC-SHA256 signature validation.
async fn slack_webhook_handler(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    body: String,
) -> Result<Json<serde_json::Value>, AppError> {
    let body_bytes = body.as_bytes();

    // Parse the event payload to determine the type
    let payload: SlackEventPayload = serde_json::from_str(&body)
        .map_err(|e| AppError::invalid_input(format!("Invalid Slack event payload: {e}")))?;

    match payload {
        // URL verification: echo back the challenge token (no signature check needed for setup)
        SlackEventPayload::UrlVerification { challenge, .. } => {
            info!("Slack URL verification challenge received");
            let response = UrlVerificationResponse { challenge };
            Ok(Json(serde_json::to_value(response).map_err(|e| {
                AppError::internal(format!("Failed to serialize response: {e}"))
            })?))
        }

        // Event callback: verify signature, then process the event
        SlackEventPayload::EventCallback(callback) => {
            let team_id = &callback.team_id;

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

            // Verify the request signature
            if !provider.verify_request(&headers, body_bytes)? {
                warn!(team_id = %team_id, "Slack webhook signature verification failed");
                return Err(AppError::auth_invalid("Invalid Slack request signature"));
            }

            // Process the event
            match callback.event {
                SlackEvent::Message(msg) | SlackEvent::AppMention(msg) => {
                    handle_slack_message(resources, &connection.id, team_id, msg).await?;
                }
            }

            Ok(Json(serde_json::json!({"ok": true})))
        }
    }
}

/// Process a Slack message event by bridging it to the Dravr chat system
async fn handle_slack_message(
    resources: Arc<ServerResources>,
    connection_id: &str,
    team_id: &str,
    msg: SlackMessageEvent,
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

    let sender_id = msg.user.as_deref().unwrap_or("unknown");
    let timestamp = msg.timestamp.as_deref().unwrap_or("0");

    // Build a normalized incoming message
    let incoming = IncomingMessage {
        channel_id: channel_id.to_owned(),
        sender_id: sender_id.to_owned(),
        sender_name: None,
        text: text.to_owned(),
        message_id: timestamp.to_owned(),
        thread_id: msg.thread_ts.clone(),
        team_id: team_id.to_owned(),
        timestamp: chrono::Utc::now(),
    };

    // Spawn the bridge processing as a background task so we respond to Slack quickly
    // (Slack requires a 200 response within 3 seconds)
    let resources_clone = resources.clone();
    let connection_id_owned = connection_id.to_owned();
    // Retrieve credentials once for the spawned task
    let conn = resources
        .database
        .get_messaging_connection_by_team("slack", team_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Slack connection for team {team_id}")))?;

    tokio::spawn(async move {
        let bg_provider = SlackProvider::new(conn.bot_token, conn.signing_secret);
        if let Err(e) = messaging_bridge::process_incoming_message(
            &resources_clone,
            &bg_provider,
            &incoming,
            &connection_id_owned,
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

/// Create a new messaging connection
async fn create_connection(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<CreateConnectionRequest>,
) -> Result<Json<ConnectionResponse>, AppError> {
    let (user_id, tenant_id) = authenticate(&headers, &resources).await?;

    let params = CreateMessagingConnectionParams {
        tenant_id: &tenant_id.to_string(),
        provider: &request.provider,
        team_id: &request.team_id,
        team_name: request.team_name.as_deref(),
        bot_token: &request.bot_token,
        signing_secret: &request.signing_secret,
        created_by: &user_id.to_string(),
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

/// Delete a messaging connection
async fn delete_connection(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (_, tenant_id) = authenticate(&headers, &resources).await?;

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

/// Create a new channel binding
async fn create_binding(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Json(request): Json<CreateBindingRequest>,
) -> Result<Json<BindingResponse>, AppError> {
    let (user_id, tenant_id) = authenticate(&headers, &resources).await?;

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

/// Delete a channel binding
async fn delete_binding(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (_, tenant_id) = authenticate(&headers, &resources).await?;

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
