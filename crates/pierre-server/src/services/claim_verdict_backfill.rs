// ABOUTME: Sprint C17 — ClaimVerdict backfill over historical chat_messages
// ABOUTME: Walks assistant messages tenant-wide, runs the Tier 5.5 heuristic pipeline, persists verdicts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! ClaimVerdict backfill worker.
//!
//! Tier 5.5 verdicts are only written as new coach replies land via
//! `chat_orchestration::apply_claim_verification`. That leaves Sprint
//! C14 coach grading starved of statistical power for tenants with
//! existing history: every coach starts as `Provisional` until the
//! live stream accumulates 3+ graded verdicts per coach.
//!
//! The backfill walks every historical `assistant` message for the
//! tenant in `(created_at ASC, conversation_id ASC)` order and runs
//! the same heuristic pipeline
//! ([`crate::services::claim_verification::verify_reply_with_config_and_corpus`])
//! that the live dispatch path uses. The pipeline is pure-Rust:
//! `extract_heuristic` + rhetoric filter + deterministic bounds +
//! evidence corpus retrieval. No LLM is ever invoked. A large backfill
//! is effectively free in LLM credits and takes O(messages) time.
//!
//! Resume support lives in `system_settings` under
//! [`BACKFILL_CURSOR_SETTING_KEY`]: the service stores the id of the
//! last-processed message per tenant so a second run with `--since`
//! and no explicit cursor picks up where the first one left off.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::Row;
use tokio::time::sleep;
use tracing::{info, warn};

use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::database::Database as SqliteBackend;
use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_database::RepositoryRegistry;
use pierre_evals::VerificationConfig;
use pierre_memory::claims::{ClaimCategory, ClaimStatus};

use crate::errors::{AppError, AppResult};

/// Maximum number of messages scanned per backfill call.
///
/// Ship the call with `--limit` below this to honor operator wishes;
/// the hard cap is a defensive guard against a runaway wall-clock on
/// very large tenants.
pub const MAX_MESSAGES_SCANNED: u64 = 100_000;

/// Default wall-clock limit when the caller omits `--limit`.
pub const DEFAULT_MESSAGES_SCANNED: u64 = 10_000;

/// `system_settings` key used to persist the resume cursor per tenant.
pub const BACKFILL_CURSOR_SETTING_KEY: &str = "harness_claim_verdict_backfill_cursor";

/// Parameters for [`run_backfill`].
///
/// Grouped into a struct to keep the CLI wiring legible and the
/// function below the `too_many_arguments` ceiling.
#[derive(Debug, Clone)]
pub struct BackfillParams<'a> {
    /// Tenant to backfill.
    pub tenant_id: TenantId,
    /// Maximum messages to scan; clamped to `1..=MAX_MESSAGES_SCANNED`.
    pub limit: u64,
    /// Earliest `created_at` RFC3339 timestamp to consider. Pass
    /// `None` for no lower bound.
    pub since: Option<&'a str>,
    /// `true` to skip persistence and only log what would happen.
    pub dry_run: bool,
    /// Optional delay between messages to pace the scan. Pass
    /// `Duration::ZERO` for full speed.
    pub sleep_between: Duration,
    /// `true` to resume from the stored cursor; `false` to scan from
    /// `since` (or the beginning of time) regardless of prior runs.
    pub resume: bool,
}

/// Per-status counter returned to the CLI caller.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackfillStatusCounts {
    /// Claims the pipeline marked `supported`.
    pub supported: u64,
    /// Claims the pipeline marked `unsupported`.
    pub unsupported: u64,
    /// Claims the pipeline marked `contradicted`.
    pub contradicted: u64,
    /// Claims the rhetoric filter dropped as rhetorical.
    pub rhetorical: u64,
    /// Claims the pipeline could not confidently verify either way.
    pub unverifiable: u64,
}

/// Per-category counter, keyed by [`ClaimCategory::as_str`].
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackfillCategoryCounts {
    /// `physiological` category hits.
    pub physiological: u64,
    /// `training_prescription` category hits.
    pub training_prescription: u64,
    /// `nutrition` category hits.
    pub nutrition: u64,
    /// `recovery` category hits.
    pub recovery: u64,
    /// `supplement` category hits.
    pub supplement: u64,
    /// `injury_rehab` category hits.
    pub injury_rehab: u64,
}

impl BackfillCategoryCounts {
    fn bump(&mut self, cat: ClaimCategory) {
        match cat {
            ClaimCategory::Physiological => self.physiological += 1,
            ClaimCategory::TrainingPrescription => self.training_prescription += 1,
            ClaimCategory::Nutrition => self.nutrition += 1,
            ClaimCategory::Recovery => self.recovery += 1,
            ClaimCategory::Supplement => self.supplement += 1,
            ClaimCategory::InjuryRehab => self.injury_rehab += 1,
        }
    }
}

