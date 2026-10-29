// ABOUTME: The chat function-calling catalogue: the tool declarations a turn sends, and their wire size
// ABOUTME: One measurement for the boot log, the per-turn dispatch log, the admin route and the CI budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Chat tool catalogue
//!
//! [`offered_schemas`] is the set one turn offers: the registry's
//! chat-callable tools, less what the turn's seat withholds and what an
//! operator switched off for the caller ([`SwitchedOff`]). The declarations,
//! the prompt's names index and the loopback listing all read it, so none of
//! them names a tool another leaves out. [`declarations`] turns a set into the
//! function declarations a turn sends, [`build_mcp_tools`] does so for the
//! whole chat-callable set, and [`measure_catalogue`] sizes either as the
//! provider receives it.
//!
//! ## Deferred loading
//!
//! The catalogue is most of a cold prompt, and the model needs a handful of
//! its tools on most turns. [`defer_beyond_core`] marks every tool outside
//! [`LOADED_CORE`] deferred, on a turn whose model [`defers_on`] names: a
//! provider with on-demand loading (Anthropic's tool search, through embacle)
//! still receives every definition but loads one into the model's context only
//! when the model looks it up by name, and every other provider sends them all
//! loaded. A fallback tier whose model does not defer is handed them all loaded
//! ([`all_loaded`]). The prompt's names index says so, so the model knows a name it cannot see the parameters of is one
//! to load rather than one that does not exist — the failure that made a meal
//! question invent a dinner when `search_recipes` was dropped (c89da2396).
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
//! `description`, `input_schema`, `defer_loading` when set, compact JSON, in
//! the order declared — the tier the cost was measured on. They leave out what
//! the provider adds: its tool-use preamble (286 tokens on the 5.5 models),
//! the cache marker on the last loaded tool, and, when tools are deferred, the
//! tool search definition embacle puts first and the move of the deferred
//! tools behind the loaded ones. Those follow from the declarations alone, so
//! the digest still tells two cache prefixes apart.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::Arc;

use pierre_core::models::{EffectiveTool, TenantId};
use pierre_core::tokens::estimate_context_tokens;
use pierre_llm::{FunctionDeclaration, Tool};
use pierre_mcp_schema::ToolSchema;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::Sha256;
use tracing::warn;
use uuid::Uuid;

use crate::coach_seat::TurnSeat;
use crate::tool_selection::ToolSelectionService;
use crate::{registry::ToolRegistry, schema_canonical::to_canonical_value};

/// Hex characters of the SHA-256 kept as the catalogue digest
const DIGEST_HEX_LEN: usize = 16;

/// The tools a turn sends loaded when the rest are deferred: the ones the
/// model reaches for on most turns, so a typical turn loads nothing.
///
/// Recent activities and the training they add up to, the plan, recovery and
/// sleep, the athlete's profile and totals, and memory. Every other tool is
/// one search away by its name in the prompt's index.
pub const LOADED_CORE: &[&str] = &[
    "get_activities",
    "get_activity_intelligence",
    "get_athlete",
    "get_recovery_metrics",
    "get_sleep_sessions",
    "get_stats",
    "get_training_history",
    "get_training_plan",
    "recall_user_memory",
    "remember_fact",
];

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
    declarations(tool_registry.chat_callable_schemas())
}

/// The function declarations for `schemas`, in the order given.
///
/// Parameters go through [`to_canonical_value`]: key order is part of the
/// prefix a provider caches.
#[must_use]
pub fn declarations(schemas: Vec<ToolSchema>) -> Tool {
    let function_declarations = schemas
        .into_iter()
        .map(|schema| {
            FunctionDeclaration::new(
                schema.name,
                schema.description,
                Some(to_canonical_value(&schema.input_schema)),
            )
        })
        .collect();
    Tool {
        function_declarations,
    }
}

/// Model families that are sent deferred tools: those the live incident
/// corpus found reach a deferred tool as reliably as a loaded one.
///
/// Opus 5.5 did, and did better deferred than loaded (2026-10-29, 0 findings
/// against 3 with every tool loaded). Haiku 5.5, dev's fallback tier, did
/// not: deferred, it answered rare-tool turns from its own reckoning without
/// loading the tool (6 of 8 against 8 of 8 loaded), so it gets every tool
/// loaded. A family joins only on the same evidence.
const DEFERRING_MODEL_PREFIXES: &[&str] = &["claude-opus"];

/// Whether a turn answered by `model` is sent deferred tools.
#[must_use]
pub fn defers_on(model: &str) -> bool {
    DEFERRING_MODEL_PREFIXES
        .iter()
        .any(|prefix| model.starts_with(prefix))
}

/// `tools` with every declaration loaded: what a tier whose model does not
/// defer ([`defers_on`]) is sent when the chain's head deferred.
#[must_use]
pub fn all_loaded(tools: &Tool) -> Tool {
    let mut loaded = tools.clone();
    for decl in &mut loaded.function_declarations {
        decl.defer_loading = false;
    }
    loaded
}

