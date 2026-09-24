// ABOUTME: PlaybookRepository trait — persistence for procedural coaching memory (playbooks + pending advice)
// ABOUTME: Statements, row parsers and the impl body both backends emit; tenant-scoped except where noted.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::cmp::Ordering;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::playbooks::{
    wilson_lower_bound_95, AdviceStatus, ArchetypePrior, Intervention, LabelSource, MetricBaseline,
    OutcomeLabel, OutcomeMetric, PendingAdvice, Playbook, TriggerPattern,
};
use sqlx::Row;
use uuid::Uuid;

/// A freshly-observed outcome to fold into a playbook's counters.
///
/// Bundled into a struct (rather than a wide argument list) to match the
/// repository's param-struct convention and stay within the argument-count
/// lint. The `trigger`/`intervention`/`outcome_metric` describe the playbook so
/// the upsert can create it on first observation; `label` + `at` are the new
/// outcome.
pub struct RecordedOutcome<'a> {
    /// Owning tenant.
    pub tenant_id: &'a str,
    /// User the playbook is personalized for.
    pub user_id: &'a str,
    /// Agent persona slug, or `None` for an agent-agnostic playbook.
    pub agent_slug: Option<&'a str>,
    /// The situation the playbook responds to.
    pub trigger: &'a TriggerPattern,
    /// The action the playbook prescribes.
    pub intervention: &'a Intervention,
    /// What the labeler measured.
    pub outcome_metric: &'a OutcomeMetric,
    /// The verdict to fold in.
    pub label: OutcomeLabel,
    /// When the outcome was observed.
    pub at: DateTime<Utc>,
}

