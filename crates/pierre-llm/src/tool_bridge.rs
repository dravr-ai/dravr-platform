// ABOUTME: Attaches tool declarations to a request and lifts a response's tool calls into ChatResponseWithTools
// ABOUTME: Lives apart from the dispatch enum so deciding and speaking to a provider stay separate jobs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The tool half of the provider boundary.
//!
//! `LlmProvider` has exactly one completion method — `complete()` — and it
//! returns embacle's `ChatResponse`, which already carries `tool_calls`. So a
//! provider reached through the trait can participate in tool calling without
//! any new trait method; what it needs is for the declarations to travel out
//! and the calls to travel back.
//!
//! The declarations are embacle's own `ToolDefinition`s and the calls are
//! embacle's `FunctionCall`s, so nothing is renamed on the way out or back.
//! These live here rather than in `provider.rs` because that file's job is
//! choosing which provider to ask, and this file's job is speaking to
//! whichever one was chosen — a dispatch enum that also owns the request and
//! response shapes ends up deciding what providers are capable of.

use embacle::types::{ToolCallRequest, ToolDefinition};

use super::{ChatRequest, ChatResponse, ChatResponseWithTools, FunctionCall, Tool};

/// Flatten the tool surface into the declarations a request carries.
///
/// `Tool` groups many declarations; embacle's request takes one flat list.
fn tool_defs_from(tools: Option<Vec<Tool>>) -> Option<Vec<ToolDefinition>> {
    let defs: Vec<ToolDefinition> = tools?
        .into_iter()
        .flat_map(|t| t.function_declarations)
        .collect();
    (!defs.is_empty()).then_some(defs)
}

/// The calls a provider reported, as the tool loop dispatches them.
///
/// `ToolCallRequest.id` is dropped by embacle's own conversion: the tool loop
/// correlates by position and name.
fn function_calls_from(calls: Option<Vec<ToolCallRequest>>) -> Option<Vec<FunctionCall>> {
    let mapped: Vec<FunctionCall> = calls?.into_iter().map(FunctionCall::from).collect();
    (!mapped.is_empty()).then_some(mapped)
}

/// Clone `request` with the tool declarations attached, so a provider reached
/// through the trait's plain `complete()` still sees what it may call.
pub fn with_tool_defs(request: &ChatRequest, tools: Option<Vec<Tool>>) -> ChatRequest {
    tool_defs_from(tools).map_or_else(|| request.clone(), |defs| request.clone().with_tools(defs))
}

/// Lift a plain `ChatResponse` into the tool-carrying shape, preserving any
/// tool calls the provider reported.
pub fn with_tools_response(response: ChatResponse) -> ChatResponseWithTools {
    ChatResponseWithTools {
        content: Some(response.content),
        model: response.model,
        usage: response.usage,
        function_calls: function_calls_from(response.tool_calls),
        finish_reason: response.finish_reason,
        warnings: response.warnings,
    }
}
