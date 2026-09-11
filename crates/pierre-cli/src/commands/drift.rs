// ABOUTME: Drift detection sub-command for pierre-cli - compares contremaitre source files to DB rows
// ABOUTME: Catches silent divergence between dravr-contremaitre and the prod agents table
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Drift Detection Sub-Command
//!
//! Background: on 2026-05-25 we discovered the `dravr-mcp-server-seed-agents`
//! Cloud Run job had been exit-1 on every execution for 3.5 weeks. The seeder
//! was broken (commit c6630e46 from 2026-05-01) and no alert fired because
//! Cloud Run Job non-zero exits weren't monitored AND no signal compared
//! contremaitre to the prod DB.
//!
//! This sub-command is the daily drift gate. It walks the contremaitre clone,
//! reuses the seeder's content-hash function (`agents::parser::AgentDefinition::content_hash`),
//! and compares against the `agents.content_hash` column in the prod DB.
//! Hash mismatches exit non-zero so the Cloud Run Job failure alert (added
//! in the same infra change) surfaces them.
//!
//! ## Usage
//!
//! ```bash
//! # Run drift check (PIERRE_AGENTS_DIR points at a contremaitre clone)
//! pierre-cli check-drift agents
//! ```
//!
//! ## Semantics
//!
//! For every contremaitre `prompts/agents/<category>/<slug>/en.md` file:
//! * If the matching DB row has `source = 'contremaitre'` AND a different
//!   `content_hash` → ERROR + exit non-zero (drift detected).
//! * If the matching DB row has any other `source` (e.g. `'seed'`, `'custom'`)
//!   → WARN (source mismatch; the contremaitre file may need re-seeding to
//!   take ownership).
//! * If the DB has no row for the slug → WARN (missing agent).
//! * If the DB holds a catalogue-owned row whose file is gone → WARN
//!   (orphaned agent; the seed job's prune pass deletes it on its next run).
//! * Clean result → INFO `agent drift check: N agents checked, all in sync`.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use pierre_agent_parser::drift::{classify_drift, orphaned_slugs, DbAgentRow, DriftOutcome};
use pierre_agent_parser::{parse_agent_file, CANONICAL_LOCALE};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::redaction::redact_url;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use tracing::{error, info, warn};

/// `pierre-cli check-drift <domain>` subcommand.
#[non_exhaustive]
#[derive(Subcommand)]
pub enum DriftCommand {
    /// Compare contremaitre agent files to the `agents` table.
    Agents(AgentsArgs),
}

/// CLI arguments for `pierre-cli check-drift agents`.
#[derive(clap::Args)]
pub struct AgentsArgs {
    /// Path to the cloned contremaitre `prompts/agents` tree.
    ///
    /// Set via the flag or `$PIERRE_AGENTS_DIR`. The Cloud Run drift-check
    /// entrypoint clones contremaitre to a temp dir and exports this
    /// variable; local development can point it at a sibling checkout.
    #[arg(long, env = "PIERRE_AGENTS_DIR")]
    pub agents_dir: PathBuf,
}

/// Dispatch a `CheckDrift` subcommand.
///
/// Returns a non-zero exit (via `AppError`) when drift is detected — the
/// Cloud Run Job failure log-based metric picks it up and the
/// `dravr-mcp-server-job-failures` alert policy fires.
pub async fn dispatch(action: DriftCommand, database_url: &str) -> AppResult<()> {
    match action {
        DriftCommand::Agents(args) => run_agents(args, database_url).await,
    }
}

async fn run_agents(args: AgentsArgs, database_url: &str) -> AppResult<()> {
    let repos = connect(database_url).await?;
    let file_hashes = scan_agent_dir(&args.agents_dir)?;
    let mut totals = compare_to_db(&repos, &file_hashes).await?;
    report_orphans(&repos, &file_hashes, &mut totals).await?;
    finalize(file_hashes.len(), &totals)
}

/// Initialize the seeding-mode DB connection and return the repository
/// registry. Split out of `run_agents` to keep that function under the
/// workspace cognitive-complexity ceiling.
async fn connect(database_url: &str) -> AppResult<RepositoryRegistry> {
    info!(
        "Connecting to database for drift check: {}",
        redact_url(database_url)
    );
    let db = Database::init_for_seeding(database_url).await?;
    Ok(db.repositories())
}

/// Walk the contremaitre `prompts/agents` tree and return `(slug, hash)`
/// pairs for every canonical (`en.md`) file.
fn scan_agent_dir(agents_dir: &Path) -> AppResult<Vec<(String, String)>> {
    info!("Scanning contremaitre coaches at {}", agents_dir.display());
    let file_hashes = collect_file_hashes(agents_dir)?;
    info!(
        "Discovered {} canonical (en.md) coach file(s) to verify",
        file_hashes.len()
    );
    Ok(file_hashes)
}

