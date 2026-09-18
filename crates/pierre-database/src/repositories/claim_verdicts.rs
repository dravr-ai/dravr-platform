// ABOUTME: Repository trait definitions for the claim verdict (bullshit detector) persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::TenantId;
use pierre_memory::{ClaimCategory, ClaimStatus, ClaimVerdict, EvidenceStrength, VerdictLayer};

/// Parameters for inserting a claim verdict via [`ClaimVerdictRepository`].
pub struct InsertClaimVerdictParams<'a> {
    /// Tenant that owns the verdict.
    pub tenant_id: TenantId,
    /// User the claim was said to.
    pub user_id: &'a str,
    /// Agent persona that authored the claim, if resolvable.
    pub agent_id: Option<&'a str>,
    /// Conversation the claim came from, if in-dispatch.
    pub conversation_id: Option<&'a str>,
    /// Message the claim was extracted from, if available.
    pub message_id: Option<&'a str>,
    /// Raw claim text.
    pub claim_text: &'a str,
    /// Category assigned by the extractor.
    pub category: ClaimCategory,
    /// Final pipeline status.
    pub status: ClaimStatus,
    /// Evidence strength backing the verdict.
    pub evidence_strength: EvidenceStrength,
    /// Pipeline confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Which layer produced the verdict.
    pub layer_fired: VerdictLayer,
    /// Optional user-facing rationale.
    pub explanation: Option<&'a str>,
    /// Optional evidence references (DOIs/PMIDs, comma-separated).
    pub evidence_refs: Option<&'a str>,
}

/// Per-status counters rolled up over a verdict scan window.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictStatusBreakdown {
    /// Claims the pipeline marked `supported`.
    pub supported: i64,
    /// Claims the pipeline marked `unsupported`.
    pub unsupported: i64,
    /// Claims the pipeline marked `contradicted`.
    pub contradicted: i64,
    /// Claims the rhetoric filter dropped as rhetorical.
    pub rhetorical: i64,
    /// Claims the pipeline could not confidently verify either way.
    pub unverifiable: i64,
}

/// One day's verdict counts, keyed by the UTC calendar date `YYYY-MM-DD`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictDailyBucket {
    /// UTC calendar date `YYYY-MM-DD`.
    pub date: String,
    /// Status breakdown for that day.
    pub counts: VerdictStatusBreakdown,
}

/// Calibration summary emitted by
/// [`ClaimVerdictRepository::aggregate_verdict_stats`].
///
/// Drives the admin "Verdict calibration" panel in the eval harness
/// tab — shows the live claim verdict mix and how it drifts day to
/// day over the requested window.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictCalibrationStats {
    /// Window lower bound (inclusive), RFC3339 UTC.
    pub window_start: String,
    /// Number of days scanned.
    pub window_days: i64,
    /// Totals over the full window.
    pub totals: VerdictStatusBreakdown,
    /// Per-day breakdown, oldest first.
    pub daily: Vec<VerdictDailyBucket>,
}

/// Claim verdict repository — persists post-LLM detector output.
#[async_trait]
pub trait ClaimVerdictRepository: Send + Sync {
    /// Persist a new claim verdict.
    async fn insert_claim_verdict(
        &self,
        params: &InsertClaimVerdictParams<'_>,
    ) -> AppResult<ClaimVerdict>;

