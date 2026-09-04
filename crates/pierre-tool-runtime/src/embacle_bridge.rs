// ABOUTME: Translation between pierre-llm tool shapes and embacle tool_simulation
// ABOUTME: The text-simulation loop's only conversion surface, kept apart from the loop
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Field-for-field conversion between the platform's function shapes and the
//! ones `embacle::tool_simulation` takes.
//!
//! It mirrors `pierre_llm::tool_bridge`, which does the same job for the native
//! function-calling path: deciding what to send and translating it are separate
//! jobs, and a module that owns both ends up deciding what providers can do.

use pierre_core::llm::tool_simulation;
use pierre_llm::{FunctionCall, FunctionDeclaration};

/// Convert pierre-llm function declarations to embacle `tool_simulation` declarations.
pub fn to_embacle_declarations(
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
pub fn from_embacle_calls(calls: Vec<tool_simulation::FunctionCall>) -> Vec<FunctionCall> {
    calls
        .into_iter()
        .map(|c| FunctionCall {
            name: c.name,
            args: c.args,
        })
        .collect()
}
