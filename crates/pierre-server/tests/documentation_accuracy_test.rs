// ABOUTME: Measures the workspace and asserts AGENTS.md, README.md and book/src state the same numbers
// ABOUTME: Guards the counts and role descriptions that drifted for months because nothing read them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Drift guard for the headline facts the project documentation asserts.
//!
//! Every figure checked here is **derived from the tree**, never pinned as a
//! literal: the test counts the crates, the server integration test files, the
//! `tool_definition` call sites and the provider features `server-production`
//! compiles, then asserts the documentation repeats what it measured. Adding a
//! crate or a tool therefore fails this test until the docs follow, which is the
//! whole point — the numbers it replaces were off by 3.4x (14 vs 48 crates), 2x
//! (53 vs 110 MCP tools) and 1.7x (325 vs the real server test file count).
//!
//! The server test file count is the one figure documented approximately
//! (`~N files`), because it moves with every new test. It is checked against a
//! 5% band rather than exactly, so ordinary growth does not force a doc edit
//! while a stale order-of-magnitude still fails.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::str_to_string
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

/// Cargo feature name -> the name the prose uses for that provider.
///
/// Every provider feature the server manifest declares must appear here, so the
/// headline check can assert both directions: named-and-shipped, and
/// absent-and-not-shipped.
const PROVIDER_DISPLAY_NAMES: &[(&str, &str)] = &[
    ("provider-strava", "Strava"),
    ("provider-garmin", "Garmin"),
    ("provider-whoop", "Whoop"),
    ("provider-sciotte", "Sciotte"),
    ("provider-intervals-icu", "Intervals.icu"),
    ("provider-fitbit", "Fitbit"),
    ("provider-terra", "Terra"),
    ("provider-coros", "Coros"),
];

/// The AGENTS.md paragraph that summarises the platform in one sentence.
const HEADLINE_PREFIX: &str = "**Multi-tenant fitness intelligence API**";

/// Repository root, derived from this crate's manifest directory
/// (`<root>/crates/pierre-server`) so the test is invocation-independent.
fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map_or_else(|| manifest.clone(), Path::to_path_buf)
}

/// Read a repository-relative file, asserting rather than panicking on failure
/// so a missing path reports the path it looked for.
fn read_repo_file(relative: &str) -> String {
    let path = repo_root().join(relative);
    let text = fs::read_to_string(&path);
    assert!(
        text.is_ok(),
        "read {relative} from the repository root ({}): {text:?}",
        path.display()
    );
    text.unwrap_or_default()
}

/// Number of directories directly under `crates/` — the workspace members,
/// since the root manifest declares `members = ["crates/*"]`.
fn workspace_crate_count() -> usize {
    let dir = repo_root().join("crates");
    let entries = fs::read_dir(&dir);
    assert!(entries.is_ok(), "read crates/: {entries:?}");
    let Ok(entries) = entries else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .count()
}

/// Number of top-level `.rs` files in `crates/pierre-server/tests` — one test
/// binary each, which is what "the full suite compiles N files" means.
fn server_test_file_count() -> usize {
    let dir = repo_root().join("crates/pierre-server/tests");
    let entries = fs::read_dir(&dir);
    assert!(
        entries.is_ok(),
        "read crates/pierre-server/tests: {entries:?}"
    );
    let Ok(entries) = entries else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .count()
}

/// Collect every `.rs` file beneath `dir` into `out`.
fn collect_rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Count `tool_definition(` call sites across every crate's `src/`, discounting
/// the helper's own `fn tool_definition(` declaration. Each call site declares
/// exactly one MCP tool, which is the same enumeration
/// `scripts/ci/check-contremaitre-sync.sh` performs.
fn tool_definition_call_sites() -> usize {
    let crates_dir = repo_root().join("crates");
    let entries = fs::read_dir(&crates_dir);
    assert!(entries.is_ok(), "read crates/: {entries:?}");
    let Ok(entries) = entries else {
        return 0;
    };

    let mut files = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let src = entry.path().join("src");
        if src.is_dir() {
            collect_rust_sources(&src, &mut files);
        }
    }

    let mut calls = 0_usize;
    let mut declarations = 0_usize;
    for file in &files {
        let Ok(text) = fs::read_to_string(file) else {
            continue;
        };
        calls += text.matches("tool_definition(").count();
        declarations += text.matches("fn tool_definition(").count();
    }
    calls.saturating_sub(declarations)
}

