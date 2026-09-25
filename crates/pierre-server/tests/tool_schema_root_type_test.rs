// ABOUTME: Every tool's inputSchema and outputSchema must be an object schema at the root
// ABOUTME: A bare anyOf root breaks the MCP Tool schema, and the TS SDK then rejects the whole tools/list reply
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The MCP schema types `Tool.inputSchema` and `Tool.outputSchema` as
//! `{ type: "object"; properties?; required? }` (2025-06-18 and 2025-11-25).
//! The official TypeScript SDK parses `tools/list` against that type, so one
//! tool whose schema root lacks `"type": "object"` fails the whole listing for
//! that client, not just the one tool (carnet#553).
//!
//! An answer type that is an untagged enum — `Formatted<T>`, the `format`
//! envelope — derives to a bare `anyOf`. The root `type` is only true if every
//! arm is an object too, so this checks that as well, resolving `$ref`s into
//! the schema's own `$defs`.

use pierre_mcp_server::tools::registry_builtin::get_tools;
use serde_json::Value;

/// Resolve a local `#/$defs/Name` reference against the root document.
fn resolve<'a>(schema: &'a Value, root: &'a Value) -> Option<&'a Value> {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .map_or(Some(schema), |reference| {
            reference
                .strip_prefix("#/$defs/")
                .and_then(|name| root.get("$defs")?.get(name))
        })
}

/// Whether every instance `schema` accepts is a JSON object.
fn only_accepts_objects(schema: &Value, root: &Value) -> bool {
    let Some(schema) = resolve(schema, root) else {
        return false;
    };
    if schema.get("type").and_then(Value::as_str) == Some("object") {
        return true;
    }
    for union in ["anyOf", "oneOf"] {
        if let Some(arms) = schema.get(union).and_then(Value::as_array) {
            return !arms.is_empty() && arms.iter().all(|arm| only_accepts_objects(arm, root));
        }
    }
    schema
        .get("allOf")
        .and_then(Value::as_array)
        .is_some_and(|parts| parts.iter().any(|part| only_accepts_objects(part, root)))
}

#[test]
fn every_tool_schema_is_an_object_at_the_root() {
    let tools = get_tools();
    let mut offenders: Vec<String> = Vec::new();
    let mut union_roots = 0_usize;
    let mut output_schemas = 0_usize;

    for tool in &tools {
        if tool.input_schema.schema_type != "object" {
            offenders.push(format!(
                "{} inputSchema: root type is not \"object\"",
                tool.name
            ));
        }
        let Some(output) = &tool.output_schema else {
            continue;
        };
        output_schemas += 1;
        if output.get("type").and_then(Value::as_str) != Some("object") {
            offenders.push(format!(
                "{} outputSchema: root type is not \"object\"",
                tool.name
            ));
        }
        if output.get("anyOf").is_some() || output.get("oneOf").is_some() {
            union_roots += 1;
        }
        let arms_are_objects = ["anyOf", "oneOf"]
            .iter()
            .filter_map(|union| output.get(*union)?.as_array())
            .all(|arms| {
                !arms.is_empty() && arms.iter().all(|arm| only_accepts_objects(arm, output))
            });
        if !arms_are_objects {
            offenders.push(format!(
                "{} outputSchema: an arm of the root accepts a non-object, so the root \
                 \"type\": \"object\" would refuse a real answer",
                tool.name
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "tool schemas that break the MCP Tool type:\n  {}",
        offenders.join("\n  ")
    );
    // A check over an empty catalogue, or one whose schemas no longer carry
    // the union roots this exists for, passes without looking at anything.
    assert!(
        output_schemas > 100,
        "this checks the declared outputSchemas, so it says nothing unless the \
         catalogue carries them: found {output_schemas}"
    );
    assert!(
        union_roots > 30,
        "the Formatted<T> answers derive an anyOf root; found only {union_roots}, so \
         this no longer covers the case it was written for"
    );
}
