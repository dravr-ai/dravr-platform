// ABOUTME: Service layer for LLM tool execution loops in chat conversations
// ABOUTME: Supports API-based, headless (Copilot ACP), and CLI-based tool calling modes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tool loop strategies for chat conversations
//!
//! Three strategies exist for executing tool calls during LLM conversations:
//! - [`run_api_tool_loop`]: Native function calling via `complete_with_tools()` (Gemini, Groq)
//! - [`run_headless_tool_loop`]: ACP-managed tool calling via Copilot Headless `converse()`
//! - [`run_cli_tool_loop`]: Text-based tool calling via `<tool_call>` blocks (CLI providers)
//!
//! All strategies share the same MCP executor infrastructure and produce
//! identical [`ToolLoopResult`] output.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use pierre_core::llm::tool_simulation;
use pierre_core::tokens::estimate_chat_tokens;
use tracing::{info, warn};

use crate::errors::AppError;
use crate::llm::{
    ChatMessage, ChatProvider, ChatRequest, ChatResponseWithTools, FunctionCall,
    FunctionDeclaration, FunctionResponse, MessageRole, TokenUsage, Tool,
};
use crate::models::TenantId;
use crate::protocols::universal::{UniversalExecutor, UniversalRequest, UniversalResponse};
use crate::services::analytics::analytics;
use crate::services::chat_pipeline::{ChatStreamEvent, ChatStreamSink};

// ============================================================================
// Shared Types
// ============================================================================

/// Per-LLM-call metric captured by the tool loop and handed to a
/// [`LlmCallRecorder`]. One record corresponds to one invocation of the
/// provider's completion API inside the tool loop.
#[derive(Debug, Clone)]
pub struct LlmCallRecord {
    /// Provider name (e.g. `"gemini"`, `"groq"`, `"claude_code"`).
    pub provider: String,
    /// Model identifier used for this call.
    pub model: String,
    /// Prompt tokens reported by the provider, 0 if unavailable.
    pub prompt_tokens: i64,
    /// Completion tokens reported by the provider, 0 if unavailable.
    pub completion_tokens: i64,
    /// Prompt tokens served from the provider's context cache. Zero
    /// when the provider does not report cache hits.
    pub cached_tokens: i64,
    /// Wall-clock latency of the provider call (milliseconds).
    pub latency_ms: i64,
    /// Whether the provider returned a non-error response.
    pub success: bool,
    /// 1-based position of this call within the owning `turn_id`, assigned
    /// by the tool loop so the persister can preserve call order.
    pub call_sequence: Option<i64>,
    /// True when token counts were estimated from character length
    /// because the provider returned no usage (CLI runners — Claude
    /// Code, Copilot, Cursor — do this). Persisters append an
    /// `"_estimated"` suffix to `call_type` so billing can flag the row.
    pub token_counts_estimated: bool,
    /// Names of MCP tools dispatched by this LLM call's response. Empty
    /// when the LLM returned a plain-text answer with no tool calls.
    pub tools_called: Vec<String>,
}

/// Sink that receives one [`LlmCallRecord`] per LLM call.
///
/// Implementations persist the record (typically to `llm_usage`) so
/// the per-turn endpoint can surface one entry per call in its
/// `llm_calls` array.
///
/// Invocations happen on the async runtime but the sink method itself
/// is synchronous; implementers should spawn a task or push to a
/// channel if the work is blocking.
pub trait LlmCallRecorder: Send + Sync {
    /// Record a completed LLM call.
    fn record(&self, record: LlmCallRecord);
}

/// Parameters for the multi-turn tool execution loop
pub struct ToolLoopParams<'a> {
    /// LLM provider to use for completions
    pub provider: &'a ChatProvider,
    /// MCP executor for running tool calls (Arc for sharing with SDK tool handler closures)
    pub executor: Arc<UniversalExecutor>,
    /// Tool definitions available for function calling
    pub tools: &'a Tool,
    /// Model identifier for the LLM request
    pub model: &'a str,
    /// User ID for tool execution context
    pub user_id: &'a str,
    /// Tenant ID for multi-tenant isolation
    pub tenant_id: TenantId,
    /// Maximum number of tool-calling iterations before forcing a response
    pub max_iterations: usize,
    /// Optional per-LLM-call sink. When set, each provider call inside
    /// the loop produces a [`LlmCallRecord`] that the sink persists.
    /// When absent, the loop still accumulates cumulative usage for
    /// the returned [`ToolLoopResult`] without recording individual calls.
    pub call_recorder: Option<Arc<dyn LlmCallRecorder>>,
    /// Optional per-coach LLM sampling temperature. When `Some`, applied
    /// to every `ChatRequest` in the loop via `with_temperature`. When
    /// `None`, the provider/server default is used.
    pub temperature: Option<f32>,
    /// Optional sink for token-level streaming events. When set on the
    /// headless tool loop branch, the loop calls Copilot's
    /// `converse_stream()` instead of `converse()` and forwards each
    /// observed text delta and tool-call snapshot through the sink.
    /// The sink is a [`crate::services::chat_pipeline::ChatStreamSink`]
    /// — see that type for the event shape.
    pub stream_sink: Option<ChatStreamSink>,
}

/// Result of running the multi-turn tool execution loop
pub struct ToolLoopResult {
    /// Final text content from LLM
    pub content: String,
    /// Token usage statistics if available
    pub usage: Option<TokenUsage>,
    /// Finish reason if available
    pub finish_reason: Option<String>,
    /// Activity list from `get_activities` tool (to prepend to response)
    pub activity_list: Option<String>,
    /// Total tool calls executed across all iterations
    pub tool_calls_count: u32,
    /// Names of every MCP tool invoked during the loop, in call order.
    /// Persisted alongside the LLM usage row so per-turn observability
    /// can answer "which tools ran for this turn".
    pub tools_called: Vec<String>,
    /// Provider slug that triggered an `AppError::ProviderAuthRequired`
    /// during a tool dispatch in this loop. The chat pipeline detects this
    /// and short-circuits the turn with a deterministic re-auth reply
    /// (containing a minted hosted-login URL) instead of letting the LLM
    /// rephrase a generic refusal.
    pub pending_provider_auth_required: Option<String>,
}

