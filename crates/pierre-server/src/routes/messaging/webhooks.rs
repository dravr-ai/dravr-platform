// ABOUTME: Webhook ingress handler for multi-channel messaging gateway
// ABOUTME: HTTP wiring layer that delegates business logic to services::messaging_ingress
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use pierre_core::models::messaging::{ChannelType, IncomingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::factory::create_adapter_from_config;
use serde::Deserialize;
use serde_json::{json, Value};
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, field, info, warn, Instrument, Span};

use crate::errors::AppError;
use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress;

/// Result of tenant-aware webhook verification
struct WebhookVerification {
    /// Resolved tenant from the matching channel config
    tenant_id: TenantId,
    /// Channel type resolved from URL path
    channel_type: ChannelType,
    /// Adapter constructed from the matching config
    adapter: Arc<dyn MessagingChannel>,
    /// Parsed inbound messages
    messages: Vec<IncomingMessage>,
}

/// Query parameters for Meta webhook verification (GET request)
///
/// Meta platforms (Messenger, `WhatsApp`) send a GET request with these params
/// when you configure the webhook URL in the App Dashboard. The server must
/// validate `hub.verify_token` and echo back `hub.challenge` as plain text.
#[derive(Debug, Deserialize)]
pub struct MetaVerifyQuery {
    /// Must be `"subscribe"`
    #[serde(rename = "hub.mode")]
    pub mode: String,
    /// Random string that must be echoed back as the response body
    #[serde(rename = "hub.challenge")]
    pub challenge: String,
    /// Token configured in the Meta App Dashboard — must match our stored verify token
    #[serde(rename = "hub.verify_token")]
    pub verify_token: String,
}

/// Handle Meta webhook verification (GET request)
///
/// Meta sends `GET /webhook?hub.mode=subscribe&hub.verify_token=TOKEN&hub.challenge=CHALLENGE`
/// when configuring the webhook URL. We validate the verify token against the channel's
/// `webhook_secret` and echo `hub.challenge` back as plain text to confirm ownership.
///
/// Supports both Messenger and `WhatsApp` channels.
pub async fn verify_webhook(
    State(resources): State<Arc<ServerContext>>,
    Path(channel): Path<String>,
    Query(query): Query<MetaVerifyQuery>,
) -> Result<impl IntoResponse, AppError> {
    if query.mode != "subscribe" {
        return Err(AppError::invalid_input(format!(
            "Expected hub.mode=subscribe, got: {}",
            query.mode
        )));
    }

    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
    let configs = db.get_configs_by_channel_type(&channel).await?;

    if configs.is_empty() {
        return Err(AppError::auth_invalid(format!(
            "No active channel configuration for {channel}"
        )));
    }

    // Prefer verify_token; fall back to webhook_secret if verify_token is not configured
    let token_matches = configs.iter().any(|config| {
        let stored_verify_token = config.get("verify_token").and_then(|v| v.as_str());
        let stored_webhook_secret = config.get("webhook_secret").and_then(|v| v.as_str());
        stored_verify_token
            .or(stored_webhook_secret)
            .is_some_and(|t| t == query.verify_token)
    });

    if !token_matches {
        warn!(
            channel = %channel,
            "Meta webhook verification failed: verify_token mismatch"
        );
        return Err(AppError::auth_invalid(
            "Webhook verify token does not match",
        ));
    }

    info!(
        channel = %channel,
        "Meta webhook verification successful"
    );

    // Echo challenge as plain text (Meta requires exactly this)
    Ok((StatusCode::OK, query.challenge))
}

/// Handle an inbound webhook from a messaging channel
///
/// Flow:
/// 1. Verify signature and resolve tenant (synchronous, fast)
/// 2. Persist inbound messages (synchronous, fast)
/// 3. Return HTTP 200 immediately
/// 4. Spawn background tasks for LLM dispatch + outbound response
///
/// The `#[instrument]` span is the root span for every messaging turn —
/// `dispatch_and_respond` and the chat pipeline inherit it via
/// `in_current_span` on the spawned task, so structured fields
/// (`channel`, `tenant_id`, `message_count`) propagate to every log line
/// emitted while the turn is in flight.
#[tracing::instrument(
    skip_all,
    fields(
        channel = %channel,
        tenant_id = field::Empty,
        message_count = field::Empty,
    )
)]
pub async fn handle_webhook(
    State(resources): State<Arc<ServerContext>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, AppError> {
    // Slack retries webhooks after 3 seconds with X-Slack-Retry-Num header.
    // DB idempotency prevents duplicates, but rejecting retries early avoids
    // redundant signature verification and DB lookups.
    if channel == "slack" && headers.get("x-slack-retry-num").is_some() {
        let retry_num = headers
            .get("x-slack-retry-num")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?");
        debug!(retry_num, "Acknowledging Slack retry without reprocessing");
        return Ok((
            StatusCode::OK,
            Json(json!({ "status": "ok", "slack_retry_acknowledged": true })),
        ));
    }

    // Slack url_verification must be handled before signature verification.
    // Slack sends this challenge when the admin first configures the Events URL,
    // potentially before the channel config is saved in Pierre's DB.
    if channel == "slack" {
        if let Some(handshake) = detect_handshake_response(&channel, &body) {
            return Ok((StatusCode::OK, Json(handshake)));
        }
    }

    let verification = parse_and_verify(&resources, &channel, &headers, &body).await?;

    // Populate the parent span so downstream log lines (including those
    // emitted from the spawned dispatch task via `in_current_span`) carry
    // these identifiers without each callee needing to repeat them.
    let span = Span::current();
    span.record("tenant_id", field::display(&verification.tenant_id));
    span.record("message_count", verification.messages.len());

    info!(
        channel = %channel,
        tenant_id = %verification.tenant_id,
        message_count = verification.messages.len(),
        "Processed inbound webhook"
    );

    // Persist messages synchronously and collect pending dispatches for background processing
    let (stored_count, pending_dispatches) = messaging_ingress::persist_inbound(
        &resources,
        &channel,
        verification.tenant_id,
        verification.channel_type,
        &verification.adapter,
        &verification.messages,
    )
    .await;

    let message_count = verification.messages.len();

    // Handle platform-specific handshake responses (Slack url_verification, Discord PING)
    if verification.messages.is_empty() {
        if let Some(handshake) = detect_handshake_response(&channel, &body) {
            return Ok((StatusCode::OK, Json(handshake)));
        }
    }

    // Spawn LLM dispatch as background tasks (non-blocking, webhook returns immediately).
    // `in_current_span` carries the webhook handler's span into the detached
    // task so every log line emitted during LLM dispatch (prompt assembly,
    // tool loop, embacle HTTP call) inherits the same `turn_id`, `channel`,
    // and `tenant_id` fields. Without this, the spawned future starts with
    // an empty span and the message-flow trace fragments.
    for dispatch in pending_dispatches {
        tokio::spawn(
            async move {
                messaging_ingress::dispatch_and_respond(dispatch).await;
            }
            .in_current_span(),
        );
    }

    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "messages_received": message_count,
            "messages_stored": stored_count
        })),
    ))
}

