// ABOUTME: Translates tool declarations and tool calls between the platform's shapes and embacle's
// ABOUTME: Lives apart from the dispatch enum so deciding and translating stay separate jobs

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
//! Both directions are field-for-field renames between equivalent types. They
//! live here rather than in `provider.rs` because that file's job is choosing
//! which provider to ask, and this file's job is speaking to whichever one was
//! chosen — a dispatch enum that also owns conversions ends up deciding what
//! providers are capable of, which is the defect this pair exists to undo.

use embacle::types::{ToolCallRequest, ToolDefinition};

use super::{ChatRequest, ChatResponse, ChatResponseWithTools, FunctionCall, Tool};

/// Translate the platform's tool declarations into embacle's request shape.
///
/// `Tool` groups many `FunctionDeclaration`s; embacle takes a flat
/// `Vec<ToolDefinition>`. The two carry the same three fields, so this is a
/// rename, not a lossy projection.
fn tool_defs_from(tools: Option<Vec<Tool>>) -> Option<Vec<ToolDefinition>> {
    let defs: Vec<ToolDefinition> = tools?
        .into_iter()
        .flat_map(|t| t.function_declarations)
        .map(|f| ToolDefinition {
            name: f.name,
            description: f.description,
            parameters: f.parameters,
        })
        .collect();
    (!defs.is_empty()).then_some(defs)
}

/// Translate embacle's tool calls back into the platform's shape.
///
/// `ToolCallRequest.id` is dropped: the tool loop correlates by position and
/// name, and carrying an id nothing reads would be a field to keep in sync for
/// no consumer.
fn function_calls_from(calls: Option<Vec<ToolCallRequest>>) -> Option<Vec<FunctionCall>> {
    let mapped: Vec<FunctionCall> = calls?
        .into_iter()
        .map(|c| FunctionCall {
            name: c.function_name,
            args: c.arguments,
        })
        .collect();
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
    }
}
