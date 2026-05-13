// ABOUTME: Slack Socket Mode background service bridging WebSocket events to the webhook pipeline
// ABOUTME: Opens WSS via apps.connections.open with the xapp- token, ACKs envelopes, feeds events through persist+dispatch
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use http::HeaderMap;
use pierre_core::models::messaging::{ChannelType, IncomingMessage};
use pierre_core::models::TenantId;
use pierre_database::repositories::MessagingRepository;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::channels::slack::transport::SlackTransport;
use pierre_messaging::channels::slack::SlackChannel;
use pierre_messaging::transport::TransportAdapter;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, info, warn};

type SlackWs = WebSocketStream<MaybeTlsStream<TcpStream>>;

use crate::context::DataContext;
use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress;

/// Start the Slack Socket Mode background service.
///
/// Reads `SLACK_APP_TOKEN` (the `xapp-…` app-level token) from env and, when
/// present, opens a WebSocket to Slack via `apps.connections.open`, ACKs each
/// `events_api` envelope, and routes the parsed message through the same
/// persist → LLM dispatch → reply pipeline as webhook ingress. A
/// comma-separated `SLACK_ALLOWED_BOT_IDS` is forwarded to the canot parser
/// so QA drivers and trusted integration bots can reach the pipeline — see
/// `routes::messaging::webhooks::enrich_slack_bot_allow_list` for the
/// parallel webhook-side knob. When the token is absent, this function is a
/// no-op, and the webhook path keeps working unchanged.
pub fn start_slack_socket_mode(resources: &Arc<ServerContext>) {
    let app_token = match env::var("SLACK_APP_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            info!("Slack Socket Mode: SLACK_APP_TOKEN not set, skipping");
            return;
        }
    };

    let allowed_bot_ids = env::var("SLACK_ALLOWED_BOT_IDS")
        .ok()
        .as_deref()
        .map(parse_allowed_bot_ids)
        .unwrap_or_default();

    let (tx, rx) = mpsc::channel::<IncomingMessage>(256);

    let producer_allow = allowed_bot_ids.clone();
    tokio::spawn(async move {
        run_reconnect_loop(app_token, producer_allow, tx).await;
        warn!("Slack Socket Mode: producer loop exited");
    });

    let processor_resources = Arc::clone(resources);
    tokio::spawn(async move {
        process_socket_messages(processor_resources, rx).await;
    });

    info!(
        allowed_bot_count = allowed_bot_ids.len(),
        "Slack Socket Mode: started"
    );
}

