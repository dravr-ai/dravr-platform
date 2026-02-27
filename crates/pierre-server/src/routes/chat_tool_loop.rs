// ABOUTME: Strategy pattern for LLM tool execution loops in chat conversations
// ABOUTME: Supports API-based, SDK-based (Copilot SDK), and CLI-based tool calling modes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tool loop strategies for chat conversations
//!
//! Three strategies exist for executing tool calls during LLM conversations:
//! - [`run_api_tool_loop`]: Native function calling via `complete_with_tools()` (Gemini, Groq)
//! - [`run_sdk_tool_loop`]: SDK-managed tool calling via `ToolHandler` callback (Copilot SDK)
//! - [`run_cli_tool_loop`]: Text-based tool calling via `<tool_call>` blocks (CLI providers)
//!
//! All strategies share the same MCP executor infrastructure and produce
//! identical [`ToolLoopResult`] output.

use crate::models::TenantId;
use crate::{
    errors::AppError,
    llm::{
        convert_function_declarations, extract_declarations_from_tool_value, ChatMessage,
        ChatProvider, ChatRequest, FunctionCall, FunctionDeclaration, FunctionResponse, TokenUsage,
        Tool, ToolHandler, ToolResultObject,
    },
    protocols::universal::{UniversalExecutor, UniversalRequest, UniversalResponse},
};
use pierre_core::llm::tool_simulation;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use tokio::task::block_in_place;
use tracing::{info, warn};

