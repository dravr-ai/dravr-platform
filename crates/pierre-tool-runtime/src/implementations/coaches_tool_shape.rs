// ABOUTME: Shared result envelope and MCP annotation sets for the coach tools
// ABOUTME: Output-format selection, TOON/JSON payload finalization, read/write/destructive hints

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde_json::Value;

use pierre_formatters::OutputFormat;
use pierre_mcp_schema::ToolAnnotations;

/// Extract output format ("json" or "toon") from tool arguments.
pub(super) fn extract_format(args: &Value) -> OutputFormat {
    args.get("format")
        .and_then(Value::as_str)
        .map(OutputFormat::from_str_param)
        .unwrap_or_default()
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
