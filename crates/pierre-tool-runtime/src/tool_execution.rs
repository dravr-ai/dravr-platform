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

use crate::guardian::{StepOutput, TurnKey};
use crate::protocol::types::META_AUTH_REQUIRED_PROVIDER;
use crate::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use crate::registry::ToolRegistry;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_llm::{
    ChatMessage, ChatProvider, ChatRequest, ChatResponseWithTools, FunctionCall,
    FunctionDeclaration, FunctionResponse, McpServerConfig, MessageRole, TokenUsage, Tool,
};
use pierre_services::chat_stream::{ChatStreamEvent, ChatStreamSink};

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

/// One round of tool dispatch as the in-memory loop sees it.
///
/// Carries an optional preamble of assistant text that accompanied the tool
/// request, plus the formatted tool result text. Persisting both lets a
/// follow-up turn replay the same [`ChatMessage`] sequence the in-memory
/// loop produced and gives the model the same evidence base when answering
/// grounded follow-ups.
#[derive(Debug, Clone)]
pub struct ToolRoundRecord {
    /// Assistant text emitted alongside the tool call. Empty when the
    /// model returned only a tool call (no preamble). The replay path
    /// inserts this as `ChatMessage::assistant(content)`.
    pub assistant_text: String,
    /// Formatted tool result the loop pushes back to the model as a user
    /// turn (e.g. `"[Tool Result for get_activities]: ..."`). Always set.
    pub tool_result_text: String,
}

/// Sink that receives one [`ToolRoundRecord`] per tool dispatch round.
///
/// Implementations persist the round so subsequent turns rebuild the same
/// `Vec<ChatMessage>` that the in-memory loop produced. Without this, the
/// next turn sees only the final assistant text and the model — having no
/// trace of the tool result it consumed — defaults to refusing grounded
/// follow-ups ("I don't have access to your Strava data").
///
/// Invocations happen on the async runtime but the sink method itself is
/// synchronous; implementers spawn a task or push to a channel if the
/// underlying persistence is async.
pub trait ToolMessageRecorder: Send + Sync {
    /// Record a completed tool dispatch round.
    fn record(&self, record: ToolRoundRecord);
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
    /// Optional sink that persists each tool dispatch round (assistant
    /// preamble + formatted tool result) so follow-up turns can replay
    /// the grounded evidence the model already saw. Absent in callers
    /// that have no conversation to attach the rows to (e.g. one-shot
    /// non-conversational tool runs).
    pub tool_message_recorder: Option<Arc<dyn ToolMessageRecorder>>,
    /// Optional per-coach LLM sampling temperature. When `Some`, applied
    /// to every `ChatRequest` in the loop via `with_temperature`. When
    /// `None`, the provider/server default is used.
    pub temperature: Option<f32>,
    /// Optional sink for token-level streaming events. When set on the
    /// headless tool loop branch, the loop calls Copilot's
    /// `converse_stream()` instead of `converse()` and forwards each
    /// observed text delta and tool-call snapshot through the sink.
    /// The sink is a [`pierre_services::chat_stream::ChatStreamSink`]
    /// — see that type for the event shape.
    pub stream_sink: Option<ChatStreamSink>,
    /// MCP servers exposed to an ACP-managed provider (Copilot Headless) so
    /// the model can call Dravr tools natively over the Agent Client Protocol
    /// instead of text-based `<tool_call>` simulation. Only the headless tool
    /// loop forwards these into the ACP `session/new`; other loops ignore
    /// them. Empty for providers without SDK tool calling.
    pub mcp_servers: Vec<McpServerConfig>,
}

