// ABOUTME: Web chat send-message handler — auth, tenant, surface profile, turn service, egress
// ABOUTME: Owns the HTTP shape of a turn and nothing about the turn itself

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The in-app surface's ingress.
//!
//! Everything a turn *is* — the usage caps, the slash dispatch, the locale,
//! the tenant's own model key, the pipeline, the counters — lives in
//! [`pierre_chat_pipeline::turn_service`], which the messaging dispatcher
//! enters through as well. What is left here is transport: resolve who is
//! asking and under which tenant, name the surface's capabilities, and turn
//! whatever came back into an HTTP body — a JSON document, or one stream of
//! frames carrying the turn's progress, its prose, its blocks, and its
//! outcome. There is no second stream and no per-turn handshake to open one.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_stream::stream;
use axum::extract::{Path, State};
use axum::http::{header::ACCEPT, HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use pierre_core::models::{default_locale, ConversationTurnId, COMMAND_FINISH_REASON};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{field, info, instrument, trace, warn, Span};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use pierre_chat_pipeline::{self as pipeline, ServedTurn};
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_database::database::ConversationRecord;
#[cfg(feature = "client-notifications")]
use pierre_notifications::triggers as notification_triggers;

use super::common::get_tenant_id;
use super::dto::{MessageResponse, SendMessageRequest};
use super::turn_response::{
    message_response, platform_blocks, AssistantResponse, TurnResponse, TurnTelemetryResponse,
};
use pierre_middleware::AuthenticatedUser;

/// HTTP header carrying the client surface identifier.
///
/// Set by first-party Dravr frontends to distinguish web from mobile. It
/// selects the turn's [`pipeline::SurfaceId`], and through it the pipeline
/// span's `channel` dimension, the `PlatformCommandContext.channel_type`
/// field and the persisted conversation origin. Absent reads as the browser,
/// which is the shape an ad-hoc `curl` turn has.
const CLIENT_PLATFORM_HEADER: &str = "x-client-platform";

/// Model label recorded for a turn a slash-command handler answered. Named
/// rather than blank so a telemetry row with no token counts is explicable.
const COMMAND_MODEL: &str = "command";
/// Provider label for the same. The finish reason is
/// [`COMMAND_FINISH_REASON`] — the stamp the persisted rows carry, so the wire
/// and the transcript name a command turn the same way.
const COMMAND_PROVIDER: &str = "platform";

/// Which first-party client sent the turn.
///
/// The header is how the two in-app clients tell themselves apart on the
/// wire; every request `@pierre/api-client` builds carries it. An ad-hoc
/// caller that sets nothing is read as the browser, which is the shape a
/// hand-written `curl` turn has.
fn client_surface(headers: &HeaderMap) -> pipeline::SurfaceId {
    let header = headers
        .get(CLIENT_PLATFORM_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase);
    match header.as_deref() {
        Some("mobile") => pipeline::SurfaceId::Mobile,
        _ => pipeline::SurfaceId::Web,
    }
}

/// The `channel_type` a slash-command handler and the conversation record
/// see.
///
/// Deliberately the wire word (`"web"` / `"mobile"`) rather than the
/// surface's telemetry label: this string is persisted on
/// `chat_conversations.channel_type` and read back by the client's channel
/// badge, which treats anything outside the in-app set as a messaging origin.
/// A telemetry rename must not repaint every in-app conversation with a
/// Telegram-style badge.
const fn channel_type_for(surface: pipeline::SurfaceId) -> &'static str {
    match surface {
        pipeline::SurfaceId::Mobile => "mobile",
        _ => "web",
    }
}