/// Metadata key set on a tool's `UniversalResponse` when the underlying
/// `AppError` was `ProviderAuthRequired`. The tool loop scans this key to
/// know it should exit early without continuing iteration.
pub const META_AUTH_REQUIRED_PROVIDER: &str = "auth_required_provider";

// ============================================================================
// API Tool Loop (Gemini/Groq native function calling)
// ============================================================================

/// Executes the tool loop using native LLM function calling (Gemini, Groq).
///
/// Flow: `complete_with_tools()` → parse `function_calls` → execute via MCP → iterate
///
/// # Errors
///
/// Returns error if the LLM call or tool execution fails.
pub async fn run_api_tool_loop(
    params: &ToolLoopParams<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> Result<ToolLoopResult, AppError> {
    let mut captured_activity_list: Option<String> = None;
    let mut tool_calls_count: u32 = 0;
    let mut tools_called: Vec<String> = Vec::new();
    let mut cumulative_usage = TokenUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
    };

    for iteration in 0..params.max_iterations {
        let llm_request = {
            let req = ChatRequest::new(llm_messages.clone()).with_model(params.model);
            match params.temperature {
                Some(t) => req.with_temperature(t),
                None => req,
            }
        };

        log_iteration_start(iteration, params, llm_messages.len());

        let call_start = Instant::now();
        let cached_slot = Arc::new(AtomicU32::new(0));
        let slot_for_scope = cached_slot.clone();
        let response_result = pierre_llm::LAST_CACHED_TOKENS
            .scope(
                slot_for_scope,
                params
                    .provider
                    .complete_with_tools(&llm_request, Some(vec![params.tools.clone()])),
            )
            .await;
        let latency_ms = millis_elapsed(call_start);
        let cached_tokens = i64::from(cached_slot.load(Ordering::SeqCst));
        let call_seq = Some(i64::try_from(iteration).unwrap_or(i64::MAX) + 1);
        let response = match response_result {
            Ok(r) => {
                let tools_in_response = r
                    .function_calls
                    .as_ref()
                    .map(|fcs| fcs.iter().map(|c| c.name.clone()).collect())
                    .unwrap_or_default();
                emit_call_record(
                    params.call_recorder.as_ref(),
                    params.provider.name(),
                    params.model,
                    r.usage.as_ref(),
                    cached_tokens,
                    latency_ms,
                    true,
                    call_seq,
                    tools_in_response,
                );
                r
            }
            Err(e) => {
                emit_call_record(
                    params.call_recorder.as_ref(),
                    params.provider.name(),
                    params.model,
                    None,
                    0,
                    latency_ms,
                    false,
                    call_seq,
                    Vec::new(),
                );
                return Err(e);
            }
        };

        log_iteration_response(iteration, latency_ms, &response);

        // Accumulate token usage from every LLM call
        if let Some(ref usage) = response.usage {
            cumulative_usage.prompt_tokens += usage.prompt_tokens;
            cumulative_usage.completion_tokens += usage.completion_tokens;
            cumulative_usage.total_tokens += usage.total_tokens;
        }

        // Check for function calls
        if let Some(ref function_calls) = response.function_calls {
            if !function_calls.is_empty() {
                info!(
                    "Iteration {}: Executing {} tool calls",
                    iteration,
                    function_calls.len()
                );

                let ExecutedFunctionCalls {
                    responses: function_responses,
                    auth_required_provider,
                } = execute_function_calls(
                    &params.executor,
                    function_calls,
                    params.user_id,
                    params.tenant_id,
                )
                .await?;

                #[allow(clippy::cast_possible_truncation)]
                {
                    tool_calls_count += function_calls.len() as u32;
                }

                tools_called.extend(function_calls.iter().map(|c| c.name.clone()));

                // Auth-required short-circuit: if any tool failed with
                // `ProviderAuthRequired`, exit the loop immediately so the
                // chat pipeline can mint a hosted-login URL and reply
                // deterministically. Continuing iteration would have the LLM
                // either rephrase a generic refusal or call the same broken
                // tool again.
                if let Some(provider) = auth_required_provider {
                    return Ok(ToolLoopResult {
                        content: String::new(),
                        usage: Some(cumulative_usage),
                        finish_reason: Some("provider_auth_required".to_owned()),
                        activity_list: captured_activity_list,
                        tool_calls_count,
                        tools_called,
                        pending_provider_auth_required: Some(provider),
                    });
                }

                // Add assistant's text to messages if present (strip synthetic function syntax)
                if let Some(ref text) = response.content {
                    let cleaned = strip_synthetic_function_calls(text);
                    if !cleaned.is_empty() {
                        llm_messages.push(ChatMessage::assistant(&*cleaned));
                    }
                }

                // Add function responses as user messages, capturing activity list if present
                if let Some(list) =
                    add_function_responses_to_messages(llm_messages, &function_responses)
                {
                    captured_activity_list = Some(list);
                }
                continue;
            }
        }

        // No function calls - we have a text response (strip any synthetic function syntax)
        let content = response
            .content
            .map(|c| strip_synthetic_function_calls(&c).into_owned())
            .unwrap_or_default();
        return Ok(ToolLoopResult {
            content,
            usage: Some(cumulative_usage),
            finish_reason: response.finish_reason,
            activity_list: captured_activity_list,
            tool_calls_count,
            tools_called,
            pending_provider_auth_required: None,
        });
    }

    // Max iterations reached
    let usage = if cumulative_usage.total_tokens > 0 {
        Some(cumulative_usage)
    } else {
        None
    };
    Ok(ToolLoopResult {
        content: String::new(),
        usage,
        finish_reason: Some("max_iterations".to_owned()),
        activity_list: captured_activity_list,
        tool_calls_count,
        tools_called,
        pending_provider_auth_required: None,
    })
}

