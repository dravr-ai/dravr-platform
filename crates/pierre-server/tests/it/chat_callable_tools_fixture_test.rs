// ABOUTME: Freezes the names of the tools a chat turn can call against the committed list the clients read
// ABOUTME: The step trail names a step for exactly these tools, so the list must be the registry's, both ways
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// The list is the shipped tool set, which registers every tool category; a
// build with fewer categories registers fewer tools and is not that set.
#![cfg(feature = "tools-all")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # The chat-callable tool list is a reviewed artifact
//!
//! A chat turn is offered `ToolRegistry::chat_callable_schemas()` and nothing
//! else. The web and phone step trail (`@pierre/chat-utils`) names a step for
//! every one of those tools, and it learns which tools they are from
//! `packages/mcp-types/src/chat-callable-tools.json`. This test builds the full
//! registry and holds that file to it, name for name, in both directions.
//!
//! After an intended change, regenerate and review the diff:
//!
//! ```bash
//! MCP_SURFACE_FIXTURE_WRITE=1 cargo test -p pierre_mcp_server --test it -- chat_callable_tools_fixture_test::
//! ```

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::{env, fs};

use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::registry::ToolRegistry;

const WRITE_ENV: &str = "MCP_SURFACE_FIXTURE_WRITE";

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/mcp-types/src/chat-callable-tools.json")
}

#[test]
fn chat_callable_tools_match_the_committed_list() {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let actual: BTreeSet<String> = registry
        .chat_callable_schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect();
    assert!(
        !actual.is_empty(),
        "the full registry offers a chat turn no tool: the registry is broken"
    );

    let path = fixture_path();
    if env::var_os(WRITE_ENV).is_some() {
        let rendered = format!("{}\n", serde_json::to_string_pretty(&actual).unwrap());
        fs::write(&path, rendered).unwrap();
        return;
    }

    // Reading the file is the test: it is a generated artefact and this is its
    // generator, and nothing at runtime joins the list the clients read to the
    // registry the server builds.
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing {}: {e}; run with {WRITE_ENV}=1", path.display()));
    let listed: Vec<String> = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!(
            "{} is not a JSON array of names: {e}; run with {WRITE_ENV}=1",
            path.display()
        )
    });
    let listed_set: BTreeSet<String> = listed.iter().cloned().collect();

    let missing_from_file: Vec<&String> = actual.difference(&listed_set).collect();
    let stale_in_file: Vec<&String> = listed_set.difference(&actual).collect();
    assert!(
        missing_from_file.is_empty() && stale_in_file.is_empty(),
        "{} disagrees with chat_callable_schemas().\n\
         Callable but not listed: {missing_from_file:?}\n\
         Listed but not callable: {stale_in_file:?}\n\
         If intended, regenerate with {WRITE_ENV}=1, review the diff, and give \
         every new tool a step in packages/chat-utils/src/turn-trail.ts",
        path.display()
    );
    let sorted: Vec<String> = actual.into_iter().collect();
    assert_eq!(
        listed,
        sorted,
        "{} must list each name once, in name order; regenerate with {WRITE_ENV}=1",
        path.display()
    );
}
