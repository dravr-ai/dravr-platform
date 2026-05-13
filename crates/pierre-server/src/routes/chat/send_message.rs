// ABOUTME: Web chat send-message handler — routes through the unified pipeline with optional AG-UI
// ABOUTME: Delegates insight-generation prompts to send_insight_message
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_stream::stream;
use axum::extract::{Path, State};
use axum::http::{header::ACCEPT, HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_core::models::{default_locale, AddMessageParams, ConversationTurnId};
use serde_json::json;
use tokio::sync::mpsc;
use tracing::{field, info, instrument, trace, warn, Span};
use uuid::Uuid;

use crate::agui::{AgUiEventFilter, BroadcastSink, RunOwner, RunScope};
use crate::errors::AppError;
use crate::mcp::resources::ServerContext;
use crate::models::TenantId;
use crate::services::chat_pipeline::{self as pipeline};
use crate::services::commands::dispatch::{try_dispatch, DispatchOutcome, DispatchRequest};
use crate::services::messaging_ingress::detect_turn_locale;
#[cfg(feature = "client-notifications")]
use pierre_database::database::ConversationRecord;
#[cfg(feature = "client-notifications")]
use pierre_notifications::triggers as notification_triggers;

use super::common::get_tenant_id;
use super::dto::{ChatCompletionResponse, ChatMessageAction, MessageResponse, SendMessageRequest};
use super::quotas::{apply_usage_warning_headers, check_pre_chat_quotas_scoped, PreChatScope};
use super::send_insight::{send_insight_message, SendInsightInputs};
use super::usage::{increment_usage_counters_scoped, tokens_from_dispatch, UsageIncrementScope};
use super::INSIGHT_PROMPT_PREFIX;
use crate::middleware::AuthenticatedUser;

/// HTTP header carrying the client surface identifier.
///
/// Set by first-party Dravr frontends to distinguish web from mobile for
/// analytics, rate-limiting, and the `PlatformCommandContext.channel_type`
/// field. Falls back to `"web"` when absent so ad-hoc API callers keep a
/// sensible default.
const CLIENT_PLATFORM_HEADER: &str = "x-client-platform";

/// Bundles the AG-UI sink and registry scope for a single chat turn.
///
/// The handler holds one of these across [`pipeline::run`] so events
/// flow into the registry while the pipeline executes; the
/// [`RunScope`] inside `scope` auto-unregisters the run when the
/// wiring drops at handler exit.
struct AgUiWiring {
    scope: RunScope,
    sink: BroadcastSink,
    thread_id: String,
}

impl AgUiWiring {
    fn run_id(&self) -> &str {
        self.scope.run_id()
    }

    fn run(&self) -> pipeline::AgUiRun<'_> {
        pipeline::AgUiRun {
            run_id: self.scope.run_id().to_owned(),
            thread_id: Some(self.thread_id.clone()),
            sink: &self.sink,
        }
    }
}

/// Wire up AG-UI progress feedback for a single turn, when the caller
/// asked for it via `agui_run_id`. Short / guessable ids open a
/// brute-force surface even with the per-run owner check, so we
/// require a UUID.
fn setup_agui(
    resources: &Arc<ServerContext>,
    requested_run_id: Option<&str>,
    user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: &str,
) -> Result<Option<AgUiWiring>, AppError> {
    let Some(raw) = requested_run_id else {
        return Ok(None);
    };
    let parsed = Uuid::parse_str(raw).map_err(|_| {
        AppError::invalid_input("agui_run_id must be a UUID string; use Uuid::new_v4() per turn")
    })?;
    let run_id = parsed.to_string();
    let owner = RunOwner::new(user_id, tenant_id);
    let scope = resources.agui_registry.register_scoped(&run_id, owner);
    let sink = BroadcastSink::new(
        (*resources.agui_registry).clone(),
        AgUiEventFilter::default(),
    );
    Ok(Some(AgUiWiring {
        scope,
        sink,
        thread_id: conversation_id.to_owned(),
    }))
}

