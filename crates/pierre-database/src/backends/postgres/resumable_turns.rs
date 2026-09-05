// ABOUTME: PostgreSQL implementation of the ResumableTurnRepository trait
// ABOUTME: Records messaging turns and leases them, in conversation order, in single UPDATE … RETURNING statements with SKIP LOCKED
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::postgres::PgRow;
use sqlx::Row;

use super::PostgresDatabase;
use crate::repositories::{
    ResumableTurnClaim, ResumableTurnRepository, ResumableTurnRow, TurnClaim, TurnLease,
};

fn row_to_turn(row: &PgRow) -> ResumableTurnRow {
    ResumableTurnRow {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        channel_tenant_id: row.get("channel_tenant_id"),
        user_tenant_id: row.get("user_tenant_id"),
        session_id: row.get("session_id"),
        conversation: row.get("conversation"),
        user_id: row.get("user_id"),
        channel: row.get("channel_type"),
        sender_id: row.get("sender_id"),
        conversation_id: row.get("conversation_id"),
        channel_message_id: row.get("channel_message_id"),
        thread_id: row.get("thread_id"),
        text_content: row.get("text_content"),
        is_group_chat: row.get("is_group_chat"),
        locale: row.get("locale"),
        turn_id: row.get("turn_id"),
        placeholder_message_id: row.get("placeholder_message_id"),
        attempts: row.get("attempts"),
        enqueue_seq: row.get("enqueue_seq"),
        created_at_ms: row.get("created_at_ms"),
    }
}

#[async_trait]
impl ResumableTurnRepository for PostgresDatabase {
    async fn record_resumable_turn(&self, row: &ResumableTurnRow) -> AppResult<bool> {
        let outcome = sqlx::query(
            "INSERT INTO messaging_resumable_turns ( \
                    id, tenant_id, channel_tenant_id, user_tenant_id, session_id, conversation, \
                    user_id, channel_type, sender_id, conversation_id, channel_message_id, thread_id, \
                    text_content, is_group_chat, locale, turn_id, placeholder_message_id, attempts, \
                    enqueue_seq, created_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20) \
             ON CONFLICT(tenant_id, channel_type, channel_message_id) DO NOTHING",
        )
            .bind(&row.id)
            .bind(&row.tenant_id)
            .bind(&row.channel_tenant_id)
            .bind(&row.user_tenant_id)
            .bind(&row.session_id)
            .bind(&row.conversation)
            .bind(&row.user_id)
            .bind(&row.channel)
            .bind(&row.sender_id)
            .bind(&row.conversation_id)
            .bind(&row.channel_message_id)
            .bind(&row.thread_id)
            .bind(&row.text_content)
            .bind(row.is_group_chat)
            .bind(&row.locale)
            .bind(&row.turn_id)
            .bind(&row.placeholder_message_id)
            .bind(row.attempts)
            .bind(row.enqueue_seq)
            .bind(row.created_at_ms)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to record resumable turn: {e}")))?;
        Ok(outcome.rows_affected() > 0)
    }

