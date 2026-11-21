// ABOUTME: Slack Socket Mode background service bridging WebSocket events to the webhook pipeline
// ABOUTME: Opens WSS via apps.connections.open with the xapp- token, ACKs envelopes, feeds events through persist+dispatch
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::sync::Arc;
use std::time::Duration;

use dravr_canot::channel::MessagingChannel;
use dravr_canot::channels::slack::transport::SlackTransport;
use dravr_canot::error::MessagingResult;
use dravr_canot::transport::TransportAdapter;
use http::HeaderMap;
use pierre_core::models::messaging::{ChannelType, InboundReaction, IncomingMessage};
use pierre_core::models::TenantId;
use pierre_database::repositories::MessagingRepository;
use pierre_services::channel_adapters::ChannelAdapterFactory;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, info, warn};

type SlackWs = WebSocketStream<MaybeTlsStream<TcpStream>>;

use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress;
use crate::services::messaging_ingress::reactions;
use crate::services::ops_slack;
use crate::services::ops_slack::SLACK_API_ORIGIN;

/// The signing secret a Socket Mode adapter is built with when the tenant's
/// Slack config stores none. Socket Mode frames are authed by the `xapp-`
/// token on the connection, never by an HMAC on the body, so nothing on
/// this path verifies with it.
const NO_SIGNING_SECRET: &str = "socket-mode-has-no-signing-secret";

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
///
/// The connection is opened at the origin the context's adapter factory
/// names for Slack (`ChannelAdapterFactory::destination_api_base`), Slack's
/// own host in a deployment, and every reply goes through an adapter that
/// factory builds.
pub fn start_slack_socket_mode(resources: &Arc<ServerContext>) {
    let app_token = match env::var("SLACK_APP_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            info!("Slack Socket Mode: SLACK_APP_TOKEN not set, skipping");
            return;
        }
    };

    let allowed_bot_ids = resources
        .common
        .config
        .webhooks
        .slack_allowed_bot_ids
        .clone();

    let (tx, rx) = mpsc::channel::<IncomingMessage>(256);
    // Reactions ride their own channel rather than the message stream: they
    // rate a reply already sent, so they never open a turn and must not be
    // mistaken for one anywhere downstream.
    let (reaction_tx, reaction_rx) = mpsc::channel::<InboundReaction>(256);

    let producer_allow = allowed_bot_ids.clone();
    let api_origin =
        ops_slack::api_origin(resources).unwrap_or_else(|| SLACK_API_ORIGIN.to_owned());
    tokio::spawn(async move {
        run_reconnect_loop(api_origin, app_token, producer_allow, tx, reaction_tx).await;
        warn!("Slack Socket Mode: producer loop exited");
    });

    let processor_resources = Arc::clone(resources);
    tokio::spawn(async move {
        process_socket_messages(processor_resources, rx).await;
    });

    let reaction_resources = Arc::clone(resources);
    tokio::spawn(async move {
        reactions::consume_reactions(reaction_resources, "Slack Socket Mode", reaction_rx).await;
    });

    info!(
        allowed_bot_count = allowed_bot_ids.len(),
        "Slack Socket Mode: started"
    );
}

/// The two streams one socket frame can feed.
///
/// A frame carries either conversational input or a rating on something
/// already said. They go to different consumers, and keeping them in one
/// struct is what stops every function in the frame path from growing an
/// argument each time the socket learns to carry something new.
struct SocketSinks {
    /// Inbound messages, drained by the LLM dispatch consumer.
    messages: mpsc::Sender<IncomingMessage>,
    /// Emoji reactions, drained by the feedback consumer.
    reactions: mpsc::Sender<InboundReaction>,
}

