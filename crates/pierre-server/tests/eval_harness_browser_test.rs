// ABOUTME: Sprint C16 — integration tests for services::eval_harness::browse_fixtures_from
// ABOUTME: Writes synthetic fixture files to a temp dir and asserts the per-case summary shape
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "tools-verification")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env::temp_dir;
use std::fs;
use std::path::PathBuf;

use pierre_mcp_server::services::eval_harness::browse_fixtures_from;

fn tmp_dir(suffix: &str) -> PathBuf {
    let mut dir = temp_dir();
    dir.push(format!("pierre-eval-harness-{suffix}"));
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
fn browse_fixtures_errors_on_missing_dir() {
    let mut dir = temp_dir();
    dir.push("pierre-eval-harness-does-not-exist-xyzzy");
    let _ = fs::remove_dir_all(&dir);

    let err = browse_fixtures_from(&dir).unwrap_err();
    assert!(err.to_string().contains("not found"));
}
