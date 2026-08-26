// ABOUTME: Insight-generation handler — one-shot JSON response on the insight-generation prompt
// ABOUTME: Skips coach + unified pipeline + AG-UI; runs its own tool loop against insight prompt
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;
use std::time::Instant;

use uuid::Uuid;

use pierre_core::models::{AddMessageParams, ConversationTurnId};

use crate::mcp::resources::ServerContext;
use pierre_chat_pipeline::stages::persistence::{persist_assistant_response, persist_user_message};
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_llm::ChatMessage;
use pierre_tool_runtime::protocol::UniversalExecutor;

use super::dto::{MessageResponse, SendMessageRequest};
use super::turn_response::{
    platform_blocks, AssistantResponse, TurnResponse, TurnTelemetryResponse,
};
use super::usage::{extract_or_estimate_tokens, post_process_content};
use super::{build_mcp_tools, get_llm_provider, INSIGHT_PROMPT_PREFIX};
use pierre_chat_pipeline::{increment_usage_counters_scoped, QuotaState, UsageIncrementScope};
use pierre_core::constants::tool_execution::DEFAULT_MAX_TOOL_ITERATIONS;
use pierre_tool_runtime::tool_execution::{self, ToolLoopParams};

/// Dispatch an insight-generation request.
///
/// Insight requests do not go through the unified chat pipeline — they
/// run on the dedicated insight-generation prompt, skip coach context
/// and tools entirely, and expect the LLM to emit structured JSON that
/// is parsed out of the raw reply.
/// Inputs for [`send_insight_message`]. Bundled so the call site doesn't need
/// a seven-arg positional invocation — every caller threads the same resolved
/// conversation/user/tenant identifiers + the request body.
pub struct SendInsightInputs {
    /// Shared server context
    pub resources: Arc<ServerContext>,
    /// Conversation identifier
    pub conversation_id: String,
    /// User identifier (stringified UUID)
    pub user_id_str: String,
    /// Tenant identifier
    pub tenant_id: TenantId,
    /// Authenticated athlete
    pub user_id: Uuid,
    /// Inbound request body
    pub request: SendMessageRequest,
    /// What the turn service's pre-turn quota check measured. Rides out as a
    /// notice block on the turn rather than as response headers.
    pub usage_warning: QuotaState,
}