/// Parse a comma-separated list of Slack bot IDs into a clean `Vec`.
///
/// Returns an empty `Vec` when `raw` contains no non-empty entries. The
/// caller is responsible for sourcing `raw` (typically from
/// `SLACK_ALLOWED_BOT_IDS`); accepting a `&str` keeps the function pure
/// for unit testing. Matches the webhook-side parser in
/// `routes::messaging::webhooks::enrich_slack_bot_allow_list`.
pub fn parse_allowed_bot_ids(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Outer reconnect loop: re-opens the Socket Mode connection on clean
/// disconnects and after transient errors.
async fn run_reconnect_loop(
    app_token: String,
    allowed_bot_ids: Vec<String>,
    tx: mpsc::Sender<IncomingMessage>,
) {
    loop {
        match connect_and_stream(&app_token, &allowed_bot_ids, &tx).await {
            Ok(()) => {
                info!("Slack Socket Mode: disconnected cleanly, reconnecting");
            }
            Err(e) => {
                warn!(error = %e, "Slack Socket Mode: connection errored, reconnecting in 5s");
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// Next action emitted from a single socket frame.
enum FrameOutcome {
    /// Nothing for the caller to do — log-only frames or silent ACKs.
    Continue,
    /// Slack requested a disconnect; return to the reconnect loop.
    Disconnect,
}

/// One connect → stream → disconnect cycle. Returns `Ok(())` for a clean
/// server-side disconnect and `Err` for transport or protocol errors that
/// warrant a backoff before retrying.
async fn connect_and_stream(
    app_token: &str,
    allowed_bot_ids: &[String],
    tx: &mpsc::Sender<IncomingMessage>,
) -> Result<(), String> {
    use futures_util::StreamExt;

    let wss_url = open_connection(app_token).await?;
    debug!("Slack Socket Mode: opening WSS connection");

    let (mut ws, _) = tokio_tungstenite::connect_async(&wss_url)
        .await
        .map_err(|e| format!("WSS connect failed: {e}"))?;

    while let Some(frame) = StreamExt::next(&mut ws).await {
        match handle_ws_frame(frame, &mut ws, allowed_bot_ids, tx).await? {
            FrameOutcome::Continue => {}
            FrameOutcome::Disconnect => return Ok(()),
        }
    }
    Ok(())
}

/// Per-frame dispatch. Keeps the outer loop small and hits every terminal
/// branch (close, pong, unknown) so silence never looks like success.
async fn handle_ws_frame(
    frame: Result<Message, WsError>,
    ws: &mut SlackWs,
    allowed_bot_ids: &[String],
    tx: &mpsc::Sender<IncomingMessage>,
) -> Result<FrameOutcome, String> {
    use futures_util::SinkExt;

    let text = match frame {
        Ok(Message::Text(t)) => t,
        Ok(Message::Ping(payload)) => {
            SinkExt::<Message>::send(ws, Message::Pong(payload))
                .await
                .map_err(|e| format!("pong send failed: {e}"))?;
            return Ok(FrameOutcome::Continue);
        }
        Ok(Message::Close(_) | Message::Pong(_) | Message::Frame(_) | Message::Binary(_)) => {
            return Ok(FrameOutcome::Continue);
        }
        Err(e) => return Err(format!("ws frame error: {e}")),
    };

    let envelope: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, frame = %text, "Slack Socket Mode: invalid JSON frame");
            return Ok(FrameOutcome::Continue);
        }
    };

    handle_envelope(&envelope, ws, allowed_bot_ids, tx).await
}

/// Branch on the envelope's `type` field. Split from `handle_ws_frame` so
/// each function stays under clippy's cognitive-complexity threshold.
async fn handle_envelope(
    envelope: &Value,
    ws: &mut SlackWs,
    allowed_bot_ids: &[String],
    tx: &mpsc::Sender<IncomingMessage>,
) -> Result<FrameOutcome, String> {
    let frame_type = envelope.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match frame_type {
        "hello" => {
            info!("Slack Socket Mode: hello received");
            Ok(FrameOutcome::Continue)
        }
        "events_api" => handle_events_api(envelope, ws, allowed_bot_ids, tx).await,
        "disconnect" => {
            let reason = envelope.get("reason").and_then(|v| v.as_str());
            info!(reason = ?reason, "Slack Socket Mode: server-side disconnect");
            Ok(FrameOutcome::Disconnect)
        }
        other => {
            debug!(frame_type = other, "Slack Socket Mode: ignoring frame");
            Ok(FrameOutcome::Continue)
        }
    }
}

/// ACK the envelope and deliver its payload. The 3s ACK SLA is preserved
/// by sending the ACK frame before any downstream work.
async fn handle_events_api(
    envelope: &Value,
    ws: &mut SlackWs,
    allowed_bot_ids: &[String],
    tx: &mpsc::Sender<IncomingMessage>,
) -> Result<FrameOutcome, String> {
    use futures_util::SinkExt;

    if let Some(envelope_id) = envelope.get("envelope_id").and_then(|v| v.as_str()) {
        let ack = json!({ "envelope_id": envelope_id });
        SinkExt::<Message>::send(ws, Message::Text(ack.to_string()))
            .await
            .map_err(|e| format!("ack send failed: {e}"))?;
    }
    if let Some(payload) = envelope.get("payload") {
        deliver_payload(payload, allowed_bot_ids, tx).await;
    }
    Ok(FrameOutcome::Continue)
}

/// POST to `apps.connections.open` and return the ephemeral WebSocket URL.
///
/// Slack auths this endpoint with the app-level `xapp-…` token via an HTTP
/// Bearer header; the response carries a short-lived WSS URL suitable for
/// immediate `connect_async`.
async fn open_connection(app_token: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("http client build failed: {e}"))?;

    let response: Value = client
        .post("https://slack.com/api/apps.connections.open")
        .bearer_auth(app_token)
        .send()
        .await
        .map_err(|e| format!("apps.connections.open transport error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("apps.connections.open decode error: {e}"))?;

    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        let err = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(format!("apps.connections.open rejected: {err}"));
    }

    response
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "apps.connections.open response missing `url`".to_owned())
}

/// Reuse canot's existing Slack event parser on the socket payload so the
/// bot-filter allow-list applies identically on both ingress paths.
///
/// The socket `payload` field carries the same Events API shape that
/// [`SlackTransport::parse_inbound`] expects, so we serialize back to bytes,
/// construct a transport with the allow-list, and feed it through. The
/// `signing_secret` argument is unused here — socket-mode frames are authed
/// by the xapp- token on the connection, not HMAC on the body.
async fn deliver_payload(
    payload: &Value,
    allowed_bot_ids: &[String],
    tx: &mpsc::Sender<IncomingMessage>,
) {
    let Some(messages) = parse_socket_payload(payload, allowed_bot_ids).await else {
        return;
    };
    for msg in messages {
        if tx.send(msg).await.is_err() {
            warn!("Slack Socket Mode: consumer channel closed, dropping message");
            return;
        }
    }
}

/// Re-serialize the socket payload and hand it to canot's parser. Returns
/// `None` on any serialize/parse failure (already logged), so the caller
/// can early-out cleanly.
pub async fn parse_socket_payload(
    payload: &Value,
    allowed_bot_ids: &[String],
) -> Option<Vec<IncomingMessage>> {
    let transport = SlackTransport::with_allowed_bot_ids(
        "socket-mode-has-no-signing-secret".to_owned(),
        allowed_bot_ids.to_vec(),
    );
    let body = match serde_json::to_vec(payload) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "Slack Socket Mode: re-serialize payload failed");
            return None;
        }
    };
    match transport.parse_inbound(&HeaderMap::new(), &body).await {
        Ok(m) => Some(m),
        Err(e) => {
            warn!(error = %e, "Slack Socket Mode: parse_inbound failed");
            None
        }
    }
}

