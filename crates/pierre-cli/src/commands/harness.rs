// ABOUTME: pierre-cli harness subcommands — ClaimVerdict backfill, memory dedup, and the blind grading of claims
// ABOUTME: Wraps the backfill and dedup services in clap, and draws and scores blind grading sheets (carnet#854)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Harness operator commands.
//!
//! The Sprint C17 claim-verdict backfill, the memory dedup backfill, and the
//! blind grading of production claims that measures the claim checker's
//! silent-miss rate (carnet#854): `grading-sheet` draws the sample from the
//! database, `grading-score` scores the graded sheets from files alone.
//! Further harness operator utilities (re-extraction, evidence registry sync,
//! eval runs) land alongside them under `pierre-cli harness <subcommand>`.

#![cfg(feature = "tools-verification")]

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use clap::Subcommand;
use tracing::info;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_evals::blind_grading::{build_export, ExportSpec, GradingExport, GradingKey};
use pierre_evals::blind_grading_score::{read_grades, score};
use pierre_evals::VerificationConfig;
use pierre_services::claim_verdict_backfill::{
    run_backfill, BackfillParams, DEFAULT_MESSAGES_SCANNED,
};
use pierre_services::memory_dedup::DedupConfig;
use pierre_services::memory_dedup_backfill::{
    run_backfill as run_memory_backfill, BackfillParams as MemoryBackfillParams,
};

/// Days of verdicts a blind grading sample is drawn from by default: the
/// recent month, the window #455's health read defaults to.
const DEFAULT_GRADING_WINDOW_DAYS: i64 = 30;

/// Claims a blind grading sample holds by default — the ~200 carnet#854 asks
/// for, enough for a two-sided 95% interval of about ±7 points on a rate.
const DEFAULT_GRADING_SAMPLE: usize = 200;

/// Of those, how many both graders see by default: a quarter of the sample,
/// enough items for a kappa that means something.
const DEFAULT_GRADING_OVERLAP: usize = 50;

/// The most eligible verdicts one export reads. A month of an alpha tenant is
/// far below it; past it the newest are kept, and the key says how many there
/// were.
const GRADING_POOL_LIMIT: i64 = 50_000;

/// The operator's key, as `grading-sheet` writes it.
const GRADING_KEY_FILE: &str = "grading-key.json";

/// `pierre-cli harness <subcommand>` dispatch.
#[derive(Subcommand)]
pub enum HarnessCommand {
    /// Backfill `claim_verdicts` from historical `chat_messages`.
    ///
    /// Runs the pure-Rust heuristic verification pipeline
    /// (`extract_heuristic` + rhetoric + deterministic + evidence) over
    /// every `role='assistant'` message for the tenant, persisting
    /// verdicts to `claim_verdicts`. Never calls an LLM, so cost is
    /// bounded by O(messages) wall time.
    BackfillVerdicts {
        /// Tenant id to backfill (UUID string).
        #[arg(long)]
        tenant_id: String,

        /// Maximum messages to scan in this run (`1..=100_000`).
        #[arg(long, default_value_t = DEFAULT_MESSAGES_SCANNED)]
        limit: u64,

        /// Optional earliest `created_at` RFC3339 timestamp.
        #[arg(long)]
        since: Option<String>,

        /// Skip persistence and only log what would happen.
        #[arg(long)]
        dry_run: bool,

        /// Sleep N milliseconds between messages to pace the scan.
        #[arg(long, default_value_t = 0)]
        sleep_ms: u64,

        /// Resume from the stored per-tenant cursor in `system_settings`.
        #[arg(long)]
        resume: bool,
    },

    /// Fold an athlete's duplicate facts into the fact they restate.
    ///
    /// The live extraction path stops new duplicates; this reaches the rows
    /// written before it existed — one athlete stating one goal, stored three
    /// times in three wordings. Reports and changes nothing unless `--apply`
    /// is given, and a merge never rewrites the athlete's own words.
    DedupMemory {
        /// Tenant id that owns the facts (UUID string).
        #[arg(long)]
        tenant_id: String,

        /// Athlete whose facts are folded.
        #[arg(long)]
        user_id: String,

        /// Most facts to read per kind.
        #[arg(long, default_value_t = 200)]
        limit: i64,

        /// Write the merges. Without it the run only reports them.
        #[arg(long)]
        apply: bool,
    },

