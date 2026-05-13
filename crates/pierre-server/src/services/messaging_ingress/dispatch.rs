// ABOUTME: LLM dispatch + outbound delivery + retry queue for messaging turns
// ABOUTME: dispatch_and_respond is the per-turn orchestrator; helpers handle send/persist/enqueue

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;
use std::time::Instant;

use pierre_core::models::messaging::{ChannelConfig, MessageContent, OutgoingMessage};
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::tokens::estimate_chat_tokens;
use pierre_database::backends::{InsertMessageParams, MessagingRepository};
use pierre_llm::TokenUsage;
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use tokio::sync::Mutex as TokioMutex;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::contremaitre::messaging_strings::{format_template, KEY_EMPTY_REPLY, KEY_ERROR_GENERIC};
use crate::errors::AppError;
use crate::services::analytics::{analytics, hash_id};
use crate::services::chat_pipeline::{self, DispatchResult, PipelineHooks, TurnInput};
use crate::services::usage_counter::UsageCounterService;

use super::agui::{setup_messaging_agui, MessagingAgUiWiring};
use super::{
    build_messaging_profile, content_body_text, PendingDispatch, CONVERSATION_DISPATCH_LOCKS,
};

/// Route the assistant reply back to the user.
///
/// Prefers finalizing the in-channel status placeholder when the
/// bridge is active (so the user sees status and reply collapse into
/// a single chat message). Falls back to the standard outbound send
/// path when the bridge is disabled (`WhatsApp`/Messenger), inactive
/// (credentials missing), or errored mid-turn (edit rejected).
async fn deliver_reply(
    dispatch: &PendingDispatch,
    messaging_agui: Option<&MessagingAgUiWiring>,
    channel_config: &ChannelConfig,
    content: String,
    turn_id: ConversationTurnId,
) {
    if let Some(wiring) = messaging_agui {
        if wiring.finalize_reply(&content).await {
            return;
        }
    }

    // Use conversation_id (channel/chat/thread) as the reply target
    // when available; fall back to sender_id for DM-only platforms
    // (e.g., WhatsApp).
    let reply_target = dispatch
        .conversation_id
        .as_deref()
        .unwrap_or(&dispatch.sender_id)
        .to_owned();

    // The outbound reply carries the turn id from the inbound utterance,
    // so a consumer inspecting the `DeliveryReceipt` can look up the full
    // turn trace via `/internal/conversation-turn`. `.into()` bridges
    // pierre-core's newtype to canot's.
    let outgoing = OutgoingMessage {
        channel_type: dispatch.channel_type,
        recipient_id: reply_target,
        content: MessageContent::Text { body: content },
        turn_id: turn_id.into(),
        reply_to: Some(dispatch.channel_message_id.clone()),
        thread_id: dispatch.thread_id.clone(),
    };
    send_outbound_response(dispatch, channel_config, &outgoing).await;
}

/// Log the pipeline failure, track analytics, and send a localized
/// generic-error reply with a short correlation id.
///
/// Extracted from `dispatch_and_respond` to keep the orchestrator's
/// cognitive complexity inside the workspace lint budget; the body
/// is otherwise a straight line of effects (log, track, template,
/// send) without branching.
async fn report_dispatch_failure(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
    err: &AppError,
) {
    // Correlation ID is surfaced in the user-facing reply and the
    // log record so an operator receiving a Slack alert can grep
    // Cloud Logging for the full error chain without access to
    // conversation IDs (which are PII-adjacent).
    let correlation_id = Uuid::new_v4();
    error!(
        correlation_id = %correlation_id,
        error = %err,
        channel = %dispatch.channel,
        conversation_id = %dispatch.session.conversation,
        "LLM dispatch failed for messaging"
    );
    let hashed_user = hash_id(&dispatch.session.user_id);
    analytics().track_error(&dispatch.channel, &hashed_user, "llm_dispatch_failed");
    let short_id = correlation_id.to_string()[..8].to_owned();
    let template = dispatch
        .resources
        .messaging_strings_registry
        .get(KEY_ERROR_GENERIC, &dispatch.locale);
    let user_message = format_template(&template, &[&short_id]);
    send_error_reply(dispatch, channel_config, &user_message).await;
}