/// Aggregate stats emitted by [`run_backfill`] after a scan.
///
/// Serialized to JSON for CLI output; every field is counted before
/// and after `dry_run`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackfillStats {
    /// Total `chat_messages.role = 'assistant'` rows inspected.
    pub messages_scanned: u64,
    /// Messages with at least one flagged claim (unsupported + contradicted).
    pub messages_with_problems: u64,
    /// Total verdicts persisted (or would-be persisted under `dry_run`).
    pub verdicts_written: u64,
    /// Breakdown by verdict status.
    pub by_status: BackfillStatusCounts,
    /// Breakdown by claim category.
    pub by_category: BackfillCategoryCounts,
    /// Last-processed message id, `None` if nothing was scanned.
    pub last_message_id: Option<String>,
    /// `true` when `dry_run` was set — verdicts were counted but not persisted.
    pub dry_run: bool,
    /// Number of database errors swallowed during persistence.
    pub persistence_errors: u64,
}

/// Row yielded by the message walker.
struct AssistantMessageRow {
    message_id: String,
    conversation_id: String,
    user_id: String,
    coach_id: Option<String>,
    content: String,
}

/// Run the backfill for `params.tenant_id` using the supplied
/// [`VerificationConfig`] (typically `VerificationConfig::default()`
/// for the CLI wiring; admins can plug in tenant overrides later).
///
/// # Errors
///
/// Returns an error if the underlying database pool is unavailable,
/// or if the `system_settings` cursor cannot be read / written.
#[allow(clippy::cognitive_complexity)]
pub async fn run_backfill(
    repos: &RepositoryRegistry,
    database: &Database,
    params: &BackfillParams<'_>,
    config: &VerificationConfig,
) -> AppResult<BackfillStats> {
    let limit = params.limit.clamp(1, MAX_MESSAGES_SCANNED);
    let cursor = if params.resume {
        load_cursor(database, params.tenant_id).await?
    } else {
        None
    };

    info!(
        tenant_id = %params.tenant_id,
        limit,
        since = ?params.since,
        dry_run = params.dry_run,
        resume = params.resume,
        cursor = ?cursor,
        "starting claim verdict backfill"
    );

    let rows = fetch_assistant_messages(database, params, cursor.as_deref(), limit).await?;
    // CLI callers skip `resolve_corpus` because they don't have a
    // `ServerResources` handle and the compiled-in `corpus()` is the
    // same data the runtime registry falls back to when empty.
    let corpus = super::claim_verification::corpus();

    let mut stats = BackfillStats {
        dry_run: params.dry_run,
        ..Default::default()
    };

    for row in &rows {
        stats.messages_scanned += 1;
        stats.last_message_id = Some(row.message_id.clone());

        let verdicts = super::claim_verification::verify_reply_with_config_and_corpus(
            &row.content,
            config,
            corpus,
        );
        if verdicts.is_empty() {
            if params.sleep_between > Duration::ZERO {
                sleep(params.sleep_between).await;
            }
            continue;
        }

        let mut row_had_problem = false;
        for (claim, outcome) in verdicts {
            if matches!(
                outcome.status,
                ClaimStatus::Unsupported | ClaimStatus::Contradicted
            ) {
                row_had_problem = true;
            }
            stats.by_category.bump(claim.category);
            match outcome.status {
                ClaimStatus::Supported => stats.by_status.supported += 1,
                ClaimStatus::Unsupported => stats.by_status.unsupported += 1,
                ClaimStatus::Contradicted => stats.by_status.contradicted += 1,
                ClaimStatus::Rhetorical => stats.by_status.rhetorical += 1,
                ClaimStatus::Unverifiable => stats.by_status.unverifiable += 1,
            }

            if params.dry_run {
                stats.verdicts_written += 1;
                continue;
            }

            let insert = InsertClaimVerdictParams {
                tenant_id: params.tenant_id,
                user_id: &row.user_id,
                coach_id: row.coach_id.as_deref(),
                conversation_id: Some(&row.conversation_id),
                message_id: Some(&row.message_id),
                claim_text: &claim.text,
                category: claim.category,
                status: outcome.status,
                evidence_strength: outcome.evidence_strength,
                confidence: outcome.confidence,
                layer_fired: outcome.layer_fired,
                explanation: Some(&outcome.explanation),
                evidence_refs: outcome.evidence_refs.as_deref(),
            };
            match repos.claim_verdicts.insert_claim_verdict(&insert).await {
                Ok(_) => stats.verdicts_written += 1,
                Err(e) => {
                    warn!(
                        error = %e,
                        message_id = %row.message_id,
                        "failed to persist backfill verdict"
                    );
                    stats.persistence_errors += 1;
                }
            }
        }

        if row_had_problem {
            stats.messages_with_problems += 1;
        }

        if params.sleep_between > Duration::ZERO {
            sleep(params.sleep_between).await;
        }
    }

    if !params.dry_run {
        if let Some(last) = stats.last_message_id.as_deref() {
            save_cursor(database, params.tenant_id, last).await?;
        }
    }

    info!(
        tenant_id = %params.tenant_id,
        messages_scanned = stats.messages_scanned,
        verdicts_written = stats.verdicts_written,
        flagged_messages = stats.messages_with_problems,
        persistence_errors = stats.persistence_errors,
        "claim verdict backfill complete"
    );

    Ok(stats)
}