/// A tool blocked by the runtime Guardian while in `enforce` mode.
///
/// Surfaced out of the tool loop as an out-of-band signal (parallel to
/// `pending_provider_auth_required`) so the chat pipeline can render a
/// deterministic, localized "blocked for safety" reply instead of feeding the
/// raw in-band denial back to the LLM and letting it paraphrase a refusal.
/// Only ever set in `enforce` mode — `observe` (the default) logs and falls
/// through to execution, so this stays `None`.
#[derive(Debug, Clone)]
pub struct GuardianDenial {
    /// Name of the tool the Guardian blocked.
    pub tool_name: String,
    /// Machine-readable denial reason (`DenyReason::as_str`), e.g.
    /// `budget_exceeded`, `tainted_sink`, `egress_forbidden`. Used for
    /// structured logging — never shown verbatim to the user.
    pub reason: String,
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
    /// The first tool the runtime Guardian blocked this turn (`enforce` mode
    /// only). The chat pipeline detects this and short-circuits with a
    /// localized "blocked for safety" reply (`KEY_GUARDIAN_DENIED`). `None`
    /// when no tool was denied (always, in `observe`).
    pub guardian_denied: Option<GuardianDenial>,
}

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
#[tracing::instrument(
    skip_all,
    fields(
        tenant_id = %params.tenant_id,
        user_id = %params.user_id,
        provider = %params.provider.name(),
        model = %params.model,
    )
)]
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

        // notify: LLM call about to start. Per-iteration so tool-loop
        // breadth is observable from Slack (paired with the completion
        // event right after the await returns).
        info!(
            target: "notify",
            event = "embacle.call_started",
            model = %params.model,
            "LLM call dispatching"
        );

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
                emit_call_record(CallRecordInputs {
                    recorder: params.call_recorder.as_ref(),
                    provider: params.provider.name(),
                    model: params.model,
                    usage: r.usage.as_ref(),
                    cached_tokens,
                    latency_ms,
                    success: true,
                    call_sequence: call_seq,
                    tools_called: tools_in_response,
                });
                // notify: LLM call succeeded — latency lands on the Slack
                // ping so a regression in tail latency surfaces in chat.
                info!(
                    target: "notify",
                    event = "embacle.call_completed",
                    model = %params.model,
                    latency_ms = latency_ms,
                    ok = true,
                    "LLM call completed"
                );
                r
            }
            Err(e) => {
                emit_call_record(CallRecordInputs {
                    recorder: params.call_recorder.as_ref(),
                    provider: params.provider.name(),
                    model: params.model,
                    usage: None,
                    cached_tokens: 0,
                    latency_ms,
                    success: false,
                    call_sequence: call_seq,
                    tools_called: Vec::new(),
                });
                // notify: LLM call failed — ok=false so the routing rule
                // can amplify failures even when sample_rate hides successes.
                info!(
                    target: "notify",
                    event = "embacle.call_completed",
                    model = %params.model,
                    latency_ms = latency_ms,
                    ok = false,
                    "LLM call failed"
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
                    guardian_denied,
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
                        guardian_denied: None,
                    });
                }

                // Guardian-denied short-circuit (enforce mode): a consequential
                // tool was blocked at the chokepoint. Exit immediately so the
                // chat pipeline renders a deterministic "blocked for safety"
                // reply instead of feeding the in-band denial back to the LLM.
                if let Some(denial) = guardian_denied {
                    return Ok(ToolLoopResult {
                        content: String::new(),
                        usage: Some(cumulative_usage),
                        finish_reason: Some("guardian_denied".to_owned()),
                        activity_list: captured_activity_list,
                        tool_calls_count,
                        tools_called,
                        pending_provider_auth_required: None,
                        guardian_denied: Some(denial),
                    });
                }

                // Add assistant's text to messages if present (strip synthetic function syntax)
                let assistant_round_text =
                    response
                        .content
                        .as_deref()
                        .map_or_else(String::new, |text| {
                            let cleaned = strip_synthetic_function_calls(text);
                            if cleaned.is_empty() {
                                String::new()
                            } else {
                                let owned = cleaned.into_owned();
                                llm_messages.push(ChatMessage::assistant(&owned));
                                owned
                            }
                        });

                // Add function responses as user messages, capturing activity list if present
                let round_responses =
                    add_function_responses_to_messages(llm_messages, &function_responses);
                if let Some(list) = round_responses.activity_list {
                    captured_activity_list = Some(list);
                }

                // Persist this round so a follow-up turn replays the same
                // grounded evidence. The recorder is None for callers that
                // don't own a conversation row (one-shot, eval, etc.).
                if let Some(recorder) = params.tool_message_recorder.as_ref() {
                    recorder.record(ToolRoundRecord {
                        assistant_text: assistant_round_text,
                        tool_result_text: round_responses.combined_text,
                    });
                }
                continue;
            }
        }

        // No structured function calls — but a native-tool model can still emit
        // the call as a text `<tool_call>` block because the shared system prompt
        // teaches that syntax (e.g. Cohere on a messaging turn). Parse and execute
        // those before treating the content as a final answer, so the turn runs
        // the tool instead of leaking the raw block to the user. run_cli_tool_loop
        // does the same for CLI providers.
        let text_tool_calls = response
            .content
            .as_deref()
            .map(parse_lenient_tool_call_blocks)
            .unwrap_or_default();
        if !text_tool_calls.is_empty() {
            info!(
                "Iteration {}: executing {} text <tool_call> block(s) from a native-tool response",
                iteration,
                text_tool_calls.len()
            );
            let ExecutedFunctionCalls {
                responses: function_responses,
                auth_required_provider,
                guardian_denied,
            } = execute_function_calls(
                &params.executor,
                &text_tool_calls,
                params.user_id,
                params.tenant_id,
            )
            .await?;
            #[allow(clippy::cast_possible_truncation)]
            {
                tool_calls_count += text_tool_calls.len() as u32;
            }
            tools_called.extend(text_tool_calls.iter().map(|c| c.name.clone()));
            if let Some(provider) = auth_required_provider {
                return Ok(ToolLoopResult {
                    content: String::new(),
                    usage: Some(cumulative_usage),
                    finish_reason: Some("provider_auth_required".to_owned()),
                    activity_list: captured_activity_list,
                    tool_calls_count,
                    tools_called,
                    pending_provider_auth_required: Some(provider),
                    guardian_denied: None,
                });
            }
            if let Some(denial) = guardian_denied {
                return Ok(ToolLoopResult {
                    content: String::new(),
                    usage: Some(cumulative_usage),
                    finish_reason: Some("guardian_denied".to_owned()),
                    activity_list: captured_activity_list,
                    tool_calls_count,
                    tools_called,
                    pending_provider_auth_required: None,
                    guardian_denied: Some(denial),
                });
            }
            let assistant_round_text =
                response
                    .content
                    .as_deref()
                    .map_or_else(String::new, |text| {
                        let cleaned = strip_synthetic_function_calls(text).into_owned();
                        if !cleaned.is_empty() {
                            llm_messages.push(ChatMessage::assistant(&cleaned));
                        }
                        cleaned
                    });
            let round_responses =
                add_function_responses_to_messages(llm_messages, &function_responses);
            if let Some(list) = round_responses.activity_list {
                captured_activity_list = Some(list);
            }
            if let Some(recorder) = params.tool_message_recorder.as_ref() {
                recorder.record(ToolRoundRecord {
                    assistant_text: assistant_round_text,
                    tool_result_text: round_responses.combined_text,
                });
            }
            continue;
        }

        // No tool calls - we have a text response (strip any synthetic function syntax)
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
            guardian_denied: None,
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
        guardian_denied: None,
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
#[tracing::instrument(
    skip_all,
    fields(
        tenant_id = %params.tenant_id,
        user_id = %params.user_id,
        provider = %params.provider.name(),
        model = %params.model,
    )
)]
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

        // notify: CLI/embacle provider call about to start. Same event as
        // the API loop so routing can hide both behind one rule.
        info!(
            target: "notify",
            event = "embacle.call_started",
            model = %params.model,
            "LLM call dispatching"
        );

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
            Ok(r) => {
                // notify: CLI call succeeded.
                info!(
                    target: "notify",
                    event = "embacle.call_completed",
                    model = %params.model,
                    latency_ms = latency_ms,
                    ok = true,
                    "LLM call completed"
                );
                r
            }
            Err(e) => {
                emit_call_record(CallRecordInputs {
                    recorder: params.call_recorder.as_ref(),
                    provider: params.provider.name(),
                    model: params.model,
                    usage: None,
                    cached_tokens: 0,
                    latency_ms,
                    success: false,
                    call_sequence: call_seq,
                    tools_called: Vec::new(),
                });
                // notify: CLI call failed.
                info!(
                    target: "notify",
                    event = "embacle.call_completed",
                    model = %params.model,
                    latency_ms = latency_ms,
                    ok = false,
                    "LLM call failed"
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
            CallRecordInputs {
                recorder: params.call_recorder.as_ref(),
                provider: params.provider.name(),
                model: params.model,
                usage: response.usage.as_ref(),
                cached_tokens,
                latency_ms,
                success: true,
                call_sequence: call_seq,
                tools_called: tools_in_response,
            },
            Some(&last_user_prompt),
            Some(&response.content),
        );

        if embacle_calls.is_empty() {
            // No tool calls — this is the final text response. Strip both the
            // model's tool calls and any echoed tool-result scaffolding (weak
            // CLI models parrot the injected `<tool_result>` turn back), so
            // neither leaks to the user.
            let content = tool_simulation::strip_simulation_artifacts(&response.content);
            return Ok(ToolLoopResult {
                content,
                usage: response.usage,
                finish_reason: response.finish_reason,
                activity_list: captured_activity_list,
                tool_calls_count,
                tools_called,
                pending_provider_auth_required: None,
                guardian_denied: None,
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
            guardian_denied,
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
                guardian_denied: None,
            });
        }

        // Guardian-denied short-circuit (mirror of the API loop): a blocked
        // consequential tool exits the turn so the chat pipeline renders the
        // deterministic "blocked for safety" reply.
        if let Some(denial) = guardian_denied {
            return Ok(ToolLoopResult {
                content: String::new(),
                usage: response.usage,
                finish_reason: Some("guardian_denied".to_owned()),
                activity_list: captured_activity_list,
                tool_calls_count,
                tools_called,
                pending_provider_auth_required: None,
                guardian_denied: Some(denial),
            });
        }

        // Add assistant message (with tool calls and any echoed tool-result
        // scaffolding stripped via embacle, so parroted output never accumulates)
        let assistant_text = tool_simulation::strip_simulation_artifacts(&response.content);
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

        // Persist this round so a follow-up turn replays the same grounded
        // evidence. Mirrors the API loop's recorder call so both paths
        // produce identical conversation history rows.
        if let Some(recorder) = params.tool_message_recorder.as_ref() {
            recorder.record(ToolRoundRecord {
                assistant_text,
                tool_result_text: tool_results_text,
            });
        }
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
        guardian_denied: None,
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
/// Shared parameters for the [`emit_call_record`] / [`emit_call_record_with_text`]
/// pair. Bundles every field a recorder needs to capture a single LLM call so
/// the call sites don't carry a nine/eleven-arg positional signature.
struct CallRecordInputs<'a> {
    /// Optional recorder; `None` short-circuits the call (no row written).
    recorder: Option<&'a Arc<dyn LlmCallRecorder>>,
    /// Provider name (e.g. `"groq"`, `"gemini"`).
    provider: &'a str,
    /// Model identifier as reported by the provider.
    model: &'a str,
    /// Token-usage payload reported by the provider; `None` when the provider
    /// emits no usage and the caller will fall back to text-based estimation.
    usage: Option<&'a TokenUsage>,
    /// Cached prompt-token count surfaced by providers that report it.
    cached_tokens: i64,
    /// End-to-end call latency in milliseconds.
    latency_ms: i64,
    /// `true` when the call completed without a provider-side error.
    success: bool,
    /// 1-based turn-local position of the call.
    call_sequence: Option<i64>,
    /// Tool function names invoked during the call.
    tools_called: Vec<String>,
}