// ============================================================================
// CLI Tool Loop (text-based tool calling for CLI providers)
// ============================================================================

/// Maximum number of tool-calling iterations for CLI providers.
/// CLI providers are slower (subprocess per call), so keep this conservative.
const CLI_MAX_TOOL_ITERATIONS: usize = 5;

/// Executes the tool loop using text-based tool calling for CLI providers.
///
/// Uses embacle's [`tool_simulation`] module for catalog generation, tool call
/// parsing, and result formatting. The MCP execution and domain-specific logic
/// (activity list capture) remain here.
///
/// Flow:
/// 1. Inject tool catalog into system prompt (via embacle)
/// 2. Call `complete()` → parse `<tool_call>` blocks (via embacle)
/// 3. If tool calls found: execute via MCP, format results (via embacle), iterate
/// 4. Repeat until text response or max iterations
///
/// # Errors
///
/// Returns error if the LLM call or tool execution fails.
pub async fn run_cli_tool_loop(
    params: &ToolLoopParams<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> Result<ToolLoopResult, AppError> {
    // Convert pierre-llm declarations to embacle declarations and generate catalog
    let embacle_decls = to_embacle_declarations(&params.tools.function_declarations);
    let tool_catalog = tool_simulation::generate_tool_catalog(&embacle_decls);
    tool_simulation::inject_tool_catalog(llm_messages, &tool_catalog);

    let mut captured_activity_list: Option<String> = None;
    let mut tool_calls_count: u32 = 0;
    let mut tools_called: Vec<String> = Vec::new();
    let max_iterations = params.max_iterations.min(CLI_MAX_TOOL_ITERATIONS);

    for iteration in 0..max_iterations {
        let llm_request = {
            let req = ChatRequest::new(llm_messages.clone()).with_model(params.model);
            match params.temperature {
                Some(t) => req.with_temperature(t),
                None => req,
            }
        };
        let call_start = Instant::now();
        let cached_slot = Arc::new(AtomicU32::new(0));
        let slot_for_scope = cached_slot.clone();
        let response_result = pierre_llm::LAST_CACHED_TOKENS
            .scope(slot_for_scope, params.provider.complete(&llm_request))
            .await;
        let latency_ms = millis_elapsed(call_start);
        let cached_tokens = i64::from(cached_slot.load(Ordering::SeqCst));
        let call_seq = Some(i64::try_from(iteration).unwrap_or(i64::MAX) + 1);
        let response = match response_result {
            Ok(r) => r,
            Err(e) => {
                emit_call_record(
                    params.call_recorder.as_ref(),
                    params.provider.name(),
                    params.model,
                    None,
                    0,
                    latency_ms,
                    false,
                    call_seq,
                    Vec::new(),
                );
                return Err(e);
            }
        };

        // Parse <tool_call> blocks from the response text (via embacle).
        // Done before `emit_call_record_with_text` so the per-call usage row
        // carries the parsed tool names — the structured `function_calls`
        // field is absent on this provider's `complete()` return type.
        let embacle_calls = tool_simulation::parse_tool_call_blocks(&response.content);
        let tools_in_response: Vec<String> = embacle_calls.iter().map(|c| c.name.clone()).collect();
        // Last user prompt feeds the character-based token estimator when
        // the provider returns `usage: None` (e.g. Copilot ACP, which
        // doesn't expose token counts). Without this fallback the per-call
        // row would land with zeros and no `_estimated` suffix.
        let last_user_prompt = llm_messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::User))
            .map(|m| m.content.as_str())
            .unwrap_or_default()
            .to_owned();
        emit_call_record_with_text(
            params.call_recorder.as_ref(),
            params.provider.name(),
            params.model,
            response.usage.as_ref(),
            cached_tokens,
            latency_ms,
            true,
            call_seq,
            Some(&last_user_prompt),
            Some(&response.content),
            tools_in_response,
        );

        if embacle_calls.is_empty() {
            // No tool calls — this is the final text response
            let content = tool_simulation::strip_tool_call_blocks(&response.content);
            return Ok(ToolLoopResult {
                content,
                usage: response.usage,
                finish_reason: response.finish_reason,
                activity_list: captured_activity_list,
                tool_calls_count,
                tools_called,
                pending_provider_auth_required: None,
            });
        }

        // Convert to pierre-llm types for MCP execution
        let parsed_tool_calls = from_embacle_calls(embacle_calls);

        info!(
            "CLI iteration {}: Parsed {} tool call(s) from text",
            iteration,
            parsed_tool_calls.len()
        );

        // Execute the parsed tool calls via MCP
        let ExecutedFunctionCalls {
            responses: function_responses,
            auth_required_provider,
        } = execute_function_calls(
            &params.executor,
            &parsed_tool_calls,
            params.user_id,
            params.tenant_id,
        )
        .await?;

        #[allow(clippy::cast_possible_truncation)]
        {
            tool_calls_count += parsed_tool_calls.len() as u32;
        }

        tools_called.extend(parsed_tool_calls.iter().map(|c| c.name.clone()));

        // Auth-required short-circuit (mirror of the API loop): exit early so
        // the chat pipeline can render the deterministic hosted-login reply.
        if let Some(provider) = auth_required_provider {
            return Ok(ToolLoopResult {
                content: String::new(),
                usage: response.usage,
                finish_reason: Some("provider_auth_required".to_owned()),
                activity_list: captured_activity_list,
                tool_calls_count,
                tools_called,
                pending_provider_auth_required: Some(provider),
            });
        }

        // Add assistant message (with tool calls stripped via embacle)
        let assistant_text = tool_simulation::strip_tool_call_blocks(&response.content);
        if !assistant_text.is_empty() {
            llm_messages.push(ChatMessage::assistant(&assistant_text));
        }

        // Format tool results as text (via embacle) and inject as user message
        let embacle_responses = to_embacle_responses(&function_responses);
        let tool_results_text = tool_simulation::format_tool_results_as_text(&embacle_responses);

        // Capture activity list if present in function responses
        if let Some(list) = extract_activity_list(&function_responses) {
            captured_activity_list = Some(list);
        }

        llm_messages.push(ChatMessage::user(&tool_results_text));
    }

    // Max iterations reached without a final text response
    Ok(ToolLoopResult {
        content: String::new(),
        usage: None,
        finish_reason: Some("max_iterations".to_owned()),
        activity_list: captured_activity_list,
        tool_calls_count,
        tools_called,
        pending_provider_auth_required: None,
    })
}