/// Dispatch a message through the LLM pipeline and send the response back via the channel
///
/// Runs as a background task after the webhook has returned HTTP 200.
/// Acquires a per-conversation lock to ensure messages are processed in order.
///
/// The `#[instrument]` span pins `turn_id`, `channel`, and `conversation_id`
/// onto every downstream log line (chat pipeline stages, embacle HTTP call)
/// so an operator can grep a single `turn_id=...` across the whole flow.
#[tracing::instrument(
    skip(dispatch),
    fields(
        turn_id = %dispatch.turn_id,
        channel = %dispatch.channel,
        conversation_id = %dispatch.session.conversation,
    )
)]
pub async fn dispatch_and_respond(dispatch: PendingDispatch) {
    let lock = CONVERSATION_DISPATCH_LOCKS
        .entry(dispatch.session.conversation.clone())
        .or_insert_with(|| Arc::new(TokioMutex::new(())))
        .clone();
    let dispatch_guard = lock.lock().await;

    let start = Instant::now();
    let hashed_tenant = hash_id(&dispatch.channel_tenant_id.to_string());
    let hashed_user = hash_id(&dispatch.session.user_id);

    // Log the inbound user message at debug. The full body is dumped at
    // trace level so an operator can run `RUST_LOG=...=trace` to follow a
    // typed message all the way to the LLM call without needing to enable
    // payload events at the ingress layer in prod.
    info!(
        text_len = dispatch.text_content.len(),
        hashed_user = %hashed_user,
        "messaging dispatch starting"
    );
    tracing::trace!(text = %dispatch.text_content, "messaging dispatch user message");

    let profile = build_messaging_profile(&dispatch);
    // Reuse the turn id canot generated at the webhook boundary
    // (stored on `dispatch.turn_id`). The inbound webhook is the
    // boundary for platform-side observability: a single inbound
    // message plus its full LLM/tool chain is one turn, and canot's
    // log spans already key off this id.
    let turn_id = dispatch.turn_id;
    let turn_input = TurnInput {
        conversation_id: dispatch.session.conversation.clone(),
        user_id: dispatch.session.user_id.clone(),
        conversation_tenant_id: dispatch.channel_tenant_id,
        tool_tenant_id: dispatch.user_tenant_id,
        content: dispatch.text_content.clone(),
        locale: Some(dispatch.locale.clone()),
        turn_id,
    };

    // Load the per-tenant channel config exactly once per turn.
    //
    // Both the AG-UI status bridge (placeholder open + finalize) and
    // the fallback outbound send need it; threading the same snapshot
    // through avoids the 2-3 DB round-trips a naive wiring would
    // incur, and keeps every reply-path consistent with the
    // credentials live at the moment the dispatch started.
    //
    // `None` means the tenant has no configured channel — we cannot
    // reply at all, so log and bail without spending compute on the
    // LLM pipeline.
    let db: &dyn MessagingRepository = dispatch.resources.repos.messaging.as_ref();
    let Some(channel_config) =
        load_channel_config(db, dispatch.channel_tenant_id, &dispatch.channel).await
    else {
        warn!(
            channel = %dispatch.channel,
            tenant_id = %dispatch.channel_tenant_id,
            "channel config unavailable at dispatch time; dropping turn with no reply"
        );
        drop(dispatch_guard);
        evict_idle_dispatch_lock(&dispatch.session.conversation, &lock);
        return;
    };

    // Register an AG-UI run for this messaging turn so in-process
    // consumers (channel-side status adapters, ops dashboards) can
    // subscribe via `resources.agui_registry.subscribe_self(run_id)`
    // and render pipeline progress to Telegram/Slack/Discord.
    // Without this wiring the pipeline's `hooks.agui` stayed `None` on
    // every messaging turn and AG-UI events were produced only for
    // HTTP web-chat callers that passed `agui_run_id`.
    //
    // The run is owned by `(session.user_id, user_tenant_id)` so any
    // cross-user HTTP subscriber (e.g. canot's `AgUiConsumer` against
    // the platform SSE route) still goes through the owner check in
    // `RunRegistry::authorize_and_subscribe`. Scope drops at function
    // exit, auto-unregistering on success, error, or panic.
    //
    // `setup_messaging_agui` also opens an in-channel status adapter
    // (Telegram editMessageText / Slack chat.update / Discord PATCH
    // messages) and spawns a background consumer that mirrors each
    // AG-UI event as a `set_status` call so the user sees the pipeline
    // stage in real time.
    let messaging_agui = setup_messaging_agui(&dispatch, &channel_config).await;
    let hooks = PipelineHooks {
        agui: messaging_agui.as_ref().map(|w| w.run()),
        ..PipelineHooks::none()
    };

    let dispatch_result =
        match chat_pipeline::run(&dispatch.resources, turn_input, &profile, &hooks).await {
            Ok(result) => result,
            Err(e) => {
                report_dispatch_failure(&dispatch, &channel_config, &e).await;
                return;
            }
        };

    // Safe cast: execution time will never exceed u64::MAX milliseconds (~584 million years)
    #[allow(clippy::cast_possible_truncation)]
    let execution_time_ms = start.elapsed().as_millis() as u64;

    analytics().track_bot_response(
        &dispatch.channel,
        &hashed_tenant,
        &hashed_user,
        "llm",
        execution_time_ms,
        &dispatch_result.model,
    );

    // Record LLM usage for cost tracking and quota enforcement
    // Per-LLM-call rows are written inline by the chat pipeline's
    // `LlmCallRecorder`; the turn-summary marker row has been removed
    // now that `llm_usage` is a pure per-call ledger.
    let _ = execution_time_ms;

    // Increment usage counters (message count, token count, tool call count)
    increment_messaging_usage_counters(&dispatch, &dispatch_result).await;

    // Guard: skip sending empty responses. The LLM occasionally returns empty
    // content (e.g., when the input is too technical or the context is exhausted).
    // Telegram rejects empty message text with HTTP 400.
    if dispatch_result.content.trim().is_empty() {
        warn!(
            conversation_id = %dispatch.session.conversation,
            "LLM returned empty response, sending fallback"
        );
        let empty_reply = dispatch
            .resources
            .messaging_strings_registry
            .get(KEY_EMPTY_REPLY, &dispatch.locale);
        send_error_reply(&dispatch, &channel_config, &empty_reply).await;
        return;
    }

    deliver_reply(
        &dispatch,
        messaging_agui.as_ref(),
        &channel_config,
        dispatch_result.content,
        dispatch_result.turn_id,
    )
    .await;

    // Dropping the wiring here aborts the consumer task (if still
    // live) and releases the RunScope so the registry entry is
    // cleaned up. Held until after `deliver_reply` so any
    // last events the pipeline emitted on the way out still render.
    drop(messaging_agui);

    // Held until here to serialize dispatches for the same conversation
    drop(dispatch_guard);
    evict_idle_dispatch_lock(&dispatch.session.conversation, &lock);
}