/// Send a message and get a response with MCP tool execution.
///
/// The `#[instrument]` span is the root span for every web/mobile chat
/// turn — same shape as the messaging webhook root span — so an operator
/// can grep a single `turn_id` across web, mobile, and Telegram/Messenger
/// traffic uniformly.
#[instrument(
    skip_all,
    fields(
        channel = field::Empty,
        conversation_id = %conversation_id,
        user_id = field::Empty,
        tenant_id = field::Empty,
        turn_id = field::Empty,
        content_len = request.content.len(),
        is_command = request.content.trim_start().starts_with('/'),
    )
)]
pub async fn send_message(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let user_id_str = auth.user_id.to_string();

    // No provider gate here any more. The 403 existed because the model could
    // not tell "no recent activity" from "no connected provider" and invented
    // the difference; that is now handled where it belongs — the system prompt
    // states the absence outright (`build_provider_context`), the athlete-data
    // verifier contradicts any specific figure asserted without a source, and
    // the dispatch chokepoint refuses every REQUIRES_PROVIDER tool. A
    // providerless athlete gets a coach that says what it cannot see instead of
    // a door.
    // Populate the parent span so downstream pipeline log lines (already
    // instrumented to read `turn_id`/`channel`/`conversation_id`) carry the
    // resolved tenant + user without each callee needing to re-record them.
    let span = Span::current();
    span.record("user_id", field::display(&user_id_str));
    span.record("tenant_id", field::display(&tenant_id.to_string()));

    // Web and mobile are separate identities resolving one shared capability
    // set, so the span, the telemetry rows and the persisted conversation
    // origin all name the client that actually sent the turn.
    let surface = client_surface(&headers);
    span.record("channel", field::display(surface.as_str()));

    info!(
        channel = surface.as_str(),
        conversation_id = %conversation_id,
        content_len = request.content.len(),
        "Processed inbound chat message"
    );
    if tracing::enabled!(tracing::Level::TRACE) {
        trace!(content = %request.content, channel = surface.as_str(), "in-app user message body");
    }

    let turn_id = ConversationTurnId::new();
    span.record("turn_id", field::display(&turn_id));

    // The conversation, loaded once for everything this handler decides from
    // it: the caller's membership (a stranger gets the same 404 every chat
    // route gives), whether the turn is a direct message — which is a fact of
    // the thread, not of the surface: a conversation bound to a coaching group
    // is a group thread even in the app — and the `updated_at` a command
    // reply reports when its rows could not be written.
    let conversation = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation_id, &user_id_str, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    // The athlete's stored preference is the turn service's starting point; it
    // refines it from the language of the message itself.
    let stored_locale = resources
        .common
        .repos
        .users
        .get_global(auth.user_id)
        .await
        .ok()
        .flatten()
        .map_or_else(default_locale, |u| u.locale);

    // The in-app surface: markdown prose, inline Scenes, a plan card, an
    // activity panel, and no transport ceiling. `transport: None` is what
    // selects that shape — there is no canot channel behind this request.
    let profile = pipeline::SurfaceProfile::resolve(&pipeline::SurfaceRequest {
        surface,
        locale: stored_locale,
        transport: None,
        prose_contract: None,
    });

    // SSE branch: when the client sends `Accept: text/event-stream`, return
    // a streaming response so nginx (default 60s `proxy_read_timeout`) sees
    // periodic activity and the turn's terminal frame rides a body that was
    // never idle. Whether any *delta* frame precedes it is the provider's
    // half of the question, answered at dispatch; the channel is opened on
    // the transport's own capability. The blocking JSON path below stays
    // available for clients (tests, scripts) that prefer the simpler shape.
    let wants_sse = profile.render.progressive.has_delta_channel()
        && headers
            .get(ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.contains("text/event-stream"));

    let egress = TurnEgress {
        resources: Arc::clone(&resources),
        conversation,
        user_id: auth.user_id,
        tenant_id,
        turn_id,
        channel_type: channel_type_for(surface).to_owned(),
    };

    if wants_sse {
        return Ok(send_message_sse(SseInputs {
            egress,
            profile,
            request,
        }));
    }

    let start_time = Instant::now();
    let ctx = resources.chat_pipeline_context();
    let served = pipeline::execute(
        &ctx,
        egress.turn_request(&request, pipeline::PipelineHooks::none()),
        &profile,
    )
    .await?;
    let response = egress.into_response_body(served, &request, start_time);

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Everything the egress needs to turn a served turn into an HTTP body.
///
/// One bundle for both branches, so the JSON body a blocking caller reads and
/// the `done` frame a streaming caller reads are produced by the same code and
/// cannot describe the same turn differently.
struct TurnEgress {
    resources: Arc<ServerContext>,
    /// The conversation as it stood when the request arrived.
    conversation: ConversationRecord,
    user_id: Uuid,
    tenant_id: TenantId,
    turn_id: ConversationTurnId,
    channel_type: String,
}

impl TurnEgress {
    /// Shape this request as the turn service's input.
    fn turn_request<'a>(
        &'a self,
        request: &SendMessageRequest,
        hooks: pipeline::PipelineHooks<'a>,
    ) -> pipeline::TurnRequest<'a> {
        pipeline::TurnRequest {
            conversation_id: self.conversation.id.clone(),
            user_id: self.user_id,
            // Web and mobile conversations are per-user by construction: the
            // athlete's own tenant owns the conversation and their tool
            // credentials alike, so there is no bot tenant to diverge from.
            conversation_tenant_id: self.tenant_id,
            tool_tenant_id: self.tenant_id,
            content: request.content.clone(),
            turn_id: self.turn_id,
            // In-app conversations are single-user; no room transcript exists.
            ambient_context: None,
            channel_type: &self.channel_type,
            // A thread bound to a coaching group is a group thread, whichever
            // surface it is read on: `/coach add` binds the group's coach there
            // and `/group …` acts on that group. Unbound, the athlete is alone
            // with the coach.
            is_direct_message: self.conversation.group_id.is_none(),
            // A solo thread is exactly that. The group commands resolve no
            // group here and are refused, instead of being aimed at whichever
            // group the athlete touched last — the messaging DM's answer, where
            // one thread stands for the whole relationship.
            ambient_group_fallback: false,
            // The in-app transcript keeps every command turn, as Telegram does.
            command_persistence: pipeline::CommandPersistence::Always,
            // No channel link on this surface, so nothing for `/logout` to
            // unlink.
            sender_id: None,
            hooks,
        }
    }

    /// Serialize whatever the turn service produced.
    fn into_response_body(
        self,
        served: ServedTurn,
        request: &SendMessageRequest,
        start_time: Instant,
    ) -> TurnResponse {
        // Safe cast: execution time will never exceed u64::MAX milliseconds
        // (~584 million years).
        #[allow(clippy::cast_possible_truncation)]
        let execution_time_ms = start_time.elapsed().as_millis() as u64;
        match served {
            ServedTurn::Pipeline(envelope) => {
                #[cfg(feature = "client-notifications")]
                notify_coach_response(
                    &self.resources,
                    &envelope.conversation,
                    self.user_id,
                    self.tenant_id,
                    &self.conversation.id,
                );
                TurnResponse::from_envelope(*envelope, execution_time_ms)
            }
            ServedTurn::Command { command, quota } => {
                self.command_response(*command, &quota, request)
            }
        }
    }

    /// Serialize a slash-command answer as a turn.
    ///
    /// A command turn is history like any other: the turn service wrote the
    /// `/…` line and the answer to the transcript, both stamped `command`, and
    /// the answer's controls with them — so a reload shows the same reply and
    /// the same buttons, and the next coaching turn's prompt sees none of it.
    /// The ids and `conversation_updated_at` here are those persisted rows'.
    ///
    /// When the rows could not be written the reply is still delivered — the
    /// athlete asked a question and has its answer — under fresh ids the
    /// transcript will not know, with the conversation's pre-turn `updated_at`;
    /// the write failure is logged where it happened.
    fn command_response(
        &self,
        command: pipeline::CommandTurn,
        quota: &pipeline::QuotaState,
        request: &SendMessageRequest,
    ) -> TurnResponse {
        let pipeline::CommandTurn {
            text,
            card_title,
            actions,
            rotated_to,
            persisted,
            ..
        } = command;

        let (user_message, assistant_message, conversation_updated_at) =
            if let Some(rows) = persisted {
                (
                    message_response(rows.user_message),
                    message_response(rows.assistant_message),
                    rows.conversation.updated_at,
                )
            } else {
                let now = Utc::now().to_rfc3339();
                (
                    unpersisted_message("user", request.content.clone(), now.clone()),
                    unpersisted_message("assistant", text.clone(), now),
                    self.conversation.updated_at.clone(),
                )
            };

        // A card's title labels its controls, so it rides the actions block
        // rather than being pre-folded onto the front of the body.
        let blocks = platform_blocks(text, card_title, actions, quota);
        TurnResponse {
            turn_id: self.turn_id.to_string(),
            user_message,
            assistant: AssistantResponse {
                message: assistant_message,
                blocks,
                finish_reason: Some(COMMAND_FINISH_REASON.to_owned()),
            },
            conversation_updated_at,
            rotated_to_conversation_id: rotated_to,
            telemetry: TurnTelemetryResponse {
                model: COMMAND_MODEL.to_owned(),
                provider_name: COMMAND_PROVIDER.to_owned(),
                tool_calls_count: 0,
                tools_called: Vec::new(),
                execution_time_ms: 0,
            },
        }
    }
}

