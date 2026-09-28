// ABOUTME: The platform's tool surface and tool-carrying response, built on embacle's function types
// ABOUTME: Groups embacle FunctionDeclarations into a Tool and carries embacle FunctionCalls back
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tool-calling types shared by every provider.
//!
//! The function shapes themselves are embacle's — [`FunctionDeclaration`] (its
//! `ToolDefinition`), [`FunctionCall`] and `FunctionResponse` — so a declaration
//! travels to a provider and a call travels back without a conversion. What
//! the platform adds is the grouping the tool loop hands around: a [`Tool`] of
//! declarations out, a [`ChatResponseWithTools`] carrying the calls back.

use embacle::{FunctionCall, FunctionDeclaration};
use serde::{Deserialize, Serialize};

use super::TokenUsage;

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
    /// Request parameters the serving provider ignored (see
    /// `ChatResponse::warnings`); `None` when it honored all of them.
    pub warnings: Option<Vec<String>>,
}

impl ChatResponseWithTools {
    /// Get the text content if present
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.content.as_deref()
    }
}