    /// List verdicts for a conversation in chronological order (oldest first).
    async fn list_verdicts_for_conversation(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ClaimVerdict>>;

    /// List the most recent verdicts for a tenant, newest first. Used by the
    /// admin "flagged claims" dashboard.
    async fn list_recent_verdicts(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<ClaimVerdict>>;

    /// Aggregate verdict counts over the last `window_days` for a tenant.
    ///
    /// Returns both the full-window totals and a per-day breakdown so
    /// the admin calibration panel can show drift. `window_days` is
    /// clamped to `1..=365` by callers.
    async fn aggregate_verdict_stats(
        &self,
        tenant_id: TenantId,
        window_days: i64,
    ) -> AppResult<VerdictCalibrationStats>;
}

/// Persist one verdict. `$n` placeholders throughout: sqlx accepts them on
/// `SQLite` as well as Postgres, so one statement serves both drivers.
pub(crate) const INSERT_CLAIM_VERDICT_SQL: &str = r"
            INSERT INTO claim_verdicts (
                id, tenant_id, user_id, agent_id, conversation_id, message_id,
                claim_text, category, status, evidence_strength, confidence,
                layer_fired, explanation, evidence_refs, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ";

/// The column list every verdict read selects, in the order `row_to_verdict`
/// decodes it.
macro_rules! verdict_columns {
    () => {
        "id, tenant_id, user_id, agent_id, conversation_id, message_id,
                   claim_text, category, status, evidence_strength, confidence,
                   layer_fired, explanation, evidence_refs, created_at"
    };
}

/// Every verdict in one conversation, oldest first, scoped by tenant.
pub(crate) const LIST_VERDICTS_FOR_CONVERSATION_SQL: &str = concat!(
    "
            SELECT ",
    verdict_columns!(),
    "
            FROM claim_verdicts
            WHERE conversation_id = $1 AND tenant_id = $2
            ORDER BY created_at ASC
            "
);

/// The tenant's most recent verdicts, newest first.
pub(crate) const LIST_RECENT_VERDICTS_SQL: &str = concat!(
    "
            SELECT ",
    verdict_columns!(),
    "
            FROM claim_verdicts
            WHERE tenant_id = $1
            ORDER BY created_at DESC
            LIMIT $2
            "
);

/// Daily status counts over a window, with `$day` the engine's own way of
/// cutting a calendar day out of `created_at`.
///
/// This is the one construct the two engines cannot share. `created_at` is
/// `TEXT` holding RFC3339 on `SQLite` and `TIMESTAMPTZ` on Postgres, so the
/// day comes from a string slice on one and a formatted cast on the other.
/// Everything else — the filter, the grouping and the ordering — is identical,
/// so only the expression is a parameter.
macro_rules! verdict_daily_counts_sql {
    ($day:literal) => {
        concat!(
            "
            SELECT ",
            $day,
            " AS day,
                   status AS status,
                   COUNT(*) AS c
            FROM claim_verdicts
            WHERE tenant_id = $1 AND created_at >= $2
            GROUP BY day, status
            ORDER BY day ASC
            "
        )
    };
}
pub(crate) use verdict_daily_counts_sql;

/// Fold one `(status, count)` pair into a breakdown.
///
/// An unrecognised status is skipped rather than rejected: it means a verdict
/// enum variant the calibration response has no column for yet, and a count it
/// cannot place is not a reason to fail the whole aggregate.
pub(crate) fn apply_status_count(breakdown: &mut VerdictStatusBreakdown, status: &str, count: i64) {
    match status {
        "supported" => breakdown.supported += count,
        "unsupported" => breakdown.unsupported += count,
        "contradicted" => breakdown.contradicted += count,
        "rhetorical" => breakdown.rhetorical += count,
        "unverifiable" => breakdown.unverifiable += count,
        _ => {}
    }
}

/// Emit the whole [`ClaimVerdictRepository`] implementation for one backend.
///
/// `$ty` is the backend type and `$row` its driver's row type; `$daily_sql`
/// is that backend's resolved daily-counts statement, which its shell builds
/// from [`verdict_daily_counts_sql`] with its own day expression.
///
/// `tenant_id` binds as text on both engines because the column is `TEXT` in
/// both schemas — `TenantId`'s own Postgres encoding is a native `uuid`, which
/// would not match it, so the stringified form is the portable one here.
/// `created_at` binds and decodes as a `DateTime<Utc>`: sqlx writes RFC3339
/// text on `SQLite`, byte-identical to the `to_rfc3339()` the rows were
/// written with, and a native timestamp on Postgres.
macro_rules! impl_claim_verdict_repository {
    ($ty:ty, $row:ty, $daily_sql:ident) => {
        /// Decode one verdict row, rejecting an enum value the application no
        /// longer knows rather than substituting a default.
        fn row_to_verdict(row: &$row) -> AppResult<ClaimVerdict> {
            let category_str: String = row.get("category");
            let status_str: String = row.get("status");
            let strength_str: String = row.get("evidence_strength");
            let layer_str: String = row.get("layer_fired");

            let category = ClaimCategory::parse(&category_str).ok_or_else(|| {
                AppError::internal(format!("Invalid claim category: {category_str}"))
            })?;
            let status = ClaimStatus::parse(&status_str)
                .ok_or_else(|| AppError::internal(format!("Invalid claim status: {status_str}")))?;
            let evidence_strength = EvidenceStrength::parse(&strength_str).ok_or_else(|| {
                AppError::internal(format!("Invalid evidence strength: {strength_str}"))
            })?;
            let layer_fired = VerdictLayer::parse(&layer_str)
                .ok_or_else(|| AppError::internal(format!("Invalid verdict layer: {layer_str}")))?;

            Ok(ClaimVerdict {
                id: row.get("id"),
                tenant_id: row.get("tenant_id"),
                user_id: row.get("user_id"),
                agent_id: row.get("agent_id"),
                conversation_id: row.get("conversation_id"),
                message_id: row.get("message_id"),
                claim_text: row.get("claim_text"),
                category,
                status,
                evidence_strength,
                confidence: row.get("confidence"),
                layer_fired,
                explanation: row.get("explanation"),
                evidence_refs: row.get("evidence_refs"),
                created_at: row.get("created_at"),
            })
        }

        #[async_trait::async_trait]
        impl ClaimVerdictRepository for $ty {
            async fn insert_claim_verdict(
                &self,
                params: &InsertClaimVerdictParams<'_>,
            ) -> AppResult<ClaimVerdict> {
                let id = Uuid::new_v4().to_string();
                let now: DateTime<Utc> = Utc::now();

                sqlx::query(INSERT_CLAIM_VERDICT_SQL)
                    .bind(&id)
                    .bind(params.tenant_id.to_string())
                    .bind(params.user_id)
                    .bind(params.agent_id)
                    .bind(params.conversation_id)
                    .bind(params.message_id)
                    .bind(params.claim_text)
                    .bind(params.category.as_str())
                    .bind(params.status.as_str())
                    .bind(params.evidence_strength.as_str())
                    .bind(params.confidence)
                    .bind(params.layer_fired.as_str())
                    .bind(params.explanation)
                    .bind(params.evidence_refs)
                    .bind(now)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert claim verdict: {e}"))
                    })?;

                Ok(ClaimVerdict {
                    id,
                    tenant_id: params.tenant_id.to_string(),
                    user_id: params.user_id.to_owned(),
                    agent_id: params.agent_id.map(ToOwned::to_owned),
                    conversation_id: params.conversation_id.map(ToOwned::to_owned),
                    message_id: params.message_id.map(ToOwned::to_owned),
                    claim_text: params.claim_text.to_owned(),
                    category: params.category,
                    status: params.status,
                    evidence_strength: params.evidence_strength,
                    confidence: params.confidence,
                    layer_fired: params.layer_fired,
                    explanation: params.explanation.map(ToOwned::to_owned),
                    evidence_refs: params.evidence_refs.map(ToOwned::to_owned),
                    created_at: now,
                })
            }

            async fn list_verdicts_for_conversation(
                &self,
                conversation_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<ClaimVerdict>> {
                let rows = sqlx::query(LIST_VERDICTS_FOR_CONVERSATION_SQL)
                    .bind(conversation_id)
                    .bind(tenant_id.to_string())
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list claim verdicts: {e}"))
                    })?;

                rows.iter().map(row_to_verdict).collect()
            }

            async fn list_recent_verdicts(
                &self,
                tenant_id: TenantId,
                limit: i64,
            ) -> AppResult<Vec<ClaimVerdict>> {
                let rows = sqlx::query(LIST_RECENT_VERDICTS_SQL)
                    .bind(tenant_id.to_string())
                    .bind(limit)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list recent verdicts: {e}"))
                    })?;

                rows.iter().map(row_to_verdict).collect()
            }

