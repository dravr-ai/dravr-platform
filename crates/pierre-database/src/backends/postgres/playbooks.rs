// ABOUTME: PostgreSQL-backed PlaybookRepository for procedural coaching memory
// ABOUTME: Mirrors the SQLite impl with $n binds + try_get (never get — Row::get panics on PG)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::cmp::Ordering;

use async_trait::async_trait;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::playbooks::{
    ArchetypePrior, LabelSource, OutcomeLabel, PendingAdvice, Playbook,
};
use sqlx::postgres::PgRow;
use sqlx::Row;
use tracing::warn;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::playbooks::{
    archetype_prior_from_row, outcome_upsert_values, pending_advice_from_row, playbook_from_row,
    ArchetypePriorUpsert, PendingAdviceRow, PlaybookAggInput, PlaybookRepository, PlaybookRow,
    RecordedOutcome,
};

/// Upper bound on rows pulled for a single user before the Rust-side confidence
/// sort. Mirrors the `SQLite` impl.
const PLAYBOOK_FETCH_CEILING: i64 = 500;

/// Wide SQL prefilter ceiling for archetype priors before the Rust confidence
/// re-sort. Mirrors the `SQLite` impl.
const ARCHETYPE_PRIOR_FETCH_CEILING: i64 = 500;

/// The `coaching_playbooks` counter upsert (Postgres form with `RETURNING id`).
/// Shared by `record_playbook_outcome` and `record_outcome_and_label`.
const UPSERT_OUTCOME_SQL: &str = r"
    INSERT INTO coaching_playbooks (
        id, tenant_id, user_id, coach_slug, trigger_hash, intervention_hash,
        trigger_json, intervention_json, outcome_metric_json,
        success_count, failure_count, neutral_count, last_outcome_at, created_at, updated_at
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
    ON CONFLICT(tenant_id, user_id, coach_slug, trigger_hash, intervention_hash)
    DO UPDATE SET
        success_count = coaching_playbooks.success_count + excluded.success_count,
        failure_count = coaching_playbooks.failure_count + excluded.failure_count,
        neutral_count = coaching_playbooks.neutral_count + excluded.neutral_count,
        last_outcome_at = excluded.last_outcome_at,
        updated_at = excluded.updated_at
    RETURNING id
";

/// Extract a [`PlaybookRow`] from a Postgres row via `try_get` only — a wrong
/// type or unexpected NULL becomes a recoverable error, never a panic.
fn pg_playbook_row(r: &PgRow) -> AppResult<PlaybookRow> {
    let col = |name: &str, e: sqlx::Error| AppError::database(format!("playbook col {name}: {e}"));
    Ok(PlaybookRow {
        id: r.try_get("id").map_err(|e| col("id", e))?,
        tenant_id: r.try_get("tenant_id").map_err(|e| col("tenant_id", e))?,
        user_id: r.try_get("user_id").map_err(|e| col("user_id", e))?,
        coach_slug: r.try_get("coach_slug").map_err(|e| col("coach_slug", e))?,
        trigger_json: r
            .try_get("trigger_json")
            .map_err(|e| col("trigger_json", e))?,
        intervention_json: r
            .try_get("intervention_json")
            .map_err(|e| col("intervention_json", e))?,
        outcome_metric_json: r
            .try_get("outcome_metric_json")
            .map_err(|e| col("outcome_metric_json", e))?,
        success_count: r
            .try_get("success_count")
            .map_err(|e| col("success_count", e))?,
        failure_count: r
            .try_get("failure_count")
            .map_err(|e| col("failure_count", e))?,
        neutral_count: r
            .try_get("neutral_count")
            .map_err(|e| col("neutral_count", e))?,
        last_outcome_at: r
            .try_get("last_outcome_at")
            .map_err(|e| col("last_outcome_at", e))?,
        created_at: r.try_get("created_at").map_err(|e| col("created_at", e))?,
        updated_at: r.try_get("updated_at").map_err(|e| col("updated_at", e))?,
    })
}

/// Extract a [`PendingAdviceRow`] from a Postgres row via `try_get` only.
fn pg_pending_row(r: &PgRow) -> AppResult<PendingAdviceRow> {
    let col = |name: &str, e: sqlx::Error| AppError::database(format!("advice col {name}: {e}"));
    Ok(PendingAdviceRow {
        id: r.try_get("id").map_err(|e| col("id", e))?,
        tenant_id: r.try_get("tenant_id").map_err(|e| col("tenant_id", e))?,
        user_id: r.try_get("user_id").map_err(|e| col("user_id", e))?,
        coach_slug: r.try_get("coach_slug").map_err(|e| col("coach_slug", e))?,
        playbook_id: r
            .try_get("playbook_id")
            .map_err(|e| col("playbook_id", e))?,
        trigger_json: r
            .try_get("trigger_json")
            .map_err(|e| col("trigger_json", e))?,
        intervention_json: r
            .try_get("intervention_json")
            .map_err(|e| col("intervention_json", e))?,
        outcome_metric_json: r
            .try_get("outcome_metric_json")
            .map_err(|e| col("outcome_metric_json", e))?,
        baseline_json: r
            .try_get("baseline_json")
            .map_err(|e| col("baseline_json", e))?,
        due_by: r.try_get("due_by").map_err(|e| col("due_by", e))?,
        status: r.try_get("status").map_err(|e| col("status", e))?,
        label: r.try_get("label").map_err(|e| col("label", e))?,
        label_source: r
            .try_get("label_source")
            .map_err(|e| col("label_source", e))?,
        source_msg_id: r
            .try_get("source_msg_id")
            .map_err(|e| col("source_msg_id", e))?,
        created_at: r.try_get("created_at").map_err(|e| col("created_at", e))?,
    })
}

/// Extract a [`PlaybookAggInput`] from a Postgres row via `try_get`.
fn pg_agg_row(r: &PgRow) -> AppResult<PlaybookAggInput> {
    let col = |name: &str, e: sqlx::Error| AppError::database(format!("agg col {name}: {e}"));
    Ok(PlaybookAggInput {
        user_id: r.try_get("user_id").map_err(|e| col("user_id", e))?,
        trigger_hash: r
            .try_get("trigger_hash")
            .map_err(|e| col("trigger_hash", e))?,
        intervention_hash: r
            .try_get("intervention_hash")
            .map_err(|e| col("intervention_hash", e))?,
        trigger_json: r
            .try_get("trigger_json")
            .map_err(|e| col("trigger_json", e))?,
        intervention_json: r
            .try_get("intervention_json")
            .map_err(|e| col("intervention_json", e))?,
        success_count: r
            .try_get("success_count")
            .map_err(|e| col("success_count", e))?,
        failure_count: r
            .try_get("failure_count")
            .map_err(|e| col("failure_count", e))?,
    })
}

/// Extract an [`ArchetypePrior`] from a Postgres row via `try_get`.
fn pg_prior_from_row(r: &PgRow) -> AppResult<ArchetypePrior> {
    let col = |name: &str, e: sqlx::Error| AppError::database(format!("prior col {name}: {e}"));
    let archetype_key: String = r
        .try_get("archetype_key")
        .map_err(|e| col("archetype_key", e))?;
    let trigger_json: String = r
        .try_get("trigger_json")
        .map_err(|e| col("trigger_json", e))?;
    let intervention_json: String = r
        .try_get("intervention_json")
        .map_err(|e| col("intervention_json", e))?;
    let success_count: i64 = r
        .try_get("success_count")
        .map_err(|e| col("success_count", e))?;
    let failure_count: i64 = r
        .try_get("failure_count")
        .map_err(|e| col("failure_count", e))?;
    let distinct_user_count: i64 = r
        .try_get("distinct_user_count")
        .map_err(|e| col("distinct_user_count", e))?;
    archetype_prior_from_row(
        archetype_key,
        &trigger_json,
        &intervention_json,
        success_count,
        failure_count,
        distinct_user_count,
    )
}

#[async_trait]
impl PlaybookRepository for PostgresDatabase {
    async fn record_playbook_outcome(&self, outcome: &RecordedOutcome<'_>) -> AppResult<String> {
        let v = outcome_upsert_values(outcome)?;
        let row = sqlx::query(UPSERT_OUTCOME_SQL)
            .bind(&v.id)
            .bind(outcome.tenant_id)
            .bind(outcome.user_id)
            .bind(&v.coach_slug)
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
            .bind(&v.coach_slug)
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
        sqlx::query(
            r"
            UPDATE pending_advice
            SET status = 'labeled', label = $1, label_source = $2, playbook_id = $3
            WHERE id = $4 AND tenant_id = $5
            ",
        )
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
        coach_slug: Option<&str>,
        limit: i64,
    ) -> AppResult<Vec<Playbook>> {
        let coach = coach_slug.unwrap_or("");
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, coach_slug, trigger_json, intervention_json,
                   outcome_metric_json, success_count, failure_count, neutral_count,
                   last_outcome_at, created_at, updated_at
            FROM coaching_playbooks
            WHERE tenant_id = $1 AND user_id = $2 AND (coach_slug = $3 OR coach_slug = '')
            ORDER BY updated_at DESC
            LIMIT $4
            ",
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(coach)
        .bind(PLAYBOOK_FETCH_CEILING)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("list playbooks: {e}")))?;

        let mut playbooks: Vec<Playbook> = rows
            .iter()
            .filter_map(|r| {
                pg_playbook_row(r)
                    .and_then(playbook_from_row)
                    .map_err(|e| warn!(error = %e, "skipping corrupt playbook row"))
                    .ok()
            })
            .collect();
        playbooks.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(Ordering::Equal)
        });
        playbooks.truncate(usize::try_from(limit.max(0)).unwrap_or(0));
        Ok(playbooks)
    }

    async fn insert_pending_advice(&self, advice: &PendingAdvice) -> AppResult<()> {
        let coach_slug = advice.coach_slug.as_deref().unwrap_or("");
        let trigger_json = serde_json::to_string(&advice.trigger)
            .map_err(|e| AppError::database(format!("serialize trigger: {e}")))?;
        let intervention_json = serde_json::to_string(&advice.intervention)
            .map_err(|e| AppError::database(format!("serialize intervention: {e}")))?;
        let outcome_metric_json = serde_json::to_string(&advice.outcome_metric)
            .map_err(|e| AppError::database(format!("serialize outcome_metric: {e}")))?;
        let baseline_json = serde_json::to_string(&advice.baseline)
            .map_err(|e| AppError::database(format!("serialize baseline: {e}")))?;
        // Insert only when no identical advice is already in flight (dedup), so a
        // coach reaffirming the same recommendation across turns cannot enqueue
        // two rows that both later record the same outcome (double-counting).
        sqlx::query(
            r"
            INSERT INTO pending_advice (
                id, tenant_id, user_id, coach_slug, playbook_id, trigger_json,
                intervention_json, outcome_metric_json, baseline_json, due_by,
                status, label, label_source, source_msg_id, created_at
            )
            SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15
            WHERE NOT EXISTS (
                SELECT 1 FROM pending_advice
                WHERE tenant_id = $2 AND user_id = $3 AND coach_slug = $4
                  AND trigger_json = $6 AND intervention_json = $7
                  AND status = 'pending'
            )
            ",
        )
        .bind(&advice.id)
        .bind(&advice.tenant_id)
        .bind(&advice.user_id)
        .bind(coach_slug)
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
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, coach_slug, playbook_id, trigger_json,
                   intervention_json, outcome_metric_json, baseline_json, due_by,
                   status, label, label_source, source_msg_id, created_at
            FROM pending_advice
            WHERE status = 'pending' AND due_by <= $1
            ORDER BY due_by ASC
            LIMIT $2
            ",
        )
        .bind(now_epoch)
        .bind(limit)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("scan due advice: {e}")))?;

        Ok(rows
            .iter()
            .filter_map(|r| {
                pg_pending_row(r)
                    .and_then(pending_advice_from_row)
                    .map_err(|e| warn!(error = %e, "skipping corrupt pending-advice row"))
                    .ok()
            })
            .collect())
    }

    async fn mark_advice_expired(&self, tenant_id: &str, advice_id: &str) -> AppResult<()> {
        sqlx::query(
            r"UPDATE pending_advice SET status = 'expired' WHERE id = $1 AND tenant_id = $2",
        )
        .bind(advice_id)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("mark advice expired: {e}")))?;
        Ok(())
    }

    async fn aggregate_playbook_rows(&self, limit: i64) -> AppResult<Vec<PlaybookAggInput>> {
        let rows = sqlx::query(
            r"
            SELECT user_id, trigger_hash, intervention_hash, trigger_json,
                   intervention_json, success_count, failure_count
            FROM coaching_playbooks
            ORDER BY id
            LIMIT $1
            ",
        )
        .bind(limit)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("aggregate playbook scan: {e}")))?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                pg_agg_row(r)
                    .map_err(|e| warn!(error = %e, "skipping corrupt aggregation row"))
                    .ok()
            })
            .collect())
    }

    async fn upsert_archetype_prior(&self, prior: &ArchetypePriorUpsert<'_>) -> AppResult<()> {
        sqlx::query(
            r"
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
            ",
        )
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
        sqlx::query(
            r"DELETE FROM archetype_priors
              WHERE archetype_key = $1 AND trigger_hash = $2 AND intervention_hash = $3",
        )
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
        // Bind one placeholder per key for the IN clause, then the ceiling as the
        // final placeholder. Only the placeholder list is built dynamically;
        // values stay parameterized (same idiom as users/recipes).
        let placeholders = (1..=archetype_keys.len())
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let ceiling_ph = archetype_keys.len() + 1;
        let sql = format!(
            r"
            SELECT archetype_key, trigger_json, intervention_json,
                   success_count, failure_count, distinct_user_count
            FROM archetype_priors
            WHERE archetype_key IN ({placeholders})
            ORDER BY success_count DESC
            LIMIT ${ceiling_ph}
            "
        );
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
                pg_prior_from_row(r)
                    .map_err(|e| warn!(error = %e, "skipping corrupt archetype prior"))
                    .ok()
            })
            .collect();
        priors.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(Ordering::Equal)
        });
        priors.truncate(usize::try_from(limit.max(0)).unwrap_or(0));
        Ok(priors)
    }

    async fn list_all_user_playbooks(
        &self,
        tenant_id: &str,
        user_id: &str,
        limit: i64,
    ) -> AppResult<Vec<Playbook>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, coach_slug, trigger_json, intervention_json,
                   outcome_metric_json, success_count, failure_count, neutral_count,
                   last_outcome_at, created_at, updated_at
            FROM coaching_playbooks
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY updated_at DESC
            LIMIT $3
            ",
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(PLAYBOOK_FETCH_CEILING)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("list all user playbooks: {e}")))?;
        let mut playbooks: Vec<Playbook> = rows
            .iter()
            .filter_map(|r| {
                pg_playbook_row(r)
                    .and_then(playbook_from_row)
                    .map_err(|e| warn!(error = %e, "skipping corrupt playbook row"))
                    .ok()
            })
            .collect();
        playbooks.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(Ordering::Equal)
        });
        playbooks.truncate(usize::try_from(limit.max(0)).unwrap_or(0));
        Ok(playbooks)
    }

    async fn delete_playbook(
        &self,
        tenant_id: &str,
        user_id: &str,
        playbook_id: &str,
    ) -> AppResult<u64> {
        // Purge the deleted playbook AND any still-pending advice that would
        // re-materialize it (advice is decoupled from playbook_id and re-derives
        // the conflict key on maturation). One transaction so erasure is durable.
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| AppError::database(format!("begin forget tx: {e}")))?;
        sqlx::query(
            r"
            DELETE FROM pending_advice
            WHERE tenant_id = $1 AND user_id = $2
              AND (coach_slug, trigger_json, intervention_json) IN (
                  SELECT coach_slug, trigger_json, intervention_json
                  FROM coaching_playbooks
                  WHERE tenant_id = $1 AND user_id = $2 AND id = $3
              )
            ",
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(playbook_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::database(format!("purge pending advice: {e}")))?;
        let res = sqlx::query(
            r"DELETE FROM coaching_playbooks WHERE tenant_id = $1 AND user_id = $2 AND id = $3",
        )
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