// ============================================================================
// Type Conversions: pierre-llm ↔ embacle::tool_simulation
// ============================================================================

/// Convert pierre-llm function declarations to embacle `tool_simulation` declarations.
fn to_embacle_declarations(
    decls: &[FunctionDeclaration],
) -> Vec<tool_simulation::FunctionDeclaration> {
    decls
        .iter()
        .map(|d| tool_simulation::FunctionDeclaration {
            name: d.name.clone(),
            description: d.description.clone(),
            parameters: d.parameters.clone(),
        })
        .collect()
}

/// Convert embacle `tool_simulation` function calls to pierre-llm function calls.
fn from_embacle_calls(calls: Vec<tool_simulation::FunctionCall>) -> Vec<FunctionCall> {
    calls
        .into_iter()
        .map(|c| FunctionCall {
            name: c.name,
            args: c.args,
        })
        .collect()
}

/// Convert pierre-llm function responses to embacle `tool_simulation` responses.
fn to_embacle_responses(resps: &[FunctionResponse]) -> Vec<tool_simulation::FunctionResponse> {
    resps
        .iter()
        .map(|r| tool_simulation::FunctionResponse {
            name: r.name.clone(),
            response: r.response.clone(),
        })
        .collect()
}

/// Convert an [`Instant`] elapsed time into milliseconds, saturating
/// at `i64::MAX` for pathologically long calls.
fn millis_elapsed(start: Instant) -> i64 {
    let ms = start.elapsed().as_millis();
    i64::try_from(ms).unwrap_or(i64::MAX)
}

/// Emit a structured log marking the start of one tool-loop iteration.
///
/// Extracted from [`run_api_tool_loop`] to keep the loop body inside
/// the workspace cognitive complexity budget. Records the iteration
/// index and resolved provider/model so an operator can tie the call
/// to its eventual `llm_usage` row by `turn_id` + `call_sequence`.
fn log_iteration_start(iteration: usize, params: &ToolLoopParams<'_>, message_count: usize) {
    info!(
        iteration,
        provider = params.provider.name(),
        model = params.model,
        message_count,
        "tool loop iteration: dispatching to provider"
    );
}

/// Emit a structured log summarizing the provider's response for one
/// tool-loop iteration.
fn log_iteration_response(iteration: usize, latency_ms: i64, response: &ChatResponseWithTools) {
    info!(
        iteration,
        latency_ms,
        content_len = response.content.as_deref().map_or(0, str::len),
        function_calls = response.function_calls.as_ref().map_or(0, Vec::len),
        prompt_tokens = response.usage.as_ref().map_or(0, |u| u.prompt_tokens),
        completion_tokens = response.usage.as_ref().map_or(0, |u| u.completion_tokens),
        "tool loop iteration: provider response received"
    );
}

/// Hand one [`LlmCallRecord`] to the optional sink. Centralises token
/// extraction so the three tool-loop variants can share the same
/// recording contract. `cached_tokens` is zero unless the provider
/// wrapped its usage in
/// [`pierre_core::llm::ExtendedTokenUsage`] and forwarded it through
/// the caller. `call_sequence` is the 1-based turn-local position of
/// the call (1, 2, 3, ...).
#[allow(clippy::too_many_arguments)]
fn emit_call_record(
    recorder: Option<&Arc<dyn LlmCallRecorder>>,
    provider: &str,
    model: &str,
    usage: Option<&TokenUsage>,
    cached_tokens: i64,
    latency_ms: i64,
    success: bool,
    call_sequence: Option<i64>,
    tools_called: Vec<String>,
) {
    emit_call_record_with_text(
        recorder,
        provider,
        model,
        usage,
        cached_tokens,
        latency_ms,
        success,
        call_sequence,
        None,
        None,
        tools_called,
    );
}

/// Variant of [`emit_call_record`] that estimates token counts from
/// character-based prompt/completion text when the provider returns
/// no usage, so CLI runners (Claude Code, Copilot, Cursor, etc.) produce
/// non-zero usage rows instead of silently dropping.
#[allow(clippy::too_many_arguments)]
fn emit_call_record_with_text(
    recorder: Option<&Arc<dyn LlmCallRecorder>>,
    provider: &str,
    model: &str,
    usage: Option<&TokenUsage>,
    cached_tokens: i64,
    latency_ms: i64,
    success: bool,
    call_sequence: Option<i64>,
    prompt_text: Option<&str>,
    completion_text: Option<&str>,
    tools_called: Vec<String>,
) {
    let Some(recorder) = recorder else {
        return;
    };
    let (prompt_tokens, completion_tokens, estimated) = usage.map_or_else(
        || match (prompt_text, completion_text) {
            (Some(p), Some(c)) => {
                let (est_p, est_c) = estimate_chat_tokens(p, c);
                (i64::from(est_p), i64::from(est_c), true)
            }
            _ => (0, 0, false),
        },
        |u| {
            (
                i64::from(u.prompt_tokens),
                i64::from(u.completion_tokens),
                false,
            )
        },
    );
    recorder.record(LlmCallRecord {
        provider: provider.to_owned(),
        model: model.to_owned(),
        prompt_tokens,
        completion_tokens,
        cached_tokens,
        latency_ms,
        success,
        call_sequence,
        token_counts_estimated: estimated,
        tools_called,
    });
}