            async fn aggregate_verdict_stats(
                &self,
                tenant_id: TenantId,
                window_days: i64,
            ) -> AppResult<VerdictCalibrationStats> {
                let days = window_days.clamp(1, 365);
                let window_start = Utc::now() - Duration::days(days);

                let rows = sqlx::query($daily_sql)
                    .bind(tenant_id.to_string())
                    .bind(window_start)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to aggregate claim verdicts: {e}"))
                    })?;

                let mut totals = VerdictStatusBreakdown::default();
                let mut daily_map: BTreeMap<String, VerdictStatusBreakdown> = BTreeMap::new();

                for row in &rows {
                    let day: String = row.get("day");
                    let status: String = row.get("status");
                    let count: i64 = row.get("c");
                    let bucket = daily_map.entry(day).or_default();
                    apply_status_count(bucket, &status, count);
                    apply_status_count(&mut totals, &status, count);
                }

                let daily = daily_map
                    .into_iter()
                    .map(|(date, counts)| VerdictDailyBucket { date, counts })
                    .collect();

                Ok(VerdictCalibrationStats {
                    window_start: window_start.to_rfc3339(),
                    window_days: days,
                    totals,
                    daily,
                })
            }
        }
    };
}
pub(crate) use impl_claim_verdict_repository;
