// ABOUTME: Conversions from the platform's host tool types onto the tronc MCP trait surface.
// ABOUTME: definition()/capabilities() builders + AppResult<ToolResult> -> tronc ToolResponse mapping.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Helpers shared by every tool's tronc `McpTool` implementation in this crate.
//!
//! They keep each tool's `definition`/`capabilities`/`execute` bodies thin and
//! identical in shape:
//!
//! - [`tool_definition`] assembles a tronc [`Tool`] from the platform's typed
//!   name/description/[`JsonSchema`]/[`ToolAnnotations`] pieces.
//! - [`task_capable`] marks one of those definitions as allowed to answer with
//!   an MCP task handle.
//! - [`capabilities_to_tronc`] maps the platform's host capability flags to
//!   tronc's generic capability set (the fitness domain flags are dropped —
//!   tronc models those as registry string categories, supplied at registration).
//! - [`tool_result_to_response`] converts a tool body's `AppResult<ToolResult>`
//!   into the wire [`ToolResponse`], preserving the dual `content` + `structuredContent`
//!   shape the dispatch layer previously produced.

use std::collections::HashMap;
use std::hash::BuildHasher;

use dravr_tronc::mcp::schema::{Content, TaskSupport, Tool, ToolExecution, ToolResponse};
use dravr_tronc::mcp::tool::ToolCapabilities as TroncCapabilities;
use pierre_core::errors::{AppError, AppResult};
use pierre_formatters::{format_output, OutputFormat};
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;
use serde::Serialize;

use crate::capabilities::ToolCapabilities;

/// Assemble a tronc [`Tool`] definition from the platform's typed pieces.
///
/// The platform builds input schemas as typed [`JsonSchema`] values; tronc's
/// `Tool::input_schema` is raw JSON, so the schema is serialized here once.
#[must_use]
pub fn tool_definition(
    name: &str,
    description: &str,
    input_schema: JsonSchema,
    annotations: Option<ToolAnnotations>,
) -> Tool {
    Tool {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: serde_json::to_value(input_schema).unwrap_or_default(),
        annotations,
        output_schema: None,
        execution: None,
    }
}

/// Declare that a tool may answer with an MCP task handle (SEP-2663).
///
/// Task support is a property of the tool, so it is declared here, beside the
/// tool's own definition, and read back off the registry by the host seam that
/// decides the wire shape. It used to be a hand-curated list of names sitting
/// next to the registry instead: a new provider-fetching tool answered
/// synchronously, unlike its twenty-five siblings, until somebody remembered to
/// edit that list — silently, with nothing failing (carnet#232).
///
/// [`TaskSupport::Optional`], never `Required`: the fast-path budget decides
/// the wire shape per call, so a cache-warm call still answers inline and only
/// work that outlives the budget becomes a handle. `Required` would refuse a
/// non-declaring client outright.
///
/// Declare it on a tool whose worst path is at least one uncached provider
/// fetch — verified by reading the tool's own `execute` body down to
/// `create_authenticated_provider` / `fetch_activities_from_provider` /
/// `fetch_provider_activities`, never attributed by which file it lives in.
/// That is why `calculate_recovery_score` carries it while `analyze_sleep_quality`
/// in the same file does not.
///
/// Deliberately undeclared: `verify_claim` (deterministic, mid-sentence),
/// `set_goal` and the stored-data reads (database only), `analyze_sleep_quality`
/// and `track_sleep_trends` (every sleep-capable backend is an API),
/// `export_intervals` / `export_routes` / `extract_activity_streams` (one bounded
/// fetch since the single-activity helper), and the three bounded fan-outs
/// (`validate_recipe`, `analyze_meal_nutrition`, `discover_routes`). The full
/// classification of all 110 tools lives in the vault note "MCP Tasks — Which
/// Tools Answer Asynchronously".
#[must_use]
pub fn task_capable(tool: Tool) -> Tool {
    Tool {
        execution: Some(ToolExecution {
            task_support: TaskSupport::Optional,
        }),
        ..tool
    }
}

