// ABOUTME: Repository trait definitions for the claim verdict (bullshit detector) persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

use pierre_core::models::TenantId;
use pierre_memory::{
    ClaimCategory, ClaimStatus, ClaimVerdict, DispositionReason, EvidenceStrength,
    VerdictDisposition, VerdictLayer,
};

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

/// Parameters for [`ClaimVerdictRepository::set_verdict_disposition`].
pub struct SetVerdictDispositionParams<'a> {
    /// Tenant that owns the verdict; a row from another tenant is not found.
    pub tenant_id: TenantId,
    /// The verdict being disposed.
    pub verdict_id: &'a str,
    /// What the triager concluded.
    pub disposition: VerdictDisposition,
    /// Which pipeline input they blamed, when they named one.
    pub reason: Option<DispositionReason>,
    /// Free-text note.
    pub note: Option<&'a str>,
    /// The admin's service name, as every other config write records it.
    pub disposed_by: &'a str,
    /// When the disposition was written.
    pub disposed_at: DateTime<Utc>,
}

/// The disposition axis of [`VerdictListFilter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispositionFilter {
    /// Rows nobody has disposed yet — the triage queue.
    Undisposed,
    /// Rows carrying exactly this disposition.
    Is(VerdictDisposition),
}

impl DispositionFilter {
    /// The value the filter binds: the sentinel for the undisposed queue, the
    /// disposition's own stable string otherwise.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Undisposed => UNDISPOSED_FILTER,
            Self::Is(d) => d.as_str(),
        }
    }

    /// Parse the wire form: the sentinel, or a disposition string.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        if s == UNDISPOSED_FILTER {
            return Some(Self::Undisposed);
        }
        VerdictDisposition::parse(s).map(Self::Is)
    }
}

/// The wire value that selects rows with no disposition, as a literal so the
/// filter statement can `concat!` it: the sentinel is spelled once.
macro_rules! undisposed_filter {
    () => {
        "undisposed"
    };
}

/// The wire value that selects rows with no disposition.
pub const UNDISPOSED_FILTER: &str = undisposed_filter!();

/// What [`ClaimVerdictRepository::list_verdicts_filtered`] narrows on.
///
/// Every axis is optional. `limit` is floored at 1 by the repository; the
/// ceiling is each caller's own, because the readers want different scans:
/// the admin list serves at most 200 rows, while agent grading and
/// myth-busting read up to their `MAX_VERDICTS_SCANNED` (1000 and 500).
#[derive(Debug, Clone, Default)]
pub struct VerdictListFilter {
    /// Only rows with this status.
    pub status: Option<ClaimStatus>,
    /// Only rows in this category.
    pub category: Option<ClaimCategory>,
    /// Only rows this agent emitted.
    pub agent_id: Option<String>,
    /// Only rows this layer decided.
    pub layer_fired: Option<VerdictLayer>,
    /// Only rows on this side of the disposition axis.
    pub disposition: Option<DispositionFilter>,
    /// Only rows said to this athlete.
    pub user_id: Option<String>,
    /// Page size, floored at 1; the caller sets the ceiling.
    pub limit: i64,
}

impl VerdictListFilter {
    /// The unfiltered recent scan: newest `limit` rows of the tenant.
    #[must_use]
    pub fn recent(limit: i64) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }
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

/// Disposition counters over one set of flagged verdicts.
///
/// A flagged verdict is one whose status is `unsupported` or `contradicted`
/// — the rows an athlete sees a warning for, and the only rows a disposition
/// says anything about.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictHealthTotals {
    /// Flagged verdicts in the window.
    pub flagged: i64,
    /// Of those, how many someone has disposed.
    pub disposed: i64,
    /// Disposed as a true catch.
    pub true_catches: i64,
    /// Disposed as a false positive.
    pub false_positives: i64,
    /// Disposed as unsure.
    pub unsure: i64,
}

impl VerdictHealthTotals {
    fn add(&mut self, disposition: Option<VerdictDisposition>, count: i64) {
        self.flagged += count;
        match disposition {
            None => {}
            Some(d) => {
                self.disposed += count;
                match d {
                    VerdictDisposition::TrueCatch => self.true_catches += count,
                    VerdictDisposition::FalsePositive => self.false_positives += count,
                    VerdictDisposition::Unsure => self.unsure += count,
                }
            }
        }
    }

