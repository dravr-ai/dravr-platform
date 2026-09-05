// ABOUTME: Webhook ingress handler for multi-channel messaging gateway
// ABOUTME: HTTP wiring layer that delegates business logic to services::messaging_ingress
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Extension;
use axum::Json;
use pierre_core::models::messaging::{ChannelType, InboundReaction, IncomingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_messaging::channel::MessagingChannel;
use pierre_middleware::redaction::mask_recipient;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, error, field, info, warn, Span};

use super::adapter_factory::ChannelAdapterFactory;
use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress;
use crate::services::messaging_ingress::reactions;
use pierre_core::errors::AppError;

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
    /// Emoji reactions parsed from the same verified body. Always empty for a
    /// channel whose descriptor delivers none.
    reactions: Vec<InboundReaction>,
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
///
/// # Errors
/// Returns an error when `hub.mode` isn't `subscribe`, no channel config exists
/// for the channel, or the presented verify token matches none of them.
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

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
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
///
/// # Errors
/// Returns an error when the webhook signature verifies against no channel
/// config (or ambiguously matches multiple tenants) or the payload can't be parsed.
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
    Extension(adapters): Extension<Arc<dyn ChannelAdapterFactory>>,
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

    let verification =
        parse_and_verify(&resources, adapters.as_ref(), &channel, &headers, &body).await?;

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

    // Surface Meta `WhatsApp` delivery-status callbacks (sent/delivered/read/failed).
    // These arrive under `value.statuses[]`, which the channel adapter's `receive`
    // drops (it parses only inbound messages), so a FAILED async push — a
    // backfill-ready notice or reconnect nudge that Meta accepted but never
    // delivered — was previously invisible (a silent `message_count=0` webhook).
    log_whatsapp_delivery_statuses(&channel, &body);

    // Persist messages synchronously and collect pending dispatches for background processing
    let (stored_count, pending_dispatches) = messaging_ingress::persist_inbound(
        &resources,
        &channel,
        verification.tenant_id,
        verification.channel_type,
        &verification.adapter,
        &verification.messages,
        adapters.status_api_base().as_deref(),
    )
    .await;

    let message_count = verification.messages.len();

    // Handle platform-specific handshake responses (Slack url_verification, Discord PING)
    if verification.messages.is_empty() {
        if let Some(handshake) = detect_handshake_response(&channel, &body) {
            return Ok((StatusCode::OK, Json(handshake)));
        }
    }

    // Reactions are rated against messages already sent, so they resolve
    // against the database this webhook just read — no LLM turn, no reply.
    // Applied before the dispatch spawn so a payload carrying only reactions
    // finishes its work inside the request.
    if !verification.reactions.is_empty() {
        messaging_ingress::reactions::apply_reactions(&resources, &verification.reactions).await;
    }

    // Each turn is recorded and handed to the runner before the 200 below
    // lands: in-process it is claimed and spawned through `common.turns`, on
    // GCP it is enqueued on Cloud Tasks and runs inside the request Cloud
    // Tasks delivers — a request Cloud Run waits for, which a detached task
    // never was (registre#126). The webhook's own span rides along so every
    // log line of the turn carries the same `turn_id`, `channel` and
    // `tenant_id` fields.
    for dispatch in pending_dispatches {
        messaging_ingress::start_turn(&resources, dispatch).await;
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

/// A Meta `WhatsApp` delivery-status callback (`sent` / `delivered` / `read` /
/// `failed`).
///
/// Meta posts these to the same webhook as inbound messages but under
/// `value.statuses[]` rather than `value.messages[]`, so the channel adapter's
/// `receive` — which only extracts inbound user messages — silently drops them.
/// Surfacing them makes a failed async push (backfill-ready notice, reconnect
/// nudge) VISIBLE instead of a `message_count=0` no-op webhook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhatsappDeliveryStatus {
    /// `sent` | `delivered` | `read` | `failed`.
    pub status: String,
    /// The recipient's `WhatsApp` id (a phone number — masked at log time).
    pub recipient_id: String,
    /// Meta's `wamid` for the message this status refers to.
    pub message_id: String,
    /// Meta error code, present only when `status == "failed"` (e.g. `131047`
    /// re-engagement / out-of-24h-window).
    pub error_code: Option<i64>,
    /// Human-readable Meta error title, present only on `failed`.
    pub error_title: Option<String>,
}

/// Parse Meta `WhatsApp` `value.statuses[]` delivery receipts from a raw webhook
/// body. Returns an empty vec for inbound-message webhooks, non-`WhatsApp`
/// payloads, or unparseable bodies.
#[must_use]
pub fn parse_whatsapp_delivery_statuses(body: &[u8]) -> Vec<WhatsappDeliveryStatus> {
    let Ok(payload) = serde_json::from_slice::<Value>(body) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in payload
        .get("entry")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for change in entry
            .get("changes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(statuses) = change.pointer("/value/statuses").and_then(Value::as_array) else {
                continue;
            };
            for s in statuses {
                let err = s
                    .get("errors")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first());
                out.push(WhatsappDeliveryStatus {
                    status: s
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    recipient_id: s
                        .get("recipient_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    message_id: s
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    error_code: err.and_then(|e| e.get("code")).and_then(Value::as_i64),
                    error_title: err
                        .and_then(|e| e.get("title"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                });
            }
        }
    }
    out
}

