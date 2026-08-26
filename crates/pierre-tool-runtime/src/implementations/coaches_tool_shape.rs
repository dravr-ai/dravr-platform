// ABOUTME: Shared result envelope and MCP annotation sets for the coach tools
// ABOUTME: Output-format selection, TOON/JSON payload finalization, read/write/destructive hints

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde_json::{json, Value};

use pierre_formatters::{format_output, OutputFormat};
use pierre_mcp_schema::ToolAnnotations;

/// Extract output format ("json" or "toon") from tool arguments.
pub(super) fn extract_format(args: &Value) -> OutputFormat {
    args.get("format")
        .and_then(Value::as_str)
        .map(OutputFormat::from_str_param)
        .unwrap_or_default()
}

/// Apply TOON formatting to a result payload, mirroring `apply_format_to_response`.
///
/// For JSON format, returns `value` unchanged. For TOON format, returns
/// `{ "<data_key>_toon": <encoded>, "format": "toon" }` on success, or falls
/// back to `{ "<data_key>": <value>, "format": "json", "format_fallback": true,
/// "format_error": "<msg>" }` if encoding fails.
pub(super) fn finalize_payload(value: Value, data_key: &str, format: OutputFormat) -> Value {
    match format {
        OutputFormat::Json => value,
        OutputFormat::Toon => match format_output(&value, OutputFormat::Toon) {
            Ok(formatted) => {
                let toon_key = format!("{data_key}_toon");
                json!({
                    toon_key: formatted.data,
                    "format": "toon",
                })
            }
            Err(e) => json!({
                data_key: value,
                "format": "json",
                "format_fallback": true,
                "format_error": e.to_string(),
            }),
        },
    }
}

/// Annotations for idempotent write operations (create, update)
pub(super) fn write_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for destructive operations (delete)
pub(super) fn destructive_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(true),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for read-only coach retrieval operations
pub(super) fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}