/// Outer reconnect loop: re-opens the Socket Mode connection on clean
/// disconnects and after transient errors, asking `api_origin` each time.
async fn run_reconnect_loop(
    api_origin: String,
    app_token: String,
    allowed_bot_ids: Vec<String>,
    tx: mpsc::Sender<IncomingMessage>,
    reaction_tx: mpsc::Sender<InboundReaction>,
) {
    let sinks = SocketSinks {
        messages: tx,
        reactions: reaction_tx,
    };
    loop {
        match connect_and_stream(&api_origin, &app_token, &allowed_bot_ids, &sinks).await {
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
    api_origin: &str,
    app_token: &str,
    allowed_bot_ids: &[String],
    sinks: &SocketSinks,
) -> Result<(), String> {
    use futures_util::StreamExt;

    let wss_url = open_connection(api_origin, app_token).await?;
    debug!("Slack Socket Mode: opening WSS connection");

    let (mut ws, _) = tokio_tungstenite::connect_async(&wss_url)
        .await
        .map_err(|e| format!("WSS connect failed: {e}"))?;

    while let Some(frame) = StreamExt::next(&mut ws).await {
        match handle_ws_frame(frame, &mut ws, allowed_bot_ids, sinks).await? {
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
    sinks: &SocketSinks,
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

    handle_envelope(&envelope, ws, allowed_bot_ids, sinks).await
}

/// Branch on the envelope's `type` field. Split from `handle_ws_frame` so
/// each function stays under clippy's cognitive-complexity threshold.
async fn handle_envelope(
    envelope: &Value,
    ws: &mut SlackWs,
    allowed_bot_ids: &[String],
    sinks: &SocketSinks,
) -> Result<FrameOutcome, String> {
    let frame_type = envelope.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match frame_type {
        "hello" => {
            info!("Slack Socket Mode: hello received");
            Ok(FrameOutcome::Continue)
        }
        "events_api" => handle_events_api(envelope, ws, allowed_bot_ids, sinks).await,
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
    sinks: &SocketSinks,
) -> Result<FrameOutcome, String> {
    use futures_util::SinkExt;

    if let Some(envelope_id) = envelope.get("envelope_id").and_then(|v| v.as_str()) {
        let ack = json!({ "envelope_id": envelope_id });
        SinkExt::<Message>::send(ws, Message::text(ack.to_string()))
            .await
            .map_err(|e| format!("ack send failed: {e}"))?;
    }
    if let Some(payload) = envelope.get("payload") {
        deliver_payload(payload, allowed_bot_ids, sinks).await;
    }
    Ok(FrameOutcome::Continue)
}

/// POST to `apps.connections.open` at `api_origin`, a scheme and authority,
/// and return the ephemeral WebSocket URL.
///
/// Slack auths this endpoint with the app-level `xapp-…` token via an HTTP
/// Bearer header; the response carries a short-lived WSS URL suitable for
/// immediate `connect_async`.
async fn open_connection(api_origin: &str, app_token: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("http client build failed: {e}"))?;

    let response: Value = client
        .post(format!("{api_origin}/api/apps.connections.open"))
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

/// Reuse canot's existing Slack parsers on the socket payload so both ingress
/// paths read a frame identically — the same bot-filter allow-list on
/// messages, the same reaction events.
///
/// The socket `payload` field carries the same Events API shape that
/// [`SlackTransport::parse_inbound`] and its reaction parser expect, so we
/// serialize back to bytes, construct a transport with the allow-list, and
/// feed it through. The `signing_secret` argument is unused here —
/// socket-mode frames are authed by the xapp- token on the connection, not
/// HMAC on the body.
///
/// Reactions go first: they rate a reply the athlete already has, so they
/// never wait behind a turn.
async fn deliver_payload(payload: &Value, allowed_bot_ids: &[String], sinks: &SocketSinks) {
    for reaction in parse_socket_reactions(payload).await {
        if sinks.reactions.send(reaction).await.is_err() {
            warn!("Slack Socket Mode: reaction consumer channel closed, dropping reaction");
            break;
        }
    }

    let Some(messages) = parse_socket_payload(payload, allowed_bot_ids).await else {
        return;
    };
    for msg in messages {
        if sinks.messages.send(msg).await.is_err() {
            warn!("Slack Socket Mode: consumer channel closed, dropping message");
            return;
        }
    }
}

/// Hand the socket payload to canot's Slack reaction parser.
///
/// Socket Mode carries `reaction_added` / `reaction_removed` in the same
/// Events API shape the webhook posts, so the same parser reads both. The
/// descriptor is consulted rather than assumed — the gate is the channel's own
/// declaration everywhere the platform parses reactions. A payload that is not
/// a reaction event yields an empty list, which is the common case.
async fn parse_socket_reactions(payload: &Value) -> Vec<InboundReaction> {
    if !reactions::channel_delivers_reactions(ChannelType::Slack) {
        return Vec::new();
    }
    let transport = SlackTransport::new(NO_SIGNING_SECRET.to_owned());
    let body = match serde_json::to_vec(payload) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "Slack Socket Mode: re-serialize payload failed");
            return Vec::new();
        }
    };
    match transport.parse_reactions(&HeaderMap::new(), &body).await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "Slack Socket Mode: parse_reactions failed");
            Vec::new()
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
        NO_SIGNING_SECRET.to_owned(),
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
        let Some((tenant_id, adapter)) = resolve_slack_config(&resources).await else {
            continue;
        };
        dispatch_message(&resources, tenant_id, adapter, message).await;
    }
    info!("Slack Socket Mode: consumer stopped");
}

