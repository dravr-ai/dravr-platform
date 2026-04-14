// ABOUTME: Phase B Sprint C16 — admin eval harness surface over pierre-evals golden fixtures
// ABOUTME: Scans the fixtures directory, parses JSONL, returns per-fixture and per-case summaries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin eval harness service.
//!
//! Read-only browser over the `pierre-evals` golden fixtures. The v1
//! scope is "let an admin see what fixtures exist, what cases they
//! cover, and what the per-turn expectations look like" — live eval
//! runs are deferred to a later sprint because a full judge pass is
//! expensive and needs its own execution queue.
//!
//! Fixtures live at the workspace root under
//! `crates/pierre-evals/fixtures/*.jsonl`. The server binary resolves
//! that directory at runtime via the `PIERRE_EVALS_FIXTURES_DIR`
//! environment variable, falling back to the canonical workspace path.

use std::env::var_os;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::error;

use pierre_evals::{GoldenCase, GoldenFixture};

use crate::errors::{AppError, AppResult};

/// Environment variable overriding the default fixtures directory.
pub const FIXTURES_DIR_ENV: &str = "PIERRE_EVALS_FIXTURES_DIR";

/// Canonical workspace-relative path the binary falls back to when the
/// env var is unset — matches the layout of a checked-out worktree.
pub const DEFAULT_FIXTURES_DIR: &str = "crates/pierre-evals/fixtures";

/// Per-case summary row returned to the admin UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureCaseSummary {
    /// Stable case id within the fixture file.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Coach persona the case targets.
    pub persona: String,
    /// Number of turns in the case.
    pub turn_count: usize,
    /// Total number of `must_contain` assertions across all turns.
    pub must_contain_total: usize,
    /// Total number of `must_not_contain` assertions across all turns.
    pub must_not_contain_total: usize,
}

/// Per-fixture summary returned to the admin UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureSummary {
    /// File stem (e.g. `injury_triage`).
    pub name: String,
    /// Absolute path the server read the fixture from.
    pub path: String,
    /// Total number of cases parsed from this file.
    pub case_count: usize,
    /// Distinct personas referenced across cases, sorted alphabetically.
    pub personas: Vec<String>,
    /// Per-case breakdown.
    pub cases: Vec<FixtureCaseSummary>,
}

/// Top-level response envelope for `GET /admin/evals/fixtures`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureBrowserResponse {
    /// Directory the server scanned.
    pub scanned_dir: String,
    /// Total fixture files parsed successfully.
    pub fixture_count: usize,
    /// Total cases across all fixtures.
    pub case_total: usize,
    /// Per-fixture summaries ordered by file name.
    pub fixtures: Vec<FixtureSummary>,
}

/// Resolve the fixtures directory via `PIERRE_EVALS_FIXTURES_DIR` or
/// the default workspace-relative path.
#[must_use]
pub fn resolve_fixtures_dir() -> PathBuf {
    var_os(FIXTURES_DIR_ENV).map_or_else(|| PathBuf::from(DEFAULT_FIXTURES_DIR), PathBuf::from)
}

/// Scan the fixtures directory and return a summary response.
///
/// # Errors
///
/// - Returns an error when the directory does not exist.
/// - Returns an error when any fixture file fails to parse. The error
///   message includes the offending file path and line number.
pub fn browse_fixtures() -> AppResult<FixtureBrowserResponse> {
    browse_fixtures_from(&resolve_fixtures_dir())
}

/// Variant used by the unit tests — takes an explicit root directory.
///
/// # Errors
///
/// Same as [`browse_fixtures`].
pub fn browse_fixtures_from(dir: &Path) -> AppResult<FixtureBrowserResponse> {
    if !dir.exists() {
        return Err(AppError::not_found(format!(
            "Fixtures directory not found: {}",
            dir.display()
        )));
    }

    let mut fixtures: Vec<FixtureSummary> = Vec::new();
    let mut case_total: usize = 0;

    let entries = fs::read_dir(dir).map_err(|e| {
        AppError::internal(format!(
            "Failed to read fixtures dir {}: {e}",
            dir.display()
        ))
    })?;

    for entry in entries {
        let entry =
            entry.map_err(|e| AppError::internal(format!("Failed to read fixture entry: {e}")))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let fixture = GoldenFixture::load_jsonl(&path).map_err(|e| {
            error!(path = %path.display(), error = %e, "failed to load eval fixture");
            e
        })?;

        let summary = summarize_fixture(&path, &fixture);
        case_total += summary.case_count;
        fixtures.push(summary);
    }

    fixtures.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(FixtureBrowserResponse {
        scanned_dir: dir.display().to_string(),
        fixture_count: fixtures.len(),
        case_total,
        fixtures,
    })
}

fn summarize_fixture(path: &Path, fixture: &GoldenFixture) -> FixtureSummary {
    let mut personas: Vec<String> = fixture.cases.iter().map(|c| c.persona.clone()).collect();
    personas.sort();
    personas.dedup();

    let cases = fixture.cases.iter().map(summarize_case).collect();

    FixtureSummary {
        name: fixture.name.clone(),
        path: path.display().to_string(),
        case_count: fixture.cases.len(),
        personas,
        cases,
    }
}

fn summarize_case(case: &GoldenCase) -> FixtureCaseSummary {
    let must_contain_total = case.turns.iter().map(|t| t.must_contain.len()).sum();
    let must_not_contain_total = case.turns.iter().map(|t| t.must_not_contain.len()).sum();
    FixtureCaseSummary {
        id: case.id.clone(),
        label: case.label.clone(),
        persona: case.persona.clone(),
        turn_count: case.turns.len(),
        must_contain_total,
        must_not_contain_total,
    }
}