/// Declare the shape this tool answers with, derived from the Rust type it
/// actually returns.
///
/// Wraps [`tool_definition`] the way [`task_capable`] does, so declaring an
/// output schema costs one call site rather than a new parameter on every
/// tool that has not been typed yet.
///
/// Derived, never hand-written, and that is the whole point. MCP requires a
/// tool that declares `outputSchema` to answer with conforming
/// `structuredContent`, so a schema is a promise about the payload. A
/// hand-written one is a promise nothing keeps: the first person to add a
/// field to a `json!` literal makes it a lie that a conforming client
/// validates against and rejects. Taking both from `T` means the compiler is
/// what keeps them agreeing.
///
/// `schemars` is configured without `preserve_order`, so its object maps are
/// `BTreeMap`s and the rendered schema is byte-stable between builds — the
/// same property the tool *input* schemas needed when a `HashMap` made them
/// render differently run to run.
#[must_use]
pub fn answers_with<T: schemars::JsonSchema>(tool: Tool) -> Tool {
    Tool {
        output_schema: serde_json::to_value(schemars::schema_for!(T)).ok(),
        ..tool
    }
}

/// Serialize a typed tool result into the payload the tool answers with.
///
/// Fails loudly rather than degrading: a tool that declares an `outputSchema`
/// and then answers with something else is worse than one that errors, because
/// a conforming client rejects the reply and the athlete sees nothing either
/// way — but only the error says why.
pub fn ok_typed<T: Serialize>(tool: &str, payload: T) -> AppResult<ToolResult> {
    serde_json::to_value(payload)
        .map(ToolResult::ok)
        .map_err(|e| AppError::internal(format!("{tool} result did not serialize: {e}")))
}

/// A tool payload after the caller's `format` argument has been applied.
///
/// Which shape arrives depends on the REQUEST rather than the data, which is
/// why this is a type rather than three call sites deciding for themselves:
/// `format=json` sends the tool's own shape, `format=toon` sends a compact
/// string envelope, and a TOON conversion that fails falls back to JSON while
/// saying so. Untagged, so the derived schema is a `oneOf` over exactly those
/// three and a client can tell which it got.
///
/// The envelope keys are fixed (`toon`, `result`) rather than derived from a
/// per-tool `data_key`. A property name that changes per tool cannot be stated
/// in a schema, so the old `result_toon` / `recipes_toon` / `recipe_toon` /
/// `results_toon` spelling is gone — the key no longer carries information the
/// tool name already gives, and the contract is now expressible.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum Formatted<T> {
    /// `format=json`, the default: the tool's own shape, unchanged.
    Json(T),
    /// `format=toon`: the payload rendered as one compact TOON string.
    Toon {
        /// The rendered payload.
        toon: String,
        /// Always `toon`.
        format: String,
    },
    /// TOON was asked for and could not be produced. The payload still arrives,
    /// as JSON, and says why it is not what was requested — a caller that
    /// silently received JSON would parse it as TOON and fail further away.
    Fallback {
        /// The payload, unformatted.
        result: T,
        /// Always `json`.
        format: String,
        /// Always true; present so the fallback is detectable on its own.
        format_fallback: bool,
        /// Why TOON rendering failed.
        format_error: String,
    },
}

/// Apply the caller's requested output format to a typed payload.
///
/// One copy. It previously existed verbatim in two tool modules — 17 identical
/// lines each, nine call sites between them — with a third variant in
/// `protocol::format`, so the TOON envelope had three places to drift.
pub fn apply_format<T: Serialize>(payload: T, format: OutputFormat) -> Formatted<T> {
    match format {
        OutputFormat::Json => Formatted::Json(payload),
        OutputFormat::Toon => match serde_json::to_value(&payload) {
            Ok(value) => match format_output(&value, OutputFormat::Toon) {
                Ok(formatted) => Formatted::Toon {
                    toon: formatted.data,
                    format: "toon".to_owned(),
                },
                Err(e) => Formatted::Fallback {
                    result: payload,
                    format: "json".to_owned(),
                    format_fallback: true,
                    format_error: e.to_string(),
                },
            },
            Err(e) => Formatted::Fallback {
                result: payload,
                format: "json".to_owned(),
                format_fallback: true,
                format_error: e.to_string(),
            },
        },
    }
}

/// Build the object input schema almost every tool declares.
///
/// `JsonSchema` carries the full JSON Schema 2020-12 vocabulary — composition,
/// `$defs`, the validation keywords — so a literal has to spell out the fields
/// it is *not* using. Nearly every tool here wants the same thing: an object
/// with these properties and these required names. This states that once.
///
/// A tool needing composition or `$defs` builds the [`JsonSchema`] directly.
#[must_use]
pub fn object_schema<S: BuildHasher>(
    properties: HashMap<String, PropertySchema, S>,
    required: Option<Vec<String>>,
) -> JsonSchema {
    JsonSchema {
        schema_type: "object".to_owned(),
        properties: Some(properties.into_iter().collect()),
        required,
        ..Default::default()
    }
}