    /// False positives over dispositions — the share of read flags that were
    /// noise. `0.0` while nothing is disposed.
    #[must_use]
    pub fn false_positive_rate(&self) -> f64 {
        ratio(self.false_positives, self.disposed)
    }
}

/// Health of one pipeline layer: what it flagged, and how much of what was
/// read turned out to be noise.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct VerdictLayerHealth {
    /// The layer, in its stable `snake_case` form.
    pub layer: String,
    /// Flagged verdicts this layer decided.
    pub flagged: i64,
    /// Of those, how many someone has disposed.
    pub disposed: i64,
    /// Disposed as a false positive.
    pub false_positives: i64,
    /// `false_positives / disposed`, `0.0` while nothing is disposed.
    pub rate: f64,
}

/// Health of one claim category.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct VerdictCategoryHealth {
    /// The category, in its stable `snake_case` form.
    pub category: String,
    /// Flagged verdicts in this category.
    pub flagged: i64,
    /// Of those, how many someone has disposed.
    pub disposed: i64,
    /// Disposed as a false positive.
    pub false_positives: i64,
    /// `false_positives / disposed`, `0.0` while nothing is disposed.
    pub rate: f64,
}

/// Health of one agent's flagged claims. `agent_id` is `None` for verdicts
/// the pipeline could not attribute to an agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct VerdictAgentHealth {
    /// The agent, when the verdict named one.
    pub agent_id: Option<String>,
    /// Flagged verdicts this agent emitted.
    pub flagged: i64,
    /// Of those, how many someone has disposed.
    pub disposed: i64,
    /// Disposed as a false positive.
    pub false_positives: i64,
    /// `false_positives / disposed`, `0.0` while nothing is disposed.
    pub rate: f64,
}

/// How many dispositions named one reason.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictReasonCount {
    /// The reason, in its stable `snake_case` form.
    pub reason: String,
    /// Dispositions carrying it.
    pub count: i64,
}

/// One day of flagged verdicts and the false positives found among them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VerdictHealthDay {
    /// UTC calendar date `YYYY-MM-DD`.
    pub date: String,
    /// Flagged verdicts written that day.
    pub flagged: i64,
    /// Of those, disposed as false positives.
    pub false_positives: i64,
}

/// Aggregate health of the claim-verification pipeline over a window, as
/// [`ClaimVerdictRepository::aggregate_verdict_health`] computes it.
///
/// Every breakdown counts only flagged verdicts, and every rate divides false
/// positives by dispositions rather than by flags: an undisposed row says
/// nothing about whether the detector was right.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct VerdictHealthStats {
    /// Window lower bound (inclusive), RFC3339 UTC.
    pub window_start: String,
    /// Number of days scanned.
    pub window_days: i64,
    /// Totals over the full window.
    pub totals: VerdictHealthTotals,
    /// `totals.false_positives / totals.disposed`, `0.0` while nothing is disposed.
    pub false_positive_rate: f64,
    /// Per layer, sorted by layer name.
    pub by_layer: Vec<VerdictLayerHealth>,
    /// Per category, sorted by category name.
    pub by_category: Vec<VerdictCategoryHealth>,
    /// Per agent, unattributed first, then sorted by agent id.
    pub by_agent: Vec<VerdictAgentHealth>,
    /// Per disposition reason, sorted by reason.
    pub by_reason: Vec<VerdictReasonCount>,
    /// Per day, oldest first.
    pub daily: Vec<VerdictHealthDay>,
}

/// Claim verdict repository — persists post-LLM detector output.
#[async_trait]
pub trait ClaimVerdictRepository: Send + Sync {
    /// Persist a new claim verdict.
    async fn insert_claim_verdict(
        &self,
        params: &InsertClaimVerdictParams<'_>,
    ) -> AppResult<ClaimVerdict>;

    /// One verdict by id, scoped by tenant; `None` when no such row.
    async fn get_verdict(
        &self,
        tenant_id: TenantId,
        verdict_id: &str,
    ) -> AppResult<Option<ClaimVerdict>>;