// ============================================================================
// Shared Types
// ============================================================================

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
pub async fn run_api_tool_loop(
    params: &ToolLoopParams<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> Result<ToolLoopResult, AppError> {
    let mut captured_activity_list: Option<String> = None;
    let mut tool_calls_count: u32 = 0;
    let mut cumulative_usage = TokenUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
    };

    for iteration in 0..params.max_iterations {
        let llm_request = ChatRequest::new(llm_messages.clone()).with_model(params.model);
        let response = params
            .provider
            .complete_with_tools(&llm_request, Some(vec![params.tools.clone()]))
            .await?;

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

                let function_responses = execute_function_calls(
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

                // Add assistant's text to messages if present (strip synthetic function syntax)
                if let Some(ref text) = response.content {
                    let cleaned = super::chat::strip_synthetic_function_calls(text);
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
            .map(|c| super::chat::strip_synthetic_function_calls(&c).into_owned())
            .unwrap_or_default();
        return Ok(ToolLoopResult {
            content,
            usage: Some(cumulative_usage),
            finish_reason: response.finish_reason,
            activity_list: captured_activity_list,
            tool_calls_count,
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
    let max_iterations = params.max_iterations.min(CLI_MAX_TOOL_ITERATIONS);

    for iteration in 0..max_iterations {
        let llm_request = ChatRequest::new(llm_messages.clone()).with_model(params.model);
        let response = params.provider.complete(&llm_request).await?;

        // Parse <tool_call> blocks from the response text (via embacle)
        let embacle_calls = tool_simulation::parse_tool_call_blocks(&response.content);

        if embacle_calls.is_empty() {
            // No tool calls — this is the final text response
            let content = tool_simulation::strip_tool_call_blocks(&response.content);
            return Ok(ToolLoopResult {
                content,
                usage: response.usage,
                finish_reason: response.finish_reason,
                activity_list: captured_activity_list,
                tool_calls_count,
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
        let function_responses = execute_function_calls(
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
) -> Result<Vec<FunctionResponse>, AppError> {
    use crate::formatters::TokenEfficiencyMetrics;

    let mut responses = Vec::with_capacity(function_calls.len());
    for function_call in function_calls {
        info!(
            tool_name = %function_call.name,
            args = %function_call.args,
            "Executing tool"
        );
        let tool_response = execute_mcp_tool(executor, function_call, user_id, tenant_id).await;
        let func_response = build_function_response(function_call, &tool_response);

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
    Ok(responses)
}

/// Execute a single MCP tool call and return the response.
///
/// Tool execution errors are converted to failed responses so the LLM
/// can handle them gracefully.
async fn execute_mcp_tool(
    executor: &UniversalExecutor,
    function_call: &FunctionCall,
    user_id: &str,
    tenant_id: TenantId,
) -> UniversalResponse {
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
        Err(e) => UniversalResponse {
            success: false,
            result: None,
            error: Some(format!("Tool execution failed: {e}")),
            metadata: None,
        },
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
/// - Providers with `SDK_TOOL_CALLING` capability use [`run_sdk_tool_loop`]
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
        run_sdk_tool_loop(params, llm_messages).await
    } else {
        run_cli_tool_loop(params, llm_messages).await
    }
}

// ============================================================================
// SDK Tool Loop (Copilot SDK native tool calling via ToolHandler)
// ============================================================================

/// Executes the tool loop using Copilot SDK native tool calling.
///
/// The SDK manages the tool call → execute → response cycle internally via the
/// `ToolHandler` callback. The caller provides a bridge function that routes
/// tool calls through the MCP executor.
///
/// # Errors
///
/// Returns error if the SDK runner cannot be extracted, or the SDK call fails.
async fn run_sdk_tool_loop(
    params: &ToolLoopParams<'_>,
    llm_messages: &[ChatMessage],
) -> Result<ToolLoopResult, AppError> {
    // Extract the CopilotSdkRunner from the ChatProvider
    let sdk_runner = params
        .provider
        .as_cli_provider()
        .and_then(|cli| cli.as_copilot_sdk_runner())
        .ok_or_else(|| {
            AppError::internal(
                "SDK tool loop requires CopilotSdkRunner but provider is not Copilot SDK",
            )
        })?;

    // Convert Pierre tool declarations to copilot-sdk Tool definitions
    let tool_value = serde_json::to_value(params.tools).unwrap_or_default();
    let declarations = extract_declarations_from_tool_value(&tool_value);
    let sdk_tools = convert_function_declarations(&declarations);

    info!(
        tool_count = sdk_tools.len(),
        "SDK tool loop: registering tools with Copilot SDK"
    );

    // Build the ChatRequest from the accumulated messages
    let request = ChatRequest::new(llm_messages.to_vec()).with_model(params.model);

    // Shared state for capturing activity list from get_activities tool calls
    let captured_activity_list: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    // Build the ToolHandler closure that bridges sync SDK callbacks to async MCP execution
    let executor = Arc::clone(&params.executor);
    let user_id = params.user_id.to_owned();
    let tenant_id = params.tenant_id;
    let activity_list_capture = Arc::clone(&captured_activity_list);

    let handler: ToolHandler = Arc::new(move |name: &str, args: &Value| {
        let executor = Arc::clone(&executor);
        let name = name.to_owned();
        let args = args.clone();
        let user_id = user_id.clone();
        let activity_list_capture = Arc::clone(&activity_list_capture);

        // Bridge sync ToolHandler to async MCP executor using block_in_place
        block_in_place(|| {
            Handle::current().block_on(async {
                info!(tool_name = %name, "SDK tool handler: executing MCP tool");

                let mcp_request = UniversalRequest {
                    tool_name: name.clone(),
                    parameters: args,
                    user_id,
                    protocol: "chat".to_owned(),
                    tenant_id: Some(tenant_id.to_string()),
                    progress_token: None,
                    cancellation_token: None,
                    progress_reporter: None,
                };

                match executor.execute_tool(mcp_request).await {
                    Ok(response) => {
                        let result_value = response.result.unwrap_or_else(|| {
                            serde_json::json!({"status": "success"})
                        });

                        // Capture activity list from get_activities responses
                        if name == "get_activities" {
                            if let Some(list) = result_value
                                .get("activity_list")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                            {
                                info!(
                                    list_len = list.len(),
                                    "SDK tool handler: captured activity list"
                                );
                                if let Ok(mut guard) = activity_list_capture.lock() {
                                    *guard = Some(list.to_owned());
                                }
                            }
                        }

                        let result_json = serde_json::to_string(&result_value)
                            .unwrap_or_else(|_| "{}".to_owned());
                        info!(
                            tool_name = %name,
                            result_len = result_json.len(),
                            "SDK tool handler: tool executed successfully"
                        );
                        ToolResultObject::text(result_json)
                    }
                    Err(e) => {
                        warn!(tool_name = %name, error = %e, "SDK tool handler: tool execution failed");
                        ToolResultObject::error(e.to_string())
                    }
                }
            })
        })
    });

    // Execute the full conversation turn with tool calling
    let sdk_response = sdk_runner
        .execute_with_tools(&request, sdk_tools, handler)
        .await
        .map_err(AppError::from)?;

    // Extract captured activity list
    let activity_list = captured_activity_list
        .lock()
        .ok()
        .and_then(|guard| guard.clone());

    info!(
        content_len = sdk_response.content.len(),
        model = %sdk_response.model,
        tool_calls = sdk_response.tool_calls_count,
        has_activity_list = activity_list.is_some(),
        "SDK tool loop completed"
    );

    Ok(ToolLoopResult {
        content: sdk_response.content,
        usage: sdk_response.usage,
        finish_reason: sdk_response.finish_reason,
        activity_list,
        tool_calls_count: sdk_response.tool_calls_count,
    })
}