/// Remove the per-conversation lock from the shared map if no other task still holds it.
///
/// Prevents unbounded growth of `CONVERSATION_DISPATCH_LOCKS` under
/// high conversation cardinality while staying safe: if a concurrent
/// dispatch cloned the `Arc` before we got here, the strong count
/// exceeds 2 and we leave the entry in place. The next waiter will
/// simply reinsert on a later call if it was already evicted.
fn evict_idle_dispatch_lock(conversation_id: &str, local: &Arc<TokioMutex<()>>) {
    // Strong references: the one in the DashMap entry + `local` held here.
    // Any higher count means another dispatch task is waiting on this lock.
    CONVERSATION_DISPATCH_LOCKS.remove_if(conversation_id, |_, stored| {
        Arc::ptr_eq(stored, local) && Arc::strong_count(stored) <= 2
    });
}

/// Send a user-facing error message when LLM dispatch fails or returns empty content.
///
/// Ensures the user always gets feedback instead of silence when something goes wrong.
async fn send_error_reply(dispatch: &PendingDispatch, channel_config: &ChannelConfig, body: &str) {
    let reply_target = dispatch
        .conversation_id
        .as_deref()
        .unwrap_or(&dispatch.sender_id)
        .to_owned();

    let outgoing = OutgoingMessage {
        channel_type: dispatch.channel_type,
        recipient_id: reply_target,
        content: MessageContent::Text {
            body: body.to_owned(),
        },
        // Error replies emit a fresh turn id — the chat pipeline never
        // reached the point where a turn would have been recorded, so
        // there's no upstream id to thread. The platform records a
        // terminal failed turn under this id so operators can correlate.
        turn_id: CanotTurnId::new(),
        reply_to: Some(dispatch.channel_message_id.clone()),
        thread_id: dispatch.thread_id.clone(),
    };

    send_outbound_response(dispatch, channel_config, &outgoing).await;
}

