// ABOUTME: A served tool list with each def_hash checked against its own definition, then set aside with any def_sig
// ABOUTME: For suites comparing served lists, and for the catalog's recorded hashes once the descriptions settle
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// The helpers tree is compiled into every top-level test binary; only `it` calls this.
#![allow(dead_code)]

use std::time::Duration;

use dravr_tronc::mcp::schema::{Tool, DEF_HASH_FIELD, TOOL_META_KEY};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::mcp::tool_selection::refresh_tool_def_hashes;
use serde_json::Value;
use tokio::time::sleep;

/// Every definition in `tools` with its `def_hash` set aside, after checking
/// that each one is the hash of the definition it is served with, and its
/// `def_sig` set aside too: a signature is under the deployment's key, which
/// differs per database, so a fixture never pins one (`tool_def_sig_test`
/// verifies them).
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
            claims.remove("def_sig");
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

/// Bring `resources`' catalog hashes to what this instance will publish: wait
/// for its tool descriptions to settle, then refresh.
///
/// With no contremaitre overlay configured (CI) they settle at boot and the
/// boot sync has already recorded them, so the refresh writes nothing. A
/// shell that configures one (a developer's `.envrc`) settles only once the
/// poll's first sync has run, which may change descriptions the boot did not
/// record: the suite then asserts the same agreement over the overlay's text.
pub(crate) async fn settle_def_hashes(resources: &ServerContext) {
    let registry = &resources.mcp.tool_registry;
    for _ in 0..600 {
        if registry.descriptions_settled() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert!(
        registry.descriptions_settled(),
        "test setup bug: the tool descriptions never settled"
    );
    refresh_tool_def_hashes(registry, resources.common.repos.tool_selection.as_ref())
        .await
        .unwrap();
}