/// Consumer task: drains the mpsc and routes each message through the same
/// persist + dispatch path the webhook handler uses.
async fn process_socket_messages(
    resources: Arc<ServerContext>,
    mut rx: mpsc::Receiver<IncomingMessage>,
) {
    while let Some(message) = rx.recv().await {
        info!(
            sender_id = %message.sender_id,
            channel_message_id = %message.channel_message_id,
            "Slack Socket Mode: received message"
        );
        let Some((tenant_id, adapter)) = resolve_slack_config(&resources.data()).await else {
            continue;
        };
        dispatch_message(&resources, tenant_id, adapter, message).await;
    }
    info!("Slack Socket Mode: consumer stopped");
}

/// Resolve the Slack `ChannelConfig` row and build a fresh adapter.
///
/// Unlike the Discord gateway we don't rely on the factory's signing secret
/// because outbound posts need the Slack bot token, not the signing secret.
/// We still pull the `tenant_id` from the row so the ingress layer can scope
/// tenant-owned conversation state.
async fn resolve_slack_config(data: &DataContext) -> Option<(TenantId, Arc<dyn MessagingChannel>)> {
    let db: &dyn MessagingRepository = data.repos().messaging.as_ref();
    let configs = db.get_configs_by_channel_type("slack").await.ok()?;
    let config = configs.first()?;
    let tenant_id_str = config.get("tenant_id").and_then(|v| v.as_str())?;
    let tenant_id = TenantId::from_str(tenant_id_str).ok()?;
    let signing_secret = config
        .get("webhook_secret")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let adapter: Arc<dyn MessagingChannel> = Arc::new(SlackChannel::new(signing_secret));
    Some((tenant_id, adapter))
}

/// Persist the message and spawn LLM dispatch tasks — mirrors the tail of
/// the webhook handler so socket-mode and webhook ingress produce identical
/// downstream behaviour.
async fn dispatch_message(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    adapter: Arc<dyn MessagingChannel>,
    message: IncomingMessage,
) {
    let messages = vec![message];

    let (_stored, pending_dispatches) = messaging_ingress::persist_inbound(
        resources,
        "slack",
        tenant_id,
        ChannelType::Slack,
        &adapter,
        &messages,
    )
    .await;

    for dispatch in pending_dispatches {
        tokio::spawn(async move {
            messaging_ingress::dispatch_and_respond(dispatch).await;
        });
    }
}