/// Log Meta `WhatsApp` delivery-status callbacks so a FAILED async push (one Meta
/// accepted but never delivered) is observable instead of silent. No-op for
/// non-`WhatsApp` channels and inbound-message webhooks.
fn log_whatsapp_delivery_statuses(channel: &str, body: &Bytes) {
    if channel != "whatsapp" {
        return;
    }
    // Re-parses the raw body: the upstream parse lives inside the channel
    // adapter's `receive` (which yields only inbound messages, not the raw
    // `Value`) and the handshake probe runs only for Slack/Discord, so no
    // already-parsed body `Value` is available at this call site to reuse.
    for s in parse_whatsapp_delivery_statuses(body) {
        let recipient = mask_recipient(&s.recipient_id);
        if s.status == "failed" {
            warn!(
                channel = "whatsapp",
                recipient = %recipient,
                message_id = %s.message_id,
                error_code = s.error_code.unwrap_or_default(),
                error = s.error_title.as_deref().unwrap_or(""),
                "WhatsApp delivery FAILED — an outbound message (async push/reply) did not reach the user"
            );
        } else {
            debug!(
                channel = "whatsapp",
                recipient = %recipient,
                message_id = %s.message_id,
                status = %s.status,
                "WhatsApp delivery status"
            );
        }
    }
}

/// Resolve tenant by matching webhook signature against DB channel configs
///
/// Queries all active configs for the channel type, constructs a temporary adapter
/// for each, and tries signature verification. The first config whose signature
/// passes identifies the tenant.
async fn parse_and_verify(
    resources: &ServerContext,
    adapters: &dyn ChannelAdapterFactory,
    channel: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<WebhookVerification, AppError> {
    let channel_type = ChannelType::from_str(channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let configs = db.get_configs_by_channel_type(channel).await?;

    if configs.is_empty() {
        return Err(AppError::auth_invalid(format!(
            "No active channel configuration for {channel}"
        )));
    }

    // Collect every config whose signing secret verifies this webhook. With a
    // correctly de-duplicated channel identity exactly one tenant should match.
    // If the signature verifies for configs owned by MORE THAN ONE tenant, two
    // tenants share an identity+secret and routing would be nondeterministic
    // (a cross-tenant message-leak path) — fail closed rather than route to
    // whichever config the DB happened to return first.
    let matches = collect_verified_matches(adapters, channel_type, &configs, headers, body)?;

    let distinct_tenants: HashSet<TenantId> = matches.iter().map(|(t, _)| *t).collect();
    if distinct_tenants.len() > 1 {
        error!(
            channel = %channel_type,
            match_count = matches.len(),
            tenant_count = distinct_tenants.len(),
            "Webhook signature verified for configs owned by multiple tenants — ambiguous cross-tenant routing, rejecting"
        );
        return Err(AppError::auth_invalid(
            "Ambiguous channel configuration: identity claimed by multiple tenants",
        ));
    }

    let Some((tenant_id, adapter)) = matches.into_iter().next() else {
        warn!(
            channel = %channel_type,
            config_count = configs.len(),
            "No matching channel configuration for webhook signature"
        );
        return Err(AppError::auth_invalid("No matching channel configuration"));
    };

    let messages = adapter.receive(headers, body).await.map_err(|e| {
        warn!(channel = %channel_type, error = %e, "Failed to parse inbound webhook");
        AppError::invalid_input(format!("Invalid webhook payload: {e}"))
    })?;

    let reactions = parse_reactions(channel_type, adapter.as_ref(), headers, body).await;

    Ok(WebhookVerification {
        tenant_id,
        channel_type,
        adapter,
        messages,
        reactions,
    })
}

/// Parse emoji reactions out of the same verified body `receive` just read.
///
/// Reactions are feedback on a message already delivered, not new
/// conversational input, so canot surfaces them on their own call. Channels
/// whose webhook API carries no reaction event are skipped by asking their
/// descriptor, never by matching a channel name — a Meta payload therefore
/// never reaches the reaction mapper at all.
///
/// A payload the reaction parser rejects is logged and treated as carrying no
/// reactions: the messages in the same body have already been parsed and must
/// still be delivered.
async fn parse_reactions(
    channel_type: ChannelType,
    adapter: &dyn MessagingChannel,
    headers: &HeaderMap,
    body: &Bytes,
) -> Vec<InboundReaction> {
    if !reactions::channel_delivers_reactions(channel_type) {
        return Vec::new();
    }
    match adapter.receive_reactions(headers, body).await {
        Ok(reactions) => reactions,
        Err(e) => {
            warn!(channel = %channel_type, error = %e, "Failed to parse inbound reactions");
            Vec::new()
        }
    }
}

/// A verified webhook config: its owning tenant paired with the constructed
/// channel adapter whose signature check passed.
type VerifiedMatch = (TenantId, Arc<dyn MessagingChannel>);

/// Build adapters for each active config and return those whose signing secret
/// verifies the webhook, paired with their owning tenant. Configs with missing
/// credentials are skipped.
fn collect_verified_matches(
    adapters: &dyn ChannelAdapterFactory,
    channel_type: ChannelType,
    configs: &[Value],
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Vec<VerifiedMatch>, AppError> {
    let mut matches: Vec<VerifiedMatch> = Vec::new();
    for config in configs {
        let enriched = enrich_slack_bot_allow_list(channel_type, config);
        let adapter_config = enriched.as_ref().unwrap_or(config);
        let Some(adapter) = adapters.build(channel_type, adapter_config) else {
            continue;
        };

        if adapter.verify_signature(headers, body).is_ok() {
            let tenant_id_str = config
                .get("tenant_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AppError::internal("Channel config missing tenant_id"))?;
            let tenant_id = TenantId::parse_str(tenant_id_str).map_err(|_| {
                AppError::internal(format!(
                    "Channel config has invalid tenant_id: {tenant_id_str}"
                ))
            })?;
            matches.push((tenant_id, adapter));
        }
    }
    Ok(matches)
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