/// The `provider-*` features the shipped `server-production` profile enables.
fn shipped_provider_features() -> BTreeSet<String> {
    let text = read_repo_file("crates/pierre-server/Cargo.toml");
    let parsed = text.parse::<Value>();
    assert!(
        parsed.is_ok(),
        "parse crates/pierre-server/Cargo.toml: {parsed:?}"
    );
    let Ok(manifest) = parsed else {
        return BTreeSet::new();
    };
    let features = manifest
        .get("features")
        .and_then(|features| features.get("server-production"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    features
        .iter()
        .filter_map(Value::as_str)
        .filter(|feature| feature.starts_with("provider-"))
        .map(str::to_owned)
        .collect()
}

/// Every `~N files` / `~N binaries` figure in `text`, in document order.
fn approximate_file_counts(text: &str) -> Vec<usize> {
    let mut found = Vec::new();
    for (index, _) in text.match_indices('~') {
        let rest = &text[index + 1..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            continue;
        }
        let tail = &rest[digits.len()..];
        if !tail.starts_with(" files") && !tail.starts_with(" binaries") {
            continue;
        }
        if let Ok(value) = digits.parse::<usize>() {
            found.push(value);
        }
    }
    found
}

#[test]
fn agents_md_states_the_real_crate_count() {
    let crates = workspace_crate_count();
    assert!(
        crates > 1,
        "crates/ holds {crates} directories — scan is stale"
    );

    let agents = read_repo_file("AGENTS.md");
    let expected = format!("Cargo workspace, {crates} crates");
    assert!(
        agents.contains(&expected),
        "AGENTS.md project map must read '{expected}': crates/ holds {crates} directories \
         and the root manifest declares members = [\"crates/*\"]"
    );
}

#[test]
fn agents_md_test_file_counts_track_the_tests_directory() {
    let actual = server_test_file_count();
    assert!(
        actual > 1,
        "crates/pierre-server/tests holds {actual} .rs files — scan is stale"
    );

    let agents = read_repo_file("AGENTS.md");
    let documented = approximate_file_counts(&agents);
    assert!(
        documented.len() >= 3,
        "AGENTS.md should carry the server test file count in at least three places \
         (project map, backend-tests section, the NEVER-cargo-test line); found {documented:?}"
    );

    for value in documented {
        let drift = actual.abs_diff(value) * 100 / actual;
        assert!(
            drift <= 5,
            "AGENTS.md documents ~{value} server integration test files but \
             crates/pierre-server/tests holds {actual} ({drift}% off)"
        );
    }
}

#[test]
fn book_introduction_states_the_registry_tool_count() {
    let tools = tool_definition_call_sites();
    assert!(
        tools > 50,
        "tool_definition scan found {tools} call sites across crates/*/src — the scan is stale"
    );

    let introduction = read_repo_file("book/src/introduction.md");
    let feature_bullet = format!("**{tools} MCP Tools**");
    let quick_link = format!("All {tools} MCP tools");
    assert!(
        introduction.contains(&feature_bullet),
        "book/src/introduction.md must advertise '{feature_bullet}': the registry declares \
         {tools} tools via tool_definition literals"
    );
    assert!(
        introduction.contains(&quick_link),
        "book/src/introduction.md Quick Links must read '{quick_link}': the registry declares \
         {tools} tools via tool_definition literals"
    );
}

#[test]
fn agents_md_headline_names_exactly_the_shipped_providers() {
    let shipped = shipped_provider_features();
    assert!(
        !shipped.is_empty(),
        "server-production declares no provider-* features — the manifest scan is stale"
    );
    for feature in &shipped {
        assert!(
            PROVIDER_DISPLAY_NAMES
                .iter()
                .any(|(name, _)| *name == feature.as_str()),
            "server-production enables {feature}, which PROVIDER_DISPLAY_NAMES does not map \
             to a prose name — add it here and to the AGENTS.md headline"
        );
    }

    let agents = read_repo_file("AGENTS.md");
    let headline = agents
        .lines()
        .find(|line| line.starts_with(HEADLINE_PREFIX));
    assert!(
        headline.is_some(),
        "AGENTS.md has no line starting with {HEADLINE_PREFIX}"
    );
    let headline = headline.unwrap_or_default();

    for (feature, display) in PROVIDER_DISPLAY_NAMES {
        let ships = shipped.contains(*feature);
        let named = headline.contains(*display);
        assert_eq!(
            ships, named,
            "AGENTS.md headline and the server-production profile disagree about {feature}: \
             shipped={ships}, named in the headline={named}. \
             docker/images/server/Dockerfile builds server-production, so the headline must \
             name that set and nothing else. Headline: {headline}"
        );
    }
}

#[test]
fn readme_describes_tronc_as_the_mcp_protocol_engine() {
    let readme = read_repo_file("README.md");

    let row = readme
        .lines()
        .find(|line| line.starts_with("| `dravr-tronc` |"));
    assert!(
        row.is_some(),
        "README.md satellite table has no `dravr-tronc` row"
    );
    let row = row.unwrap_or_default();
    assert!(
        row.contains("MCP protocol engine"),
        "README.md describes dravr-tronc as: {row}\n\
         It is the MCP protocol engine — nearly every dravr_tronc:: import in crates/*/src is \
         mcp::tool, mcp::schema, mcp::transport, mcp::host or mcp::server, and both the stdio \
         entry point and the HTTP mcp_router come from it. Notify is one small part."
    );

    assert!(
        !readme.contains("dravr-tronc<br/>alerting"),
        "README.md architecture diagram still labels dravr-tronc as the alerting layer"
    );
    assert!(
        !readme.contains("dravr-tronc-svc"),
        "README.md still shows dravr-tronc extracting as an alerting service — the MCP \
         protocol engine is linked into the server, not called over RPC"
    );
}