    /// List verdicts for a conversation in chronological order (oldest first).
    async fn list_verdicts_for_conversation(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ClaimVerdict>>;

    /// Every verdict extracted from one message, oldest first. Support's entry
    /// point: an athlete disputes a reply, support pastes its message id.
    async fn list_verdicts_for_message(
        &self,
        tenant_id: TenantId,
        message_id: &str,
    ) -> AppResult<Vec<ClaimVerdict>>;

    /// The tenant's verdicts matching `filter`, newest first. The admin
    /// triage list.
    async fn list_verdicts_filtered(
        &self,
        tenant_id: TenantId,
        filter: &VerdictListFilter,
    ) -> AppResult<Vec<ClaimVerdict>>;

    /// List the most recent verdicts for a tenant, newest first — the
    /// unfiltered scan agent grading and myth-busting read.
    async fn list_recent_verdicts(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<ClaimVerdict>> {
        self.list_verdicts_filtered(tenant_id, &VerdictListFilter::recent(limit))
            .await
    }

    /// Record support's judgement on a verdict and return the updated row.
    ///
    /// A second call overwrites the first: the disposition is a state, not a
    /// log. A verdict that does not exist in the tenant is
    /// [`AppError::not_found`].
    async fn set_verdict_disposition(
        &self,
        params: &SetVerdictDispositionParams<'_>,
    ) -> AppResult<ClaimVerdict>;

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

    /// Flagged verdicts and their dispositions over the last `window_days`,
    /// broken down by layer, category, agent, reason and day. `window_days`
    /// is clamped to `1..=365`.
    async fn aggregate_verdict_health(
        &self,
        tenant_id: TenantId,
        window_days: i64,
    ) -> AppResult<VerdictHealthStats>;
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

/// The column list every verdict read selects, in the order
/// [`verdict_from_row`] decodes it.
macro_rules! verdict_columns {
    () => {
        "id, tenant_id, user_id, agent_id, conversation_id, message_id,
                   claim_text, category, status, evidence_strength, confidence,
                   layer_fired, explanation, evidence_refs, created_at,
                   disposition, disposition_reason, disposition_note, disposed_by, disposed_at"
    };
}

/// One verdict by id, scoped by tenant.
pub(crate) const GET_VERDICT_SQL: &str = concat!(
    "
            SELECT ",
    verdict_columns!(),
    "
            FROM claim_verdicts
            WHERE id = $1 AND tenant_id = $2
            "
);

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

/// Every verdict extracted from one message, oldest first, scoped by tenant.
pub(crate) const LIST_VERDICTS_FOR_MESSAGE_SQL: &str = concat!(
    "
            SELECT ",
    verdict_columns!(),
    "
            FROM claim_verdicts
            WHERE message_id = $1 AND tenant_id = $2
            ORDER BY created_at ASC
            "
);

/// The tenant's verdicts matching every bound axis, newest first.
///
/// Each optional axis binds `NULL` to mean "any", so one statement serves
/// every combination the admin list can ask for and the plan can use the
/// `(tenant_id, …)` indexes on whichever axis is set. The disposition axis is
/// three-way — any, none, or one value — so it binds a sentinel
/// ([`UNDISPOSED_FILTER`]) for "none": `disposition = 'undisposed'` can never
/// match a row (the `CHECK` forbids it), so the sentinel selects only through
/// its `IS NULL` arm.
pub(crate) const LIST_VERDICTS_FILTERED_SQL: &str = concat!(
    "
            SELECT ",
    verdict_columns!(),
    "
            FROM claim_verdicts
            WHERE tenant_id = $1
              AND ($2 IS NULL OR status = $2)
              AND ($3 IS NULL OR category = $3)
              AND ($4 IS NULL OR agent_id = $4)
              AND ($5 IS NULL OR layer_fired = $5)
              AND ($6 IS NULL OR ($6 = '",
    undisposed_filter!(),
    "' AND disposition IS NULL) OR disposition = $6)
              AND ($7 IS NULL OR user_id = $7)
            ORDER BY created_at DESC
            LIMIT $8
            "
);

/// Overwrite the disposition columns of one verdict, scoped by tenant. Zero
/// rows affected is the not-found signal on both drivers.
pub(crate) const SET_VERDICT_DISPOSITION_SQL: &str = r"
            UPDATE claim_verdicts
            SET disposition = $3,
                disposition_reason = $4,
                disposition_note = $5,
                disposed_by = $6,
                disposed_at = $7
            WHERE id = $1 AND tenant_id = $2
            ";

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

/// Flagged verdicts in a window, counted by `$dim` and disposition.
///
/// One shape serves every health breakdown: `$dim` is a column for the layer,
/// category and agent tables, and the engine's own day expression for the
/// daily one — the same split [`verdict_daily_counts_sql`] makes, for the same
/// reason. Flagged means `unsupported` or `contradicted`, the two statuses an
/// athlete is warned about.
macro_rules! verdict_health_breakdown_sql {
    ($dim:literal) => {
        concat!(
            "
            SELECT ",
            $dim,
            " AS dim,
                   disposition AS disposition,
                   COUNT(*) AS c
            FROM claim_verdicts
            WHERE tenant_id = $1
              AND created_at >= $2
              AND status IN ('unsupported', 'contradicted')
            GROUP BY dim, disposition
            ORDER BY dim ASC
            "
        )
    };
}
pub(crate) use verdict_health_breakdown_sql;

/// Flagged verdicts by layer.
pub(crate) const HEALTH_BY_LAYER_SQL: &str = verdict_health_breakdown_sql!("layer_fired");
/// Flagged verdicts by category.
pub(crate) const HEALTH_BY_CATEGORY_SQL: &str = verdict_health_breakdown_sql!("category");
/// Flagged verdicts by agent; the unattributed rows group under `NULL`.
pub(crate) const HEALTH_BY_AGENT_SQL: &str = verdict_health_breakdown_sql!("agent_id");

/// Dispositions in a window counted by the reason they named.
pub(crate) const HEALTH_BY_REASON_SQL: &str = r"
            SELECT disposition_reason AS reason,
                   COUNT(*) AS c
            FROM claim_verdicts
            WHERE tenant_id = $1
              AND created_at >= $2
              AND status IN ('unsupported', 'contradicted')
              AND disposition_reason IS NOT NULL
            GROUP BY reason
            ORDER BY reason ASC
            ";

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

/// `numerator / denominator` as a share, `0.0` when there is nothing to divide by.
fn ratio(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        return 0.0;
    }
    numerator as f64 / denominator as f64
}

/// One `(dim, disposition, count)` row of a health breakdown, already decoded.
pub(crate) struct HealthBreakdownRow {
    /// The grouping key; `None` only for the unattributed agent group.
    pub dim: Option<String>,
    /// The disposition column, parsed; `None` for an undisposed row.
    pub disposition: Option<VerdictDisposition>,
    /// How many rows share the key and disposition.
    pub count: i64,
}

/// Decode one breakdown row, rejecting a disposition the application does
/// not know rather than folding it into the undisposed count.
pub(crate) fn health_breakdown_from_row<R>(row: &R) -> AppResult<HealthBreakdownRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str, e: sqlx::Error| {
        AppError::database(format!("claim_verdicts health {name}: {e}"))
    };
    let dim: Option<String> = row.try_get("dim").map_err(|e| col("dim", e))?;
    let raw: Option<String> = row
        .try_get("disposition")
        .map_err(|e| col("disposition", e))?;
    let disposition = raw
        .as_deref()
        .map(|s| {
            VerdictDisposition::parse(s)
                .ok_or_else(|| AppError::internal(format!("Invalid verdict disposition: {s}")))
        })
        .transpose()?;
    let count: i64 = row.try_get("c").map_err(|e| col("c", e))?;
    Ok(HealthBreakdownRow {
        dim,
        disposition,
        count,
    })
}

/// Decode every row of one breakdown statement.
pub(crate) fn health_breakdown_rows<R>(rows: &[R]) -> AppResult<Vec<HealthBreakdownRow>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    rows.iter().map(health_breakdown_from_row).collect()
}

/// Decode one `(reason, count)` row of the reason table.
pub(crate) fn reason_count_from_row<R>(row: &R) -> AppResult<VerdictReasonCount>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str, e: sqlx::Error| {
        AppError::database(format!("claim_verdicts health {name}: {e}"))
    };
    Ok(VerdictReasonCount {
        reason: row.try_get("reason").map_err(|e| col("reason", e))?,
        count: row.try_get("c").map_err(|e| col("c", e))?,
    })
}

/// Fold breakdown rows into per-key totals, keyed by the grouping value.
///
/// A `BTreeMap` so every breakdown comes out sorted by key — and, for the
/// agent table, with the unattributed `None` group first.
pub(crate) fn fold_health_breakdown(
    rows: &[HealthBreakdownRow],
) -> BTreeMap<Option<String>, VerdictHealthTotals> {
    let mut out: BTreeMap<Option<String>, VerdictHealthTotals> = BTreeMap::new();
    for row in rows {
        out.entry(row.dim.clone())
            .or_default()
            .add(row.disposition, row.count);
    }
    out
}

/// The layer table from its folded breakdown. A `None` key cannot occur —
/// `layer_fired` is `NOT NULL` — and is skipped rather than invented.
pub(crate) fn layer_health(
    folded: BTreeMap<Option<String>, VerdictHealthTotals>,
) -> Vec<VerdictLayerHealth> {
    folded
        .into_iter()
        .filter_map(|(key, t)| {
            key.map(|layer| VerdictLayerHealth {
                layer,
                flagged: t.flagged,
                disposed: t.disposed,
                false_positives: t.false_positives,
                rate: t.false_positive_rate(),
            })
        })
        .collect()
}

/// The category table from its folded breakdown; `category` is `NOT NULL`.
pub(crate) fn category_health(
    folded: BTreeMap<Option<String>, VerdictHealthTotals>,
) -> Vec<VerdictCategoryHealth> {
    folded
        .into_iter()
        .filter_map(|(key, t)| {
            key.map(|category| VerdictCategoryHealth {
                category,
                flagged: t.flagged,
                disposed: t.disposed,
                false_positives: t.false_positives,
                rate: t.false_positive_rate(),
            })
        })
        .collect()
}

/// The agent table from its folded breakdown, unattributed group included.
pub(crate) fn agent_health(
    folded: BTreeMap<Option<String>, VerdictHealthTotals>,
) -> Vec<VerdictAgentHealth> {
    folded
        .into_iter()
        .map(|(agent_id, t)| VerdictAgentHealth {
            agent_id,
            flagged: t.flagged,
            disposed: t.disposed,
            false_positives: t.false_positives,
            rate: t.false_positive_rate(),
        })
        .collect()
}

/// The daily series from its folded breakdown; the day expression is never null.
pub(crate) fn daily_health(
    folded: BTreeMap<Option<String>, VerdictHealthTotals>,
) -> Vec<VerdictHealthDay> {
    folded
        .into_iter()
        .filter_map(|(key, t)| {
            key.map(|date| VerdictHealthDay {
                date,
                flagged: t.flagged,
                false_positives: t.false_positives,
            })
        })
        .collect()
}

/// Window totals: the layer breakdown summed, since every flagged row has
/// exactly one layer.
pub(crate) fn health_totals(rows: &[HealthBreakdownRow]) -> VerdictHealthTotals {
    let mut totals = VerdictHealthTotals::default();
    for row in rows {
        totals.add(row.disposition, row.count);
    }
    totals
}

/// Decode one verdict row via `try_get` only, rejecting an enum value the
/// application no longer knows rather than substituting a default.
///
/// # Errors
/// A database error naming the first column that cannot be decoded, or an
/// internal error naming an enum value outside the application's vocabulary.
pub(crate) fn verdict_from_row<R>(row: &R) -> AppResult<ClaimVerdict>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    f32: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col =
        |name: &str, e: sqlx::Error| AppError::database(format!("claim_verdicts {name}: {e}"));
    let category_str: String = row.try_get("category").map_err(|e| col("category", e))?;
    let status_str: String = row.try_get("status").map_err(|e| col("status", e))?;
    let strength_str: String = row
        .try_get("evidence_strength")
        .map_err(|e| col("evidence_strength", e))?;
    let layer_str: String = row
        .try_get("layer_fired")
        .map_err(|e| col("layer_fired", e))?;
    let disposition_str: Option<String> = row
        .try_get("disposition")
        .map_err(|e| col("disposition", e))?;
    let reason_str: Option<String> = row
        .try_get("disposition_reason")
        .map_err(|e| col("disposition_reason", e))?;