#[tracing::instrument(
    skip_all,
    fields(
        channel = "web_insight",
        conversation_id = %inputs.conversation_id,
        user_id = %inputs.user_id_str,
        content_len = inputs.request.content.len(),
    )
)]
pub async fn send_insight_message(inputs: SendInsightInputs) -> Result<TurnResponse, AppError> {
    let SendInsightInputs {
        resources,
        conversation_id,
        user_id_str,
        tenant_id,
        user_id,
        request,
        usage_warning,
    } = inputs;
    let tenant_id_str = tenant_id.to_string();
    // Verify ownership and persist user message.
    let msg_result = persist_user_message(
        resources.common.repos.chat.as_ref(),
        resources.common.repos.groups.as_ref(),
        &conversation_id,
        &user_id_str,
        tenant_id,
        &request.content,
    )
    .await?;
    let conv = msg_result.conversation;
    let user_msg = msg_result.message;

    // The insight path has no per-turn language detection, so scenes resolve in
    // the athlete's stored locale. Falls back to the default when the row is
    // unreadable — a chart with French axis labels is a far smaller problem
    // than dropping the chart.
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

    let tools = build_mcp_tools(&resources);
    let provider = get_llm_provider(&resources).await?;
    let executor = Arc::new(UniversalExecutor::new(resources.clone()));

    let turn_id = ConversationTurnId::new();
    let start_time = Instant::now();
    // Per-LLM-call recorder so every insight-tool-loop iteration
    // writes its own `llm_usage` row under this turn id.
    let call_recorder: Option<Arc<dyn tool_execution::LlmCallRecorder>> =
        Some(Arc::new(pierre_chat_pipeline::TurnCallRecorder::new(
            Arc::clone(&resources.common.repos.llm_usage),
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
        max_iterations: usize::from(DEFAULT_MAX_TOOL_ITERATIONS),
        call_recorder,
        // Insight generation runs out-of-band on an existing conversation
        // and writes its own assistant row afterwards. Tool-round messages
        // aren't part of that contract, so the recorder stays absent here.
        tool_message_recorder: None,
        temperature: None,
        // Insight generation is a one-shot JSON response — no progressive
        // UX, so the streaming sink stays absent.
        stream_sink: None,
        // Insight generation uses text-based tool calling; the ACP MCP bridge
        // is a chat-turn concern only.
        mcp_servers: Vec::new(),
    };
    let result = tool_execution::run_tool_loop(&tool_params, &mut llm_messages).await?;

    #[allow(clippy::cast_possible_truncation)]
    let execution_time_ms = start_time.elapsed().as_millis() as u64;

    let (prompt_tokens, token_count) = extract_or_estimate_tokens(&result, &llm_messages);

    let final_content = post_process_content(&result.content, true);

    let assistant_params = AddMessageParams {
        tenant_id,
        conversation_id: &conversation_id,
        user_id: &user_id_str,
        role: "assistant",
        content: &final_content,
        token_count,
        finish_reason: result.finish_reason.as_deref(),
        prompt_tokens,
        model: Some(&conv.model),
        content_blocks: None,
    };
    let (assistant_msg, updated_conv) = persist_assistant_response(
        resources.common.repos.chat.as_ref(),
        resources.common.repos.groups.as_ref(),
        &assistant_params,
        tenant_id,
    )
    .await?;

    // Per-call `llm_usage` rows are written inline by the chat pipeline's
    // `UsageRepoCallRecorder` with real token counts, cost_usd, and
    // cached_tokens; the zero-token turn_summary marker row is no longer
    // written — aggregate queries index the per-call rows directly.

    let total_tokens_used =
        i64::from(prompt_tokens.unwrap_or(0)) + i64::from(token_count.unwrap_or(0));
    increment_usage_counters_scoped(
        &resources.chat_pipeline_context(),
        tenant_id,
        user_id,
        total_tokens_used,
        &UsageIncrementScope {
            conversation_id: Some(conversation_id.as_str()),
            coach_id: conv.coach_id.as_deref(),
        },
    )
    .await;

    // An insight is one JSON document, not a conversation turn with charts:
    // the prose block is the whole reply. The activity list the tool loop
    // captured folds into it the same way it would on any surface without an
    // activity panel — an insight card has none.
    let insight_text = match result.activity_list {
        Some(list) => format!("{}\n\n{}", list.trim_end(), assistant_msg.content),
        None => assistant_msg.content.clone(),
    };
    let blocks = platform_blocks(insight_text, None, Vec::new(), &usage_warning);
    let response = TurnResponse {
        turn_id: turn_id.to_string(),
        user_message: MessageResponse {
            id: user_msg.id,
            role: user_msg.role,
            content: user_msg.content,
            token_count: user_msg.token_count,
            scene_blocks: None,
            created_at: user_msg.created_at,
        },
        assistant: AssistantResponse {
            message: MessageResponse {
                id: assistant_msg.id,
                role: assistant_msg.role,
                content: assistant_msg.content,
                token_count: assistant_msg.token_count,
                scene_blocks: None,
                created_at: assistant_msg.created_at,
            },
            blocks,
            finish_reason: result.finish_reason.clone(),
        },
        conversation_updated_at: updated_conv.updated_at,
        telemetry: TurnTelemetryResponse {
            model: conv.model.clone(),
            provider_name: provider.name().to_owned(),
            tool_calls_count: result.tool_calls_count,
            tools_called: result.tools_called.clone(),
            execution_time_ms,
        },
    };

    Ok(response)
}
