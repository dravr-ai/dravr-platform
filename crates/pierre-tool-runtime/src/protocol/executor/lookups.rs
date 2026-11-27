// ABOUTME: What an executor answers about the tools it holds and the turn it is bound to, without dispatching
// ABOUTME: A tool's presence, whether its schema declares an argument, and the turn token bound to it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::UniversalExecutor;

impl UniversalExecutor {
    /// Check if executor has a specific tool
    #[must_use]
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.resources.tool_registry().get(tool_name).is_some()
    }

    /// Whether `tool_name`'s registered input schema declares `property` as a
    /// top-level argument.
    ///
    /// Shape-tolerant argument parsing uses this to distinguish a
    /// provider-emitted `{"parameters": {...}}` envelope from a tool that
    /// really takes a `parameters` argument (e.g. `validate_configuration`).
    /// An unknown tool reports `false`: dispatch fails on the name lookup
    /// anyway, so the answer never reaches a tool body.
    #[must_use]
    pub fn tool_declares_property(&self, tool_name: &str, property: &str) -> bool {
        self.resources
            .tool_registry()
            .get(tool_name)
            .is_some_and(|tool| {
                tool.definition()
                    .input_schema
                    .get("properties")
                    .and_then(|props| props.get(property))
                    .is_some()
            })
    }

    /// The turn token bound to this executor, `None` for a one-shot MCP or A2A
    /// call: such a call mints a nonce that no later reader could ever name.
    #[must_use]
    pub fn turn_token(&self) -> Option<&str> {
        self.turn_token.as_deref()
    }
}