// Re-export embacle's pure functions for direct use (no type conversion needed)
pub use tool_simulation::inject_tool_catalog as inject_tool_catalog_into_system_prompt;
pub use tool_simulation::strip_tool_call_blocks;

/// Generate a text-based tool catalog from pierre-llm function declarations.
///
/// Thin wrapper around [`embacle::tool_simulation::generate_tool_catalog`] that
/// handles type conversion.
#[must_use]
pub fn generate_tool_catalog(declarations: &[FunctionDeclaration]) -> String {
    let embacle_decls = to_embacle_declarations(declarations);
    tool_simulation::generate_tool_catalog(&embacle_decls)
}

/// Parse `<tool_call>` blocks from CLI text output into pierre-llm function calls.
///
/// Thin wrapper around [`embacle::tool_simulation::parse_tool_call_blocks`] that
/// handles type conversion.
#[must_use]
pub fn parse_tool_call_blocks(content: &str) -> Vec<FunctionCall> {
    from_embacle_calls(tool_simulation::parse_tool_call_blocks(content))
}

/// Format pierre-llm function responses as `<tool_result>` text blocks.
///
/// Thin wrapper around [`embacle::tool_simulation::format_tool_results_as_text`] that
/// handles type conversion.
#[must_use]
pub fn format_tool_results_as_text(responses: &[FunctionResponse]) -> String {
    let embacle_responses = to_embacle_responses(responses);
    tool_simulation::format_tool_results_as_text(&embacle_responses)
}

/// Extract activity list from function responses (for `get_activities` results).
pub fn extract_activity_list(responses: &[FunctionResponse]) -> Option<String> {
    for resp in responses {
        if resp.name == "get_activities" {
            if let Some(activity_list) = resp
                .response
                .get("activity_list")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                let list_len = activity_list.len();
                info!("Extracted activity list ({list_len} chars) to prepend to response");
                return Some(activity_list.to_owned());
            }
        }
    }
    None
}

// ============================================================================
// Shared Infrastructure
// ============================================================================

/// Execute a batch of function calls via the MCP executor and return responses.
///
/// # Errors
///
/// Returns error if any tool execution produces an unrecoverable failure.
pub async fn execute_function_calls(
    executor: &UniversalExecutor,
    function_calls: &[FunctionCall],
    user_id: &str,
    tenant_id: TenantId,
) -> Result<ExecutedFunctionCalls, AppError> {
    use crate::formatters::TokenEfficiencyMetrics;

    let mut responses = Vec::with_capacity(function_calls.len());
    let mut auth_required_provider: Option<String> = None;
    for function_call in function_calls {
        info!(
            tool_name = %function_call.name,
            args = %function_call.args,
            "Executing tool"
        );
        let tool_start = Instant::now();
        let tool_response = execute_mcp_tool(executor, function_call, user_id, tenant_id).await;
        let tool_duration_ms = u64::try_from(tool_start.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Capture the auth-required provider before building the
        // `FunctionResponse`, which intentionally drops `metadata` (the LLM
        // doesn't need it). First tool to trip wins so we don't lose it across
        // a multi-tool batch.
        if auth_required_provider.is_none() {
            if let Some(meta) = tool_response.metadata.as_ref() {
                if let Some(serde_json::Value::String(p)) = meta.get(META_AUTH_REQUIRED_PROVIDER) {
                    auth_required_provider = Some(p.clone());
                }
            }
        }

        let func_response = build_function_response(function_call, &tool_response);

        // Phase 4: PostHog visibility on every tool dispatch.
        analytics().track_tool_executed(
            "chat",
            &tenant_id.to_string(),
            user_id,
            &function_call.name,
            tool_response.success,
            tool_duration_ms,
        );

        // Measure serialized response size and estimate token cost
        let serialized = serde_json::to_string(&func_response.response).unwrap_or_default();
        let byte_size = serialized.len();
        let estimated_tokens = TokenEfficiencyMetrics::estimate_tokens(&serialized);
        let name = &func_response.name;
        info!(
            event_type = "tool_response_size",
            tool_name = %name,
            response_bytes = byte_size,
            estimated_tokens = estimated_tokens,
            "Tool response measurement"
        );

        responses.push(func_response);
    }
    Ok(ExecutedFunctionCalls {
        responses,
        auth_required_provider,
    })
}

/// Output of [`execute_function_calls`].
///
/// Carries the function responses for the LLM plus an out-of-band signal
/// for the tool loop when one of the calls failed with
/// `AppError::ProviderAuthRequired`. The signal travels separately
/// because `FunctionResponse` drops the underlying
/// `UniversalResponse::metadata` to keep the LLM-visible payload minimal.
pub struct ExecutedFunctionCalls {
    /// LLM-visible function responses, one per call in input order.
    pub responses: Vec<FunctionResponse>,
    /// Provider slug of the first tool that returned `ProviderAuthRequired`,
    /// or `None` if every call landed cleanly. The tool loop short-circuits
    /// on `Some(_)` and the chat pipeline mints a hosted-login URL.
    pub auth_required_provider: Option<String>,
}

/// Execute a single MCP tool call and return the response.
///
/// Runs the Sprint C10 post-LLM allowlist check before dispatch so a
/// prompt-injected tool name that slipped past the catalog filter cannot
/// actually reach the tool handler. Tool execution errors are converted
/// to failed responses so the LLM can observe them in the next turn.
async fn execute_mcp_tool(
    executor: &UniversalExecutor,
    function_call: &FunctionCall,
    user_id: &str,
    tenant_id: TenantId,
) -> UniversalResponse {
    if let Some(blocked) = enforce_tool_allowlist(executor, &function_call.name, tenant_id).await {
        return blocked;
    }

    let request = UniversalRequest {
        tool_name: function_call.name.clone(),
        parameters: function_call.args.clone(),
        user_id: user_id.to_owned(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant_id.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };

    match executor.execute_tool(request).await {
        Ok(response) => response,
        Err(e) => {
            // Preserve the `ProviderAuthRequired` signal across the
            // `ProtocolError → UniversalResponse` boundary by stuffing the
            // provider slug into `metadata` under `META_AUTH_REQUIRED_PROVIDER`.
            // The tool loop scans for this key and exits early; the chat
            // pipeline mints a hosted-login URL and surfaces it deterministically.
            let metadata = e.provider_auth_required_provider().map(|provider| {
                let mut m: HashMap<String, serde_json::Value> = HashMap::new();
                m.insert(
                    META_AUTH_REQUIRED_PROVIDER.to_owned(),
                    serde_json::Value::String(provider.to_owned()),
                );
                m
            });
            UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Tool execution failed: {e}")),
                metadata,
            }
        }
    }
}

/// Phase C Sprint C10 post-LLM tool allowlist enforcement.
///
/// Queries [`ToolSelectionService::is_tool_enabled`] for `(tenant_id,
/// tool_name)`. Returns `Some(blocked_response)` when the tenant has
/// disabled the tool — the caller forwards that as the tool's wire
/// response so the LLM sees structured feedback rather than silent
/// failure. Returns `None` when the tool is enabled (or the selection
/// service cannot resolve an override, in which case we fail-open to
/// avoid wedging coaches during tool-selection outages).
///
/// Tools not known to the tool-selection catalog at all are allowed
/// through — the pre-LLM catalog already filters the tool list exposed
/// to the model, so any tool name the LLM produced is one we shipped;
/// the only failure mode this guards against is a coach being
/// prompt-injected into calling a tool the tenant has *explicitly
/// disabled*.
async fn enforce_tool_allowlist(
    executor: &UniversalExecutor,
    tool_name: &str,
    tenant_id: TenantId,
) -> Option<UniversalResponse> {
    let service = executor.resources.tool_selection.as_ref();
    match service.is_tool_enabled(tenant_id, tool_name).await {
        Ok(true) => None,
        Ok(false) => {
            warn!(
                tenant_id = %tenant_id,
                tool_name = %tool_name,
                "tool_allowlist_block: LLM picked a tool the tenant has disabled — possible prompt injection"
            );
            let mut metadata: HashMap<String, serde_json::Value> = HashMap::new();
            metadata.insert(
                "blocked_reason".to_owned(),
                serde_json::Value::String("tool_allowlist".to_owned()),
            );
            metadata.insert(
                "tool_name".to_owned(),
                serde_json::Value::String(tool_name.to_owned()),
            );
            Some(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!(
                    "Tool '{tool_name}' is disabled for this tenant. \
                     Pick a different tool or answer from memory."
                )),
                metadata: Some(metadata),
            })
        }
        Err(e) => {
            // Fail-open: tool-selection outages must not wedge every
            // coach conversation. Log and let the request through so
            // the existing pre-LLM catalog filter remains the primary
            // gate.
            warn!(
                tenant_id = %tenant_id,
                tool_name = %tool_name,
                error = %e,
                "tool_allowlist_check_failed: falling open to pre-LLM catalog filter"
            );
            None
        }
    }
}