/// Persistence for procedural coaching memory.
///
/// Playbooks and pending advice are **tenant-scoped**: every query carries
/// `tenant_id` in its `WHERE` clause. The one deliberate exception is the
/// archetype-prior aggregate (added with the cold-start phase), which is
/// non-tenant by design (k-anonymous, counts-only). `agent_slug` is stored as
/// `''` for agent-agnostic rows; the repository maps `Option<&str>` <-> `''` at
/// the boundary so the uniqueness key constrains those rows too.
#[async_trait]
pub trait PlaybookRepository: Send + Sync {
    /// Fold an observed outcome into the matching playbook, creating the
    /// playbook on first observation. Atomic: the counter increment is a single
    /// `ON CONFLICT … DO UPDATE SET count = count + …`, so concurrent writers
    /// cannot lose an increment. Returns the playbook id (new or existing) so
    /// the caller can back-link the originating advice.
    async fn record_playbook_outcome(&self, outcome: &RecordedOutcome<'_>) -> AppResult<String>;

    /// Atomically fold an observed outcome into its playbook and mark the
    /// originating advice labeled, in one transaction.
    ///
    /// This is the production reinforcement path: doing the counter upsert and
    /// the advice mark as one unit closes the crash window where two separate
    /// writes would leave the advice `pending` with a past `due_by` and let the
    /// next sweep re-record the same outcome (double-counting into confidence).
    /// The mark is tenant-scoped. Returns the playbook id the outcome was folded
    /// into.
    async fn record_outcome_and_label(
        &self,
        outcome: &RecordedOutcome<'_>,
        advice_id: &str,
        label_source: LabelSource,
    ) -> AppResult<String>;

    /// List a user's playbooks, most-confident first. When `agent_slug` is
    /// `Some`, returns both that agent's playbooks and agent-agnostic (`''`)
    /// ones; when `None`, returns only agent-agnostic playbooks. `confidence`
    /// on each returned [`Playbook`] is the Wilson lower bound computed from the
    /// stored counters. `limit` is clamped by the caller.
    async fn list_playbooks(
        &self,
        tenant_id: &str,
        user_id: &str,
        agent_slug: Option<&str>,
        limit: i64,
    ) -> AppResult<Vec<Playbook>>;

    /// Persist a new in-flight advice record awaiting its outcome.
    async fn insert_pending_advice(&self, advice: &PendingAdvice) -> AppResult<()>;

    /// Fetch pending advice whose observation window has closed
    /// (`status = 'pending' AND due_by <= now_epoch`), oldest first. This is a
    /// **cross-tenant** system scan run by the background outcome evaluator; the
    /// per-row `tenant_id` is carried forward into all subsequent writes.
    async fn due_pending_advice(&self, now_epoch: i64, limit: i64)
        -> AppResult<Vec<PendingAdvice>>;

    /// Mark a pending advice row expired — its window closed without enough data
    /// to label, so it never reinforces a playbook. Tenant-scoped.
    async fn mark_advice_expired(&self, tenant_id: &str, advice_id: &str) -> AppResult<()>;

    /// Scan playbook rows across **all** tenants for the archetype aggregation
    /// job (cold-start). Returns the lean fields the job groups in memory; it
    /// never persists user identity. Bounded by `limit`.
    async fn aggregate_playbook_rows(&self, limit: i64) -> AppResult<Vec<PlaybookAggInput>>;

    /// Replace an archetype prior's aggregate counts (the job recomputes them
    /// wholesale each run, so this overwrites rather than increments).
    async fn upsert_archetype_prior(&self, prior: &ArchetypePriorUpsert<'_>) -> AppResult<()>;

    /// Delete one archetype prior bucket by its `(archetype_key, trigger_hash,
    /// intervention_hash)` conflict key. Used to prune a bucket that has fallen
    /// below the k-anonymity floor so a stale, no-longer-anonymous aggregate
    /// (which may still carry erased users' outcomes) stops being surfaced.
    async fn delete_archetype_prior(
        &self,
        archetype_key: &str,
        trigger_hash: &str,
        intervention_hash: &str,
    ) -> AppResult<()>;

    /// List archetype priors for several archetype buckets in one query,
    /// most-confident first.
    ///
    /// The cold-start fetch batches all of an athlete's sports plus the
    /// sport-agnostic `"any"` bucket. A wide SQL ceiling prefilters by raw
    /// success count, then the results are re-ranked by Wilson confidence and
    /// truncated to `limit` so a high-confidence, lower-count prior is never
    /// dropped by the SQL `LIMIT` before the confidence sort. `confidence` is
    /// computed on read. An empty key slice returns no rows.
    async fn list_archetype_priors_for_keys(
        &self,
        archetype_keys: &[String],
        limit: i64,
    ) -> AppResult<Vec<ArchetypePrior>>;

    /// List ALL of a user's playbooks across every agent scope, most-confident
    /// first — the GDPR "what has the coach learned about me" surface. Tenant +
    /// user scoped.
    async fn list_all_user_playbooks(
        &self,
        tenant_id: &str,
        user_id: &str,
        limit: i64,
    ) -> AppResult<Vec<Playbook>>;

    /// Delete one of a user's playbooks by id — the GDPR "forget this" surface.
    /// Tenant + user scoped so a user can only delete their own. Returns the
    /// number of rows removed (0 = no such playbook for this user).
    async fn delete_playbook(
        &self,
        tenant_id: &str,
        user_id: &str,
        playbook_id: &str,
    ) -> AppResult<u64>;
}

/// Backend-agnostic computed bind values for the `coaching_playbooks` counter
/// upsert. Shared by both backends' `record_playbook_outcome` and the
/// transactional `record_outcome_and_label` so the hashing/serialization and the
/// label → `(success, failure, neutral)` mapping are defined in exactly one place.
pub(crate) struct OutcomeUpsertValues {
    /// Candidate new-row id (used only on first insert; ignored on conflict).
    pub id: String,
    /// `''` for an agent-agnostic playbook.
    pub agent_slug: String,
    /// Trigger conflict hash.
    pub trigger_hash: String,
    /// Intervention conflict hash.
    pub intervention_hash: String,
    /// Serialized `TriggerPattern`.
    pub trigger_json: String,
    /// Serialized `Intervention`.
    pub intervention_json: String,
    /// Serialized `OutcomeMetric`.
    pub outcome_metric_json: String,
    /// Success increment (0 or 1).
    pub sc: i64,
    /// Failure increment (0 or 1).
    pub fc: i64,
    /// Neutral increment (0 or 1).
    pub nc: i64,
    /// Observation epoch seconds (`last_outcome_at`/`created_at`/`updated_at`).
    pub now: i64,
}

/// Compute the upsert bind values for one observed outcome (hashes, serialized
/// JSON columns, the label → counter-increment triple, and a candidate id).
pub(crate) fn outcome_upsert_values(
    outcome: &RecordedOutcome<'_>,
) -> AppResult<OutcomeUpsertValues> {
    let (sc, fc, nc) = match outcome.label {
        OutcomeLabel::Success => (1_i64, 0_i64, 0_i64),
        OutcomeLabel::Failure => (0, 1, 0),
        OutcomeLabel::Neutral => (0, 0, 1),
    };
    Ok(OutcomeUpsertValues {
        id: Uuid::new_v4().to_string(),
        agent_slug: outcome.agent_slug.unwrap_or("").to_owned(),
        trigger_hash: outcome.trigger.hash_key(),
        intervention_hash: outcome.intervention.hash_key(),
        trigger_json: serde_json::to_string(outcome.trigger)
            .map_err(|e| AppError::database(format!("serialize trigger: {e}")))?,
        intervention_json: serde_json::to_string(outcome.intervention)
            .map_err(|e| AppError::database(format!("serialize intervention: {e}")))?,
        outcome_metric_json: serde_json::to_string(outcome.outcome_metric)
            .map_err(|e| AppError::database(format!("serialize outcome_metric: {e}")))?,
        sc,
        fc,
        nc,
        now: outcome.at.timestamp(),
    })
}

/// Lean projection of a playbook row for the archetype aggregation job. Carries
/// `user_id` only transiently (for in-memory distinct-user counting); it is
/// never written to the non-tenant prior store.
#[derive(Debug, Clone)]
pub struct PlaybookAggInput {
    /// User the playbook belongs to (used only to count distinct contributors).
    pub user_id: String,
    /// Trigger conflict hash.
    pub trigger_hash: String,
    /// Intervention conflict hash.
    pub intervention_hash: String,
    /// Serialized `TriggerPattern` (the job derives the archetype sport from it).
    pub trigger_json: String,
    /// Serialized `Intervention`.
    pub intervention_json: String,
    /// Successes for this user's playbook.
    pub success_count: i64,
    /// Failures for this user's playbook.
    pub failure_count: i64,
}

/// Parameters for upserting one recomputed archetype prior. Counts are summed
/// across athletes; `distinct_user_count` is the k-anonymity guard.
pub struct ArchetypePriorUpsert<'a> {
    /// Non-identifying archetype bucket.
    pub archetype_key: &'a str,
    /// Trigger conflict hash.
    pub trigger_hash: &'a str,
    /// Intervention conflict hash.
    pub intervention_hash: &'a str,
    /// Serialized `TriggerPattern`.
    pub trigger_json: &'a str,
    /// Serialized `Intervention`.
    pub intervention_json: &'a str,
    /// Summed successes across contributors.
    pub success_count: i64,
    /// Summed failures across contributors.
    pub failure_count: i64,
    /// Number of distinct contributing athletes (>= K).
    pub distinct_user_count: i64,
}

/// Build an [`ArchetypePrior`] from extracted row primitives, deserializing the
/// JSON columns and computing the Wilson confidence from the counts.
pub(crate) fn archetype_prior_from_row(
    archetype_key: String,
    trigger_json: &str,
    intervention_json: &str,
    success_count: i64,
    failure_count: i64,
    distinct_user_count: i64,
) -> AppResult<ArchetypePrior> {
    let trigger: TriggerPattern = serde_json::from_str(trigger_json)
        .map_err(|e| AppError::database(format!("prior trigger_json: {e}")))?;
    let intervention: Intervention = serde_json::from_str(intervention_json)
        .map_err(|e| AppError::database(format!("prior intervention_json: {e}")))?;
    let success = count_to_u32(success_count);
    let failure = count_to_u32(failure_count);
    Ok(ArchetypePrior {
        archetype_key,
        trigger,
        intervention,
        success_count: success,
        failure_count: failure,
        distinct_user_count: count_to_u32(distinct_user_count),
        confidence: wilson_lower_bound_95(success, failure),
    })
}

/// Convert Unix epoch seconds to a UTC timestamp. `None` only for values outside
/// the representable range (not reachable from our own writes).
fn epoch_to_dt(secs: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(secs, 0)
}

/// Clamp a DB counter (`i64`, always non-negative in our schema) into the `u32`
/// the domain type uses. Saturating rather than `as` so a corrupt huge value can
/// never wrap to a small one.
fn count_to_u32(v: i64) -> u32 {
    u32::try_from(v.max(0)).unwrap_or(u32::MAX)
}

/// Primitive column values for one `coaching_playbooks` row, as both backends
/// extract them (epoch seconds for timestamps). Shared so the `SQLite` and
/// Postgres impls run one parse/build path — and so the Wilson confidence is
/// computed in exactly one place on read.
pub(crate) struct PlaybookRow {
    /// `id` column.
    pub id: String,
    /// `tenant_id` column.
    pub tenant_id: String,
    /// `user_id` column.
    pub user_id: String,
    /// `agent_slug` column (`''` = agent-agnostic).
    pub agent_slug: String,
    /// Serialized `TriggerPattern`.
    pub trigger_json: String,
    /// Serialized `Intervention`.
    pub intervention_json: String,
    /// Serialized `OutcomeMetric`.
    pub outcome_metric_json: String,
    /// `success_count` column.
    pub success_count: i64,
    /// `failure_count` column.
    pub failure_count: i64,
    /// `neutral_count` column.
    pub neutral_count: i64,
    /// `last_outcome_at` epoch seconds, or `None`.
    pub last_outcome_at: Option<i64>,
    /// `created_at` epoch seconds.
    pub created_at: i64,
    /// `updated_at` epoch seconds.
    pub updated_at: i64,
}

/// Build a [`Playbook`] from extracted row primitives, deserializing the JSON
/// columns and computing the Wilson-lower-bound confidence from the counters.
pub(crate) fn playbook_from_row(row: PlaybookRow) -> AppResult<Playbook> {
    let trigger: TriggerPattern = serde_json::from_str(&row.trigger_json)
        .map_err(|e| AppError::database(format!("playbook trigger_json: {e}")))?;
    let intervention: Intervention = serde_json::from_str(&row.intervention_json)
        .map_err(|e| AppError::database(format!("playbook intervention_json: {e}")))?;
    let outcome_metric: OutcomeMetric = serde_json::from_str(&row.outcome_metric_json)
        .map_err(|e| AppError::database(format!("playbook outcome_metric_json: {e}")))?;
    let agent_slug = (!row.agent_slug.is_empty()).then_some(row.agent_slug);
    let mut playbook = Playbook {
        id: row.id,
        tenant_id: row.tenant_id,
        user_id: row.user_id,
        agent_slug,
        trigger,
        intervention,
        outcome_metric,
        success_count: count_to_u32(row.success_count),
        failure_count: count_to_u32(row.failure_count),
        neutral_count: count_to_u32(row.neutral_count),
        confidence: 0.0,
        last_outcome_at: row.last_outcome_at.and_then(epoch_to_dt),
        created_at: epoch_to_dt(row.created_at).unwrap_or_else(Utc::now),
        updated_at: epoch_to_dt(row.updated_at).unwrap_or_else(Utc::now),
    };
    playbook.confidence = playbook.wilson_lower_bound();
    Ok(playbook)
}

/// Primitive column values for one `pending_advice` row.
pub(crate) struct PendingAdviceRow {
    /// `id` column.
    pub id: String,
    /// `tenant_id` column.
    pub tenant_id: String,
    /// `user_id` column.
    pub user_id: String,
    /// `agent_slug` column (`''` = agent-agnostic).
    pub agent_slug: String,
    /// `playbook_id` column, or `None` for a not-yet-instantiated playbook.
    pub playbook_id: Option<String>,
    /// Serialized `TriggerPattern`.
    pub trigger_json: String,
    /// Serialized `Intervention`.
    pub intervention_json: String,
    /// Serialized `OutcomeMetric`.
    pub outcome_metric_json: String,
    /// Serialized `MetricBaseline`.
    pub baseline_json: String,
    /// `due_by` epoch seconds.
    pub due_by: i64,
    /// `status` column.
    pub status: String,
    /// `label` column, or `None` while pending.
    pub label: Option<String>,
    /// `label_source` column, or `None` while pending.
    pub label_source: Option<String>,
    /// `source_msg_id` column.
    pub source_msg_id: Option<String>,
    /// `created_at` epoch seconds.
    pub created_at: i64,
}

/// Build a [`PendingAdvice`] from extracted row primitives.
pub(crate) fn pending_advice_from_row(row: PendingAdviceRow) -> AppResult<PendingAdvice> {
    let trigger: TriggerPattern = serde_json::from_str(&row.trigger_json)
        .map_err(|e| AppError::database(format!("advice trigger_json: {e}")))?;
    let intervention: Intervention = serde_json::from_str(&row.intervention_json)
        .map_err(|e| AppError::database(format!("advice intervention_json: {e}")))?;
    let outcome_metric: OutcomeMetric = serde_json::from_str(&row.outcome_metric_json)
        .map_err(|e| AppError::database(format!("advice outcome_metric_json: {e}")))?;
    let baseline: MetricBaseline = serde_json::from_str(&row.baseline_json)
        .map_err(|e| AppError::database(format!("advice baseline_json: {e}")))?;
    Ok(PendingAdvice {
        id: row.id,
        tenant_id: row.tenant_id,
        user_id: row.user_id,
        agent_slug: (!row.agent_slug.is_empty()).then_some(row.agent_slug),
        playbook_id: row.playbook_id,
        trigger,
        intervention,
        outcome_metric,
        baseline,
        due_by: epoch_to_dt(row.due_by).unwrap_or_else(Utc::now),
        status: AdviceStatus::parse_lenient(&row.status),
        label: row.label.as_deref().map(OutcomeLabel::parse_lenient),
        label_source: row.label_source.as_deref().map(LabelSource::parse_lenient),
        source_msg_id: row.source_msg_id,
        created_at: epoch_to_dt(row.created_at).unwrap_or_else(Utc::now),
    })
}

/// Upper bound on rows pulled for a single user's playbook list before the
/// Rust-side confidence sort. A user accrues at most one playbook per distinct
/// `(trigger, intervention)` pair, so this never truncates a real working set.
pub(crate) const PLAYBOOK_FETCH_CEILING: i64 = 500;

/// Wide SQL prefilter ceiling for archetype priors before the Rust confidence
/// re-sort. Fetching by raw success count then re-ranking by Wilson confidence
/// (and only then truncating to the caller's limit) keeps a high-confidence,
/// lower-count prior from being dropped by the SQL `LIMIT`.
pub(crate) const ARCHETYPE_PRIOR_FETCH_CEILING: i64 = 500;

/// The `coaching_playbooks` counter upsert, returning the surviving row's id
/// (the new id on insert, the existing one on conflict). Shared by
/// `record_playbook_outcome` and `record_outcome_and_label` so the atomic
/// `ON CONFLICT` increment is written once.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, and every bind on these tables is a plain `&str`/`i64`, so one
/// statement serves both backends. The increments name the target table:
/// Postgres rejects the bare column as ambiguous against `excluded`, and
/// `SQLite` accepts the qualified form. `RETURNING` is likewise common to
/// both engines.
pub(crate) const UPSERT_OUTCOME_SQL: &str = r"
    INSERT INTO coaching_playbooks (
        id, tenant_id, user_id, agent_slug, trigger_hash, intervention_hash,
        trigger_json, intervention_json, outcome_metric_json,
        success_count, failure_count, neutral_count, last_outcome_at, created_at, updated_at
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
    ON CONFLICT(tenant_id, user_id, agent_slug, trigger_hash, intervention_hash)
    DO UPDATE SET
        success_count = coaching_playbooks.success_count + excluded.success_count,
        failure_count = coaching_playbooks.failure_count + excluded.failure_count,
        neutral_count = coaching_playbooks.neutral_count + excluded.neutral_count,
        last_outcome_at = excluded.last_outcome_at,
        updated_at = excluded.updated_at
    RETURNING id
";

/// Mark one advice row labeled and back-link it to its playbook.
pub(crate) const MARK_ADVICE_LABELED_SQL: &str = r"
            UPDATE pending_advice
            SET status = 'labeled', label = $1, label_source = $2, playbook_id = $3
            WHERE id = $4 AND tenant_id = $5
            ";

/// The thirteen columns every playbook read returns, in the order
/// [`playbook_row`] reads them.
macro_rules! playbook_columns {
    () => {
        "id, tenant_id, user_id, agent_slug, trigger_json, intervention_json, \
         outcome_metric_json, success_count, failure_count, neutral_count, \
         last_outcome_at, created_at, updated_at"
    };
}

/// A user's playbooks for one agent plus the agent-agnostic ones, most
/// recently updated first, up to the prefilter ceiling.
pub(crate) const LIST_PLAYBOOKS_SQL: &str = concat!(
    "
            SELECT ",
    playbook_columns!(),
    "
            FROM coaching_playbooks
            WHERE tenant_id = $1 AND user_id = $2 AND (agent_slug = $3 OR agent_slug = '')
            ORDER BY updated_at DESC
            LIMIT $4
            "
);

/// Every playbook of a user across agent scopes, most recently updated
/// first, up to the prefilter ceiling.
pub(crate) const LIST_ALL_USER_PLAYBOOKS_SQL: &str = concat!(
    "
            SELECT ",
    playbook_columns!(),
    "
            FROM coaching_playbooks
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY updated_at DESC
            LIMIT $3
            "
);

/// Insert an advice row only when no identical advice is already in flight,
/// so an agent reaffirming the same recommendation across turns cannot
/// enqueue two rows that both later record the same outcome.
pub(crate) const INSERT_PENDING_ADVICE_SQL: &str = r"
            INSERT INTO pending_advice (
                id, tenant_id, user_id, agent_slug, playbook_id, trigger_json,
                intervention_json, outcome_metric_json, baseline_json, due_by,
                status, label, label_source, source_msg_id, created_at
            )
            SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15
            WHERE NOT EXISTS (
                SELECT 1 FROM pending_advice
                WHERE tenant_id = $2 AND user_id = $3 AND agent_slug = $4
                  AND trigger_json = $6 AND intervention_json = $7
                  AND status = 'pending'
            )
            ";

/// Pending advice whose observation window has closed, oldest first: the
/// background evaluator's cross-tenant scan.
pub(crate) const DUE_PENDING_ADVICE_SQL: &str = r"
            SELECT id, tenant_id, user_id, agent_slug, playbook_id, trigger_json,
                   intervention_json, outcome_metric_json, baseline_json, due_by,
                   status, label, label_source, source_msg_id, created_at
            FROM pending_advice
            WHERE status = 'pending' AND due_by <= $1
            ORDER BY due_by ASC
            LIMIT $2
            ";

/// Expire one advice row.
pub(crate) const MARK_ADVICE_EXPIRED_SQL: &str =
    "UPDATE pending_advice SET status = 'expired' WHERE id = $1 AND tenant_id = $2";

/// The lean projection the archetype aggregation job groups in memory,
/// across every tenant.
pub(crate) const AGGREGATE_PLAYBOOK_ROWS_SQL: &str = r"
            SELECT user_id, trigger_hash, intervention_hash, trigger_json,
                   intervention_json, success_count, failure_count
            FROM coaching_playbooks
            ORDER BY id
            LIMIT $1
            ";

/// Replace an archetype prior's aggregate counts wholesale.
pub(crate) const UPSERT_ARCHETYPE_PRIOR_SQL: &str = r"
            INSERT INTO archetype_priors (
                archetype_key, trigger_hash, intervention_hash, trigger_json,
                intervention_json, success_count, failure_count, distinct_user_count, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT(archetype_key, trigger_hash, intervention_hash)
            DO UPDATE SET
                success_count = excluded.success_count,
                failure_count = excluded.failure_count,
                distinct_user_count = excluded.distinct_user_count,
                trigger_json = excluded.trigger_json,
                intervention_json = excluded.intervention_json,
                updated_at = excluded.updated_at
            ";

/// Drop one archetype prior bucket.
pub(crate) const DELETE_ARCHETYPE_PRIOR_SQL: &str = r"DELETE FROM archetype_priors
              WHERE archetype_key = $1 AND trigger_hash = $2 AND intervention_hash = $3";

/// The archetype-prior read for a set of buckets, with `IN (...)` holding one
/// `$n` placeholder per key and the prefilter ceiling as the final one. Only
/// the placeholder list is built dynamically; every value stays bound.
pub(crate) fn list_archetype_priors_sql(key_count: usize) -> String {
    let placeholders = (1..=key_count)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let ceiling_ph = key_count + 1;
    format!(
        r"
            SELECT archetype_key, trigger_json, intervention_json,
                   success_count, failure_count, distinct_user_count
            FROM archetype_priors
            WHERE archetype_key IN ({placeholders})
            ORDER BY success_count DESC
            LIMIT ${ceiling_ph}
            "
    )
}

/// Purge the pending advice that would re-materialise a playbook being
/// forgotten: advice carries its own agent slug, trigger and intervention and
/// is decoupled from `playbook_id`, so on maturation its upsert would
/// re-insert the playbook.
pub(crate) const PURGE_PLAYBOOK_ADVICE_SQL: &str = r"
            DELETE FROM pending_advice
            WHERE tenant_id = $1 AND user_id = $2
              AND (agent_slug, trigger_json, intervention_json) IN (
                  SELECT agent_slug, trigger_json, intervention_json
                  FROM coaching_playbooks
                  WHERE tenant_id = $1 AND user_id = $2 AND id = $3
              )
            ";

/// Delete one of a user's playbooks.
pub(crate) const DELETE_PLAYBOOK_SQL: &str =
    "DELETE FROM coaching_playbooks WHERE tenant_id = $1 AND user_id = $2 AND id = $3";

/// Read one column by name via `try_get` only, so a type or NULL surprise
/// surfaces as a recoverable error naming the column rather than a panic.
/// That matters on `SQLite`, whose type affinity can let a non-integer value
/// land in an `INTEGER` column.
fn column<'r, R, T>(row: &'r R, what: &str, name: &str) -> AppResult<T>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    T: sqlx::Type<R::Database> + sqlx::Decode<'r, R::Database>,
{
    row.try_get(name)
        .map_err(|e| AppError::database(format!("{what} col {name}: {e}")))
}

/// Extract a [`PlaybookRow`] from a row of either backend.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn playbook_row<R>(r: &R) -> AppResult<PlaybookRow>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let what = "playbook";
    Ok(PlaybookRow {
        id: column(r, what, "id")?,
        tenant_id: column(r, what, "tenant_id")?,
        user_id: column(r, what, "user_id")?,
        agent_slug: column(r, what, "agent_slug")?,
        trigger_json: column(r, what, "trigger_json")?,
        intervention_json: column(r, what, "intervention_json")?,
        outcome_metric_json: column(r, what, "outcome_metric_json")?,
        success_count: column(r, what, "success_count")?,
        failure_count: column(r, what, "failure_count")?,
        neutral_count: column(r, what, "neutral_count")?,
        last_outcome_at: column(r, what, "last_outcome_at")?,
        created_at: column(r, what, "created_at")?,
        updated_at: column(r, what, "updated_at")?,
    })
}

/// Extract a [`PendingAdviceRow`] from a row of either backend.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn pending_row<R>(r: &R) -> AppResult<PendingAdviceRow>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let what = "advice";
    Ok(PendingAdviceRow {
        id: column(r, what, "id")?,
        tenant_id: column(r, what, "tenant_id")?,
        user_id: column(r, what, "user_id")?,
        agent_slug: column(r, what, "agent_slug")?,
        playbook_id: column(r, what, "playbook_id")?,
        trigger_json: column(r, what, "trigger_json")?,
        intervention_json: column(r, what, "intervention_json")?,
        outcome_metric_json: column(r, what, "outcome_metric_json")?,
        baseline_json: column(r, what, "baseline_json")?,
        due_by: column(r, what, "due_by")?,
        status: column(r, what, "status")?,
        label: column(r, what, "label")?,
        label_source: column(r, what, "label_source")?,
        source_msg_id: column(r, what, "source_msg_id")?,
        created_at: column(r, what, "created_at")?,
    })
}