/// Send a message and get a response (non-streaming) with MCP tool execution.
///
/// Insight-generation requests (prompts starting with
/// `INSIGHT_PROMPT_PREFIX`) take a dedicated inline path — no coach, no
/// tools, no memory — because they run on the insight-generation
/// system prompt and expect JSON output. All other turns go through
/// [`pipeline::run`].
///
/// The `#[instrument]` span is the root span for every web/mobile chat
/// turn — same shape as the messaging webhook root span — so an operator
/// can grep a single `turn_id` across web, mobile, and Telegram/Messenger
/// traffic uniformly.
#[instrument(
    skip_all,
    fields(
        channel = "web_chat",
        conversation_id = %conversation_id,
        user_id = field::Empty,
        tenant_id = field::Empty,
        turn_id = field::Empty,
        content_len = request.content.len(),
        is_command = request.content.trim_start().starts_with('/'),
        is_insight = request.content.starts_with(INSIGHT_PROMPT_PREFIX),
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
    let tenant_id = get_tenant_id(auth.user_id, &resources).await?;
    let user_id_str = auth.user_id.to_string();
    let tenant_id_str = tenant_id.to_string();

    // Populate the parent span so downstream pipeline log lines (already
    // instrumented to read `turn_id`/`channel`/`conversation_id`) carry the
    // resolved tenant + user without each callee needing to re-record them.
    let span = Span::current();
    span.record("user_id", field::display(&user_id_str));
    span.record("tenant_id", field::display(&tenant_id_str));

    info!(
        channel = "web_chat",
        conversation_id = %conversation_id,
        content_len = request.content.len(),
        "Processed inbound chat message"
    );
    if tracing::enabled!(tracing::Level::TRACE) {
        trace!(content = %request.content, "web_chat user message body");
    }

    // Resolve the conversation row once so the per-conversation and
    // per-coach scoped caps fire in the same call as the global daily/
    // weekly caps. A missing conversation is fine — the user might be
    // creating a new one — so we silently degrade to global-only.
    let conversation_row = resources
        .repos
        .chat
        .get_conversation(&conversation_id, &user_id_str, tenant_id)
        .await
        .ok()
        .flatten();
    let scope_coach_id = conversation_row.as_ref().and_then(|c| c.coach_id.clone());
    let pre_scope = PreChatScope {
        conversation_id: Some(conversation_id.as_str()),
        coach_id: scope_coach_id.as_deref(),
    };

    // Pre-chat quota check: verify message, token, per-conversation,
    // and per-coach quotas before LLM dispatch.
    let usage_warning = check_pre_chat_quotas_scoped(
        &resources,
        &tenant_id_str,
        &user_id_str,
        auth.user_id,
        &pre_scope,
    )
    .await?;

    // Insight-generation requests bypass the unified pipeline (different
    // prompt, no coach, no tools, JSON response shape).
    if request.content.starts_with(INSIGHT_PROMPT_PREFIX) {
        // Insights are a one-shot JSON response — the unified-pipeline
        // AG-UI lifecycle (`RUN_STARTED` → steps → `RUN_FINISHED`) does
        // not fit. Rather than silently swallowing `agui_run_id` the
        // caller passed, reject it loudly so clients surface a clear
        // 400 instead of opening an SSE subscription that would never
        // receive any events.
        if request.agui_run_id.is_some() {
            return Err(AppError::invalid_input(
                "agui_run_id is not supported on insight-generation requests; \
                 insights return a single JSON payload and emit no AG-UI events",
            ));
        }
        return send_insight_message(SendInsightInputs {
            resources,
            conversation_id,
            user_id_str,
            tenant_id,
            tenant_id_str,
            request,
            usage_warning,
        })
        .await;
    }

    // Slash-command pre-pipeline: web and mobile chat share the same
    // command dispatcher as the messaging channels. When the text starts
    // with `/`, we resolve it through
    // [`services::commands::dispatch::try_dispatch`] and persist the
    // result as a regular conversation turn — no LLM call, no token
    // spend, no AG-UI pipeline lifecycle. Non-command turns fall through
    // to the existing pipeline path below.
    if request.content.trim_start().starts_with('/') {
        if let Some(response) = try_handle_chat_command(ChatCommandInputs {
            resources: &resources,
            headers: &headers,
            user_id: auth.user_id,
            tenant_id,
            conversation_id: &conversation_id,
            user_id_str: &user_id_str,
            request: &request,
            usage_warning: usage_warning.clone(),
        })
        .await?
        {
            return Ok(response);
        }
        // Fell through: the matcher treated the `/`-prefixed text as
        // NotACommand (edge case — shouldn't happen after the prefix
        // check, but we stay defensive). Continue to the LLM path.
    }

    // Resolve the per-turn locale: prefer the language the user just
    // typed in (so the verification banner, scope refusals, and status
    // placeholders match what the LLM will reply in) and fall back to
    // the stored `users.locale` when whatlang's signal is unreliable
    // (short messages, mixed input). Mirrors the messaging-ingress
    // path's `detect_turn_locale` usage so web chat and Slack/Telegram
    // resolve identically.
    let stored_locale = resources
        .repos
        .users
        .get_global(auth.user_id)
        .await
        .ok()
        .flatten()
        .map_or_else(default_locale, |u| u.locale);
    let turn_locale = detect_turn_locale(&request.content, &stored_locale);

    let turn_id = ConversationTurnId::new();
    span.record("turn_id", field::display(&turn_id));

    let turn_input = pipeline::TurnInput {
        conversation_id: conversation_id.clone(),
        user_id: user_id_str.clone(),
        conversation_tenant_id: tenant_id,
        tool_tenant_id: tenant_id,
        content: request.content.clone(),
        turn_id,
        locale: Some(turn_locale),
    };
    let profile = pipeline::ChannelProfile::web_chat();

    // Set up AG-UI progress feedback for callers that asked for it.
    // The scope guard auto-unregisters when this handler returns,
    // so a leak from an early `?` propagation is impossible.
    let agui_wiring = setup_agui(
        &resources,
        request.agui_run_id.as_deref(),
        auth.user_id,
        tenant_id,
        &conversation_id,
    )?;

    // SSE branch: when the client sends `Accept: text/event-stream`, return
    // a streaming response so nginx (default 60s `proxy_read_timeout`) sees
    // periodic activity and the user gets progressive token-level feedback
    // while the LLM produces its reply. The blocking JSON path below stays
    // available for clients (tests, scripts) that prefer the simpler shape.
    let wants_sse = headers
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("text/event-stream"));

    if wants_sse {
        return Ok(send_message_sse(SseInputs {
            resources: Arc::clone(&resources),
            turn_input,
            profile,
            agui_wiring,
            user_content: request.content,
            scope_coach_id,
            tenant_id_str,
            user_id_str,
            auth_user_id: auth.user_id,
            tenant_id,
            conversation_id,
            usage_warning,
        }));
    }

    let start_time = Instant::now();
    let hooks = pipeline::PipelineHooks {
        agui: agui_wiring.as_ref().map(AgUiWiring::run),
        ..pipeline::PipelineHooks::none()
    };
    let dispatch = pipeline::run(&resources, turn_input, &profile, &hooks).await?;

    // Safe cast: execution time will never exceed u64::MAX milliseconds (~584 million years)
    #[allow(clippy::cast_possible_truncation)]
    let execution_time_ms = start_time.elapsed().as_millis() as u64;

    // Extract tokens for usage recording — use real values from the LLM
    // when available, fall back to character-based estimation otherwise.
    let (prompt_tokens, completion_tokens) = tokens_from_dispatch(&dispatch, &request.content);

    // Per-call `llm_usage` rows are written inline by the chat pipeline's
    // `UsageRepoCallRecorder` with real token counts, cost_usd, and
    // cached_tokens. The zero-token turn_summary marker row is no longer
    // written — aggregate queries index the per-call rows directly.

    let total_tokens_used =
        i64::from(prompt_tokens.unwrap_or(0)) + i64::from(completion_tokens.unwrap_or(0));
    let inc_scope = UsageIncrementScope {
        conversation_id: Some(conversation_id.as_str()),
        coach_id: scope_coach_id.as_deref(),
    };
    increment_usage_counters_scoped(
        &resources,
        &tenant_id_str,
        &user_id_str,
        total_tokens_used,
        dispatch.tool_calls_count,
        &inc_scope,
    )
    .await;

    let response = ChatCompletionResponse {
        user_message: MessageResponse {
            id: dispatch.user_message.id.clone(),
            role: dispatch.user_message.role.clone(),
            content: dispatch.user_message.content.clone(),
            token_count: dispatch.user_message.token_count,
            created_at: dispatch.user_message.created_at,
        },
        assistant_message: MessageResponse {
            id: dispatch.assistant_message.id.clone(),
            role: dispatch.assistant_message.role.clone(),
            content: dispatch.assistant_message.content.clone(),
            token_count: dispatch.assistant_message.token_count,
            created_at: dispatch.assistant_message.created_at,
        },
        conversation_updated_at: dispatch.conversation.updated_at.clone(),
        model: dispatch.model.clone(),
        execution_time_ms,
        activity_list: dispatch.activity_list.clone(),
        card_title: None,
        actions: None,
        is_command_response: false,
        agui_run_id: agui_wiring.as_ref().map(|w| w.run_id().to_owned()),
    };

    // Notify user when a coach conversation produces a response.
    #[cfg(feature = "client-notifications")]
    notify_coach_response(
        &resources,
        &dispatch.conversation,
        auth.user_id,
        tenant_id,
        &conversation_id,
    );

    let mut http_response = (StatusCode::OK, Json(response)).into_response();
    apply_usage_warning_headers(&mut http_response, usage_warning);
    Ok(http_response)
}

