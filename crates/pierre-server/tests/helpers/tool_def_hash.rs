// ABOUTME: A served tool list with each def_hash checked against its own definition, then set aside
// ABOUTME: For suites that compare a list derived by editing schemas with the list another caller is served
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// The helpers tree is compiled into every top-level test binary; only `it` calls this.
#![allow(dead_code)]

use dravr_tronc::mcp::schema::{Tool, DEF_HASH_FIELD, TOOL_META_KEY};
use serde_json::Value;

/// Every definition in `tools` with its `def_hash` set aside, after checking
/// that each one is the hash of the definition it is served with.
///
/// A view derived from another by editing a schema (the coach's list without
/// `athlete`) has a different definition, so a different hash: the derivation
/// compares definitions, and the hash is pinned to each one here instead.
pub(crate) fn without_def_hashes(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .map(|served| {
            let name = served["name"].as_str().unwrap();
            let definition: Tool = serde_json::from_value(served.clone()).unwrap();
            assert_eq!(
                served["_meta"][TOOL_META_KEY][DEF_HASH_FIELD].as_str(),
                Some(definition.def_hash().as_str()),
                "`{name}` is served with the hash of its own definition"
            );
            let mut tool = served.clone();
            let meta = tool["_meta"].as_object_mut().unwrap();
            let claims = meta[TOOL_META_KEY].as_object_mut().unwrap();
            claims.remove(DEF_HASH_FIELD);
            if claims.is_empty() {
                meta.remove(TOOL_META_KEY);
            }
            if meta.is_empty() {
                tool.as_object_mut().unwrap().remove("_meta");
            }
            tool
        })
        .collect()
}
