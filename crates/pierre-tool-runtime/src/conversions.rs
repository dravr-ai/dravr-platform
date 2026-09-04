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
use pierre_core::errors::AppResult;
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

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