/// Streaming SSE branch of [`send_message`].
///
/// Spawns the chat pipeline on a background task with a token-stream sink
/// installed on [`pipeline::PipelineHooks`], then returns an SSE response
/// whose body forwards each [`pipeline::ChatStreamEvent`] as it arrives:
///
/// - `event: delta` carrying `{ "delta": "<chunk>" }` for partial text
/// - `event: tool_call` carrying `{ "id, title, status }` for ACP tool
///   observations
/// - `event: done` carrying the same [`ChatCompletionResponse`] JSON the
///   blocking branch returns, signalling the turn is finished
/// - `event: error` carrying `{ "error": "<message>" }` if the pipeline
///   fails
///
/// A 15-second SSE keep-alive prevents nginx (default 60s
/// `proxy_read_timeout`) from dropping the connection during long
/// LLM-side stalls between deltas.
/// Inputs threaded into [`send_message_sse`]. Resolved identifiers + the
/// turn payload + the AG-UI wiring are bundled so the SSE entry point
/// doesn't need a twelve-arg positional signature.
struct SseInputs {
    resources: Arc<ServerContext>,
    turn_input: pipeline::TurnInput,
    profile: pipeline::ChannelProfile,
    agui_wiring: Option<AgUiWiring>,
    user_content: String,
    scope_coach_id: Option<String>,
    tenant_id_str: String,
    user_id_str: String,
    auth_user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: String,
    usage_warning: super::quotas::UsageWarning,
}