/// Extract real token counts from provider usage, or estimate from content length.
///
/// Returns `(prompt_tokens, completion_tokens)` as i64 for direct use in usage recording.
/// When real usage is unavailable (CLI-based providers), estimates from the user message
/// and completion text using a character-based heuristic.
fn estimate_or_extract_messaging_tokens(
    usage: Option<&TokenUsage>,
    user_text: &str,
    completion_text: &str,
) -> (i64, i64) {
    usage.map_or_else(
        || {
            let (est_prompt, est_completion) = estimate_chat_tokens(user_text, completion_text);
            debug!(
                est_prompt,
                est_completion,
                "Using estimated token counts for messaging (provider returned no usage)"
            );
            (i64::from(est_prompt), i64::from(est_completion))
        },
        |u| (i64::from(u.prompt_tokens), i64::from(u.completion_tokens)),
    )
}

/// Increment usage counters (messages, tokens, tool calls) after a messaging dispatch
async fn increment_messaging_usage_counters(dispatch: &PendingDispatch, result: &DispatchResult) {
    let Some(ref admin_config) = dispatch.resources.admin_config else {
        return;
    };

    let tenant_id_str = dispatch.channel_tenant_id.to_string();
    let usage_svc = UsageCounterService::new(
        dispatch.resources.repos.usage_counters.as_ref(),
        admin_config,
    );

    // Use real token counts when available, fall back to estimation for CLI providers
    let (est_prompt, est_completion) = estimate_or_extract_messaging_tokens(
        result.usage.as_ref(),
        &dispatch.text_content,
        &result.content,
    );
    let total_tokens = est_prompt + est_completion;

    // Build list of (counter_type, amount) pairs to increment
    let mut counters: Vec<(&str, i64)> = vec![("daily_messages", 1), ("weekly_messages", 1)];
    if total_tokens > 0 {
        counters.push(("daily_tokens", total_tokens));
        counters.push(("weekly_tokens", total_tokens));
    }
    if result.tool_calls_count > 0 {
        let tool_calls = i64::from(result.tool_calls_count);
        counters.push(("daily_tool_calls", tool_calls));
        counters.push(("weekly_tool_calls", tool_calls));
    }

    for (counter_type, amount) in counters {
        if let Err(e) = usage_svc
            .increment(
                &tenant_id_str,
                &dispatch.session.user_id,
                counter_type,
                amount,
            )
            .await
        {
            error!("Failed to increment {counter_type} counter for messaging: {e}");
        }
    }
}

/// Load channel config, send outbound message, and persist the result
async fn send_outbound_response(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
    outgoing: &OutgoingMessage,
) {
    let db: &dyn MessagingRepository = dispatch.resources.repos.messaging.as_ref();

    match dispatch.adapter.send(outgoing, channel_config).await {
        Ok(receipt) => {
            let channel_msg_id = receipt.channel_message_id.as_deref().unwrap_or("");
            info!(
                channel_message_id = %channel_msg_id,
                channel = %dispatch.channel,
                "Outbound message sent successfully"
            );
            persist_outbound_message(db, dispatch, channel_msg_id, outgoing).await;
        }
        Err(e) => {
            warn!(
                error = %e,
                channel = %dispatch.channel,
                "Failed to send outbound message, enqueuing for retry"
            );
            enqueue_failed_outbound(db, dispatch, outgoing).await;
        }
    }
}

