// ABOUTME: The platform's tool-calling shapes: declarations sent to a provider and the calls it returns
// ABOUTME: Consumed by pierre-tool-runtime and translated to embacle's shapes by tool_bridge
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tool-calling types shared by every provider.
//!
//! These were born with the Gemini provider and kept its vocabulary
//! (`function_declarations`, `functionCall`), but the tool loop in
//! `pierre-tool-runtime` speaks this shape to every provider alike;
//! [`crate::tool_bridge`] renames the fields to embacle's `ToolDefinition` /
//! `ToolCallRequest` on the way out and back.

use serde::{Deserialize, Serialize};

use super::TokenUsage;

/// Function call made by the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    /// Name of the function to call
    pub name: String,
    /// Arguments for the function as JSON object
    pub args: serde_json::Value,
}

/// Response to a function call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResponse {
    /// Name of the function that was called
    pub name: String,
    /// Response content from the function
    pub response: serde_json::Value,
}

/// Function declaration for tool definitions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDeclaration {
    /// Name of the function
    pub name: String,
    /// Description of what the function does
    pub description: String,
    /// Parameters schema (JSON Schema format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

/// A group of function declarations offered to the model as one tool surface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Function declarations for this tool
    pub function_declarations: Vec<FunctionDeclaration>,
}

/// Response from a chat completion that may contain function calls
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponseWithTools {
    /// Generated message content (None if function calls present)
    pub content: Option<String>,
    /// Function calls requested by the model
    pub function_calls: Option<Vec<FunctionCall>>,
    /// Model used for generation
    pub model: String,
    /// Token usage statistics
    pub usage: Option<TokenUsage>,
    /// Finish reason (stop, length, etc.)
    pub finish_reason: Option<String>,
}

impl ChatResponseWithTools {
    /// Check if this response contains function calls
    #[must_use]
    pub fn has_function_calls(&self) -> bool {
        self.function_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
    }

    /// Get the text content if present
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.content.as_deref()
    }
}
