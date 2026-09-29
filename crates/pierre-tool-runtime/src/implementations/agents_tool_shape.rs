// ABOUTME: Shared schema pieces, output-format selection and MCP annotation sets for the agent tools
// ABOUTME: The category vocabulary and string-list properties the agent and admin tools declare alike

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde_json::Value;

use pierre_core::models::agents::AgentCategory;
use pierre_formatters::OutputFormat;
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};

/// A `category` property: `lead`, then the vocabulary `AgentCategory::parse` reads.
pub(super) fn category_property(lead: &str) -> PropertySchema {
    let names: Vec<&str> = AgentCategory::ALL
        .iter()
        .map(AgentCategory::as_str)
        .collect();
    PropertySchema {
        property_type: "string".to_owned(),
        description: Some(format!(
            "{lead} One of: {}; any other name reads as custom.",
            names.join(", ")
        )),
        ..Default::default()
    }
}

/// An array-of-strings property such as `tags` or `sample_prompts`.
pub(super) fn string_list_property(description: &str, item: &str) -> PropertySchema {
    PropertySchema {
        property_type: "array".to_owned(),
        description: Some(description.to_owned()),
        items: Some(Box::new(PropertySchema {
            property_type: "string".to_owned(),
            description: Some(item.to_owned()),
            ..Default::default()
        })),
        ..Default::default()
    }
}

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

/// Annotations for read-only agent retrieval operations
pub(super) fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}