    let category = ClaimCategory::parse(&category_str)
        .ok_or_else(|| AppError::internal(format!("Invalid claim category: {category_str}")))?;
    let status = ClaimStatus::parse(&status_str)
        .ok_or_else(|| AppError::internal(format!("Invalid claim status: {status_str}")))?;
    let evidence_strength = EvidenceStrength::parse(&strength_str)
        .ok_or_else(|| AppError::internal(format!("Invalid evidence strength: {strength_str}")))?;
    let layer_fired = VerdictLayer::parse(&layer_str)
        .ok_or_else(|| AppError::internal(format!("Invalid verdict layer: {layer_str}")))?;
    let disposition = disposition_str
        .as_deref()
        .map(|s| {
            VerdictDisposition::parse(s)
                .ok_or_else(|| AppError::internal(format!("Invalid verdict disposition: {s}")))
        })
        .transpose()?;
    let disposition_reason = reason_str
        .as_deref()
        .map(|s| {
            DispositionReason::parse(s)
                .ok_or_else(|| AppError::internal(format!("Invalid disposition reason: {s}")))
        })
        .transpose()?;

    Ok(ClaimVerdict {
        id: row.try_get("id").map_err(|e| col("id", e))?,
        tenant_id: row.try_get("tenant_id").map_err(|e| col("tenant_id", e))?,
        user_id: row.try_get("user_id").map_err(|e| col("user_id", e))?,
        agent_id: row.try_get("agent_id").map_err(|e| col("agent_id", e))?,
        conversation_id: row
            .try_get("conversation_id")
            .map_err(|e| col("conversation_id", e))?,
        message_id: row
            .try_get("message_id")
            .map_err(|e| col("message_id", e))?,
        claim_text: row
            .try_get("claim_text")
            .map_err(|e| col("claim_text", e))?,
        category,
        status,
        evidence_strength,
        confidence: row
            .try_get("confidence")
            .map_err(|e| col("confidence", e))?,
        layer_fired,
        explanation: row
            .try_get("explanation")
            .map_err(|e| col("explanation", e))?,
        evidence_refs: row
            .try_get("evidence_refs")
            .map_err(|e| col("evidence_refs", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| col("created_at", e))?,
        disposition,
        disposition_reason,
        disposition_note: row
            .try_get("disposition_note")
            .map_err(|e| col("disposition_note", e))?,
        disposed_by: row
            .try_get("disposed_by")
            .map_err(|e| col("disposed_by", e))?,
        disposed_at: row
            .try_get("disposed_at")
            .map_err(|e| col("disposed_at", e))?,
    })
}

/// Emit the whole [`ClaimVerdictRepository`] implementation for one backend.
///
/// `$ty` is the backend type; `$daily_sql` and `$health_daily_sql` are that
/// backend's resolved day-bucketed statements, which its shell builds from
/// [`verdict_daily_counts_sql`] and [`verdict_health_breakdown_sql`] with its
/// own day expression.
///
/// `tenant_id` binds as text on both engines because the column is `TEXT` in
/// both schemas — `TenantId`'s own Postgres encoding is a native `uuid`, which
/// would not match it, so the stringified form is the portable one here.
/// `created_at` and `disposed_at` bind and decode as a `DateTime<Utc>`: sqlx
/// writes RFC3339 text on `SQLite`, byte-identical to the `to_rfc3339()` the
/// rows were written with, and a native timestamp on Postgres.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_claim_verdict_repository {
    ($ty:ty, $daily_sql:ident, $health_daily_sql:ident) => {
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
                    disposition: None,
                    disposition_reason: None,
                    disposition_note: None,
                    disposed_by: None,
                    disposed_at: None,
                })
            }

            async fn get_verdict(
                &self,
                tenant_id: TenantId,
                verdict_id: &str,
            ) -> AppResult<Option<ClaimVerdict>> {
                let row = sqlx::query(GET_VERDICT_SQL)
                    .bind(verdict_id)
                    .bind(tenant_id.to_string())
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get claim verdict: {e}")))?;
                row.as_ref().map(verdict_from_row).transpose()
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

                rows.iter().map(verdict_from_row).collect()
            }

            async fn list_verdicts_for_message(
                &self,
                tenant_id: TenantId,
                message_id: &str,
            ) -> AppResult<Vec<ClaimVerdict>> {
                let rows = sqlx::query(LIST_VERDICTS_FOR_MESSAGE_SQL)
                    .bind(message_id)
                    .bind(tenant_id.to_string())
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list verdicts for message: {e}"))
                    })?;

                rows.iter().map(verdict_from_row).collect()
            }

            async fn list_verdicts_filtered(
                &self,
                tenant_id: TenantId,
                filter: &VerdictListFilter,
            ) -> AppResult<Vec<ClaimVerdict>> {
                let rows = sqlx::query(LIST_VERDICTS_FILTERED_SQL)
                    .bind(tenant_id.to_string())
                    .bind(filter.status.map(ClaimStatus::as_str))
                    .bind(filter.category.map(ClaimCategory::as_str))
                    .bind(filter.agent_id.as_deref())
                    .bind(filter.layer_fired.map(VerdictLayer::as_str))
                    .bind(filter.disposition.map(DispositionFilter::as_str))
                    .bind(filter.user_id.as_deref())
                    .bind(filter.limit.max(1))
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list filtered verdicts: {e}"))
                    })?;

                rows.iter().map(verdict_from_row).collect()
            }

            async fn set_verdict_disposition(
                &self,
                params: &SetVerdictDispositionParams<'_>,
            ) -> AppResult<ClaimVerdict> {
                let result = sqlx::query(SET_VERDICT_DISPOSITION_SQL)
                    .bind(params.verdict_id)
                    .bind(params.tenant_id.to_string())
                    .bind(params.disposition.as_str())
                    .bind(params.reason.map(DispositionReason::as_str))
                    .bind(params.note)
                    .bind(params.disposed_by)
                    .bind(params.disposed_at)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set verdict disposition: {e}"))
                    })?;
                if result.rows_affected() == 0 {
                    return Err(AppError::not_found(format!(
                        "Claim verdict {}",
                        params.verdict_id
                    )));
                }
                self.get_verdict(params.tenant_id, params.verdict_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::not_found(format!("Claim verdict {}", params.verdict_id))
                    })
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

            async fn aggregate_verdict_health(
                &self,
                tenant_id: TenantId,
                window_days: i64,
            ) -> AppResult<VerdictHealthStats> {
                let days = window_days.clamp(1, 365);
                let window_start = Utc::now() - Duration::days(days);
                let tenant = tenant_id.to_string();
                let fetch_failed =
                    |e| AppError::database(format!("Failed to aggregate verdict health: {e}"));

                // One statement per breakdown, all over the same window; the
                // decode is shared so a disposition the application does not
                // know fails every table the same way.
                let layer_rows = health_breakdown_rows(
                    &sqlx::query(HEALTH_BY_LAYER_SQL)
                        .bind(&tenant)
                        .bind(window_start)
                        .fetch_all(&self.pool)
                        .await
                        .map_err(fetch_failed)?,
                )?;
                let category_rows = health_breakdown_rows(
                    &sqlx::query(HEALTH_BY_CATEGORY_SQL)
                        .bind(&tenant)
                        .bind(window_start)
                        .fetch_all(&self.pool)
                        .await
                        .map_err(fetch_failed)?,
                )?;
                let agent_rows = health_breakdown_rows(
                    &sqlx::query(HEALTH_BY_AGENT_SQL)
                        .bind(&tenant)
                        .bind(window_start)
                        .fetch_all(&self.pool)
                        .await
                        .map_err(fetch_failed)?,
                )?;
                let daily_rows = health_breakdown_rows(
                    &sqlx::query($health_daily_sql)
                        .bind(&tenant)
                        .bind(window_start)
                        .fetch_all(&self.pool)
                        .await
                        .map_err(fetch_failed)?,
                )?;
                let by_reason = sqlx::query(HEALTH_BY_REASON_SQL)
                    .bind(&tenant)
                    .bind(window_start)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(fetch_failed)?
                    .iter()
                    .map(reason_count_from_row)
                    .collect::<AppResult<Vec<_>>>()?;

                let totals = health_totals(&layer_rows);
                let false_positive_rate = totals.false_positive_rate();

                Ok(VerdictHealthStats {
                    window_start: window_start.to_rfc3339(),
                    window_days: days,
                    totals,
                    false_positive_rate,
                    by_layer: layer_health(fold_health_breakdown(&layer_rows)),
                    by_category: category_health(fold_health_breakdown(&category_rows)),
                    by_agent: agent_health(fold_health_breakdown(&agent_rows)),
                    by_reason,
                    daily: daily_health(fold_health_breakdown(&daily_rows)),
                })
            }
        }
    };
}
pub(crate) use impl_claim_verdict_repository;
