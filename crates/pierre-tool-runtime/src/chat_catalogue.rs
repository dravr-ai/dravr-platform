// ABOUTME: The chat function-calling catalogue: the tool declarations a turn sends, and their wire size
// ABOUTME: One measurement for the boot log, the per-turn dispatch log, the admin route and the CI budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Chat tool catalogue
//!
//! [`build_mcp_tools`] turns the registry's chat-callable schemas into the
//! function declarations a turn offers the model; [`measure_catalogue`] sizes
//! a set of them as the provider receives it.
//!
//! The catalogue is most of a cold prompt. A coach-seat turn on 2026-10-26
//! sent 62,199 prompt tokens before the athlete had typed more than a line,
//! an estimated two thirds of it tool definitions, and every cold turn writes
//! that prefix to cache once per model (carnet#1196). It is also the first
//! thing a provider caches: change one description or one key's position and
//! the next call on every model writes the whole prefix again. That is why the
//! measurement carries a digest — two turns with different digests cannot
//! share a cache entry, whatever else they have in common.
//!
//! Sizes are taken in the Anthropic Messages wire shape — `name`,
//! `description`, `input_schema`, compact JSON, in the order sent — the tier
//! the cost was measured on. They leave out what the provider adds: its
//! tool-use preamble (286 tokens on the 5.5 models) and the cache marker on
//! the last tool.

use std::cmp::Reverse;

use pierre_core::tokens::estimate_context_tokens;
use pierre_llm::{FunctionDeclaration, Tool};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::Sha256;

use crate::{registry::ToolRegistry, schema_canonical::to_canonical_value};

/// Hex characters of the SHA-256 kept as the catalogue digest
const DIGEST_HEX_LEN: usize = 16;

/// Build the LLM tool definition set for chat-mode function calling.
///
/// Derives function declarations from the registry's chat-callable schemas
/// so the surface stays in lockstep with what the prose "Available Tools"
/// section of the system prompt advertises. The previous hand-curated 15-tool
/// list drifted from the registry — newer tools (endurance dossier/history,
/// nutrition, mobility) ended up advertised in prose but missing from the
/// function-calling surface, producing "no callable tool" refusals when
/// agent prompts referenced them.
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
            parameters: Some(to_canonical_value(&schema.input_schema)),
        })
        .collect();
    Tool {
        function_declarations,
    }
}

/// One tool's share of a catalogue
#[derive(Debug, Clone, Serialize)]
pub struct ToolSize {
    /// Tool name
    pub name: String,
    /// Bytes of the tool's serialized wire definition
    pub bytes: usize,
    /// Structure-aware token estimate of those bytes
    pub estimated_tokens: u32,
}

/// The wire size of a tool catalogue
#[derive(Debug, Clone, Serialize)]
pub struct CatalogueSize {
    /// Number of tool declarations
    pub tool_count: usize,
    /// Bytes of the serialized `tools` array
    pub bytes: usize,
    /// Structure-aware estimate from [`estimate_context_tokens`]. A provider's
    /// own count differs by tokenizer, and by model within one provider.
    pub estimated_tokens: u32,
    /// Leading hex of the SHA-256 of those bytes: two catalogues with the same
    /// digest open with the same cache prefix
    pub digest: String,
    /// Every tool, largest first
    pub per_tool: Vec<ToolSize>,
}