/// Extract a [`PlaybookAggInput`] from a row of either backend.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn agg_row<R>(r: &R) -> AppResult<PlaybookAggInput>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let what = "agg";
    Ok(PlaybookAggInput {
        user_id: column(r, what, "user_id")?,
        trigger_hash: column(r, what, "trigger_hash")?,
        intervention_hash: column(r, what, "intervention_hash")?,
        trigger_json: column(r, what, "trigger_json")?,
        intervention_json: column(r, what, "intervention_json")?,
        success_count: column(r, what, "success_count")?,
        failure_count: column(r, what, "failure_count")?,
    })
}

/// Extract an [`ArchetypePrior`] from a row of either backend.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// or one for a JSON column that no longer parses.
pub(crate) fn prior_from_row<R>(r: &R) -> AppResult<ArchetypePrior>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let what = "prior";
    let archetype_key: String = column(r, what, "archetype_key")?;
    let trigger_json: String = column(r, what, "trigger_json")?;
    let intervention_json: String = column(r, what, "intervention_json")?;
    let success_count: i64 = column(r, what, "success_count")?;
    let failure_count: i64 = column(r, what, "failure_count")?;
    let distinct_user_count: i64 = column(r, what, "distinct_user_count")?;
    archetype_prior_from_row(
        archetype_key,
        &trigger_json,
        &intervention_json,
        success_count,
        failure_count,
        distinct_user_count,
    )
}