/// Mark every declaration outside [`LOADED_CORE`] deferred.
///
/// Nothing is deferred when none of the core is in `tools`: a provider needs
/// one loaded tool besides its search, and a turn whose seat or operator left
/// out the whole core is better served by the narrower set sent loaded.
pub fn defer_beyond_core(tools: &mut Tool) {
    let in_core = |name: &str| LOADED_CORE.contains(&name);
    if !tools
        .function_declarations
        .iter()
        .any(|decl| in_core(&decl.name))
    {
        return;
    }
    for decl in &mut tools.function_declarations {
        decl.defer_loading = !in_core(&decl.name);
    }
}

/// The tools an operator switched off for one caller, by name
/// ([`EffectiveTool::is_switched_off`]): globally, for the tenant, for this
/// user, or off by catalogue default.
///
/// `tools/list` leaves them out, and a chat turn must leave out the same ones,
/// or the agent is offered a tool the deployment does not. Resolved once per
/// turn and shared by every surface that lists the turn's tools. Cheap to
/// clone.
#[derive(Debug, Clone, Default)]
pub struct SwitchedOff(Arc<HashSet<String>>);

impl SwitchedOff {
    /// What is switched off for `user_id` in `tenant_id`.
    ///
    /// A lookup that fails switches nothing off, as `tools/list` does: the
    /// listing falls back to the whole set and the executor still refuses a
    /// disabled tool.
    pub async fn resolve(
        tool_selection: &ToolSelectionService,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> Self {
        match tool_selection
            .get_effective_tools_for_user(tenant_id, user_id)
            .await
        {
            Ok(effective) => Self::from_effective(&effective),
            Err(e) => {
                warn!(
                    tenant_id = %tenant_id,
                    error = %e,
                    "chat tools: enablement lookup failed; offering every chat-callable tool"
                );
                Self::default()
            }
        }
    }

    /// The switched-off names among `effective`.
    #[must_use]
    pub fn from_effective(effective: &[EffectiveTool]) -> Self {
        Self(Arc::new(
            effective
                .iter()
                .filter(|tool| tool.is_switched_off())
                .map(|tool| tool.tool_name.clone())
                .collect(),
        ))
    }

    /// Whether `tool_name` is switched off.
    #[must_use]
    pub fn contains(&self, tool_name: &str) -> bool {
        self.0.contains(tool_name)
    }

    /// How many tools are switched off.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether nothing is switched off.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The chat-callable tools one turn offers, in registry order: less those
/// `seat` withholds and those switched off for the caller.
///
/// The one set a turn's declarations, its names index and its loopback listing
/// are built from, so the three cannot disagree. Neither input moves within a
/// conversation turn to turn, so the set stays byte-stable and the provider's
/// cached prefix survives (carnet#1256).
#[must_use]
pub fn offered_schemas(
    registry: &ToolRegistry,
    seat: TurnSeat,
    switched_off: &SwitchedOff,
) -> Vec<ToolSchema> {
    registry
        .chat_callable_schemas()
        .into_iter()
        .filter(|schema| {
            !switched_off.contains(&schema.name) && !seat.withholds(registry, &schema.name)
        })
        .collect()
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
    /// How many of the tools are deferred
    pub deferred_count: usize,
    /// Bytes of the loaded tools alone: what the model holds in context before
    /// it searches. Equal to [`Self::bytes`] when nothing is deferred.
    pub loaded_bytes: usize,
    /// Structure-aware estimate of [`Self::loaded_bytes`]
    pub loaded_estimated_tokens: u32,
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
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    defer_loading: bool,
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
            defer_loading: decl.defer_loading,
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

    let loaded: Vec<&WireTool<'_>> = wire.iter().filter(|tool| !tool.defer_loading).collect();
    let loaded_serialized = serde_json::to_string(&loaded).unwrap_or_default();

    CatalogueSize {
        tool_count: wire.len(),
        bytes: serialized.len(),
        estimated_tokens: estimate_context_tokens(&serialized),
        digest,
        deferred_count: wire.len() - loaded.len(),
        loaded_bytes: loaded_serialized.len(),
        loaded_estimated_tokens: estimate_context_tokens(&loaded_serialized),
        per_tool,
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::models::{TenantPlan, ToolCategory, ToolEnablementSource};

    use super::*;

    fn effective(name: &str, is_enabled: bool, source: ToolEnablementSource) -> EffectiveTool {
        EffectiveTool {
            tool_name: name.to_owned(),
            display_name: name.to_owned(),
            description: String::new(),
            category: ToolCategory::Fitness,
            is_enabled,
            source,
            min_plan: TenantPlan::Starter,
        }
    }

    #[test]
    fn what_an_operator_switched_off_is_off_and_a_plan_gap_is_not() {
        let switched_off = SwitchedOff::from_effective(&[
            effective("on", true, ToolEnablementSource::Default),
            effective("catalogue_off", false, ToolEnablementSource::Default),
            effective("tenant_off", false, ToolEnablementSource::TenantOverride),
            effective("user_off", false, ToolEnablementSource::UserOverride),
            effective("global_off", false, ToolEnablementSource::GlobalDisabled),
            effective("above_plan", false, ToolEnablementSource::PlanRestriction),
            effective("user_on", true, ToolEnablementSource::UserOverride),
        ]);

        for name in ["catalogue_off", "tenant_off", "user_off", "global_off"] {
            assert!(switched_off.contains(name), "{name} must be switched off");
        }
        for name in ["on", "above_plan", "user_on"] {
            assert!(!switched_off.contains(name), "{name} must stay offered");
        }
        assert_eq!(
            switched_off.len(),
            4,
            "exactly the four operator-switched-off tools are off"
        );
    }

    #[test]
    fn beyond_the_core_every_tool_is_deferred() {
        let mut tools = catalogue(vec![
            declaration("get_activities", "core", None),
            declaration("search_recipes", "rare", None),
            declaration("get_athlete", "core", None),
        ]);

        defer_beyond_core(&mut tools);

        let deferred: Vec<(&str, bool)> = tools
            .function_declarations
            .iter()
            .map(|decl| (decl.name.as_str(), decl.defer_loading))
            .collect();
        assert_eq!(
            deferred,
            [
                ("get_activities", false),
                ("search_recipes", true),
                ("get_athlete", false)
            ],
            "only the tool outside LOADED_CORE is deferred, in declaration order"
        );
    }

    #[test]
    fn a_set_without_the_core_is_sent_loaded() {
        let mut tools = catalogue(vec![
            declaration("search_recipes", "rare", None),
            declaration("get_group_member_activities", "rare", None),
        ]);

        defer_beyond_core(&mut tools);

        assert!(
            tools
                .function_declarations
                .iter()
                .all(|decl| !decl.defer_loading),
            "a set holding none of the core must be sent loaded: a provider needs one loaded tool"
        );
    }

    #[test]
    fn the_loaded_size_leaves_the_deferred_tools_out_and_the_digest_sees_them() {
        let loaded = catalogue(vec![
            declaration("get_activities", "core", None),
            declaration("search_recipes", "rare", None),
        ]);
        let mut deferred = loaded.clone();
        defer_beyond_core(&mut deferred);

        let all_loaded = measure_catalogue(&loaded);
        let size = measure_catalogue(&deferred);

        assert_eq!(
            all_loaded.deferred_count, 0,
            "nothing is deferred before defer_beyond_core runs"
        );
        assert_eq!(
            all_loaded.loaded_bytes, all_loaded.bytes,
            "with nothing deferred the loaded size is the whole size"
        );
        assert_eq!(
            size.deferred_count, 1,
            "the one tool outside the core is deferred"
        );
        let core_only = measure_catalogue(&catalogue(vec![declaration(
            "get_activities",
            "core",
            None,
        )]));
        assert_eq!(
            size.loaded_bytes, core_only.bytes,
            "the loaded size must count the core tool alone"
        );
        assert_ne!(
            size.digest, all_loaded.digest,
            "deferring a tool changes the prefix, so it must change the digest"
        );
    }

    #[test]
    fn only_the_opus_family_is_sent_deferred_tools() {
        assert!(defers_on("claude-opus-5-5"), "Opus 5.5 defers");
        for model in [
            "claude-haiku-5-5",
            "claude-sonnet-5-5",
            "gemini-flash-lite-latest",
            "",
        ] {
            assert!(
                !defers_on(model),
                "{model:?} must be sent every tool loaded"
            );
        }
    }

    #[test]
    fn all_loaded_clears_every_deferral() {
        let mut tools = catalogue(vec![
            declaration("get_activities", "core", None),
            declaration("search_recipes", "rare", None),
        ]);
        defer_beyond_core(&mut tools);

        let loaded = all_loaded(&tools);

        assert!(
            loaded
                .function_declarations
                .iter()
                .all(|decl| !decl.defer_loading),
            "a non-deferring tier must receive every tool loaded"
        );
        assert_eq!(
            loaded.function_declarations.len(),
            tools.function_declarations.len(),
            "loading the tools must not drop one"
        );
    }

    #[test]
    fn nothing_is_switched_off_by_default() {
        let switched_off = SwitchedOff::default();

        assert!(
            switched_off.is_empty(),
            "a default SwitchedOff must hold nothing"
        );
        assert!(
            !switched_off.contains("get_activities"),
            "a default SwitchedOff must switch no tool off"
        );
    }

    fn declaration(
        name: &str,
        description: &str,
        parameters: Option<Value>,
    ) -> FunctionDeclaration {
        FunctionDeclaration::new(name.to_owned(), description.to_owned(), parameters)
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