/// Map the platform's host capability flags to tronc's generic capability set.
///
/// Only the seven host-agnostic flags cross over. The fitness domain flags
/// (`ANALYTICS`, `GOALS`, `CONFIGURATION`, `RECIPES`, `COACHES`, `SLEEP_RECOVERY`)
/// are intentionally dropped: tronc models domain taxonomy as registry string
/// categories, which `register_builtin_tools` supplies via `register_with_category`.
#[must_use]
pub fn capabilities_to_tronc(caps: ToolCapabilities) -> TroncCapabilities {
    let mut out = TroncCapabilities::empty();
    if caps.contains(ToolCapabilities::REQUIRES_AUTH) {
        out |= TroncCapabilities::REQUIRES_AUTH;
    }
    if caps.contains(ToolCapabilities::REQUIRES_TENANT) {
        out |= TroncCapabilities::REQUIRES_TENANT;
    }
    if caps.contains(ToolCapabilities::REQUIRES_PROVIDER) {
        out |= TroncCapabilities::REQUIRES_PROVIDER;
    }
    if caps.contains(ToolCapabilities::READS_DATA) {
        out |= TroncCapabilities::READS_DATA;
    }
    if caps.contains(ToolCapabilities::WRITES_DATA) {
        out |= TroncCapabilities::WRITES_DATA;
    }
    if caps.contains(ToolCapabilities::ADMIN_ONLY) {
        out |= TroncCapabilities::ADMIN_ONLY;
    }
    if caps.contains(ToolCapabilities::PROFILE) {
        out |= TroncCapabilities::PROFILE;
    }
    out
}

/// `structuredContent` key under which a raised [`AppError`] records its
/// originating [`ErrorCode`] so the executor can rebuild the matching
/// [`crate::protocols::ProtocolError`] variant.
///
/// Its presence is the marker that distinguishes the two failure paths the
/// E3 cutover flattened into `is_error == true`: a body that returned
/// `Err(AppError)` ("the tool refused to run") carries this key, while a body
/// that returned `Ok(ToolResult::error(..))` (an in-band failure payload the
/// tool chose to surface as data) does not. The executor returns `Err` only
/// for the former, preserving the pre-E3 `Result` contract.
pub const RAISED_ERROR_CODE_KEY: &str = "__error_code";

/// Convert a tool body's `AppResult<ToolResult>` into a wire [`ToolResponse`].
///
/// A successful result keeps the dual representation the dispatch layer
/// produced: the JSON value both as a serialized text block (for pre-2025
/// clients) and as `structuredContent`. An `Err` becomes an in-band error
/// result carrying the error's message (MCP reports tool failures via
/// `isError`, not protocol-level JSON-RPC errors).
///
/// A raised `Err(AppError)` also records its originating [`ErrorCode`] under
/// [`RAISED_ERROR_CODE_KEY`] in `structuredContent`. The MCP wire path ignores
/// this field, but `UniversalExecutor::execute_tool` reads it to rebuild the
/// matching `ProtocolError` and re-raise — so callers that distinguish "tool
/// refused to run" (`Err`) from "tool returned a failure payload"
/// (`Ok` with `success == false`) keep their pre-E3 behaviour.
#[must_use]
pub fn tool_result_to_response(result: AppResult<ToolResult>) -> ToolResponse {
    match result {
        Ok(tool_result) => ToolResponse {
            content: vec![Content::Text {
                text: tool_result.content.to_string(),
            }],
            is_error: tool_result.is_error,
            structured_content: Some(tool_result.content),
        },
        Err(error) => {
            let mut structured = serde_json::Map::new();
            structured.insert(
                "error".to_owned(),
                serde_json::Value::String(error.to_string()),
            );
            structured.insert(
                RAISED_ERROR_CODE_KEY.to_owned(),
                serde_json::Value::String(format!("{:?}", error.code)),
            );
            // Preserve the `ProviderAuthRequired` signal across the tronc
            // boundary so the chat tool loop can short-circuit to OAuth recovery
            // (the executor re-raises it as `ProtocolError::ProviderAuthRequired`).
            if let Some(provider) = error.provider_auth_required_provider() {
                structured.insert(
                    "error_code".to_owned(),
                    serde_json::Value::String("provider_auth_required".to_owned()),
                );
                structured.insert("provider".to_owned(), serde_json::Value::String(provider));
            }
            ToolResponse {
                content: vec![Content::Text {
                    text: error.to_string(),
                }],
                is_error: true,
                structured_content: Some(serde_json::Value::Object(structured)),
            }
        }
    }
}