/// A command-turn message the transcript does not hold, rendered for the
/// reply that is still owed to the athlete.
fn unpersisted_message(role: &str, content: String, created_at: String) -> MessageResponse {
    MessageResponse {
        id: Uuid::new_v4().to_string(),
        role: role.to_owned(),
        content,
        token_count: None,
        scene_blocks: None,
        finish_reason: Some(COMMAND_FINISH_REASON.to_owned()),
        actions: None,
        created_at,
    }
}

/// Inputs threaded into [`send_message_sse`].
struct SseInputs {
    egress: TurnEgress,
    profile: pipeline::SurfaceProfile,
    request: SendMessageRequest,
}

/// Streaming branch of [`send_message`].
///
/// Spawns the turn on a background task with a [`pipeline::TurnEventSink`]
/// installed on [`pipeline::PipelineHooks`], then returns an SSE response
/// whose body is that channel, frame for frame. One ordered channel carries
/// the whole turn — the progress the pipeline reports, the text the model
/// produces, the blocks the reply resolved to, and the terminal event — so
/// there is nothing to interleave and nothing to correlate:
///
/// - `event: progress` — a stage entered/left, or a tool call's latest state
/// - `event: delta` — the next slice of assistant prose
/// - `event: block` — one renderable piece of the finished reply
/// - `event: done` — the whole [`TurnResponse`], identical to the body the
///   blocking branch returns
/// - `event: failed` — the sanitized reason the turn did not finish
///
/// The frame names live on [`pipeline::TurnEvent::frame`], not here, so the
/// producer's vocabulary and the wire cannot drift.
///
/// A 15-second SSE keep-alive prevents nginx (default 60s
/// `proxy_read_timeout`) from dropping the connection during long LLM-side
/// stalls between events.
fn send_message_sse(inputs: SseInputs) -> Response {
    let SseInputs {
        egress,
        profile,
        request,
    } = inputs;
    let start_time = Instant::now();

    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<pipeline::TurnEvent>();

    let terminal_tx = events_tx.clone();
    tokio::spawn(async move {
        let hooks = pipeline::PipelineHooks {
            stream_sink: Some(events_tx),
            ..pipeline::PipelineHooks::none()
        };
        let ctx = egress.resources.chat_pipeline_context();
        let outcome =
            match pipeline::execute(&ctx, egress.turn_request(&request, hooks), &profile).await {
                Ok(served) => Ok(egress.into_response_body(served, &request, start_time)),
                Err(e) => Err(e),
            };
        for event in terminal_events(outcome) {
            // A send error means the client hung up; the turn is already
            // persisted, so there is nothing left to do about it.
            let _ = terminal_tx.send(event);
        }
    });

    let body = stream! {
        while let Some(event) = events_rx.recv().await {
            let terminal = event.is_terminal();
            let (name, data) = event.frame();
            yield Ok::<_, Infallible>(Event::default().event(name).data(data));
            if terminal {
                break;
            }
        }
    };

    Sse::new(body)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response()
}