/// Resolve the Slack `ChannelConfig` row and build its adapter through the
/// context's factory, the one every other Slack sender builds through.
///
/// The `tenant_id` comes from the row so the ingress layer can scope
/// tenant-owned conversation state.
async fn resolve_slack_config(
    resources: &ServerContext,
) -> Option<(TenantId, Arc<dyn MessagingChannel>)> {
    let data = resources.data();
    let db: &dyn MessagingRepository = data.repos().messaging.as_ref();
    let configs = db.get_configs_by_channel_type("slack").await.ok()?;
    let config = configs.first()?;
    let tenant_id_str = config.get("tenant_id").and_then(|v| v.as_str())?;
    let tenant_id = TenantId::parse_str(tenant_id_str).ok()?;
    match socket_adapter(resources.mcp.channel_adapters.as_ref(), config) {
        Ok(adapter) => Some((tenant_id, adapter)),
        Err(e) => {
            warn!(
                tenant_id = %tenant_id,
                error = %e,
                "Slack Socket Mode: the stored Slack config builds no adapter"
            );
            None
        }
    }
}

/// The adapter `factory` builds for a stored Slack `config`.
///
/// A Socket Mode tenant may store no signing secret: its frames are authed
/// on the connection, and the adapter only sends. The factory requires one,
/// so a config without it is handed [`NO_SIGNING_SECRET`], which nothing on
/// this path verifies with.
fn socket_adapter(
    factory: &dyn ChannelAdapterFactory,
    config: &Value,
) -> MessagingResult<Arc<dyn MessagingChannel>> {
    let has_secret = config
        .get("webhook_secret")
        .and_then(Value::as_str)
        .is_some_and(|secret| !secret.is_empty());
    if has_secret {
        return factory.build(ChannelType::Slack, config);
    }
    let mut with_secret = config.clone();
    if let Some(fields) = with_secret.as_object_mut() {
        fields.insert("webhook_secret".to_owned(), json!(NO_SIGNING_SECRET));
    }
    factory.build(ChannelType::Slack, &with_secret)
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
        // The placeholder opens where the factory's adapters send: the
        // channel's real API in a deployment, as every turn does.
        resources.mcp.channel_adapters.status_api_base().as_deref(),
    )
    .await;

    // Recorded and handed to the runner exactly like webhook turns: Slack socket-mode
    // turns are detached from the event that started them and vanish the
    // same way on a scaledown, and the runner is what makes them survive it
    // (registre#126).
    for dispatch in pending_dispatches {
        messaging_ingress::start_turn(resources, dispatch).await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use pierre_services::channel_adapters::ConfigChannelAdapters;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// A loopback Slack that answers `apps.connections.open` with a socket
    /// URL, and keeps each request's first line.
    async fn stand_in() -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&asked);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut head = Vec::new();
                let mut chunk = [0_u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(read) if read > 0 => head.extend_from_slice(&chunk[..read]),
                        _ => break,
                    }
                }
                let line = String::from_utf8_lossy(&head)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                record.lock().unwrap().push(line);
                let body = r#"{"ok":true,"url":"wss://socket.example/link"}"#;
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                             content-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .ok();
            }
        });
        (origin, asked)
    }

    #[tokio::test]
    async fn the_connection_is_opened_at_the_origin_it_is_given() {
        let (origin, asked) = stand_in().await;
        let url = open_connection(&origin, "xapp-socket-test").await;
        assert_eq!(
            url.as_deref(),
            Ok("wss://socket.example/link"),
            "bug: the socket URL the origin answered was not returned"
        );
        assert_eq!(
            *asked.lock().unwrap(),
            ["POST /api/apps.connections.open HTTP/1.1"],
            "bug: the connection was not opened at the origin given"
        );
    }

    /// A tenant that stores no signing secret keeps its Socket Mode replies:
    /// the factory, which requires one, is handed the placeholder.
    #[test]
    fn a_config_without_a_signing_secret_still_builds_an_adapter() {
        let factory = ConfigChannelAdapters;
        for config in [
            json!({ "tenant_id": "t" }),
            json!({ "tenant_id": "t", "webhook_secret": "" }),
            json!({ "tenant_id": "t", "webhook_secret": "stored-secret" }),
        ] {
            assert_eq!(
                socket_adapter(&factory, &config)
                    .ok()
                    .map(|adapter| adapter.channel_type()),
                Some(ChannelType::Slack),
                "bug: no Slack adapter for {config}"
            );
        }
        assert!(
            factory.build(ChannelType::Slack, &json!({})).is_err(),
            "the factory itself requires a signing secret"
        );
    }
}
