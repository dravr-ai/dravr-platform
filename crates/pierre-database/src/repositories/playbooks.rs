// ABOUTME: PlaybookRepository trait — persistence for procedural coaching memory (playbooks + pending advice)
// ABOUTME: Dual SQLite/Postgres impls live in database/ and backends/postgres/. Tenant-scoped except where noted.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::playbooks::{
    wilson_lower_bound_95, AdviceStatus, ArchetypePrior, Intervention, LabelSource, MetricBaseline,
    OutcomeLabel, OutcomeMetric, PendingAdvice, Playbook, TriggerPattern,
};
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
    /// Coach persona slug, or `None` for a coach-agnostic playbook.
    pub coach_slug: Option<&'a str>,
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
/// non-tenant by design (k-anonymous, counts-only). `coach_slug` is stored as
/// `''` for coach-agnostic rows; the repository maps `Option<&str>` <-> `''` at
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

    /// List a user's playbooks, most-confident first. When `coach_slug` is
    /// `Some`, returns both that coach's playbooks and coach-agnostic (`''`)
    /// ones; when `None`, returns only coach-agnostic playbooks. `confidence`
    /// on each returned [`Playbook`] is the Wilson lower bound computed from the
    /// stored counters. `limit` is clamped by the caller.
    async fn list_playbooks(
        &self,
        tenant_id: &str,
        user_id: &str,
        coach_slug: Option<&str>,
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

    /// List ALL of a user's playbooks across every coach scope, most-confident
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
    /// `''` for a coach-agnostic playbook.
    pub coach_slug: String,
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
        coach_slug: outcome.coach_slug.unwrap_or("").to_owned(),
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
    /// `coach_slug` column (`''` = coach-agnostic).
    pub coach_slug: String,
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
    let coach_slug = (!row.coach_slug.is_empty()).then_some(row.coach_slug);
    let mut playbook = Playbook {
        id: row.id,
        tenant_id: row.tenant_id,
        user_id: row.user_id,
        coach_slug,
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
    /// `coach_slug` column (`''` = coach-agnostic).
    pub coach_slug: String,
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
        coach_slug: (!row.coach_slug.is_empty()).then_some(row.coach_slug),
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
