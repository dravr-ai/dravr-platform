// ABOUTME: PostgreSQL implementation of the coaching harness HarnessMemoryRepository trait
// ABOUTME: Mirrors the SQLite path; embeddings stored as BYTEA for parity with sqlite-vec fallback
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_memory::{
    CoachFollowup, CoachNote, CoachSession, CompactionBlock, FactKind, FollowupStatus, MemoryScope,
    SessionStatus, UserFact, UserFactMetrics,
};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::{
    HarnessMemoryRepository, InsertCoachFollowupParams, InsertCoachNoteParams,
    InsertCompactionBlockParams, UpsertUserFactParams,
};

// Embeddings are serialized as little-endian f32 sequences in BYTEA to keep
// SQLite and Postgres on identical byte formats. A follow-up migration can
// promote the column to `pgvector::vector` in environments where the
// extension is available.

/// Clamp a signed row count from a `COUNT`/`SUM` query to `u64`, folding
/// impossible negative values to `0`. Aggregate counts are domain-guaranteed
/// non-negative but `sqlx` decodes them as `i64`.
fn i64_to_u64_saturating(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for v in embedding {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

fn bytes_to_embedding(bytes: &[u8]) -> AppResult<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return Err(AppError::internal(format!(
            "Invalid embedding bytea length: {} bytes is not a multiple of 4",
            bytes.len()
        )));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

fn optional_embedding(row: &PgRow) -> AppResult<Option<Vec<f32>>> {
    let bytes: Option<Vec<u8>> = row
        .try_get("embedding")
        .map_err(|e| AppError::database(format!("Failed to read embedding column: {e}")))?;
    bytes.as_deref().map(bytes_to_embedding).transpose()
}

fn row_to_compaction_block(row: &PgRow) -> AppResult<CompactionBlock> {
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| AppError::database(format!("Failed to read created_at: {e}")))?;
    Ok(CompactionBlock {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        conversation_id: row.get("conversation_id"),
        summary: row.get("summary"),
        summary_tokens: row.get("summary_tokens"),
        original_tokens: row.get("original_tokens"),
        first_message_id: row.get("first_message_id"),
        last_message_id: row.get("last_message_id"),
        created_at,
    })
}

fn row_to_user_fact(row: &PgRow) -> AppResult<UserFact> {
    let scope_str: String = row.get("scope");
    let kind_str: String = row.get("kind");
    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");
    let scope = MemoryScope::parse(&scope_str)
        .ok_or_else(|| AppError::internal(format!("Invalid scope in user_facts: {scope_str}")))?;
    Ok(UserFact {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        user_id: row.get("user_id"),
        coach_id: row.get("coach_id"),
        scope,
        kind: FactKind::parse_lenient(&kind_str),
        subject: row.get("subject"),
        predicate: row.get("predicate"),
        object: row.get("object"),
        confidence: row.get("confidence"),
        source_msg_id: row.get("source_msg_id"),
        embedding: optional_embedding(row)?,
        created_at,
        updated_at,
    })
}

fn row_to_coach_note(row: &PgRow) -> AppResult<CoachNote> {
    let scope_str: String = row.get("scope");
    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");
    let scope = MemoryScope::parse(&scope_str)
        .ok_or_else(|| AppError::internal(format!("Invalid scope in coach_notes: {scope_str}")))?;
    let suppressed: bool = row.try_get("suppressed").unwrap_or(false);
    Ok(CoachNote {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        user_id: row.get("user_id"),
        coach_id: row.get("coach_id"),
        conversation_id: row.get("conversation_id"),
        scope,
        content: row.get("content"),
        embedding: optional_embedding(row)?,
        created_at,
        updated_at,
        suppressed,
    })
}

fn row_to_coach_followup(row: &PgRow) -> AppResult<CoachFollowup> {
    let status_str: String = row.get("status");
    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");
    let due_at: Option<DateTime<Utc>> = row.get("due_at");
    let delivered_at: Option<DateTime<Utc>> = row.get("delivered_at");
    let status = FollowupStatus::parse(&status_str).ok_or_else(|| {
        AppError::internal(format!("Invalid status in coach_followups: {status_str}"))
    })?;
    Ok(CoachFollowup {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        user_id: row.get("user_id"),
        coach_id: row.get("coach_id"),
        conversation_id: row.get("conversation_id"),
        content: row.get("content"),
        due_at,
        status,
        created_at,
        updated_at,
        delivered_at,
    })
}

fn row_to_coach_session(row: &PgRow) -> AppResult<CoachSession> {
    let status_str: String = row.get("status");
    let opened_at: DateTime<Utc> = row.get("opened_at");
    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");
    let last_turn_at: Option<DateTime<Utc>> = row.get("last_turn_at");
    let archived_at: Option<DateTime<Utc>> = row.get("archived_at");
    let status = SessionStatus::parse(&status_str).ok_or_else(|| {
        AppError::internal(format!("Invalid status in coach_sessions: {status_str}"))
    })?;
    Ok(CoachSession {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        user_id: row.get("user_id"),
        coach_id: row.get("coach_id"),
        status,
        opened_at,
        last_turn_at,
        archived_at,
        created_at,
        updated_at,
    })
}

#[async_trait]
impl HarnessMemoryRepository for PostgresDatabase {
    async fn insert_compaction_block(
        &self,
        params: &InsertCompactionBlockParams<'_>,
    ) -> AppResult<CompactionBlock> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        sqlx::query(
            r"
            INSERT INTO compaction_blocks (
                id, tenant_id, conversation_id, summary, summary_tokens,
                original_tokens, first_message_id, last_message_id, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ",
        )
        .bind(&id)
        .bind(params.tenant_id.to_string())
        .bind(params.conversation_id)
        .bind(params.summary)
        .bind(params.summary_tokens)
        .bind(params.original_tokens)
        .bind(params.first_message_id)
        .bind(params.last_message_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert compaction block: {e}")))?;

        Ok(CompactionBlock {
            id,
            tenant_id: params.tenant_id.to_string(),
            conversation_id: params.conversation_id.to_owned(),
            summary: params.summary.to_owned(),
            summary_tokens: params.summary_tokens,
            original_tokens: params.original_tokens,
            first_message_id: params.first_message_id.to_owned(),
            last_message_id: params.last_message_id.to_owned(),
            created_at: now,
        })
    }

    async fn list_compaction_blocks(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CompactionBlock>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, conversation_id, summary, summary_tokens,
                   original_tokens, first_message_id, last_message_id, created_at
            FROM compaction_blocks
            WHERE conversation_id = $1 AND tenant_id = $2
            ORDER BY created_at ASC
            ",
        )
        .bind(conversation_id)
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list compaction blocks: {e}")))?;

        rows.iter().map(row_to_compaction_block).collect()
    }

    async fn upsert_user_fact(&self, params: &UpsertUserFactParams<'_>) -> AppResult<UserFact> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let embedding_bytes = params.embedding.map(embedding_to_bytes);

        sqlx::query(
            r"
            INSERT INTO user_facts (
                id, tenant_id, user_id, coach_id, scope, kind,
                subject, predicate, object, confidence, source_msg_id,
                embedding, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)
            ",
        )
        .bind(&id)
        .bind(params.tenant_id.to_string())
        .bind(params.user_id)
        .bind(params.coach_id)
        .bind(params.scope.as_str())
        .bind(params.kind.as_str())
        .bind(params.subject)
        .bind(params.predicate)
        .bind(params.object)
        .bind(params.confidence)
        .bind(params.source_msg_id)
        .bind(embedding_bytes)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert user fact: {e}")))?;

        Ok(UserFact {
            id,
            tenant_id: params.tenant_id.to_string(),
            user_id: params.user_id.to_owned(),
            coach_id: params.coach_id.map(ToOwned::to_owned),
            scope: params.scope,
            kind: params.kind,
            subject: params.subject.to_owned(),
            predicate: params.predicate.to_owned(),
            object: params.object.to_owned(),
            confidence: params.confidence,
            source_msg_id: params.source_msg_id.map(ToOwned::to_owned),
            embedding: params.embedding.map(<[f32]>::to_vec),
            created_at: now,
            updated_at: now,
        })
    }

    async fn list_user_facts(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: Option<&str>,
        kind: Option<FactKind>,
        limit: i64,
    ) -> AppResult<Vec<UserFact>> {
        let tenant_str = tenant_id.to_string();
        let rows = match (coach_id, kind) {
            (Some(cid), Some(k)) => {
                sqlx::query(
                    r"
                    SELECT * FROM user_facts
                    WHERE tenant_id = $1 AND user_id = $2 AND coach_id = $3 AND kind = $4
                    ORDER BY updated_at DESC
                    LIMIT $5
                    ",
                )
                .bind(tenant_str)
                .bind(user_id)
                .bind(cid)
                .bind(k.as_str())
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            (Some(cid), None) => {
                sqlx::query(
                    r"
                    SELECT * FROM user_facts
                    WHERE tenant_id = $1 AND user_id = $2 AND coach_id = $3
                    ORDER BY updated_at DESC
                    LIMIT $4
                    ",
                )
                .bind(tenant_str)
                .bind(user_id)
                .bind(cid)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            (None, Some(k)) => {
                sqlx::query(
                    r"
                    SELECT * FROM user_facts
                    WHERE tenant_id = $1 AND user_id = $2 AND kind = $3
                    ORDER BY updated_at DESC
                    LIMIT $4
                    ",
                )
                .bind(tenant_str)
                .bind(user_id)
                .bind(k.as_str())
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            (None, None) => {
                sqlx::query(
                    r"
                    SELECT * FROM user_facts
                    WHERE tenant_id = $1 AND user_id = $2
                    ORDER BY updated_at DESC
                    LIMIT $3
                    ",
                )
                .bind(tenant_str)
                .bind(user_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| AppError::database(format!("Failed to list user facts: {e}")))?;

        rows.iter().map(row_to_user_fact).collect()
    }

    async fn delete_user_fact(
        &self,
        fact_id: &str,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM user_facts
            WHERE id = $1 AND tenant_id = $2 AND user_id = $3
            ",
        )
        .bind(fact_id)
        .bind(tenant_id.to_string())
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete user fact: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn count_user_facts_metrics(&self, tenant_id: TenantId) -> AppResult<UserFactMetrics> {
        let tenant_str = tenant_id.to_string();
        let now = Utc::now();
        let cutoff_24h = now - chrono::Duration::hours(24);
        let cutoff_7d = now - chrono::Duration::days(7);

        let aggregates = sqlx::query(
            r"
            SELECT
                COUNT(*)::BIGINT                                                AS total,
                COUNT(DISTINCT user_id)::BIGINT                                 AS distinct_users,
                SUM(CASE WHEN updated_at >= $1 THEN 1 ELSE 0 END)::BIGINT       AS last_24h,
                SUM(CASE WHEN updated_at >= $2 THEN 1 ELSE 0 END)::BIGINT       AS last_7d,
                MAX(updated_at)                                                 AS newest_updated_at
            FROM user_facts
            WHERE tenant_id = $3
            ",
        )
        .bind(cutoff_24h)
        .bind(cutoff_7d)
        .bind(&tenant_str)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count user facts: {e}")))?;

        // `COUNT(*)` is non-nullable; `SUM(CASE ...)` and `MAX(...)` can be
        // NULL when zero rows match the tenant filter, so both decode as
        // `Option<T>`.
        let total: i64 = aggregates
            .try_get("total")
            .map_err(|e| AppError::database(format!("Failed to read total count: {e}")))?;
        let distinct_users: i64 = aggregates
            .try_get("distinct_users")
            .map_err(|e| AppError::database(format!("Failed to read distinct_users: {e}")))?;
        let last_24h: Option<i64> = aggregates
            .try_get("last_24h")
            .map_err(|e| AppError::database(format!("Failed to read last_24h: {e}")))?;
        let last_7d: Option<i64> = aggregates
            .try_get("last_7d")
            .map_err(|e| AppError::database(format!("Failed to read last_7d: {e}")))?;
        let newest_updated_at: Option<DateTime<Utc>> = aggregates
            .try_get("newest_updated_at")
            .map_err(|e| AppError::database(format!("Failed to read newest_updated_at: {e}")))?;

        let kind_rows = sqlx::query(
            r"
            SELECT kind, COUNT(*)::BIGINT AS c
            FROM user_facts
            WHERE tenant_id = $1
            GROUP BY kind
            ",
        )
        .bind(&tenant_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to group user facts by kind: {e}")))?;

        let mut facts_by_kind: BTreeMap<String, u64> = BTreeMap::new();
        for row in &kind_rows {
            let kind: String = row.get("kind");
            let count: i64 = row
                .try_get("c")
                .map_err(|e| AppError::database(format!("Failed to read kind count: {e}")))?;
            facts_by_kind.insert(kind, i64_to_u64_saturating(count));
        }

        Ok(UserFactMetrics {
            total_facts: i64_to_u64_saturating(total),
            facts_last_24h: i64_to_u64_saturating(last_24h.unwrap_or(0)),
            facts_last_7d: i64_to_u64_saturating(last_7d.unwrap_or(0)),
            distinct_users: i64_to_u64_saturating(distinct_users),
            facts_by_kind,
            newest_updated_at,
        })
    }

    async fn insert_coach_note(&self, params: &InsertCoachNoteParams<'_>) -> AppResult<CoachNote> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let embedding_bytes = params.embedding.map(embedding_to_bytes);

        sqlx::query(
            r"
            INSERT INTO coach_notes (
                id, tenant_id, user_id, coach_id, conversation_id,
                scope, content, embedding, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
            ",
        )
        .bind(&id)
        .bind(params.tenant_id.to_string())
        .bind(params.user_id)
        .bind(params.coach_id)
        .bind(params.conversation_id)
        .bind(params.scope.as_str())
        .bind(params.content)
        .bind(embedding_bytes)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert coach note: {e}")))?;

        Ok(CoachNote {
            id,
            tenant_id: params.tenant_id.to_string(),
            user_id: params.user_id.to_owned(),
            coach_id: params.coach_id.to_owned(),
            conversation_id: params.conversation_id.map(ToOwned::to_owned),
            scope: params.scope,
            content: params.content.to_owned(),
            embedding: params.embedding.map(<[f32]>::to_vec),
            created_at: now,
            updated_at: now,
            suppressed: false,
        })
    }

    async fn list_coach_notes(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: &str,
        limit: i64,
    ) -> AppResult<Vec<CoachNote>> {
        // Recall queries exclude suppressed rows. Admins re-surface them
        // via `list_coach_notes_for_tenant` in the audit panel.
        let rows = sqlx::query(
            r"
            SELECT * FROM coach_notes
            WHERE tenant_id = $1
              AND user_id = $2
              AND coach_id = $3
              AND suppressed = FALSE
            ORDER BY created_at DESC
            LIMIT $4
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .bind(coach_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list coach notes: {e}")))?;

        rows.iter().map(row_to_coach_note).collect()
    }

    async fn list_coach_notes_for_tenant(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<CoachNote>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM coach_notes
            WHERE tenant_id = $1
            ORDER BY created_at DESC
            LIMIT $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list coach notes for tenant: {e}")))?;

        rows.iter().map(row_to_coach_note).collect()
    }

    async fn set_coach_note_suppressed(
        &self,
        note_id: &str,
        tenant_id: TenantId,
        suppressed: bool,
        actor: &str,
    ) -> AppResult<bool> {
        let now = Utc::now();
        let result = sqlx::query(
            r"
            UPDATE coach_notes
            SET suppressed = $1,
                suppressed_at = CASE WHEN $1 THEN $2 ELSE NULL END,
                suppressed_by = CASE WHEN $1 THEN $3 ELSE NULL END,
                updated_at = $2
            WHERE id = $4 AND tenant_id = $5 AND suppressed != $1
            ",
        )
        .bind(suppressed)
        .bind(now)
        .bind(actor)
        .bind(note_id)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set coach note suppressed: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn insert_coach_followup(
        &self,
        params: &InsertCoachFollowupParams<'_>,
    ) -> AppResult<CoachFollowup> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO coach_followups (
                id, tenant_id, user_id, coach_id, conversation_id,
                content, due_at, status, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8, $8)
            ",
        )
        .bind(&id)
        .bind(params.tenant_id.to_string())
        .bind(params.user_id)
        .bind(params.coach_id)
        .bind(params.conversation_id)
        .bind(params.content)
        .bind(params.due_at)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert coach followup: {e}")))?;

        Ok(CoachFollowup {
            id,
            tenant_id: params.tenant_id.to_string(),
            user_id: params.user_id.to_owned(),
            coach_id: params.coach_id.to_owned(),
            conversation_id: params.conversation_id.map(ToOwned::to_owned),
            content: params.content.to_owned(),
            due_at: params.due_at,
            status: FollowupStatus::Pending,
            created_at: now,
            updated_at: now,
            delivered_at: None,
        })
    }

    async fn list_pending_followups(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: &str,
    ) -> AppResult<Vec<CoachFollowup>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM coach_followups
            WHERE tenant_id = $1 AND user_id = $2 AND coach_id = $3 AND status = 'pending'
            ORDER BY due_at ASC NULLS LAST, created_at ASC
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .bind(coach_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list pending followups: {e}")))?;

        rows.iter().map(row_to_coach_followup).collect()
    }

    async fn list_pending_followups_for_tenant(
        &self,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<CoachFollowup>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM coach_followups
            WHERE tenant_id = $1 AND status = 'pending'
            ORDER BY due_at ASC NULLS LAST, created_at ASC
            LIMIT $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to list pending followups for tenant: {e}"))
        })?;

        rows.iter().map(row_to_coach_followup).collect()
    }

    async fn mark_followup_delivered(
        &self,
        followup_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let now = Utc::now();
        let result = sqlx::query(
            r"
            UPDATE coach_followups
            SET status = 'delivered', delivered_at = $1, updated_at = $1
            WHERE id = $2 AND tenant_id = $3 AND status = 'pending'
            ",
        )
        .bind(now)
        .bind(followup_id)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark followup delivered: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn list_due_followups(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> AppResult<Vec<CoachFollowup>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM coach_followups
            WHERE status = 'pending'
              AND due_at IS NOT NULL
              AND due_at <= $1
            ORDER BY due_at ASC
            LIMIT $2
            ",
        )
        .bind(now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list due followups: {e}")))?;

        rows.iter().map(row_to_coach_followup).collect()
    }

    async fn cancel_followup(&self, followup_id: &str, tenant_id: TenantId) -> AppResult<bool> {
        let now = Utc::now();
        let result = sqlx::query(
            r"
            UPDATE coach_followups
            SET status = 'cancelled', updated_at = $1
            WHERE id = $2 AND tenant_id = $3 AND status = 'pending'
            ",
        )
        .bind(now)
        .bind(followup_id)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to cancel followup: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_or_open_coach_session(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        coach_id: &str,
    ) -> AppResult<CoachSession> {
        if let Some(row) = sqlx::query(
            r"
            SELECT * FROM coach_sessions
            WHERE tenant_id = $1 AND user_id = $2 AND coach_id = $3 AND status = 'active'
            ORDER BY opened_at DESC
            LIMIT 1
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .bind(coach_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get active coach session: {e}")))?
        {
            return row_to_coach_session(&row);
        }

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        sqlx::query(
            r"
            INSERT INTO coach_sessions (
                id, tenant_id, user_id, coach_id, status,
                opened_at, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, 'active', $5, $5, $5)
            ",
        )
        .bind(&id)
        .bind(tenant_id.to_string())
        .bind(user_id)
        .bind(coach_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to open coach session: {e}")))?;

        Ok(CoachSession {
            id,
            tenant_id: tenant_id.to_string(),
            user_id: user_id.to_owned(),
            coach_id: coach_id.to_owned(),
            status: SessionStatus::Active,
            opened_at: now,
            last_turn_at: None,
            archived_at: None,
            created_at: now,
            updated_at: now,
        })
    }

    async fn touch_coach_session(&self, session_id: &str, tenant_id: TenantId) -> AppResult<()> {
        let now = Utc::now();
        sqlx::query(
            r"
            UPDATE coach_sessions
            SET last_turn_at = $1, updated_at = $1
            WHERE id = $2 AND tenant_id = $3
            ",
        )
        .bind(now)
        .bind(session_id)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to touch coach session: {e}")))?;
        Ok(())
    }

    async fn archive_coach_session(
        &self,
        session_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let now = Utc::now();
        let result = sqlx::query(
            r"
            UPDATE coach_sessions
            SET status = 'archived', archived_at = $1, updated_at = $1
            WHERE id = $2 AND tenant_id = $3 AND status = 'active'
            ",
        )
        .bind(now)
        .bind(session_id)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to archive coach session: {e}")))?;

        Ok(result.rows_affected() > 0)
    }
}