    /// Draw a blind grading sample of recent claim verdicts (carnet#854).
    ///
    /// Reads the tenant's verdicts of the window that a grader outside the
    /// conversation may see — the athlete opted in to analytics and did not
    /// withhold sharing with their coach, and the reply was not built from
    /// first-party-only data — and draws a sample stratified by layer,
    /// category, locale and agent. Writes two grader sheets carrying the claims
    /// and no verdict, and the operator's key holding the verdicts. Refuses to
    /// overwrite a key or sheet already in the directory. Score the graded
    /// sheets with `harness grading-score`.
    ///
    /// Prints the share of the window the first-party-only rule withheld, and
    /// warns when it is most of it: the rates then hold for replies that read
    /// little or no provider data, not for the tenant's replies.
    GradingSheet {
        /// Tenant id to sample (UUID string).
        #[arg(long)]
        tenant_id: String,

        /// Days of verdicts to draw from (`1..=365`).
        #[arg(long, default_value_t = DEFAULT_GRADING_WINDOW_DAYS)]
        window_days: i64,

        /// Claims to draw.
        #[arg(long, default_value_t = DEFAULT_GRADING_SAMPLE)]
        size: usize,

        /// Of those, how many both graders see, for inter-grader agreement.
        #[arg(long, default_value_t = DEFAULT_GRADING_OVERLAP)]
        overlap: usize,

        /// Seed for the draw; recorded in the key. Defaults to the clock.
        #[arg(long)]
        seed: Option<u64>,

        /// Directory to write the key and the sheets into.
        #[arg(long)]
        out_dir: PathBuf,
    },

    /// Score graded blind sheets against their key (carnet#854).
    ///
    /// Reads files only — the key `grading-sheet` wrote and the sheets the
    /// graders returned — and prints the silent-miss, false-omission and
    /// false-positive rates overall and per layer with 95% intervals, and
    /// Cohen's kappa between graders. Needs no database.
    GradingScore {
        /// The key `grading-sheet` wrote.
        #[arg(long)]
        key: PathBuf,

        /// A graded sheet as `NAME=PATH`; repeat once per grader.
        #[arg(long = "grades", required = true)]
        grades: Vec<String>,

        /// A sheet of adjudicated grades that settles the items the graders
        /// disagreed on.
        #[arg(long)]
        adjudication: Option<PathBuf>,

        /// Also write the report as JSON here.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

/// Write `contents` to a file that must not exist yet: a key overwritten
/// after its sheets went out could never be scored again.
fn write_new(path: &Path, contents: &str) -> AppResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| AppError::invalid_input(format!("{}: {e}", path.display())))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| AppError::internal(format!("write {}: {e}", path.display())))
}

/// Write the key and every sheet of an export into `dir`, or nothing at all
/// when any of them is already there.
fn write_export(export: &GradingExport, dir: &Path) -> AppResult<Vec<PathBuf>> {
    fs::create_dir_all(dir)
        .map_err(|e| AppError::invalid_input(format!("{}: {e}", dir.display())))?;
    let key = serde_json::to_string_pretty(&export.key)
        .map_err(|e| AppError::internal(format!("Failed to serialize the grading key: {e}")))?;
    let mut files = vec![(dir.join(GRADING_KEY_FILE), key)];
    for sheet in &export.sheets {
        files.push((
            dir.join(format!("grading-sheet-{}.csv", sheet.number)),
            sheet.to_csv(),
        ));
    }
    // Checked as a set first, so a directory holding one old sheet does not
    // end up with a fresh key beside it; `write_new` still refuses a file that
    // appears in between.
    if let Some((existing, _)) = files.iter().find(|(path, _)| path.exists()) {
        return Err(AppError::invalid_input(format!(
            "{} already exists; draw into an empty directory",
            existing.display()
        )));
    }
    for (path, contents) in &files {
        write_new(path, contents)?;
    }
    Ok(files.into_iter().map(|(path, _)| path).collect())
}

fn read_file(path: &Path) -> AppResult<String> {
    fs::read_to_string(path)
        .map_err(|e| AppError::invalid_input(format!("{}: {e}", path.display())))
}

/// Score graded sheets against their key and print the report.
///
/// Dispatched before the database bootstrap: scoring reads files only.
///
/// # Errors
///
/// An invalid-input error for an unreadable file, a key that does not parse, a
/// `--grades` value that is not `NAME=PATH`, or a sheet [`read_grades`] or
/// [`score`] refuses.
pub fn score_grades(
    key: &Path,
    grades: &[String],
    adjudication: Option<&Path>,
    out: Option<&Path>,
) -> AppResult<()> {
    // Every flag is checked before any file is read, so a typo is named
    // first rather than surfacing as whatever the first unreadable file says.
    let specs = grades
        .iter()
        .map(|spec| {
            spec.split_once('=')
                .filter(|(name, path)| !name.trim().is_empty() && !path.is_empty())
                .ok_or_else(|| {
                    AppError::invalid_input(format!("--grades takes NAME=PATH, got `{spec}`"))
                })
        })
        .collect::<AppResult<Vec<_>>>()?;
    let key: GradingKey = serde_json::from_str(&read_file(key)?)
        .map_err(|e| AppError::invalid_input(format!("{}: {e}", key.display())))?;
    let sheets = specs
        .into_iter()
        .map(|(name, path)| read_grades(name.trim(), &read_file(Path::new(path))?))
        .collect::<AppResult<Vec<_>>>()?;
    let adjudication = adjudication
        .map(|path| read_grades("adjudication", &read_file(path)?))
        .transpose()?;

    let report = score(&key, &sheets, adjudication.as_ref())?;
    print!("{}", report.render());
    if let Some(out) = out {
        let json = serde_json::to_string_pretty(&report)
            .map_err(|e| AppError::internal(format!("Failed to serialize the report: {e}")))?;
        fs::write(out, json)
            .map_err(|e| AppError::invalid_input(format!("{}: {e}", out.display())))?;
        println!("Wrote the report to {}", out.display());
    }
    Ok(())
}