/// Build a function response from an MCP tool response.
fn build_function_response(
    function_call: &FunctionCall,
    response: &UniversalResponse,
) -> FunctionResponse {
    let result_value = if response.success {
        response
            .result
            .clone()
            .unwrap_or_else(|| serde_json::json!({"status": "success"}))
    } else {
        serde_json::json!({
            "error": response.error.as_deref().unwrap_or("Unknown error")
        })
    };

    FunctionResponse {
        name: function_call.name.clone(),
        response: result_value,
    }
}

/// Add function responses as user messages for the next LLM iteration.
///
/// Returns the activity list if found (to prepend to final response).
pub fn add_function_responses_to_messages(
    llm_messages: &mut Vec<ChatMessage>,
    function_responses: &[FunctionResponse],
) -> Option<String> {
    let mut activity_list_content: Option<String> = None;

    for func_response in function_responses {
        let response_text =
            serde_json::to_string(&func_response.response).unwrap_or_else(|_| "{}".to_owned());

        // For get_activities, extract the activity_list to prepend to final response
        if func_response.name == "get_activities" {
            if let Some(activity_list) = func_response
                .response
                .get("activity_list")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                let list_len = activity_list.len();
                activity_list_content = Some(activity_list.to_owned());
                info!("Extracted activity list ({list_len} chars) to prepend to response");
            }
        }

        // All tool results use the same format
        let name = &func_response.name;
        let message = format!("[Tool Result for {name}]: {response_text}");
        llm_messages.push(ChatMessage::user(message));
    }

    activity_list_content
}