fn send_message_sse(inputs: SseInputs) -> Response {
    let SseInputs {
        resources,
        turn_input,
        profile,
        agui_wiring,
        user_content,
        scope_coach_id,
        tenant_id_str,
        user_id_str,
        auth_user_id,
        tenant_id,
        conversation_id,
        usage_warning,
    } = inputs;
    let start_time = Instant::now();
    let agui_run_id = agui_wiring.as_ref().map(|w| w.run_id().to_owned());

    let (stream_tx, mut stream_rx) = mpsc::unbounded_channel::<pipeline::ChatStreamEvent>();
    let (done_tx, mut done_rx) =
        mpsc::unbounded_channel::<Result<pipeline::DispatchResult, AppError>>();

    let resources_for_task = Arc::clone(&resources);
    tokio::spawn(async move {
        let agui_run = agui_wiring.as_ref().map(AgUiWiring::run);
        let hooks = pipeline::PipelineHooks {
            agui: agui_run,
            stream_sink: Some(stream_tx),
            ..pipeline::PipelineHooks::none()
        };
        let outcome = pipeline::run(&resources_for_task, turn_input, &profile, &hooks).await;
        let _ = done_tx.send(outcome);
        // agui_wiring drops here, unregistering the AG-UI run.
    });

    let body = stream! {
        loop {
            tokio::select! {
                biased;
                Some(event) = stream_rx.recv() => {
                    let (name, data) = match event {
                        pipeline::ChatStreamEvent::TextDelta(delta) => (
                            "delta",
                            json!({ "delta": delta }).to_string(),
                        ),
                        pipeline::ChatStreamEvent::ToolCall { id, title, status } => (
                            "tool_call",
                            json!({ "id": id, "title": title, "status": status }).to_string(),
                        ),
                    };
                    yield Ok::<_, Infallible>(Event::default().event(name).data(data));
                }
                Some(outcome) = done_rx.recv() => {
                    // Drain any remaining buffered stream events before emitting
                    // the terminal `done` so clients see the full token sequence.
                    while let Ok(event) = stream_rx.try_recv() {
                        let (name, data) = match event {
                            pipeline::ChatStreamEvent::TextDelta(delta) => (
                                "delta",
                                json!({ "delta": delta }).to_string(),
                            ),
                            pipeline::ChatStreamEvent::ToolCall { id, title, status } => (
                                "tool_call",
                                json!({ "id": id, "title": title, "status": status }).to_string(),
                            ),
                        };
                        yield Ok(Event::default().event(name).data(data));
                    }

                    match outcome {
                        Ok(dispatch) => {
                            // Mirror the blocking-path post-processing so SSE
                            // and JSON clients see identical final payloads
                            // and identical usage-counter side effects.
                            #[allow(clippy::cast_possible_truncation)]
                            let execution_time_ms = start_time.elapsed().as_millis() as u64;
                            let (prompt_tokens, completion_tokens) =
                                tokens_from_dispatch(&dispatch, &user_content);
                            let total_tokens = i64::from(prompt_tokens.unwrap_or(0))
                                + i64::from(completion_tokens.unwrap_or(0));
                            let inc_scope = UsageIncrementScope {
                                conversation_id: Some(conversation_id.as_str()),
                                coach_id: scope_coach_id.as_deref(),
                            };
                            increment_usage_counters_scoped(
                                &resources,
                                &tenant_id_str,
                                &user_id_str,
                                total_tokens,
                                dispatch.tool_calls_count,
                                &inc_scope,
                            )
                            .await;

                            let response = ChatCompletionResponse {
                                user_message: MessageResponse {
                                    id: dispatch.user_message.id.clone(),
                                    role: dispatch.user_message.role.clone(),
                                    content: dispatch.user_message.content.clone(),
                                    token_count: dispatch.user_message.token_count,
                                    created_at: dispatch.user_message.created_at,
                                },
                                assistant_message: MessageResponse {
                                    id: dispatch.assistant_message.id.clone(),
                                    role: dispatch.assistant_message.role.clone(),
                                    content: dispatch.assistant_message.content.clone(),
                                    token_count: dispatch.assistant_message.token_count,
                                    created_at: dispatch.assistant_message.created_at,
                                },
                                conversation_updated_at: dispatch.conversation.updated_at.clone(),
                                model: dispatch.model.clone(),
                                execution_time_ms,
                                activity_list: dispatch.activity_list.clone(),
                                card_title: None,
                                actions: None,
                                is_command_response: false,
                                agui_run_id: agui_run_id.clone(),
                            };

                            #[cfg(feature = "client-notifications")]
                            notify_coach_response(
                                &resources,
                                &dispatch.conversation,
                                auth_user_id,
                                tenant_id,
                                &conversation_id,
                            );
                            // Reference the values when the feature is off
                            // so they aren't reported as unused; matches the
                            // blocking branch's silent capture.
                            #[cfg(not(feature = "client-notifications"))]
                            {
                                let _ = (auth_user_id, tenant_id);
                            }

                            let payload = serde_json::to_string(&response)
                                .unwrap_or_else(|_| "{}".to_owned());
                            yield Ok(Event::default().event("done").data(payload));
                        }
                        Err(err) => {
                            warn!(error = %err, "SSE chat pipeline failed");
                            let payload = json!({ "error": err.to_string() }).to_string();
                            yield Ok(Event::default().event("error").data(payload));
                        }
                    }
                    break;
                }
                else => break,
            }
        }
    };

    let mut response = Sse::new(body)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response();
    apply_usage_warning_headers(&mut response, usage_warning);
    response
}