/// Order most-confident first and keep the caller's `limit`.
pub(crate) fn rank_by_confidence<T>(
    items: &mut Vec<T>,
    confidence: impl Fn(&T) -> f32,
    limit: i64,
) {
    items.sort_by(|a, b| {
        confidence(b)
            .partial_cmp(&confidence(a))
            .unwrap_or(Ordering::Equal)
    });
    items.truncate(usize::try_from(limit.max(0)).unwrap_or(0));
}

/// Emit the whole [`PlaybookRepository`] implementation for one backend type.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_playbook_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl PlaybookRepository for $ty {
            async fn record_playbook_outcome(
                &self,
                outcome: &RecordedOutcome<'_>,
            ) -> AppResult<String> {
                let v = outcome_upsert_values(outcome)?;
                let row = sqlx::query(UPSERT_OUTCOME_SQL)
                    .bind(&v.id)
                    .bind(outcome.tenant_id)
                    .bind(outcome.user_id)
                    .bind(&v.agent_slug)
                    .bind(&v.trigger_hash)
                    .bind(&v.intervention_hash)
                    .bind(&v.trigger_json)
                    .bind(&v.intervention_json)
                    .bind(&v.outcome_metric_json)
                    .bind(v.sc)
                    .bind(v.fc)
                    .bind(v.nc)
                    .bind(v.now)
                    .bind(v.now)
                    .bind(v.now)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("upsert playbook outcome: {e}")))?;
                row.try_get::<String, _>("id")
                    .map_err(|e| AppError::database(format!("resolve playbook id: {e}")))
            }

            async fn record_outcome_and_label(
                &self,
                outcome: &RecordedOutcome<'_>,
                advice_id: &str,
                label_source: LabelSource,
            ) -> AppResult<String> {
                let v = outcome_upsert_values(outcome)?;
                // One transaction spans the counter upsert AND the advice mark so a
                // crash between them cannot leave the advice pending (which the next
                // sweep would re-record, double-counting into confidence).
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| AppError::database(format!("begin outcome tx: {e}")))?;
                let row = sqlx::query(UPSERT_OUTCOME_SQL)
                    .bind(&v.id)
                    .bind(outcome.tenant_id)
                    .bind(outcome.user_id)
                    .bind(&v.agent_slug)
                    .bind(&v.trigger_hash)
                    .bind(&v.intervention_hash)
                    .bind(&v.trigger_json)
                    .bind(&v.intervention_json)
                    .bind(&v.outcome_metric_json)
                    .bind(v.sc)
                    .bind(v.fc)
                    .bind(v.nc)
                    .bind(v.now)
                    .bind(v.now)
                    .bind(v.now)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("upsert playbook outcome: {e}")))?;
                let playbook_id: String = row
                    .try_get("id")
                    .map_err(|e| AppError::database(format!("resolve playbook id: {e}")))?;
                sqlx::query(MARK_ADVICE_LABELED_SQL)
                    .bind(outcome.label.as_str())
                    .bind(label_source.as_str())
                    .bind(&playbook_id)
                    .bind(advice_id)
                    .bind(outcome.tenant_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("mark advice labeled: {e}")))?;
                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("commit outcome tx: {e}")))?;
                Ok(playbook_id)
            }

            async fn list_playbooks(
                &self,
                tenant_id: &str,
                user_id: &str,
                agent_slug: Option<&str>,
                limit: i64,
            ) -> AppResult<Vec<Playbook>> {
                let agent = agent_slug.unwrap_or("");
                let rows = sqlx::query(LIST_PLAYBOOKS_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(agent)
                    .bind(PLAYBOOK_FETCH_CEILING)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("list playbooks: {e}")))?;

                let mut playbooks: Vec<Playbook> = rows
                    .iter()
                    .filter_map(|r| {
                        playbook_row(r)
                            .and_then(playbook_from_row)
                            .map_err(|e| warn!(error = %e, "skipping corrupt playbook row"))
                            .ok()
                    })
                    .collect();
                rank_by_confidence(&mut playbooks, |p| p.confidence, limit);
                Ok(playbooks)
            }

            async fn insert_pending_advice(&self, advice: &PendingAdvice) -> AppResult<()> {
                let agent_slug = advice.agent_slug.as_deref().unwrap_or("");
                let trigger_json = serde_json::to_string(&advice.trigger)
                    .map_err(|e| AppError::database(format!("serialize trigger: {e}")))?;
                let intervention_json = serde_json::to_string(&advice.intervention)
                    .map_err(|e| AppError::database(format!("serialize intervention: {e}")))?;
                let outcome_metric_json = serde_json::to_string(&advice.outcome_metric)
                    .map_err(|e| AppError::database(format!("serialize outcome_metric: {e}")))?;
                let baseline_json = serde_json::to_string(&advice.baseline)
                    .map_err(|e| AppError::database(format!("serialize baseline: {e}")))?;
                sqlx::query(INSERT_PENDING_ADVICE_SQL)
                    .bind(&advice.id)
                    .bind(&advice.tenant_id)
                    .bind(&advice.user_id)
                    .bind(agent_slug)
                    .bind(advice.playbook_id.as_deref())
                    .bind(&trigger_json)
                    .bind(&intervention_json)
                    .bind(&outcome_metric_json)
                    .bind(&baseline_json)
                    .bind(advice.due_by.timestamp())
                    .bind(advice.status.as_str())
                    .bind(advice.label.map(OutcomeLabel::as_str))
                    .bind(advice.label_source.map(LabelSource::as_str))
                    .bind(advice.source_msg_id.as_deref())
                    .bind(advice.created_at.timestamp())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("insert pending advice: {e}")))?;
                Ok(())
            }

            async fn due_pending_advice(
                &self,
                now_epoch: i64,
                limit: i64,
            ) -> AppResult<Vec<PendingAdvice>> {
                let rows = sqlx::query(DUE_PENDING_ADVICE_SQL)
                    .bind(now_epoch)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("scan due advice: {e}")))?;

                Ok(rows
                    .iter()
                    .filter_map(|r| {
                        pending_row(r)
                            .and_then(pending_advice_from_row)
                            .map_err(|e| warn!(error = %e, "skipping corrupt pending-advice row"))
                            .ok()
                    })
                    .collect())
            }

            async fn mark_advice_expired(&self, tenant_id: &str, advice_id: &str) -> AppResult<()> {
                sqlx::query(MARK_ADVICE_EXPIRED_SQL)
                    .bind(advice_id)
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("mark advice expired: {e}")))?;
                Ok(())
            }

            async fn aggregate_playbook_rows(&self, limit: i64) -> AppResult<Vec<PlaybookAggInput>> {
                let rows = sqlx::query(AGGREGATE_PLAYBOOK_ROWS_SQL)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("aggregate playbook scan: {e}")))?;
                Ok(rows
                    .iter()
                    .filter_map(|r| {
                        agg_row(r)
                            .map_err(|e| warn!(error = %e, "skipping corrupt aggregation row"))
                            .ok()
                    })
                    .collect())
            }

            async fn upsert_archetype_prior(
                &self,
                prior: &ArchetypePriorUpsert<'_>,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_ARCHETYPE_PRIOR_SQL)
                    .bind(prior.archetype_key)
                    .bind(prior.trigger_hash)
                    .bind(prior.intervention_hash)
                    .bind(prior.trigger_json)
                    .bind(prior.intervention_json)
                    .bind(prior.success_count)
                    .bind(prior.failure_count)
                    .bind(prior.distinct_user_count)
                    .bind(Utc::now().timestamp())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("upsert archetype prior: {e}")))?;
                Ok(())
            }

            async fn delete_archetype_prior(
                &self,
                archetype_key: &str,
                trigger_hash: &str,
                intervention_hash: &str,
            ) -> AppResult<()> {
                sqlx::query(DELETE_ARCHETYPE_PRIOR_SQL)
                    .bind(archetype_key)
                    .bind(trigger_hash)
                    .bind(intervention_hash)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("delete archetype prior: {e}")))?;
                Ok(())
            }

            async fn list_archetype_priors_for_keys(
                &self,
                archetype_keys: &[String],
                limit: i64,
            ) -> AppResult<Vec<ArchetypePrior>> {
                if archetype_keys.is_empty() {
                    return Ok(Vec::new());
                }
                let sql = list_archetype_priors_sql(archetype_keys.len());
                let mut query = sqlx::query(&sql);
                for key in archetype_keys {
                    query = query.bind(key);
                }
                let rows = query
                    .bind(ARCHETYPE_PRIOR_FETCH_CEILING)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("list archetype priors: {e}")))?;
                let mut priors: Vec<ArchetypePrior> = rows
                    .iter()
                    .filter_map(|r| {
                        prior_from_row(r)
                            .map_err(|e| warn!(error = %e, "skipping corrupt archetype prior"))
                            .ok()
                    })
                    .collect();
                rank_by_confidence(&mut priors, |p| p.confidence, limit);
                Ok(priors)
            }

            async fn list_all_user_playbooks(
                &self,
                tenant_id: &str,
                user_id: &str,
                limit: i64,
            ) -> AppResult<Vec<Playbook>> {
                let rows = sqlx::query(LIST_ALL_USER_PLAYBOOKS_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(PLAYBOOK_FETCH_CEILING)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("list all user playbooks: {e}")))?;
                let mut playbooks: Vec<Playbook> = rows
                    .iter()
                    .filter_map(|r| {
                        playbook_row(r)
                            .and_then(playbook_from_row)
                            .map_err(|e| warn!(error = %e, "skipping corrupt playbook row"))
                            .ok()
                    })
                    .collect();
                rank_by_confidence(&mut playbooks, |p| p.confidence, limit);
                Ok(playbooks)
            }

            async fn delete_playbook(
                &self,
                tenant_id: &str,
                user_id: &str,
                playbook_id: &str,
            ) -> AppResult<u64> {
                // Both deletes run in one transaction so erasure is durable
                // (GDPR forget).
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| AppError::database(format!("begin forget tx: {e}")))?;
                sqlx::query(PURGE_PLAYBOOK_ADVICE_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(playbook_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("purge pending advice: {e}")))?;
                let res = sqlx::query(DELETE_PLAYBOOK_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(playbook_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::database(format!("delete playbook: {e}")))?;
                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("commit forget tx: {e}")))?;
                Ok(res.rows_affected())
            }
        }
    };
}
pub(crate) use impl_playbook_repository;