/// Select and run the appropriate tool loop strategy based on provider capabilities.
///
/// Three-way dispatch:
/// - Providers with `FUNCTION_CALLING` capability use [`run_api_tool_loop`]
/// - Providers with `SDK_TOOL_CALLING` capability use [`run_headless_tool_loop`]
/// - Providers with neither use [`run_cli_tool_loop`] (text-based tool calling)
///
/// # Errors
///
/// Returns error if the LLM call or tool execution fails.
pub async fn run_tool_loop(
    params: &ToolLoopParams<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> Result<ToolLoopResult, AppError> {
    let capabilities = params.provider.capabilities();
    if capabilities.supports_function_calling() {
        run_api_tool_loop(params, llm_messages).await
    } else if capabilities.supports_sdk_tool_calling() {
        run_headless_tool_loop(params, llm_messages).await
    } else {
        run_cli_tool_loop(params, llm_messages).await
    }
}

// ============================================================================
// Headless Tool Loop (Copilot ACP autonomous tool calling)
// ============================================================================

/// Executes the tool loop using Copilot Headless ACP (Agent Client Protocol).
///
/// Copilot Headless manages its own tool execution loop internally via ACP.
/// Tool calls are observed and reported via [`HeadlessToolResponse`], but the
/// caller does not execute tools — Copilot handles that autonomously.
///
/// # Errors
///
/// Returns error if the headless runner cannot be extracted, or the ACP call fails.
async fn run_headless_tool_loop(
    params: &ToolLoopParams<'_>,
    llm_messages: &[ChatMessage],
) -> Result<ToolLoopResult, AppError> {
    // Extract the CopilotHeadlessRunner from the ChatProvider
    let cli_provider = params.provider.as_cli_provider().ok_or_else(|| {
        AppError::internal(
            "Headless tool loop requires CopilotHeadlessRunner but provider is not a CLI provider",
        )
    })?;

    let headless_runner = cli_provider.as_headless_runner().ok_or_else(|| {
        AppError::internal(
            "Headless tool loop requires CopilotHeadlessRunner but inner runner is a different type",
        )
    })?;

    let stream = params.stream_sink.is_some();
    info!(
        stream,
        "Headless tool loop: invoking Copilot ACP {}",
        if stream {
            "converse_stream()"
        } else {
            "converse()"
        }
    );

    // Build the ChatRequest from the accumulated messages
    let request = {
        let req = ChatRequest::new(llm_messages.to_vec()).with_model(params.model);
        match params.temperature {
            Some(t) => req.with_temperature(t),
            None => req,
        }
    };

    // Copilot Headless handles tool execution internally via ACP
    let call_start = Instant::now();
    let converse_result = if let Some(sink) = params.stream_sink.as_ref() {
        run_headless_streaming(headless_runner, &request, sink).await
    } else {
        headless_runner
            .converse(&request)
            .await
            .map_err(AppError::from)
    };
    let latency_ms = millis_elapsed(call_start);
    let last_user_prompt = llm_messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, super::super::llm::MessageRole::User))
        .map(|m| m.content.as_str())
        .unwrap_or_default()
        .to_owned();
    let headless_response = match converse_result {
        Ok(r) => {
            let tools_in_response: Vec<String> =
                r.tool_calls.iter().map(|tc| tc.title.clone()).collect();
            emit_call_record_with_text(
                params.call_recorder.as_ref(),
                params.provider.name(),
                params.model,
                r.usage.as_ref(),
                0,
                latency_ms,
                true,
                Some(1),
                Some(&last_user_prompt),
                Some(&r.content),
                tools_in_response,
            );
            r
        }
        Err(e) => {
            emit_call_record(
                params.call_recorder.as_ref(),
                params.provider.name(),
                params.model,
                None,
                0,
                latency_ms,
                false,
                Some(1),
                Vec::new(),
            );
            return Err(e);
        }
    };

    let tool_calls_count = u32::try_from(headless_response.tool_calls.len()).unwrap_or(u32::MAX);

    info!(
        content_len = headless_response.content.len(),
        model = %headless_response.model,
        tool_calls = tool_calls_count,
        "Headless tool loop completed"
    );

    // `ObservedToolCall` only exposes `title` on the ACP wire; there's no
    // distinct `name` field, so the title is the best available identifier.
    let tools_called: Vec<String> = headless_response
        .tool_calls
        .iter()
        .map(|call| call.title.clone())
        .collect();

    Ok(ToolLoopResult {
        content: headless_response.content,
        usage: headless_response.usage,
        finish_reason: headless_response.finish_reason,
        // Copilot Headless manages tools autonomously via ACP — the platform
        // never sees individual tool responses, so activity_list extraction
        // (which requires inspecting get_activities results) is not possible.
        activity_list: None,
        tool_calls_count,
        tools_called,
        // Copilot Headless owns its own tool calls inside the ACP subprocess;
        // platform-side ProviderAuthRequired handoff is not possible without
        // visibility into the subprocess tool dispatches. Leave as None.
        pending_provider_auth_required: None,
    })
}

/// Run a streaming Copilot ACP turn, forwarding text deltas and tool-call
/// observations to `sink` while accumulating the same final
/// [`HeadlessToolResponse`] that the non-streaming `converse()` produces.
///
/// Returns the aggregated response so the caller can record per-call usage
/// and fold it into `ToolLoopResult` exactly like the non-streaming branch.
async fn run_headless_streaming(
    headless_runner: &pierre_llm::CopilotHeadlessRunner,
    request: &ChatRequest,
    sink: &ChatStreamSink,
) -> Result<pierre_llm::HeadlessToolResponse, AppError> {
    use pierre_llm::HeadlessStreamEvent;
    use tokio_stream::StreamExt;

    let mut stream = headless_runner
        .converse_stream(request)
        .await
        .map_err(AppError::from)?;
    let mut final_response: Option<pierre_llm::HeadlessToolResponse> = None;

    while let Some(item) = stream.next().await {
        let event = item.map_err(AppError::from)?;
        match event {
            HeadlessStreamEvent::TextDelta(delta) => {
                // Send may fail if the receiver was dropped (client disconnected
                // or the pipeline aborted) — treat as a benign no-op so the ACP
                // session keeps draining and the run still completes cleanly.
                let _ = sink.send(ChatStreamEvent::TextDelta(delta));
            }
            HeadlessStreamEvent::ToolCall(tc) => {
                let _ = sink.send(ChatStreamEvent::ToolCall {
                    id: tc.id,
                    title: tc.title,
                    status: tc.status,
                });
            }
            HeadlessStreamEvent::Done(response) => {
                final_response = Some(response);
            }
        }
    }

    final_response.ok_or_else(|| {
        AppError::external_service(
            "copilot-headless",
            "converse_stream completed without a Done event",
        )
    })
}

// ============================================================================
// Tool Declarations
// ============================================================================