/// Attempt to execute the user's content as a slash command.
///
/// Returns `Ok(Some(response))` when the command dispatched (including the
/// unknown-command case) and the caller should return the response as-is —
/// no LLM pipeline call, no token usage recording. Returns `Ok(None)` when
/// the text did not match any command after all (matcher treated it as
/// `NotACommand` despite the `/` prefix); the caller falls through to the
/// regular pipeline path.
///
/// Persists both the user's original message and the command's reply as
/// conversation turns so chat history looks uniform across LLM and command
/// responses. Interactive actions (buttons) ride back on the response DTO
/// but are **not** persisted — they are live only for the turn that
/// produced them; reloading the conversation shows the rendered text
/// body without buttons, and the user can always re-invoke the command.
/// Bundled inputs for [`try_handle_chat_command`]. Combines the resolved
/// auth/tenant context with the incoming request + usage warning so the
/// slash-command dispatcher doesn't need an eight-arg positional signature.
struct ChatCommandInputs<'a> {
    resources: &'a Arc<ServerContext>,
    headers: &'a HeaderMap,
    user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: &'a str,
    user_id_str: &'a str,
    request: &'a SendMessageRequest,
    usage_warning: super::quotas::UsageWarning,
}

async fn try_handle_chat_command(
    inputs: ChatCommandInputs<'_>,
) -> Result<Option<Response>, AppError> {
    let ChatCommandInputs {
        resources,
        headers,
        user_id,
        tenant_id,
        conversation_id,
        user_id_str,
        request,
        usage_warning,
    } = inputs;
    let channel_type = headers
        .get(CLIENT_PLATFORM_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase)
        .filter(|s| matches!(s.as_str(), "web" | "mobile"))
        .unwrap_or_else(|| "web".to_owned());

    let user_row = resources
        .repos
        .users
        .get_global(user_id)
        .await
        .ok()
        .flatten();
    let locale = user_row
        .as_ref()
        .map_or_else(default_locale, |u| u.locale.clone());

    let outcome = try_dispatch(DispatchRequest {
        resources,
        user_id,
        tenant_id,
        channel_type: &channel_type,
        locale: &locale,
        // Web and mobile conversations are per-user by definition —
        // no multi-party DM surface here.
        is_direct_message: true,
        conversation_id: Some(conversation_id),
        text: &request.content,
    })
    .await?;

    let (body_text, card_title, actions) = match outcome {
        DispatchOutcome::NotACommand => return Ok(None),
        DispatchOutcome::UnknownCommand { body } => (body, None, None),
        DispatchOutcome::Executed { response, .. } => {
            let card_title = if response.is_card() {
                response.card_title.clone()
            } else {
                None
            };
            let actions = if response.actions.is_empty() {
                None
            } else {
                Some(
                    response
                        .actions
                        .iter()
                        .map(|a| ChatMessageAction {
                            label: a.label.clone(),
                            action_type: a.action_type.clone(),
                            value: a.value.clone(),
                        })
                        .collect(),
                )
            };
            // Render the full body: "title\n\nbody" when a card title is
            // present, otherwise just the body. History rows (and any
            // frontend that ignores the actions field) see a readable
            // text representation.
            let body_text = match &card_title {
                Some(title) if !title.is_empty() => format!("{title}\n\n{}", response.text),
                _ => response.text,
            };
            (body_text, card_title, actions)
        }
    };

    // Persist both turns. If the conversation doesn't exist for this
    // user+tenant, add_message surfaces a not-found error which we
    // propagate back to the caller unchanged.
    let user_message = resources
        .repos
        .chat
        .add_message(&AddMessageParams {
            conversation_id,
            user_id: user_id_str,
            role: "user",
            content: &request.content,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
        })
        .await?;

    let assistant_message = resources
        .repos
        .chat
        .add_message(&AddMessageParams {
            conversation_id,
            user_id: user_id_str,
            role: "assistant",
            content: &body_text,
            token_count: None,
            finish_reason: Some("command"),
            prompt_tokens: None,
            model: Some("command"),
        })
        .await?;

    let conversation = resources
        .repos
        .chat
        .get_conversation(conversation_id, user_id_str, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Conversation {conversation_id}")))?;

    let chat_response = ChatCompletionResponse {
        user_message: MessageResponse {
            id: user_message.id.clone(),
            role: user_message.role.clone(),
            content: user_message.content.clone(),
            token_count: user_message.token_count,
            created_at: user_message.created_at.clone(),
        },
        assistant_message: MessageResponse {
            id: assistant_message.id.clone(),
            role: assistant_message.role.clone(),
            content: assistant_message.content.clone(),
            token_count: assistant_message.token_count,
            created_at: assistant_message.created_at.clone(),
        },
        conversation_updated_at: conversation.updated_at,
        model: "command".to_owned(),
        execution_time_ms: 0,
        activity_list: None,
        card_title,
        actions,
        is_command_response: true,
        // Commands bypass the AG-UI pipeline (no stages, no progress
        // events). If the caller passed agui_run_id, echo it back so
        // their SSE subscription closes cleanly, but no events flow.
        agui_run_id: request.agui_run_id.clone(),
    };

    let mut http_response = (StatusCode::OK, Json(chat_response)).into_response();
    apply_usage_warning_headers(&mut http_response, usage_warning);
    Ok(Some(http_response))
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
        if let Some(service) = &resources.notification_service {
            let coach_title = conv.title.clone();
            notification_triggers::trigger_coach_message(
                service,
                user_id,
                pierre_notifications::TenantId(tenant_id.0),
                conversation_id,
                &coach_title,
            );
        }
    }
}