fn emit_call_record(inputs: CallRecordInputs<'_>) {
    emit_call_record_with_text(inputs, None, None);
}

/// Variant of [`emit_call_record`] that estimates token counts from
/// character-based prompt/completion text when the provider returns
/// no usage, so CLI runners (Claude Code, Copilot, Cursor, etc.) produce
/// non-zero usage rows instead of silently dropping.
fn emit_call_record_with_text(
    inputs: CallRecordInputs<'_>,
    prompt_text: Option<&str>,
    completion_text: Option<&str>,
) {
    let CallRecordInputs {
        recorder,
        provider,
        model,
        usage,
        cached_tokens,
        latency_ms,
        success,
        call_sequence,
        tools_called,
    } = inputs;
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
pub use tool_simulation::strip_simulation_artifacts;

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

/// Parse `<tool_call>` blocks tolerantly into pierre-llm function calls.
///
/// Accepts both the canonical `{"name": X, "arguments": {...}}` shape and the
/// flat `{"name": X, ...args}` shape some native-tool models (e.g. Cohere) emit
/// when they return a call as text instead of a structured `function_calls`
/// payload. The canonical [`parse_tool_call_blocks`] drops the flat args (it
/// only reads a nested `arguments` field), which would run the tool with no
/// parameters. Used as the API-loop fallback so such a call still executes
/// correctly instead of leaking the raw block to the user.
#[must_use]
pub fn parse_lenient_tool_call_blocks(content: &str) -> Vec<FunctionCall> {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";

    let mut calls = Vec::new();
    let mut rest = content;
    while let Some(open) = rest.find(OPEN) {
        let after_open = &rest[open + OPEN.len()..];
        let Some(close) = after_open.find(CLOSE) else {
            break;
        };
        let json_str = after_open[..close].trim();
        rest = &after_open[close + CLOSE.len()..];

        if let Ok(serde_json::Value::Object(mut obj)) =
            serde_json::from_str::<serde_json::Value>(json_str)
        {
            let Some(name) = obj
                .remove("name")
                .and_then(|n| n.as_str().map(str::to_owned))
            else {
                continue;
            };
            // Canonical shape carries a nested `arguments`; the flat shape (e.g.
            // Cohere's `{"name":X,"after":..,"limit":..}`) leaves the parameters
            // as the remaining top-level keys.
            let args = obj
                .remove("arguments")
                .unwrap_or(serde_json::Value::Object(obj));
            calls.push(FunctionCall { name, args });
        }
    }
    calls
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
    let mut responses = Vec::with_capacity(function_calls.len());
    let mut auth_required_provider: Option<String> = None;
    let mut guardian_denied: Option<GuardianDenial> = None;
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

        // Capture the first Guardian denial (enforce mode) before
        // `build_function_response` reshapes the payload. Key on the
        // operator-stamped `metadata.blocked_reason == "guardian"` (set ONLY by
        // the chokepoint's `guardian_denied_response`, never emittable by a tool
        // body) AND require `!success` — NOT the tool-controllable
        // `result.error_code`, so attacker-influenced tool output carrying
        // `{"error_code":"guardian_denied"}` cannot spuriously abort the turn
        // (S12: data-plane must not drive the control-plane). First to trip wins.
        if guardian_denied.is_none() && !tool_response.success {
            let blocked_by_guardian = tool_response
                .metadata
                .as_ref()
                .and_then(|m| m.get("blocked_reason"))
                .and_then(serde_json::Value::as_str)
                == Some("guardian");
            if blocked_by_guardian {
                let reason = tool_response
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("guardian_reason"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("denied")
                    .to_owned();
                guardian_denied = Some(GuardianDenial {
                    tool_name: function_call.name.clone(),
                    reason,
                });
            }
        }

        let func_response = build_function_response(function_call, &tool_response);

        // notify: visibility on every tool dispatch. Operational tier — the
        // sink keys on the hashed tenant and drops the user dimension, so
        // emit `tenant_id` inline and omit user.
        info!(
            target: "notify",
            event = "messaging.tool_executed",
            tenant_id = %tenant_id,
            channel = "chat",
            tool_name = %function_call.name,
            success = tool_response.success,
            duration_ms = tool_duration_ms,
            "tool executed"
        );

        log_tool_response_size(&func_response);

        responses.push(func_response);

        // P4: once the Guardian denies a tool in this batch, stop — do not
        // execute the remaining siblings (they would commit side effects while
        // the user is shown a "blocked" reply). The turn short-circuits anyway.
        if guardian_denied.is_some() {
            break;
        }
    }
    Ok(ExecutedFunctionCalls {
        responses,
        auth_required_provider,
        guardian_denied,
    })
}

/// Measure a tool's serialized response size and estimated token cost, logging
/// both for token-efficiency observability.
fn log_tool_response_size(func_response: &FunctionResponse) {
    use pierre_formatters::TokenEfficiencyMetrics;

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
    /// The first tool the runtime Guardian blocked in `enforce` mode, or
    /// `None` if no call was denied. The tool loop short-circuits on `Some(_)`
    /// and the chat pipeline renders a localized "blocked for safety" reply.
    /// Travels separately from `responses` because the in-band denial would
    /// otherwise be fed back to the LLM as an ordinary failed tool result.
    pub guardian_denied: Option<GuardianDenial>,
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
    // The tenant tool-disable allowlist (formerly enforced here, chat-only) now
    // runs inside `UniversalExecutor::execute_tool` via the Guardian, so every
    // transport — not just chat — honours it. See `guardian::tenant_tool_enabled`.
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

/// Outcome of [`add_function_responses_to_messages`].
///
/// Carries the captured `get_activities` list (used to prepend a deterministic
/// activity summary to the final reply) plus the joined tool-result text
/// representing the round. The joined text is what a [`ToolMessageRecorder`]
/// persists as a single `tool_result` row so a follow-up turn can replay the
/// same evidence the model just consumed.
pub struct AddedFunctionResponses {
    /// Activity list extracted from `get_activities` when present.
    pub activity_list: Option<String>,
    /// Every formatted `[Tool Result for X]: ...` block from this round,
    /// joined by `"\n\n"`. Empty when `function_responses` is empty.
    pub combined_text: String,
}

/// Add function responses as user messages for the next LLM iteration.
///
/// Returns the captured `get_activities` list (to prepend to the final
/// response) and the joined tool-result text for persistence.
pub fn add_function_responses_to_messages(
    llm_messages: &mut Vec<ChatMessage>,
    function_responses: &[FunctionResponse],
) -> AddedFunctionResponses {
    let mut activity_list_content: Option<String> = None;
    let mut combined_blocks: Vec<String> = Vec::with_capacity(function_responses.len());

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
        combined_blocks.push(message.clone());
        llm_messages.push(ChatMessage::user(message));
    }

    AddedFunctionResponses {
        activity_list: activity_list_content,
        combined_text: combined_blocks.join("\n\n"),
    }
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

    // Plan-then-verify (Phase 3): when enabled, the LLM emits a verified up-front
    // plan instead of the interleaved ReAct loop. API/CLI providers only — the
    // headless ACP loop owns its own tool loop inside the subprocess.
    {
        use crate::guardian::PlanMode;
        let plan_mode = params.executor.resources.guardian().policy().plan_mode;
        if matches!(plan_mode, PlanMode::Enforce) && !capabilities.supports_sdk_tool_calling() {
            return run_planned_tool_loop(params, llm_messages).await;
        }
    }

    if capabilities.supports_function_calling() {
        run_api_tool_loop(params, llm_messages).await
    } else if capabilities.supports_sdk_tool_calling() {
        // The headless (Copilot ACP) loop extracts the primary CLI runner from
        // a runtime-fallback `Chain` and calls it directly, so the chain's own
        // retryable-error fallback never fires for SDK-tool-calling turns. Catch
        // a retryable primary failure (e.g. an ACP prompt timeout) here and
        // re-run the turn against the secondary, so a Copilot stall degrades to
        // the configured fallback provider instead of erroring the whole turn.
        match run_headless_tool_loop(params, llm_messages).await {
            Ok(result) => Ok(result),
            Err(err) if pierre_llm::is_retryable_for_fallback(&err) => {
                run_headless_fallback(params, llm_messages, err).await
            }
            Err(err) => Err(err),
        }
    } else {
        run_cli_tool_loop(params, llm_messages).await
    }
}

/// Build the [`ToolLoopResult`] for a Guardian-rejected plan.
///
/// Covers both the over-step-cap and the failed-static-verification cases.
/// Surfaced as a `guardian_denied` so the chat pipeline renders the localized
/// `KEY_GUARDIAN_DENIED` reply (P3) instead of the rejection flowing through
/// post-processing as if it were model output.
fn guardian_plan_denied(reason: &str) -> ToolLoopResult {
    ToolLoopResult {
        content: String::new(),
        usage: None,
        finish_reason: Some("guardian_plan_rejected".to_owned()),
        activity_list: None,
        tool_calls_count: 0,
        tools_called: Vec::new(),
        pending_provider_auth_required: None,
        guardian_denied: Some(GuardianDenial {
            tool_name: "(plan)".to_owned(),
            reason: reason.to_owned(),
        }),
    }
}

/// Append each executed plan step's result to `llm_messages` as synthesis
/// evidence.
///
/// Never re-proposed as tool calls. Extracted from [`run_planned_tool_loop`] to
/// keep that loop under the cognitive-complexity bar.
fn append_plan_results(llm_messages: &mut Vec<ChatMessage>, outputs: &[StepOutput]) {
    for output in outputs {
        let result_text = serde_json::to_string(&output.result).unwrap_or_default();
        llm_messages.push(ChatMessage::user(format!(
            "[Tool Result for {}]: {result_text}",
            output.tool_name
        )));
    }
}

/// Plan-then-verify loop (Phase 3).
///
/// The LLM emits the entire tool plan up front, a static verifier rejects any
/// tainted-source→sink flow *before* execution, and only a verified, frozen
/// plan runs. Because the plan is fixed before any tool result is seen, a
/// malicious result cannot inject a new tool call.
///
/// Falls back to [`run_api_tool_loop`] when the model does not emit a parseable
/// plan (graceful degrade for weaker models).
///
/// # Errors
/// Returns error if the LLM plan/synthesis call fails or a verified plan hits an
/// unresolvable reference (a binding bug).
pub async fn run_planned_tool_loop(
    params: &ToolLoopParams<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> Result<ToolLoopResult, AppError> {
    use crate::guardian::{
        planner_system_prompt, verify, PlanParseError, VerifyOutcome, Workflow, WorkflowExecutor,
    };

    // 1. Plan call — ask for the whole plan up front (no tool-calling needed).
    let mut plan_messages = llm_messages.clone();
    plan_messages.insert(0, ChatMessage::system(planner_system_prompt()));
    let plan_request = ChatRequest::new(plan_messages).with_model(params.model);
    let plan_response = params.provider.complete(&plan_request).await?;
    let plan_json = plan_response.content;

    // 2. Parse. An unparseable plan is a weak-model artifact → degrade to ReAct.
    //    An over-cap plan is the S15 cost-amplification vector → FAIL CLOSED
    //    (reject the turn); it must NOT drop to the unbudgeted ReAct loop, which
    //    would reopen exactly what the step cap closes.
    let workflow = match Workflow::from_json(&plan_json) {
        Ok(workflow) => workflow,
        Err(PlanParseError::Unparseable(error)) => {
            warn!(%error, "guardian plan: unparseable workflow JSON; falling back to ReAct loop");
            return run_api_tool_loop(params, llm_messages).await;
        }
        Err(PlanParseError::TooManySteps { steps, max }) => {
            warn!(
                steps,
                max,
                "guardian plan: over the step cap (S15); rejecting the turn (no ReAct fallback)"
            );
            return Ok(guardian_plan_denied("plan_too_large"));
        }
    };

    // 3. Verify the frozen plan before anything executes.
    let resources = &params.executor.resources;
    let outcome = verify(
        &workflow,
        resources.tool_registry().as_ref(),
        resources.guardian().policy(),
        Some(params.tenant_id.as_uuid()),
    );
    if let VerifyOutcome::Reject(reason) = outcome {
        warn!(reason = ?reason, "guardian plan: REJECTED before execution");
        return Ok(guardian_plan_denied("plan_rejected"));
    }

    // 4. Execute the verified plan (each call still passes the runtime Guardian).
    let workflow_executor =
        WorkflowExecutor::new(params.executor.as_ref(), params.user_id, params.tenant_id);
    let (outputs, plan_denial) = workflow_executor.run(&workflow).await?;
    let tools_called: Vec<String> = outputs.iter().map(|o| o.tool_name.clone()).collect();
    let tool_calls_count = u32::try_from(outputs.len()).unwrap_or(u32::MAX);

    // S9: a runtime Guardian denial mid-plan short-circuits with a deterministic
    // block reply — the denied step's JSON must NOT be fed to the synthesis LLM
    // to paraphrase (the exact softened-refusal the guard exists to prevent).
    if let Some(denial) = plan_denial {
        return Ok(ToolLoopResult {
            content: String::new(),
            usage: None,
            finish_reason: Some("guardian_denied".to_owned()),
            activity_list: None,
            tool_calls_count,
            tools_called,
            pending_provider_auth_required: None,
            guardian_denied: Some(GuardianDenial {
                tool_name: denial.tool_name,
                reason: denial.reason,
            }),
        });
    }

    // 5. Synthesis — feed the bound results back as evidence (never as new tool
    //    proposals) and produce the user-facing answer.
    append_plan_results(llm_messages, &outputs);
    let synth_request = ChatRequest::new(llm_messages.clone()).with_model(params.model);
    let synth_response = params.provider.complete(&synth_request).await?;

    Ok(ToolLoopResult {
        content: synth_response.content,
        usage: synth_response.usage,
        finish_reason: synth_response.finish_reason,
        activity_list: None,
        tool_calls_count,
        tools_called,
        pending_provider_auth_required: None,
        guardian_denied: None,
    })
}

/// Re-run a failed headless (Copilot ACP) turn against the runtime-fallback
/// secondary provider.
///
/// [`run_headless_tool_loop`] reaches past a fallback `Chain` to the primary
/// runner, bypassing the chain's retryable-error fallback. When that primary
/// call fails with a retryable error this re-runs the whole tool loop against
/// the chain's secondary — routed by the secondary's own capabilities (native
/// function calling for Cohere/Gemini), so the turn still produces a grounded
/// answer. With no secondary configured the original error is returned
/// unchanged, preserving behavior when runtime fallback is disabled.
async fn run_headless_fallback(
    params: &ToolLoopParams<'_>,
    llm_messages: &mut Vec<ChatMessage>,
    primary_err: AppError,
) -> Result<ToolLoopResult, AppError> {
    let Some(secondary) = params.provider.fallback_secondary() else {
        return Err(primary_err);
    };
    warn!(
        primary = params.provider.name(),
        secondary = secondary.name(),
        error = %primary_err,
        "Headless tool loop failed with retryable error; falling back to secondary provider"
    );
    let fallback_params = ToolLoopParams {
        provider: secondary,
        executor: Arc::clone(&params.executor),
        tools: params.tools,
        // The secondary picks its own model: the primary's model (e.g.
        // Copilot's `claude-opus-4.8`) is meaningless to Cohere/Gemini.
        // Mirrors `request_for_secondary` nulling the model in the
        // ChatProvider chain fallback.
        model: secondary.default_model(),
        user_id: params.user_id,
        tenant_id: params.tenant_id,
        max_iterations: params.max_iterations,
        call_recorder: params.call_recorder.clone(),
        tool_message_recorder: params.tool_message_recorder.clone(),
        temperature: params.temperature,
        stream_sink: params.stream_sink.clone(),
        mcp_servers: params.mcp_servers.clone(),
    };
    Box::pin(run_tool_loop(&fallback_params, llm_messages)).await
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

    // Build the ChatRequest from the accumulated messages. Borrowed (not
    // consumed) by the converse/stream call, so the degenerate-turn retry
    // below reuses the same request.
    let request = {
        let req = ChatRequest::new(llm_messages.to_vec())
            .with_model(params.model)
            .with_mcp_servers(params.mcp_servers.clone());
        match params.temperature {
            Some(t) => req.with_temperature(t),
            None => req,
        }
    };

    // Copilot Headless handles tool execution internally via ACP — and, when
    // mcp_servers are present, calls Dravr tools natively over those servers.
    // #10: clear any stale denial for this (tenant, user) so only a block raised
    // during THIS turn's ACP subprocess is surfaced by finalize_headless_turn.
    params
        .executor
        .resources
        .guardian_turns()
        .clear_denial(&headless_denial_key(params));

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
        .find(|m| matches!(m.role, pierre_llm::MessageRole::User))
        .map(|m| m.content.as_str())
        .unwrap_or_default()
        .to_owned();
    let headless_response = match converse_result {
        Ok(r) => {
            let tools_in_response: Vec<String> =
                r.tool_calls.iter().map(|tc| tc.title.clone()).collect();
            emit_call_record_with_text(
                CallRecordInputs {
                    recorder: params.call_recorder.as_ref(),
                    provider: params.provider.name(),
                    model: params.model,
                    usage: r.usage.as_ref(),
                    cached_tokens: 0,
                    latency_ms,
                    success: true,
                    call_sequence: Some(1),
                    tools_called: tools_in_response,
                },
                Some(&last_user_prompt),
                Some(&r.content),
            );
            r
        }
        Err(e) => {
            emit_call_record(CallRecordInputs {
                recorder: params.call_recorder.as_ref(),
                provider: params.provider.name(),
                model: params.model,
                usage: None,
                cached_tokens: 0,
                latency_ms,
                success: false,
                call_sequence: Some(1),
                tools_called: Vec::new(),
            });
            return Err(e);
        }
    };

    info!(
        content_len = headless_response.content.len(),
        model = %headless_response.model,
        tool_calls = headless_response.tool_calls.len(),
        "Headless tool loop completed"
    );

    finalize_headless_turn(
        headless_response,
        headless_runner,
        &request,
        params,
        &last_user_prompt,
    )
    .await
}

/// The Guardian denial key for a headless turn — `(tenant, user)`. A block that
/// fires at the chokepoint during the ACP subprocess's loopback calls is recorded
/// and later consumed under this key, so the headless loop can surface it (#10).
/// One user runs one headless turn at a time, so `(tenant, user)` identifies it;
/// [`run_headless_tool_loop`] clears the key before the subprocess and
/// [`finalize_headless_turn`] takes it after, bounding it to this turn.
fn headless_denial_key(params: &ToolLoopParams<'_>) -> TurnKey {
    TurnKey::new(Some(params.tenant_id.as_uuid()), params.user_id)
}

/// Strip the headless reply, retry once if the turn was degenerate, and
/// assemble the [`ToolLoopResult`].
///
/// Copilot ACP intermittently ends a turn without synthesizing an answer:
/// it returns empty content, or parrots the injected tool-result turn
/// verbatim (the `format_tool_results_as_text` preamble + `<tool_result>`
/// blocks) instead of reasoning over it. The scaffolding is stripped
/// unconditionally so a parroted echo never leaks to the user; when
/// stripping leaves nothing the turn is degenerate, so [`retry_headless_turn`]
/// runs once — the failure is intermittent (~5% per call) and the next turn
/// recovers in practice.
async fn finalize_headless_turn(
    headless_response: pierre_llm::HeadlessToolResponse,
    headless_runner: &pierre_llm::CopilotHeadlessRunner,
    request: &ChatRequest,
    params: &ToolLoopParams<'_>,
    last_user_prompt: &str,
) -> Result<ToolLoopResult, AppError> {
    let mut tool_calls_count =
        u32::try_from(headless_response.tool_calls.len()).unwrap_or(u32::MAX);
    // `ObservedToolCall` only exposes `title` on the ACP wire; there's no
    // distinct `name` field, so the title is the best available identifier.
    let mut tools_called: Vec<String> = headless_response
        .tool_calls
        .iter()
        .map(|call| call.title.clone())
        .collect();
    let mut content = tool_simulation::strip_simulation_artifacts(&headless_response.content);
    let mut usage = headless_response.usage;
    let mut finish_reason = headless_response.finish_reason;

    if content.is_empty() {
        warn!(
            provider = %params.provider.name(),
            model = %params.model,
            "Headless turn produced no synthesized answer (empty or parroted tool-result echo); retrying converse once"
        );
        let retry = retry_headless_turn(headless_runner, request, params, last_user_prompt).await?;
        content = retry.content;
        usage = retry.usage;
        finish_reason = retry.finish_reason;
        tool_calls_count = tool_calls_count
            .saturating_add(u32::try_from(retry.tool_calls.len()).unwrap_or(u32::MAX));
        tools_called.extend(retry.tool_calls);
    }

    // #10: surface a Guardian block that fired inside the ACP subprocess's `/mcp`
    // loopback as a deterministic denial (the chat pipeline renders it as the
    // localized KEY_GUARDIAN_DENIED) instead of the model's paraphrase. Consumed
    // once at the end of the turn — after any degenerate-turn retry.
    let guardian_denied = params
        .executor
        .resources
        .guardian_turns()
        .take_denial(&headless_denial_key(params))
        .map(|reason| GuardianDenial {
            tool_name: "(headless)".to_owned(),
            reason: reason.as_str().to_owned(),
        });

    Ok(ToolLoopResult {
        content,
        usage,
        finish_reason,
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
        // #10: a block that fired at the platform chokepoint during the ACP
        // subprocess's loopback calls is recovered from the shared Guardian store
        // above (keyed by this turn's (tenant, user)), so the headless path now
        // surfaces the same deterministic refusal every other transport does.
        guardian_denied,
    })
}

/// Outcome of a single degenerate-turn retry: the stripped reply plus the
/// accounting [`run_headless_tool_loop`] folds into its `ToolLoopResult`.
struct HeadlessRetry {
    content: String,
    usage: Option<TokenUsage>,
    finish_reason: Option<String>,
    tool_calls: Vec<String>,
}

/// Retry a degenerate headless turn once via non-streaming `converse()`.
///
/// Called only when the first turn produced no synthesized answer (empty or
/// a parroted tool-result echo). Records the retry as its own per-call usage
/// row and returns the stripped reply so the caller can decide whether the
/// model recovered. Non-streaming on purpose: a re-parroted echo must never
/// reach the user's stream a second time.
async fn retry_headless_turn(
    headless_runner: &pierre_llm::CopilotHeadlessRunner,
    request: &ChatRequest,
    params: &ToolLoopParams<'_>,
    last_user_prompt: &str,
) -> Result<HeadlessRetry, AppError> {
    let retry_start = Instant::now();
    let retry = headless_runner
        .converse(request)
        .await
        .map_err(AppError::from)?;
    let retry_latency_ms = millis_elapsed(retry_start);
    let tool_calls: Vec<String> = retry.tool_calls.iter().map(|tc| tc.title.clone()).collect();
    emit_call_record_with_text(
        CallRecordInputs {
            recorder: params.call_recorder.as_ref(),
            provider: params.provider.name(),
            model: params.model,
            usage: retry.usage.as_ref(),
            cached_tokens: 0,
            latency_ms: retry_latency_ms,
            success: true,
            call_sequence: Some(2),
            tools_called: tool_calls.clone(),
        },
        Some(last_user_prompt),
        Some(&retry.content),
    );
    Ok(HeadlessRetry {
        content: tool_simulation::strip_simulation_artifacts(&retry.content),
        usage: retry.usage,
        finish_reason: retry.finish_reason,
        tool_calls,
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
/// Derives function declarations from the registry's chat-callable schemas
/// so the surface stays in lockstep with what the prose "Available Tools"
/// section of the system prompt advertises. The previous hand-curated 15-tool
/// list drifted from the registry — newer tools (endurance dossier/history,
/// nutrition, mobility) ended up advertised in prose but missing from the
/// function-calling surface, producing "no callable tool" refusals when
/// coach prompts referenced them.
///
/// Tool descriptions and parameter schemas come from each `McpTool`
/// implementation's `description()` / `input_schema()` methods, with any
/// contremaitre overlay already applied by `ToolRegistry::build_schema`.
#[must_use]
pub fn build_mcp_tools(tool_registry: &ToolRegistry) -> Tool {
    let schemas = tool_registry.chat_callable_schemas();
    let function_declarations = schemas
        .into_iter()
        .map(|schema| FunctionDeclaration {
            name: schema.name,
            description: schema.description,
            parameters: serde_json::to_value(&schema.input_schema).ok(),
        })
        .collect();
    Tool {
        function_declarations,
    }
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