    async fn claim_resumable_turns(
        &self,
        claim: &ResumableTurnClaim<'_>,
    ) -> AppResult<Vec<ResumableTurnRow>> {
        // One statement: the lease and the attempt bump land together, and
        // the subquery picks the rows under the same write lock, so a second
        // sweeper running at the same instant sees them already leased. The
        // NOT EXISTS keeps a younger turn behind its older sibling in the
        // same conversation until that sibling is finished; a sibling past
        // the attempt cap no longer counts, or it would block forever.
        let rows = sqlx::query(
            "UPDATE messaging_resumable_turns \
             SET leased_by = $1, leased_until_ms = $2, attempts = attempts + 1 \
             WHERE id IN ( \
                 SELECT t.id FROM messaging_resumable_turns t \
                 WHERE ((t.leased_until_ms IS NULL AND t.created_at_ms < $3) \
                        OR (t.leased_until_ms IS NOT NULL AND t.leased_until_ms < $4)) \
                   AND t.attempts <= $5 \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM messaging_resumable_turns o \
                       WHERE o.tenant_id = t.tenant_id AND o.conversation = t.conversation \
                         AND o.id <> t.id AND o.created_at_ms < t.created_at_ms \
                         AND o.attempts <= $5) \
                 ORDER BY t.created_at_ms ASC \
                 LIMIT $6 \
                 FOR UPDATE SKIP LOCKED \
             ) \
             RETURNING id, tenant_id, channel_tenant_id, user_tenant_id, session_id, conversation, \
                    user_id, channel_type, sender_id, conversation_id, channel_message_id, thread_id, \
                    text_content, is_group_chat, locale, turn_id, placeholder_message_id, attempts, \
                    enqueue_seq, created_at_ms",
        )
            .bind(claim.lease.leased_by)
            .bind(claim.lease.lease_until_ms)
            .bind(claim.queued_older_than_ms)
            .bind(claim.lease.now_ms)
            .bind(claim.lease.max_attempts)
            .bind(claim.limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to claim resumable turns: {e}")))?;
        Ok(rows.iter().map(row_to_turn).collect())
    }

    async fn list_stale_resumable_turns(
        &self,
        claim: &ResumableTurnClaim<'_>,
    ) -> AppResult<Vec<ResumableTurnRow>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, channel_tenant_id, user_tenant_id, session_id, conversation, \
                    user_id, channel_type, sender_id, conversation_id, channel_message_id, thread_id, \
                    text_content, is_group_chat, locale, turn_id, placeholder_message_id, attempts, \
                    enqueue_seq, created_at_ms \
             FROM messaging_resumable_turns t \
             WHERE ((t.leased_until_ms IS NULL AND t.created_at_ms < $1) \
                    OR (t.leased_until_ms IS NOT NULL AND t.leased_until_ms < $2)) \
               AND t.attempts <= $3 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM messaging_resumable_turns o \
                   WHERE o.tenant_id = t.tenant_id AND o.conversation = t.conversation \
                     AND o.id <> t.id AND o.created_at_ms < t.created_at_ms \
                     AND o.attempts <= $3) \
             ORDER BY t.created_at_ms ASC \
             LIMIT $4",
        )
        .bind(claim.queued_older_than_ms)
        .bind(claim.lease.now_ms)
        .bind(claim.lease.max_attempts)
        .bind(claim.limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list stale resumable turns: {e}")))?;
        Ok(rows.iter().map(row_to_turn).collect())
    }

    async fn claim_resumable_turn(
        &self,
        tenant_id: TenantId,
        id: &str,
        lease: &TurnLease<'_>,
    ) -> AppResult<TurnClaim> {
        let tenant = tenant_id.to_string();
        let claimed = sqlx::query(
            "UPDATE messaging_resumable_turns \
             SET leased_by = $1, leased_until_ms = $2, attempts = attempts + 1 \
             WHERE tenant_id = $3 AND id = $4 \
               AND (leased_until_ms IS NULL OR leased_until_ms < $5) \
               AND attempts <= $6 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM messaging_resumable_turns o \
                   WHERE o.tenant_id = messaging_resumable_turns.tenant_id \
                     AND o.conversation = messaging_resumable_turns.conversation \
                     AND o.id <> messaging_resumable_turns.id \
                     AND o.created_at_ms < messaging_resumable_turns.created_at_ms \
                     AND o.attempts <= $6) \
             RETURNING id, tenant_id, channel_tenant_id, user_tenant_id, session_id, conversation, \
                    user_id, channel_type, sender_id, conversation_id, channel_message_id, thread_id, \
                    text_content, is_group_chat, locale, turn_id, placeholder_message_id, attempts, \
                    enqueue_seq, created_at_ms",
        )
        .bind(lease.leased_by)
        .bind(lease.lease_until_ms)
        .bind(&tenant)
        .bind(id)
        .bind(lease.now_ms)
        .bind(lease.max_attempts)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to claim resumable turn: {e}")))?;
        if let Some(row) = claimed {
            return Ok(TurnClaim::Claimed(Box::new(row_to_turn(&row))));
        }
        let standing = sqlx::query(
            "SELECT attempts FROM messaging_resumable_turns WHERE tenant_id = $1 AND id = $2",
        )
        .bind(&tenant)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to read resumable turn: {e}")))?;
        Ok(match standing {
            None => TurnClaim::Missing,
            Some(row) if row.get::<i64, _>("attempts") > lease.max_attempts => TurnClaim::Exhausted,
            Some(_) => TurnClaim::Blocked,
        })
    }

    async fn renew_resumable_turn_lease(
        &self,
        tenant_id: TenantId,
        id: &str,
        leased_by: &str,
        lease_until_ms: i64,
    ) -> AppResult<bool> {
        let outcome = sqlx::query(
            "UPDATE messaging_resumable_turns SET leased_until_ms = $1 \
             WHERE tenant_id = $2 AND id = $3 AND leased_by = $4",
        )
        .bind(lease_until_ms)
        .bind(tenant_id.to_string())
        .bind(id)
        .bind(leased_by)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to renew resumable turn lease: {e}")))?;
        Ok(outcome.rows_affected() > 0)
    }

    async fn set_resumable_turn_placeholder(
        &self,
        tenant_id: TenantId,
        id: &str,
        placeholder_message_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE messaging_resumable_turns SET placeholder_message_id = $1 \
             WHERE tenant_id = $2 AND id = $3",
        )
        .bind(placeholder_message_id)
        .bind(tenant_id.to_string())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to record resumable turn placeholder: {e}"))
        })?;
        Ok(())
    }

    async fn bump_resumable_turn_enqueue(
        &self,
        tenant_id: TenantId,
        id: &str,
    ) -> AppResult<Option<i64>> {
        let row = sqlx::query(
            "UPDATE messaging_resumable_turns SET enqueue_seq = enqueue_seq + 1 \
             WHERE tenant_id = $1 AND id = $2 RETURNING enqueue_seq",
        )
        .bind(tenant_id.to_string())
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count resumable turn enqueue: {e}")))?;
        Ok(row.map(|r| r.get::<i64, _>("enqueue_seq")))
    }

    async fn release_resumable_turn(
        &self,
        tenant_id: TenantId,
        id: &str,
        now_ms: i64,
    ) -> AppResult<()> {
        // The lease is marked as ended rather than cleared: a row that was
        // never leased waits for the sweep's grace, a released one is
        // claimable at once.
        sqlx::query(
            "UPDATE messaging_resumable_turns \
             SET leased_by = NULL, leased_until_ms = $1 \
             WHERE tenant_id = $2 AND id = $3",
        )
        .bind(now_ms)
        .bind(tenant_id.to_string())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to release resumable turn: {e}")))?;
        Ok(())
    }

    async fn finish_resumable_turn(&self, tenant_id: TenantId, id: &str) -> AppResult<bool> {
        let outcome =
            sqlx::query("DELETE FROM messaging_resumable_turns WHERE tenant_id = $1 AND id = $2")
                .bind(tenant_id.to_string())
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to finish resumable turn: {e}")))?;
        Ok(outcome.rows_affected() > 0)
    }

    async fn reap_exhausted_turns(&self, now_ms: i64, max_attempts: i64) -> AppResult<u64> {
        let outcome = sqlx::query(
            "DELETE FROM messaging_resumable_turns \
             WHERE attempts > $1 AND leased_until_ms IS NOT NULL AND leased_until_ms < $2",
        )
        .bind(max_attempts)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to reap exhausted turns: {e}")))?;
        Ok(outcome.rows_affected())
    }
}
