// ABOUTME: Insight-generation handler — one-shot JSON response on the insight-generation prompt
// ABOUTME: Skips coach + unified pipeline + AG-UI; runs its own tool loop against insight prompt
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_core::models::{AddMessageParams, ConversationTurnId};

use crate::errors::AppError;
use crate::llm::ChatMessage;
use crate::mcp::resources::ServerContext;
use crate::models::TenantId;
use crate::protocols::universal::UniversalExecutor;
use crate::services::chat_pipeline;
use crate::services::chat_pipeline::stages::persistence::{
    persist_assistant_response, persist_user_message,
};

use super::super::chat_tool_loop::{self, ToolLoopParams};
use super::dto::{ChatCompletionResponse, MessageResponse, SendMessageRequest};
use super::quotas::{apply_usage_warning_headers, UsageWarning};
use super::usage::{extract_or_estimate_tokens, increment_usage_counters, post_process_content};
use super::{
    build_mcp_tools, get_llm_provider, DEFAULT_MAX_TOOL_ITERATIONS, INSIGHT_PROMPT_PREFIX,
};

/// Dispatch an insight-generation request.
///
/// Insight requests do not go through the unified chat pipeline — they
/// run on the dedicated insight-generation prompt, skip coach context
/// and tools entirely, and expect the LLM to emit structured JSON that
/// is parsed out of the raw reply.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(
    skip_all,
    fields(
        channel = "web_insight",
        conversation_id = %conversation_id,
        user_id = %user_id_str,
        content_len = request.content.len(),
    )
)]
pub async fn send_insight_message(
    resources: Arc<ServerContext>,
    conversation_id: String,
    user_id_str: String,
    tenant_id: TenantId,
    tenant_id_str: String,
    request: SendMessageRequest,
    usage_warning: UsageWarning,
) -> Result<Response, AppError> {
    // Verify ownership and persist user message.
    let msg_result = persist_user_message(
        resources.repos.chat.as_ref(),
        &conversation_id,
        &user_id_str,
        tenant_id,
        &request.content,
    )
    .await?;
    let conv = msg_result.conversation;
    let user_msg = msg_result.message;

    let insight_prompt = resources.insight_generation_prompt();
    let analysis_content = request
        .content
        .strip_prefix(INSIGHT_PROMPT_PREFIX)
        .unwrap_or(&request.content)
        .trim_start_matches(':')
        .trim();
    let mut llm_messages = vec![
        ChatMessage::system(insight_prompt),
        ChatMessage::user(analysis_content),
    ];

    let tools = build_mcp_tools();
    let provider = get_llm_provider().await?;
    let executor = Arc::new(UniversalExecutor::new(resources.clone()));

    let turn_id = ConversationTurnId::new();
    let start_time = Instant::now();
    // Per-LLM-call recorder so every insight-tool-loop iteration
    // writes its own `llm_usage` row under this turn id.
    let call_recorder: Option<Arc<dyn chat_tool_loop::LlmCallRecorder>> =
        Some(Arc::new(chat_pipeline::TurnCallRecorder::new(
            Arc::clone(&resources.repos.llm_usage),
            tenant_id_str.clone(),
            user_id_str.clone(),
            Some(conversation_id.clone()),
            turn_id,
            "insight",
        )));
    let tool_params = ToolLoopParams {
        provider: &provider,
        executor: Arc::clone(&executor),
        tools: &tools,
        model: &conv.model,
        user_id: &user_id_str,
        tenant_id,
        max_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
        call_recorder,
        temperature: None,
    };
    let result = chat_tool_loop::run_tool_loop(&tool_params, &mut llm_messages).await?;

    #[allow(clippy::cast_possible_truncation)]
    let execution_time_ms = start_time.elapsed().as_millis() as u64;

    let (prompt_tokens, token_count) = extract_or_estimate_tokens(&result, &llm_messages);

    let final_content = post_process_content(&result.content, true);

    let assistant_params = AddMessageParams {
        conversation_id: &conversation_id,
        user_id: &user_id_str,
        role: "assistant",
        content: &final_content,
        token_count,
        finish_reason: result.finish_reason.as_deref(),
        prompt_tokens,
        model: Some(&conv.model),
    };
    let (assistant_msg, updated_conv) =
        persist_assistant_response(resources.repos.chat.as_ref(), &assistant_params, tenant_id)
            .await?;

    // Per-call `llm_usage` rows are written inline by the chat pipeline's
    // `UsageRepoCallRecorder` with real token counts, cost_usd, and
    // cached_tokens; the zero-token turn_summary marker row is no longer
    // written — aggregate queries index the per-call rows directly.

    let total_tokens_used =
        i64::from(prompt_tokens.unwrap_or(0)) + i64::from(token_count.unwrap_or(0));
    increment_usage_counters(
        &resources,
        &tenant_id_str,
        &user_id_str,
        total_tokens_used,
        result.tool_calls_count,
    )
    .await;

    let response = ChatCompletionResponse {
        user_message: MessageResponse {
            id: user_msg.id,
            role: user_msg.role,
            content: user_msg.content,
            token_count: user_msg.token_count,
            created_at: user_msg.created_at,
        },
        assistant_message: MessageResponse {
            id: assistant_msg.id,
            role: assistant_msg.role,
            content: assistant_msg.content,
            token_count: assistant_msg.token_count,
            created_at: assistant_msg.created_at,
        },
        conversation_updated_at: updated_conv.updated_at,
        model: conv.model.clone(),
        execution_time_ms,
        activity_list: result.activity_list,
        card_title: None,
        actions: None,
        is_command_response: false,
        // Insight requests bypass the unified pipeline and never
        // emit AG-UI events; the field is omitted from the JSON
        // body via `skip_serializing_if = "Option::is_none"`.
        agui_run_id: None,
    };

    let mut http_response = (StatusCode::OK, Json(response)).into_response();
    apply_usage_warning_headers(&mut http_response, usage_warning);
    Ok(http_response)
}
