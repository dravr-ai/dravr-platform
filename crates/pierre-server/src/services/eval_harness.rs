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
use tracing::{error, warn};

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
    // Missing-dir is not an operational error: production builds may
    // ship without the workspace fixtures tree, and dev binaries run
    // with whatever cwd the operator started them from. Returning an
    // empty response (with a single warn-level log per invocation)
    // keeps the admin UI usable and avoids paging on what is really
    // "no fixtures available here."
    if !dir.exists() {
        warn!(
            dir = %dir.display(),
            "Eval fixtures directory not found; returning empty response. \
             Set PIERRE_EVALS_FIXTURES_DIR to override or ship fixtures into the image."
        );
        return Ok(FixtureBrowserResponse {
            scanned_dir: dir.display().to_string(),
            fixture_count: 0,
            case_total: 0,
            fixtures: Vec::new(),
        });
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

/// Maximum size in bytes accepted by [`write_fixture`].
///
/// Golden fixtures are expected to be tiny hand-authored dialogue sets —
/// the cap stops an admin from accidentally pasting a 5 MB blob that
/// would bloat the repo and slow fixture scans.
pub const MAX_FIXTURE_BYTES: usize = 256 * 1024;

/// Read a single fixture's raw JSONL text so the admin UI can edit it.
///
/// # Errors
///
/// - Returns an error if `name` contains path separators or characters
///   outside `[A-Za-z0-9_-]`.
/// - Returns `AppError::not_found` if the fixture does not exist.
pub fn read_fixture_text(name: &str) -> AppResult<String> {
    read_fixture_text_from(&resolve_fixtures_dir(), name)
}

/// [`read_fixture_text`] variant taking an explicit root directory. Used by tests.
///
/// # Errors
///
/// Same as [`read_fixture_text`].
pub fn read_fixture_text_from(dir: &Path, name: &str) -> AppResult<String> {
    let path = validated_fixture_path(dir, name)?;
    if !path.exists() {
        return Err(AppError::not_found(format!("Fixture {name} not found")));
    }
    fs::read_to_string(&path)
        .map_err(|e| AppError::internal(format!("Failed to read fixture {name}: {e}")))
}

/// Write a fixture's raw JSONL text, creating the file if missing.
///
/// The body is validated by parsing it through
/// [`pierre_evals::GoldenFixture::parse_jsonl`] before any bytes touch
/// disk so the repo never holds a malformed fixture.
///
/// # Errors
///
/// - Invalid `name` (see [`read_fixture_text`]).
/// - Body larger than [`MAX_FIXTURE_BYTES`].
/// - Parse error from the JSONL body.
/// - Filesystem write errors.
pub fn write_fixture(name: &str, body: &str) -> AppResult<FixtureSummary> {
    write_fixture_to(&resolve_fixtures_dir(), name, body)
}

/// [`write_fixture`] variant taking an explicit root directory. Used by tests.
///
/// # Errors
///
/// Same as [`write_fixture`].
pub fn write_fixture_to(dir: &Path, name: &str, body: &str) -> AppResult<FixtureSummary> {
    let path = validated_fixture_path(dir, name)?;
    if body.len() > MAX_FIXTURE_BYTES {
        return Err(AppError::invalid_input(format!(
            "Fixture body exceeds {MAX_FIXTURE_BYTES}-byte cap ({} bytes)",
            body.len()
        )));
    }

    // Parse first, write second — rejects malformed JSONL without
    // touching disk. `parse_jsonl` also enforces line comments and
    // empty-line tolerance, so admin pasteable content is stable.
    let fixture = GoldenFixture::parse_jsonl(name, body)?;

    if !dir.exists() {
        fs::create_dir_all(dir).map_err(|e| {
            AppError::internal(format!(
                "Failed to create fixtures dir {}: {e}",
                dir.display()
            ))
        })?;
    }

    fs::write(&path, body)
        .map_err(|e| AppError::internal(format!("Failed to write fixture {name}: {e}")))?;

    Ok(summarize_fixture(&path, &fixture))
}

/// Delete a fixture file by name.
///
/// # Errors
///
/// - Invalid `name` (see [`read_fixture_text`]).
/// - `AppError::not_found` when the file does not exist.
/// - Filesystem errors.
pub fn delete_fixture(name: &str) -> AppResult<()> {
    delete_fixture_from(&resolve_fixtures_dir(), name)
}

/// [`delete_fixture`] variant taking an explicit root directory. Used by tests.
///
/// # Errors
///
/// Same as [`delete_fixture`].
pub fn delete_fixture_from(dir: &Path, name: &str) -> AppResult<()> {
    let path = validated_fixture_path(dir, name)?;
    if !path.exists() {
        return Err(AppError::not_found(format!("Fixture {name} not found")));
    }
    fs::remove_file(&path)
        .map_err(|e| AppError::internal(format!("Failed to delete fixture {name}: {e}")))
}

/// Resolve `<dir>/<name>.jsonl` after validating that `name` is a bare
/// identifier. Blocks path traversal and overlong names.
fn validated_fixture_path(dir: &Path, name: &str) -> AppResult<PathBuf> {
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::invalid_input(
            "Fixture name must be 1..=64 characters".to_owned(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::invalid_input(format!(
            "Invalid fixture name {name}: only [A-Za-z0-9_-] allowed"
        )));
    }
    Ok(dir.join(format!("{name}.jsonl")))
}