/// Detect platform-specific handshake payloads that require a non-generic response
///
/// Slack `url_verification`: must echo the `challenge` value back.
/// Discord PING (interaction type 1): must respond with `{"type": 1}` (PONG).
fn detect_handshake_response(channel: &str, body: &Bytes) -> Option<Value> {
    let payload: Value = serde_json::from_slice(body).ok()?;

    match channel {
        "slack" => {
            if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
                let challenge = payload.get("challenge")?.as_str()?;
                info!(
                    channel = "slack",
                    "Responding to Slack url_verification challenge"
                );
                return Some(json!({ "challenge": challenge }));
            }
            None
        }
        "discord" => {
            if payload.get("type").and_then(Value::as_u64) == Some(1) {
                info!(channel = "discord", "Responding to Discord PING with PONG");
                return Some(json!({ "type": 1 }));
            }
            None
        }
        _ => None,
    }
}

/// Resolve tenant by matching webhook signature against DB channel configs
///
/// Queries all active configs for the channel type, constructs a temporary adapter
/// for each, and tries signature verification. The first config whose signature
/// passes identifies the tenant.
async fn parse_and_verify(
    resources: &ServerContext,
    channel: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<WebhookVerification, AppError> {
    let channel_type = ChannelType::from_str(channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
    let configs = db.get_configs_by_channel_type(channel).await?;

    if configs.is_empty() {
        return Err(AppError::auth_invalid(format!(
            "No active channel configuration for {channel}"
        )));
    }

    // Try each config's signing secret until one verifies
    for config in &configs {
        let enriched = enrich_slack_bot_allow_list(channel_type, config);
        let adapter_config = enriched.as_ref().unwrap_or(config);
        let adapter = match create_adapter_from_config(channel_type, adapter_config) {
            Ok(a) => a,
            Err(e) => {
                debug!(
                    channel = %channel_type,
                    error = %e,
                    "Skipping config with missing credentials"
                );
                continue;
            }
        };

        if adapter.verify_signature(headers, body).is_ok() {
            let tenant_id_str = config
                .get("tenant_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AppError::internal("Channel config missing tenant_id"))?;

            let tenant_id = TenantId::from_str(tenant_id_str).map_err(|_| {
                AppError::internal(format!(
                    "Channel config has invalid tenant_id: {tenant_id_str}"
                ))
            })?;

            let messages = adapter.receive(headers, body).await.map_err(|e| {
                warn!(channel = %channel_type, error = %e, "Failed to parse inbound webhook");
                AppError::invalid_input(format!("Invalid webhook payload: {e}"))
            })?;

            return Ok(WebhookVerification {
                tenant_id,
                channel_type,
                adapter,
                messages,
            });
        }
    }

    warn!(
        channel = %channel_type,
        config_count = configs.len(),
        "No matching channel configuration for webhook signature"
    );
    Err(AppError::auth_invalid("No matching channel configuration"))
}

/// Inject `allowed_bot_ids` from the `SLACK_ALLOWED_BOT_IDS` env var into the
/// Slack config value so canot's factory exposes it on the transport.
///
/// The env var is a comma-separated list of Slack bot IDs (e.g.
/// `"B0ABC123,B0DEF456"`). When set, the listed bots bypass canot's default
/// bot-loop filter and their messages flow into the pipeline as real user
/// input. Intended for QA drivers and trusted integration bots only — see
/// `dravr-canot::channels::slack::SlackTransport::with_allowed_bot_ids` for
/// the security contract. Never list the workspace's own Pierre bot ID.
///
/// Returns `None` when the env var is absent, empty, or the channel isn't
/// Slack, leaving the original config untouched. The caller should fall back
/// to the original config in that case.
fn enrich_slack_bot_allow_list(channel_type: ChannelType, config: &Value) -> Option<Value> {
    if !matches!(channel_type, ChannelType::Slack) {
        return None;
    }
    let raw = env::var("SLACK_ALLOWED_BOT_IDS").ok()?;
    let ids: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if ids.is_empty() {
        return None;
    }
    let mut cloned = config.clone();
    if let Value::Object(map) = &mut cloned {
        map.insert("allowed_bot_ids".to_owned(), json!(ids));
    }
    Some(cloned)
}
