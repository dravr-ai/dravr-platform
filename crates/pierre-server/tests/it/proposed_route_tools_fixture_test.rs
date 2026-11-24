// ABOUTME: Freezes the names of the tools that store a proposed route's line against the committed list
// ABOUTME: The pipeline's route gate and the clients' route card are both held to that list, so it must be the registry's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// The route tools are registered by the shipped tool set; a build with fewer
// categories leaves them out and is not that set.
#![cfg(feature = "tools-all")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # The proposed-route tool list is a reviewed artifact
//!
//! A `route` block that cites a `route_ref` draws a route the coach proposes,
//! and only a tool that stores such a line may be cited. Three places act on
//! those names: the tools themselves (`create_route_tools`), the pipeline's
//! route gate (`PROPOSED_ROUTE_TOOLS` in `pierre-chat-pipeline`'s `viz_route`),
//! and the web and phone route card (`PROPOSED_ROUTE_TOOLS` in
//! `@pierre/chat-utils`). The two lists cannot see the tools, so each is held
//! to `packages/mcp-types/src/proposed-route-tools.json` by a test beside it,
//! and this test holds that file to the tools, name for name.
//!
//! After an intended change, regenerate and review the diff:
//!
//! ```bash
//! MCP_SURFACE_FIXTURE_WRITE=1 cargo test -p pierre_mcp_server --test it -- proposed_route_tools_fixture_test::
//! ```

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::{env, fs};

use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::implementations::routes::create_route_tools;
use pierre_tool_runtime::registry::ToolRegistry;

const WRITE_ENV: &str = "MCP_SURFACE_FIXTURE_WRITE";

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/mcp-types/src/proposed-route-tools.json")
}

#[test]
fn proposed_route_tools_match_the_committed_list() {
    let actual: BTreeSet<String> = create_route_tools()
        .iter()
        .map(|tool| tool.definition().name)
        .collect();
    assert!(
        !actual.is_empty(),
        "no tool stores a proposed route's line: the route tools are gone"
    );

    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let unregistered: Vec<&String> = actual
        .iter()
        .filter(|name| !registry.contains(name))
        .collect();
    assert!(
        unregistered.is_empty(),
        "a route tool is not in the registry, so no turn can cite it: {unregistered:?}"
    );

    let path = fixture_path();
    if env::var_os(WRITE_ENV).is_some() {
        let rendered = format!("{}\n", serde_json::to_string_pretty(&actual).unwrap());
        fs::write(&path, rendered).unwrap();
        return;
    }

    // Reading the file is the test: it is a generated artefact and this is its
    // generator, and nothing at runtime joins the lists that act on these
    // names to the tools that carry them.
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing {}: {e}; run with {WRITE_ENV}=1", path.display()));
    let listed: Vec<String> = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!(
            "{} is not a JSON array of names: {e}; run with {WRITE_ENV}=1",
            path.display()
        )
    });
    let sorted: Vec<String> = actual.into_iter().collect();
    assert_eq!(
        listed,
        sorted,
        "{} must list each tool that stores a proposed route's line once, in name order. \
         If intended, regenerate with {WRITE_ENV}=1, review the diff, and move both \
         PROPOSED_ROUTE_TOOLS lists with it",
        path.display()
    );
}
