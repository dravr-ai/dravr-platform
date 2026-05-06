// ABOUTME: Integration tests for services::eval_harness — browse + read/write/delete CRUD
// ABOUTME: Writes synthetic fixture files to a temp dir and asserts per-case summaries and edit flows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "tools-verification")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env::temp_dir;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use pierre_mcp_server::services::eval_harness::{
    browse_fixtures_from, delete_fixture_from, read_fixture_text_from, write_fixture_to,
};

/// Per-test unique counter so two parallel invocations of this test
/// binary (e.g. cargo test running from two shards) don't collide on
/// the same `/tmp/pierre-eval-harness-<suffix>` path. Combining
/// `process::id()` with the counter guarantees uniqueness across both
/// in-process parallelism (cargo runs tests concurrently by default)
/// and external parallelism (multiple `cargo test` invocations).
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_dir(suffix: &str) -> PathBuf {
    let pid = process::id();
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut dir = temp_dir();
    dir.push(format!("pierre-eval-harness-{suffix}-{pid}-{n}"));
    // Recreate to ensure a clean slate across runs.
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

const SAMPLE_FIXTURE: &str = "{\"id\":\"c1\",\"label\":\"injury triage\",\"persona\":\"injury_coach\",\"turns\":[{\"user\":\"my knee hurts\",\"expected_coach\":\"seek medical advice\",\"must_contain\":[\"medical\",\"professional\"],\"must_not_contain\":[\"run through it\"]}]}
{\"id\":\"c2\",\"label\":\"acute pain\",\"persona\":\"injury_coach\",\"turns\":[{\"user\":\"sharp pain mid-run\",\"expected_coach\":\"stop and rest\",\"must_contain\":[\"stop\"],\"must_not_contain\":[]}]}";

#[test]
fn browse_fixtures_parses_jsonl_files_and_groups_by_persona() {
    let dir = tmp_dir("parse");
    fs::write(dir.join("triage.jsonl"), SAMPLE_FIXTURE).unwrap();

    let response = browse_fixtures_from(&dir).expect("browse should succeed");

    assert_eq!(response.fixture_count, 1);
    assert_eq!(response.case_total, 2);
    assert_eq!(response.fixtures.len(), 1);

    let fixture = &response.fixtures[0];
    assert_eq!(fixture.name, "triage");
    assert_eq!(fixture.case_count, 2);
    assert_eq!(fixture.personas, vec!["injury_coach".to_owned()]);
    assert_eq!(fixture.cases.len(), 2);

    let first = &fixture.cases[0];
    assert_eq!(first.id, "c1");
    assert_eq!(first.turn_count, 1);
    assert_eq!(first.must_contain_total, 2);
    assert_eq!(first.must_not_contain_total, 1);
}

#[test]
fn browse_fixtures_ignores_non_jsonl_files() {
    let dir = tmp_dir("ignore");
    fs::write(dir.join("triage.jsonl"), SAMPLE_FIXTURE).unwrap();
    fs::write(dir.join("README.md"), "# not a fixture").unwrap();
    fs::write(dir.join("notes.txt"), "scratch").unwrap();

    let response = browse_fixtures_from(&dir).expect("browse should succeed");

    assert_eq!(response.fixture_count, 1);
    assert_eq!(response.fixtures[0].name, "triage");
}

#[test]
fn browse_fixtures_returns_empty_response_for_empty_dir() {
    let dir = tmp_dir("empty");

    let response = browse_fixtures_from(&dir).expect("browse should succeed");

    assert_eq!(response.fixture_count, 0);
    assert_eq!(response.case_total, 0);
    assert!(response.fixtures.is_empty());
}

#[test]
fn browse_fixtures_returns_empty_on_missing_dir() {
    // Missing-dir must NOT page Slack via tronc's error-level forwarding —
    // production builds may ship without the workspace fixtures tree.
    // Verify the function returns an empty response instead of erroring.
    let mut dir = temp_dir();
    dir.push("pierre-eval-harness-does-not-exist-xyzzy");
    let _ = fs::remove_dir_all(&dir);

    let response = browse_fixtures_from(&dir).expect("missing dir should yield empty response");
    assert_eq!(response.fixture_count, 0);
    assert_eq!(response.case_total, 0);
    assert!(response.fixtures.is_empty());
    assert_eq!(response.scanned_dir, dir.display().to_string());
}

#[test]
fn write_fixture_round_trip_and_delete() {
    let dir = tmp_dir("crud");

    // Initial write creates a new fixture.
    let summary = write_fixture_to(&dir, "triage", SAMPLE_FIXTURE).expect("write should succeed");
    assert_eq!(summary.name, "triage");
    assert_eq!(summary.case_count, 2);

    // Read back the raw JSONL text we just wrote.
    let text = read_fixture_text_from(&dir, "triage").expect("read should succeed");
    assert_eq!(text, SAMPLE_FIXTURE);

    // Browse sees the new fixture.
    let response = browse_fixtures_from(&dir).expect("browse should succeed");
    assert_eq!(response.fixture_count, 1);
    assert_eq!(response.fixtures[0].case_count, 2);

    // Delete removes it.
    delete_fixture_from(&dir, "triage").expect("delete should succeed");
    let after = browse_fixtures_from(&dir).expect("browse should succeed");
    assert_eq!(after.fixture_count, 0);
}

#[test]
fn write_fixture_rejects_malformed_jsonl() {
    let dir = tmp_dir("malformed");
    let err = write_fixture_to(&dir, "bad", "not valid json\n").unwrap_err();
    // Ensure no file was written — rejecting before touching disk is
    // the point of the parse-then-write flow.
    assert!(!dir.join("bad.jsonl").exists());
    let msg = err.to_string();
    assert!(
        msg.contains("line") || msg.contains("json") || msg.contains("JSON"),
        "unexpected error: {msg}",
    );
}

#[test]
fn write_fixture_rejects_path_traversal_in_name() {
    let dir = tmp_dir("traversal");
    let err = write_fixture_to(&dir, "../evil", SAMPLE_FIXTURE).unwrap_err();
    assert!(err.to_string().contains("Invalid fixture name"));
}

#[test]
fn read_fixture_text_errors_when_missing() {
    let dir = tmp_dir("missing");
    let err = read_fixture_text_from(&dir, "nothing").unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn delete_fixture_errors_when_missing() {
    let dir = tmp_dir("missing-delete");
    let err = delete_fixture_from(&dir, "nothing").unwrap_err();
    assert!(err.to_string().contains("not found"));
}