/// The tail of a streamed turn: each reply block in order, then one terminal
/// event.
///
/// A block also rides inside the `done` envelope — the same relationship a
/// prose delta has with the assistant message it accumulates into. The frames
/// are what a client draws as the turn lands; the envelope is the durable
/// record it stores.
fn terminal_events(outcome: Result<TurnResponse, AppError>) -> Vec<pipeline::TurnEvent> {
    let response = match outcome {
        Ok(response) => response,
        Err(err) => {
            // Log full detail server-side; send only the sanitized, per-code
            // message to the client (never raw internals).
            warn!(error = %err, "SSE chat turn failed");
            return vec![pipeline::TurnEvent::Failed(err.sanitized_message())];
        }
    };
    let Ok(Value::Object(envelope)) = serde_json::to_value(&response) else {
        warn!("SSE chat turn produced an unserializable envelope");
        return vec![pipeline::TurnEvent::Failed(
            AppError::internal("The turn could not be serialized.").sanitized_message(),
        )];
    };
    let blocks = envelope
        .get("assistant")
        .and_then(|assistant| assistant.get("blocks"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut events: Vec<pipeline::TurnEvent> =
        blocks.into_iter().map(pipeline::TurnEvent::Block).collect();
    events.push(pipeline::TurnEvent::Done(Value::Object(envelope)));
    events
}

/// Fire-and-forget notification when a coach conversation produces a
/// response. Only sends if the conversation has a `coach_id`
/// (indicates coach persona).
#[cfg(feature = "client-notifications")]
fn notify_coach_response(
    resources: &Arc<ServerContext>,
    conv: &ConversationRecord,
    user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: &str,
) {
    if conv.coach_id.is_some() {
        if let Some(service) = &resources.common.notification_service {
            let coach_title = conv.title.clone();
            notification_triggers::trigger_coach_message(
                service,
                user_id,
                pierre_notifications::TenantId(tenant_id.as_uuid()),
                conversation_id,
                &coach_title,
            );
        }
    }
}