impl CatalogueSize {
    /// The `count` largest tools as `name=bytes`, comma-separated, for one log field
    #[must_use]
    pub fn largest(&self, count: usize) -> String {
        self.per_tool
            .iter()
            .take(count)
            .map(|tool| format!("{}={}", tool.name, tool.bytes))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// One tool definition as the Anthropic Messages API receives it
#[derive(Serialize)]
struct WireTool<'decl> {
    name: &'decl str,
    #[serde(skip_serializing_if = "str::is_empty")]
    description: &'decl str,
    input_schema: &'decl Value,
}

/// Measure `tools` as the provider receives them.
///
/// A declaration without parameters is sized with the empty object schema
/// embacle sends in its place.
#[must_use]
pub fn measure_catalogue(tools: &Tool) -> CatalogueSize {
    use sha2::Digest;

    let empty_schema = json!({"type": "object", "properties": {}});
    let wire: Vec<WireTool<'_>> = tools
        .function_declarations
        .iter()
        .map(|decl| WireTool {
            name: &decl.name,
            description: &decl.description,
            input_schema: decl.parameters.as_ref().unwrap_or(&empty_schema),
        })
        .collect();

    // Serializing borrowed strings and `Value`s cannot fail.
    let serialized = serde_json::to_string(&wire).unwrap_or_default();
    let digest: String = format!("{:x}", Sha256::digest(serialized.as_bytes()))
        .chars()
        .take(DIGEST_HEX_LEN)
        .collect();

    let mut per_tool: Vec<ToolSize> = wire
        .iter()
        .map(|tool| {
            let one = serde_json::to_string(tool).unwrap_or_default();
            ToolSize {
                name: tool.name.to_owned(),
                bytes: one.len(),
                estimated_tokens: estimate_context_tokens(&one),
            }
        })
        .collect();
    per_tool.sort_by_key(|tool| Reverse(tool.bytes));

    CatalogueSize {
        tool_count: wire.len(),
        bytes: serialized.len(),
        estimated_tokens: estimate_context_tokens(&serialized),
        digest,
        per_tool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(
        name: &str,
        description: &str,
        parameters: Option<Value>,
    ) -> FunctionDeclaration {
        FunctionDeclaration {
            name: name.to_owned(),
            description: description.to_owned(),
            parameters,
        }
    }

    fn catalogue(declarations: Vec<FunctionDeclaration>) -> Tool {
        Tool {
            function_declarations: declarations,
        }
    }

    #[test]
    fn bytes_are_the_compact_wire_array() {
        let schema = json!({"type": "object", "properties": {"days": {"type": "integer"}}});
        let tools = catalogue(vec![
            declaration("get_stats", "Totals for a period.", Some(schema.clone())),
            declaration("ping", "", None),
        ]);
        let expected = serde_json::to_string(&json!([
            {"name": "get_stats", "description": "Totals for a period.", "input_schema": schema},
            {"name": "ping", "input_schema": {"type": "object", "properties": {}}},
        ]))
        .unwrap();

        let size = measure_catalogue(&tools);

        assert_eq!(size.tool_count, 2);
        assert_eq!(size.bytes, expected.len());
        assert_eq!(size.estimated_tokens, estimate_context_tokens(&expected));
    }

    #[test]
    fn per_tool_is_largest_first_and_sums_to_the_array_less_its_punctuation() {
        let tools = catalogue(vec![
            declaration("a", "short", None),
            declaration("b", "a much longer description than the first one", None),
            declaration("c", "medium length", None),
        ]);

        let size = measure_catalogue(&tools);

        let names: Vec<&str> = size.per_tool.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["b", "c", "a"]);
        // `[`, `]` and one comma between each pair of tools.
        let punctuation = 2 + (tools.function_declarations.len() - 1);
        let summed: usize = size.per_tool.iter().map(|t| t.bytes).sum();
        assert_eq!(summed + punctuation, size.bytes);
        assert_eq!(
            size.largest(2),
            format!("b={},c={}", size.per_tool[0].bytes, size.per_tool[1].bytes)
        );
    }

    #[test]
    fn digest_is_stable_and_moves_with_any_byte() {
        let base = catalogue(vec![declaration("get_stats", "Totals for a period.", None)]);
        let edited = catalogue(vec![declaration("get_stats", "Totals for a period!", None)]);

        let first = measure_catalogue(&base);

        assert_eq!(first.digest.len(), DIGEST_HEX_LEN);
        assert_eq!(first.digest, measure_catalogue(&base).digest);
        assert_ne!(first.digest, measure_catalogue(&edited).digest);
    }

    #[test]
    fn digest_moves_with_tool_order() {
        let forward = catalogue(vec![
            declaration("a", "x", None),
            declaration("b", "y", None),
        ]);
        let reversed = catalogue(vec![
            declaration("b", "y", None),
            declaration("a", "x", None),
        ]);

        assert_eq!(
            measure_catalogue(&forward).bytes,
            measure_catalogue(&reversed).bytes
        );
        assert_ne!(
            measure_catalogue(&forward).digest,
            measure_catalogue(&reversed).digest
        );
    }

    #[test]
    fn an_empty_catalogue_is_the_empty_array() {
        let size = measure_catalogue(&catalogue(Vec::new()));

        assert_eq!(size.tool_count, 0);
        assert_eq!(size.bytes, 2);
        assert!(size.per_tool.is_empty());
        assert_eq!(size.largest(20), "");
    }
}