/// Dispatch a `pierre-cli harness ...` subcommand.
///
/// # Errors
///
/// Returns an error when the tenant id is malformed or when the
/// underlying backfill service propagates a database error.
pub async fn dispatch(
    command: HarnessCommand,
    repos: &RepositoryRegistry,
    database: &Database,
) -> AppResult<()> {
    match command {
        HarnessCommand::BackfillVerdicts {
            tenant_id,
            limit,
            since,
            dry_run,
            sleep_ms,
            resume,
        } => {
            let parsed_tenant = TenantId::parse_str(&tenant_id)
                .map_err(|_| AppError::invalid_input(format!("Invalid tenant id: {tenant_id}")))?;
            let config = VerificationConfig::default();
            let params = BackfillParams {
                tenant_id: parsed_tenant,
                limit,
                since: since.as_deref(),
                dry_run,
                sleep_between: Duration::from_millis(sleep_ms),
                resume,
            };
            let stats = run_backfill(repos, database, &params, &config).await?;

            // Emit a human-readable summary on stdout so operators
            // piping the CLI output to a log file see the totals
            // without digging through trace lines.
            info!(
                messages_scanned = stats.messages_scanned,
                messages_with_problems = stats.messages_with_problems,
                verdicts_written = stats.verdicts_written,
                supported = stats.by_status.supported,
                unsupported = stats.by_status.unsupported,
                contradicted = stats.by_status.contradicted,
                rhetorical = stats.by_status.rhetorical,
                unverifiable = stats.by_status.unverifiable,
                persistence_errors = stats.persistence_errors,
                dry_run = stats.dry_run,
                "backfill_complete"
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&stats).map_err(|e| AppError::internal(format!(
                    "Failed to serialize backfill stats: {e}"
                )))?
            );
        }
        HarnessCommand::DedupMemory {
            tenant_id,
            user_id,
            limit,
            apply,
        } => {
            let parsed_tenant = TenantId::parse_str(&tenant_id)
                .map_err(|_| AppError::invalid_input(format!("Invalid tenant id: {tenant_id}")))?;
            let params = MemoryBackfillParams {
                tenant_id: parsed_tenant,
                user_id: &user_id,
                limit,
                dry_run: !apply,
            };
            let config = DedupConfig {
                candidate_limit: usize::try_from(limit).unwrap_or(usize::MAX),
            };
            let stats = run_memory_backfill(repos.memory.as_ref(), &params, config).await?;
            println!(
                "scanned {} · would merge {} · merged {} · deleted {}{}",
                stats.facts_scanned,
                if apply { 0 } else { stats.facts_merged },
                if apply { stats.facts_merged } else { 0 },
                stats.facts_deleted,
                if apply {
                    ""
                } else {
                    "  (dry run — pass --apply to write)"
                }
            );
        }
        HarnessCommand::GradingSheet {
            tenant_id,
            window_days,
            size,
            overlap,
            seed,
            out_dir,
        } => {
            let parsed_tenant = TenantId::parse_str(&tenant_id)
                .map_err(|_| AppError::invalid_input(format!("Invalid tenant id: {tenant_id}")))?;
            let pool = repos
                .claim_verdicts
                .grading_pool(parsed_tenant, window_days, GRADING_POOL_LIMIT)
                .await?;
            // The clock's milliseconds are never negative after 1970.
            let seed = seed.unwrap_or_else(|| Utc::now().timestamp_millis().unsigned_abs());
            let export = build_export(
                pool,
                &ExportSpec {
                    tenant_id,
                    size,
                    overlap,
                    seed,
                },
            );
            let written = write_export(&export, &out_dir)?;
            let key = &export.key;
            println!(
                "{} claims drawn (seed {}) from the last {} days",
                key.sample_size, key.seed, key.window_days,
            );
            // The share the first-party-only rule withheld, with a warning
            // when it took most of the window: the rates the grade produces
            // hold for the pool that is left.
            print!("{}", key.coverage().render());
            if usize::try_from(key.eligible).is_ok_and(|eligible| eligible > key.pool_read) {
                println!(
                    "read the newest {} of {} eligible verdicts",
                    key.pool_read, key.eligible
                );
            }
            for path in written {
                println!("wrote {}", path.display());
            }
            println!(
                "Give each grader one sheet; keep {GRADING_KEY_FILE} to yourself until the grades are in."
            );
        }
        HarnessCommand::GradingScore { .. } => {
            return Err(AppError::internal(
                "harness grading-score dispatches before the database bootstrap",
            ));
        }
    }
    Ok(())
}