async fn fetch_assistant_messages(
    database: &Database,
    params: &BackfillParams<'_>,
    cursor_message_id: Option<&str>,
    limit: u64,
) -> AppResult<Vec<AssistantMessageRow>> {
    let tenant_str = params.tenant_id.to_string();
    let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);

    match database {
        Database::SQLite(db) => {
            let (cursor_created, cursor_id) = match cursor_message_id {
                Some(id) => load_cursor_timestamp_sqlite(db, id).await?,
                None => (None, None),
            };
            let since = params.since.unwrap_or("");
            let cursor_created_ref = cursor_created.as_deref().unwrap_or("");
            let cursor_id_ref = cursor_id.as_deref().unwrap_or("");

            let sql = r"
                SELECT m.id AS message_id,
                       m.conversation_id AS conversation_id,
                       c.user_id AS user_id,
                       c.coach_id AS coach_id,
                       m.content AS content,
                       m.created_at AS created_at
                FROM chat_messages m
                INNER JOIN chat_conversations c ON c.id = m.conversation_id
                WHERE c.tenant_id = $1
                  AND m.role = 'assistant'
                  AND ($2 = '' OR m.created_at >= $2)
                  AND ($3 = '' OR m.created_at > $3
                       OR (m.created_at = $3 AND m.id > $4))
                ORDER BY m.created_at ASC, m.id ASC
                LIMIT $5
            ";

            let rows = sqlx::query(sql)
                .bind(&tenant_str)
                .bind(since)
                .bind(cursor_created_ref)
                .bind(cursor_id_ref)
                .bind(limit_i64)
                .fetch_all(db.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to fetch messages: {e}")))?;

            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                out.push(AssistantMessageRow {
                    message_id: row.get("message_id"),
                    conversation_id: row.get("conversation_id"),
                    user_id: row.get("user_id"),
                    coach_id: row.try_get("coach_id").ok(),
                    content: row.get("content"),
                });
            }
            Ok(out)
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(_) => Err(AppError::internal(
            "claim verdict backfill currently only supports the SQLite backend; \
             implement the Postgres path when a production tenant needs it",
        )),
    }
}

async fn load_cursor_timestamp_sqlite(
    db: &SqliteBackend,
    message_id: &str,
) -> AppResult<(Option<String>, Option<String>)> {
    let sql = "SELECT id, created_at FROM chat_messages WHERE id = $1";
    let row = sqlx::query(sql)
        .bind(message_id)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to load cursor message: {e}")))?;
    row.map_or_else(
        || Ok((None, None)),
        |row| Ok((Some(row.get("created_at")), Some(row.get("id")))),
    )
}

async fn load_cursor(database: &Database, tenant_id: TenantId) -> AppResult<Option<String>> {
    let key = cursor_key(tenant_id);
    let setting = database.get_system_setting(&key).await?;
    Ok(setting.and_then(|s| {
        serde_json::from_str::<CursorDoc>(&s.value)
            .ok()
            .map(|d| d.last_message_id)
    }))
}

async fn save_cursor(
    database: &Database,
    tenant_id: TenantId,
    last_message_id: &str,
) -> AppResult<()> {
    let key = cursor_key(tenant_id);
    let doc = CursorDoc {
        last_message_id: last_message_id.to_owned(),
    };
    let value = serde_json::to_string(&doc)
        .map_err(|e| AppError::internal(format!("Failed to serialize backfill cursor: {e}")))?;
    database.set_system_setting(&key, &value).await?;
    Ok(())
}

fn cursor_key(tenant_id: TenantId) -> String {
    format!("{BACKFILL_CURSOR_SETTING_KEY}:{tenant_id}")
}

#[derive(Debug, Serialize, Deserialize)]
struct CursorDoc {
    last_message_id: String,
}