/// Build the LLM tool definition set for chat-mode function calling.
///
/// Returns the provider-agnostic `Tool` struct consumed by `ToolLoopParams`.
/// Lives here instead of the routes layer because tool definitions are
/// business logic (what capabilities the LLM sees), not transport concerns.
#[must_use]
pub fn build_mcp_tools() -> Tool {
    let mut declarations = Vec::with_capacity(14);
    declarations.extend(build_connection_tools());
    declarations.extend(build_activity_tools());
    declarations.extend(build_analysis_tools());
    declarations.extend(build_recovery_tools());
    Tool {
        function_declarations: declarations,
    }
}

fn build_connection_tools() -> Vec<FunctionDeclaration> {
    vec![
        FunctionDeclaration {
            name: "get_connection_status".to_owned(),
            description: "Check which fitness providers are connected".to_owned(),
            parameters: Some(serde_json::json!({"type": "object", "properties": {}})),
        },
        FunctionDeclaration {
            name: "connect_provider".to_owned(),
            description: "Connect to a fitness provider via OAuth".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {"provider": {"type": "string"}},
                "required": ["provider"]
            })),
        },
        FunctionDeclaration {
            name: "disconnect_provider".to_owned(),
            description: "Disconnect a fitness provider".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {"provider": {"type": "string"}},
                "required": ["provider"]
            })),
        },
    ]
}

fn build_activity_tools() -> Vec<FunctionDeclaration> {
    vec![
        FunctionDeclaration {
            name: "get_activities".to_owned(),
            description: "Get user's recent fitness activities".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string"},
                    "limit": {"type": "integer"},
                    "offset": {"type": "integer"}
                },
                "required": ["provider"]
            })),
        },
        FunctionDeclaration {
            name: "get_athlete".to_owned(),
            description: "Get user's athlete profile information".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {"provider": {"type": "string"}},
                "required": ["provider"]
            })),
        },
        FunctionDeclaration {
            name: "get_stats".to_owned(),
            description: "Get user's overall fitness statistics".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {"provider": {"type": "string"}},
                "required": ["provider"]
            })),
        },
    ]
}

fn build_analysis_tools() -> Vec<FunctionDeclaration> {
    vec![
        FunctionDeclaration {
            name: "analyze_activity".to_owned(),
            description: "Deep analysis of a specific activity".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string"},
                    "activity_id": {"type": "string"}
                },
                "required": ["provider", "activity_id"]
            })),
        },
        FunctionDeclaration {
            name: "get_activity_intelligence".to_owned(),
            description: "AI-powered insights including location and weather".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string"},
                    "activity_id": {"type": "string"},
                    "include_location": {"type": "boolean"},
                    "include_weather": {"type": "boolean"}
                },
                "required": ["provider", "activity_id"]
            })),
        },
        FunctionDeclaration {
            name: "analyze_performance_trends".to_owned(),
            description: "Analyze performance trends over time".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string"},
                    "timeframe": {"type": "string"},
                    "metric": {"type": "string"},
                    "sport_type": {"type": "string"}
                },
                "required": ["provider", "timeframe", "metric"]
            })),
        },
        FunctionDeclaration {
            name: "compare_activities".to_owned(),
            description: "Compare activity against similar or personal bests".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string"},
                    "activity_id": {"type": "string"},
                    "compare_type": {"type": "string"}
                },
                "required": ["provider", "activity_id"]
            })),
        },
        FunctionDeclaration {
            name: "calculate_fitness_score".to_owned(),
            description: "Calculate overall fitness score and trends".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string"},
                    "timeframe": {"type": "string"},
                    "sleep_provider": {"type": "string"}
                },
                "required": ["provider"]
            })),
        },
        FunctionDeclaration {
            name: "analyze_training_load".to_owned(),
            description: "Analyze training load and recovery needs".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string"},
                    "timeframe": {"type": "string"},
                    "sleep_provider": {"type": "string"}
                },
                "required": ["provider"]
            })),
        },
    ]
}

fn build_recovery_tools() -> Vec<FunctionDeclaration> {
    vec![
        FunctionDeclaration {
            name: "calculate_recovery_score".to_owned(),
            description: "Calculate recovery score with daily strain (WHOOP cycles), HRV, sleep quality, and TSB. Use when user asks about recovery, daily strain, WHOOP cycles, or training readiness.".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "activity_provider": {"type": "string"},
                    "sleep_provider": {"type": "string"}
                }
            })),
        },
        FunctionDeclaration {
            name: "suggest_rest_day".to_owned(),
            description: "AI recommendation for rest day".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "activity_provider": {"type": "string"},
                    "sleep_provider": {"type": "string"}
                }
            })),
        },
        FunctionDeclaration {
            name: "generate_recommendations".to_owned(),
            description: "Get personalized training recommendations".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string"},
                    "recommendation_type": {"type": "string"},
                    "activity_id": {"type": "string"}
                },
                "required": ["provider"]
            })),
        },
    ]
}

// ============================================================================
// Content Sanitization
// ============================================================================

/// Strip synthetic function call syntax from LLM content.
///
/// Some models (like Llama via Groq) output function calls both as proper
/// `tool_calls` AND as text content using syntax like
/// `<function(name)>{...}</function>`. This helper removes that synthetic
/// syntax to avoid displaying raw tool-call markup to users.
#[must_use]
pub fn strip_synthetic_function_calls(content: &str) -> Cow<'_, str> {
    use regex::Regex;
    use std::sync::OnceLock;

    fn function_pattern() -> Option<&'static Regex> {
        static PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
        PATTERN
            .get_or_init(|| Regex::new(r"<function[/\(][^>]+>[\s\S]*?</function>").ok())
            .as_ref()
    }

    let Some(pattern) = function_pattern() else {
        return Cow::Borrowed(content);
    };

    let cleaned = pattern.replace_all(content, "");
    let trimmed = cleaned.trim();

    if trimmed.is_empty() {
        Cow::Borrowed("")
    } else if trimmed.len() == content.len() {
        Cow::Borrowed(content)
    } else {
        Cow::Owned(trimmed.to_owned())
    }
}