/// Load and deserialize a channel config for outbound sending
pub(super) async fn load_channel_config(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
) -> Option<ChannelConfig> {
    let config = match db.get_channel_config(tenant_id, channel).await {
        Ok(Some(cfg)) => cfg,
        Ok(None) => {
            warn!(channel = %channel, "No channel config found for outbound send");
            return None;
        }
        Err(e) => {
            error!(error = %e, "Failed to load channel config for outbound");
            return None;
        }
    };

    match serde_json::from_value::<ChannelConfig>(config) {
        Ok(c) => Some(c),
        Err(e) => {
            error!(error = %e, "Failed to deserialize channel config");
            None
        }
    }
}

/// Persist an outbound message after successful delivery
async fn persist_outbound_message(
    db: &dyn MessagingRepository,
    dispatch: &PendingDispatch,
    channel_message_id: &str,
    outgoing: &OutgoingMessage,
) {
    let out_msg_id = Uuid::new_v4().to_string();
    let body = content_body_text(&outgoing.content);
    let correlation_str = outgoing.turn_id.to_string();
    let out_params = InsertMessageParams {
        id: &out_msg_id,
        tenant_id: dispatch.channel_tenant_id,
        session_id: &dispatch.session.session_id,
        direction: "outbound",
        channel_type: &dispatch.channel,
        channel_message_id,
        sender_id: "pierre",
        content_type: "text",
        content_body: body.as_deref(),
        correlation_id: &correlation_str,
        raw_payload: None,
    };
    if let Err(e) = db.insert_message(&out_params).await {
        error!(error = %e, "Failed to persist outbound message");
    }
}

/// Enqueue a failed outbound message for retry delivery
///
/// Renders the outgoing message to the channel's native payload format, persists
/// the outbound message record, then enqueues it in the retry queue.
async fn enqueue_failed_outbound(
    db: &dyn MessagingRepository,
    dispatch: &PendingDispatch,
    outgoing: &OutgoingMessage,
) {
    if let Err(e) = try_enqueue_for_retry(db, dispatch, outgoing).await {
        error!(error = %e, channel = %dispatch.channel, "Failed to enqueue outbound for retry");
    }
}

/// Render, persist, and enqueue an outbound message for retry
///
/// Returns an error if any step fails (rendering, persistence, or enqueue).
async fn try_enqueue_for_retry(
    db: &dyn MessagingRepository,
    dispatch: &PendingDispatch,
    outgoing: &OutgoingMessage,
) -> Result<(), AppError> {
    let payload = dispatch
        .adapter
        .render(outgoing)
        .map_err(|e| AppError::internal(format!("Failed to render for retry: {e}")))?;

    let payload_str = payload.to_string();

    // Persist the outbound message record first (FK requirement for queue entry).
    // Use a unique retry-prefixed ID to avoid colliding with the (tenant_id, channel_message_id)
    // uniqueness constraint — retry messages have no real channel ID yet.
    let out_msg_id = Uuid::new_v4().to_string();
    let retry_channel_msg_id = format!("retry-{out_msg_id}");
    let body = content_body_text(&outgoing.content);
    let correlation_str = outgoing.turn_id.to_string();
    let out_params = InsertMessageParams {
        id: &out_msg_id,
        tenant_id: dispatch.channel_tenant_id,
        session_id: &dispatch.session.session_id,
        direction: "outbound",
        channel_type: &dispatch.channel,
        channel_message_id: &retry_channel_msg_id,
        sender_id: "pierre",
        content_type: "text",
        content_body: body.as_deref(),
        correlation_id: &correlation_str,
        raw_payload: Some(&payload_str),
    };
    let inserted = db.insert_message(&out_params).await?;
    if !inserted {
        return Err(AppError::internal(
            "Failed to persist retry message: duplicate channel_message_id",
        ));
    }

    let queue_id = Uuid::new_v4().to_string();
    db.enqueue_outbound(
        &queue_id,
        &out_msg_id,
        dispatch.channel_tenant_id,
        Some(dispatch.session.user_id.as_str()),
        &dispatch.channel,
        &payload_str,
    )
    .await?;

    info!(
        queue_id = %queue_id,
        channel = %dispatch.channel,
        "Outbound message enqueued for retry"
    );
    Ok(())
}
