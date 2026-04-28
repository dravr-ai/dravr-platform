// ABOUTME: pierre-cli harness subcommands — Sprint C17 ClaimVerdict backfill over chat_messages
// ABOUTME: Wraps services::claim_verdict_backfill in a clap interface with dry-run + resume support
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Harness operator commands.
//!
//! Today this module exposes the Sprint C17 claim-verdict backfill
//! only. Future harness operator utilities (re-extraction, evidence
//! registry sync, eval runs) should land alongside it under
//! `pierre-cli harness <subcommand>`.

#![cfg(feature = "tools-verification")]

use std::time::Duration;

use clap::Subcommand;
use tracing::info;

use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_evals::VerificationConfig;
use pierre_mcp_server::errors::{AppError, AppResult};
use pierre_mcp_server::services::claim_verdict_backfill::{
    run_backfill, BackfillParams, DEFAULT_MESSAGES_SCANNED,
};

/// `pierre-cli harness <subcommand>` dispatch.
#[derive(Subcommand)]
pub enum HarnessCommand {
    /// Backfill Tier 5.5 `claim_verdicts` from historical `chat_messages`.
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
            let parsed_tenant: TenantId = tenant_id
                .parse()
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
    }
    Ok(())
}