/// Iterate the contremaitre snapshot, fetch each matching DB row, and
/// classify drift. Logging happens inside `record_outcome`; this function
/// just accumulates totals.
async fn compare_to_db(
    repos: &RepositoryRegistry,
    file_hashes: &[(String, String)],
) -> AppResult<DriftTotals> {
    let mut totals = DriftTotals::default();
    for (slug, file_hash) in file_hashes {
        let db_row = fetch_db_row(repos, slug).await?;
        record_outcome(
            slug,
            classify_drift(file_hash, db_row.as_ref()),
            &mut totals,
        );
    }
    Ok(totals)
}

/// Warn about catalogue-owned rows whose contremaitre file is gone.
///
/// The walk above only looks from the checkout towards the database, so a
/// row that outlived its file was invisible to the gate. The seed job's
/// prune pass removes such rows, but a seed job that has stopped running
/// would leave them, and this is the check that notices.
async fn report_orphans(
    repos: &RepositoryRegistry,
    file_hashes: &[(String, String)],
    totals: &mut DriftTotals,
) -> AppResult<()> {
    let file_slugs: Vec<String> = file_hashes.iter().map(|(slug, _)| slug.clone()).collect();
    let db_slugs = repos.seeder.seed_list_catalogue_slugs().await?;
    for slug in orphaned_slugs(&file_slugs, &db_slugs) {
        warn!(
            "coach orphaned in database: slug={} (DB has it; contremaitre does not — the seed job's prune pass deletes it on its next run)",
            slug
        );
        totals.warn += 1;
    }
    Ok(())
}

/// Convert the accumulated totals into the command's exit signal: an
/// `Err` for any hash drift (which the Cloud Run Job alert picks up), or
/// a clean `Ok` with a summary INFO line otherwise.
fn finalize(checked: usize, totals: &DriftTotals) -> AppResult<()> {
    if totals.drift > 0 {
        return Err(AppError::internal(format!(
            "coach drift check failed: {} coach(es) out of sync",
            totals.drift
        )));
    }
    info!(
        "coach drift check: {} coaches checked, all in sync ({} warnings)",
        checked, totals.warn
    );
    Ok(())
}

/// Per-slug outcome counters threaded through `run_agents`.
#[derive(Default)]
struct DriftTotals {
    drift: u32,
    warn: u32,
}

/// Emit the structured log line for a single slug's classification and
/// bump the matching counter. Lives outside `run_agents` to keep that
/// function under the workspace cognitive-complexity ceiling.
fn record_outcome(slug: &str, outcome: DriftOutcome, totals: &mut DriftTotals) {
    match outcome {
        DriftOutcome::InSync => {}
        DriftOutcome::HashDrift { expected, actual } => {
            let got = actual.as_deref().unwrap_or("<null>");
            error!(
                "coach drift detected: slug={} expected={} got={}",
                slug, expected, got
            );
            totals.drift += 1;
        }
        DriftOutcome::SourceMismatch { actual_source } => {
            warn!(
                "coach source mismatch: slug={} expected_source=contremaitre got_source={}",
                slug, actual_source
            );
            totals.warn += 1;
        }
        DriftOutcome::Missing => {
            warn!(
                "coach missing from database: slug={} (contremaitre has it; DB does not)",
                slug
            );
            totals.warn += 1;
        }
    }
}

/// Walk the contremaitre tree and compute the canonical (`en.md`) content
/// hash for every agent.
///
/// Reuses [`parse_agent_file`] so the hash matches whatever the seeder
/// would have stored — keeping a single source of truth for the
/// algorithm. Returns `(slug, content_hash)` pairs sorted by slug.
fn collect_file_hashes(agents_dir: &Path) -> AppResult<Vec<(String, String)>> {
    let pattern = agents_dir.join(format!("*/*/{CANONICAL_LOCALE}.md"));
    let pattern_str = pattern.to_string_lossy();

    let mut out: Vec<(String, String)> = Vec::new();
    for entry in glob::glob(&pattern_str)
        .map_err(|e| AppError::internal(format!("Glob pattern error: {e}")))?
    {
        let path = entry.map_err(|e| AppError::internal(format!("Glob error: {e}")))?;
        match parse_agent_file(&path) {
            Ok(agent) => {
                out.push((agent.frontmatter.name.clone(), agent.content_hash.clone()));
            }
            Err(e) => {
                // A malformed file is itself a drift signal — surface it
                // loudly and keep going so we don't lose visibility into
                // the rest of the corpus.
                error!(
                    "failed to parse contremaitre coach file {}: {}",
                    path.display(),
                    e
                );
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Fetch `(source, content_hash)` for a single agent slug via the
/// repository registry. Wraps the tuple into the typed [`DbAgentRow`].
async fn fetch_db_row(repos: &RepositoryRegistry, slug: &str) -> AppResult<Option<DbAgentRow>> {
    let row = repos.seeder.seed_find_agent_drift_info(slug).await?;
    Ok(row.map(|(source, content_hash)| DbAgentRow {
        source,
        content_hash,
    }))
}
