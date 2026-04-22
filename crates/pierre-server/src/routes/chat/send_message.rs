// ABOUTME: Web chat send-message handler — routes through the unified pipeline with optional AG-UI
// ABOUTME: Delegates insight-generation prompts to send_insight_message
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_core::models::ConversationTurnId;
use uuid::Uuid;

use crate::agui::{AgUiEventFilter, BroadcastSink, RunOwner, RunScope};
use crate::errors::AppError;
use crate::mcp::resources::ServerResources;
use crate::models::TenantId;
use crate::services::chat_pipeline::{self as pipeline};
#[cfg(feature = "client-notifications")]
use pierre_database::database::ConversationRecord;
#[cfg(feature = "client-notifications")]
use pierre_notifications::triggers as notification_triggers;

use super::common::{authenticate, get_tenant_id};
use super::dto::{ChatCompletionResponse, MessageResponse, SendMessageRequest};
use super::quotas::{apply_usage_warning_headers, check_pre_chat_quotas};
use super::send_insight::send_insight_message;
use super::usage::{
    increment_usage_counters, record_llm_usage, tokens_from_dispatch, RecordLlmUsageParams,
};
use super::{get_llm_provider, INSIGHT_PROMPT_PREFIX};

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
    resources: &Arc<ServerResources>,
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
pub async fn send_message(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Response, AppError> {
    let auth = authenticate(&headers, &resources).await?;
    let tenant_id = get_tenant_id(auth.user_id, &resources).await?;
    let user_id_str = auth.user_id.to_string();
    let tenant_id_str = tenant_id.to_string();

    // Pre-chat quota check: verify message and token quotas before LLM dispatch.
    let usage_warning = check_pre_chat_quotas(
        &resources,
        &tenant_id_str,
        &user_id_str,
        auth.user_id,
        tenant_id,
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
        return send_insight_message(
            resources,
            conversation_id,
            user_id_str,
            tenant_id,
            tenant_id_str,
            request,
            usage_warning,
        )
        .await;
    }

    let turn_input = pipeline::TurnInput {
        conversation_id: conversation_id.clone(),
        user_id: user_id_str.clone(),
        conversation_tenant_id: tenant_id,
        tool_tenant_id: tenant_id,
        content: request.content.clone(),
        turn_id: ConversationTurnId::new(),
        // Web chat resolves the locale from the authenticated user's
        // profile; pipeline stages fall back to `DEFAULT_LOCALE` when
        // lookup fails so an empty locale never becomes an empty string.
        locale: resources
            .repos
            .users
            .get_global(auth.user_id)
            .await
            .ok()
            .flatten()
            .map(|u| u.locale),
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

    // Record LLM usage for cost tracking and quota enforcement.
    let provider = get_llm_provider().await?;
    record_llm_usage(
        &resources,
        &RecordLlmUsageParams {
            tenant_id,
            user_id: &user_id_str,
            conversation_id: &conversation_id,
            turn_id: dispatch.turn_id,
            provider: &provider,
            model: &dispatch.model,
            tool_calls_count: dispatch.tool_calls_count,
            tools_called: &dispatch.tools_called,
            execution_time_ms,
        },
    )
    .await;

    let total_tokens_used =
        i64::from(prompt_tokens.unwrap_or(0)) + i64::from(completion_tokens.unwrap_or(0));
    increment_usage_counters(
        &resources,
        &tenant_id_str,
        &user_id_str,
        total_tokens_used,
        dispatch.tool_calls_count,
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

/// Fire-and-forget notification when a coach conversation produces a
/// response. Only sends if the conversation has a `coach_id`
/// (indicates coach persona).
#[cfg(feature = "client-notifications")]
fn notify_coach_response(
    resources: &Arc<ServerResources>,
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
