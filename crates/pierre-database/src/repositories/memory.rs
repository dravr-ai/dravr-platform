// ABOUTME: The one HarnessMemoryRepository body both backends emit: compaction blocks, user facts, agent notes, followups, sessions
// ABOUTME: Statements as $n consts, row parsers generic over sqlx::Row, the impl block from one macro per backend type
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Statements, row parsers and the impl body of the coaching harness memory
//! repository.
//!
//! Every statement here runs unchanged on `SQLite` and Postgres, so nothing
//! in this module takes a per-backend argument:
//!
//! - Timestamps bind and decode as `DateTime<Utc>`. sqlx writes RFC3339 text
//!   on `SQLite`, byte-identical to the `to_rfc3339()` the rows were written
//!   with, so the string comparisons the `TEXT` columns rely on keep ordering
//!   the way the instants do; Postgres binds a native `timestamptz`.
//! - `tenant_id` binds as text on both: the columns are `TEXT`/`VARCHAR` in
//!   both schemas, and `TenantId`'s own Postgres encoding is a native `uuid`.
//! - `suppressed` binds and decodes as `bool`. `SQLite` declares it `INTEGER`
//!   and sqlx encodes a bool there as `0`/`1`, which is what the migration's
//!   default and its partial index compare against; `FALSE` is a keyword on
//!   both engines.
//! - The higher-confidence pick in [`MERGE_USER_FACT_SQL`] is a `CASE`, not
//!   `MAX`/`GREATEST`: `SQLite`'s two-argument `MAX` is an aggregate on
//!   Postgres, and `GREATEST` does not exist on `SQLite`.
//! - An optional filter is spelled `(col = $n OR $n IS NULL)`, column first.
//!   Postgres fixes a parameter's type at its first coercion, so the
//!   comparison must come before the `IS NULL` test or the statement fails
//!   to prepare with "could not determine data type of parameter"; `SQLite`
//!   does not care about the order.
//! - `COUNT(*)` and `SUM(CASE …)` decode as `i64` on both without a cast:
//!   Postgres returns `bigint` for each already.

/// Row decoders generic over `sqlx::Row`.
mod rows;

pub(crate) use rows::{
    agent_followup_from_row, agent_note_from_row, agent_session_from_row,
    compaction_block_from_row, user_fact_from_row, user_fact_metrics_from_rows,
};

/// Persist one compaction block.
pub(crate) const INSERT_COMPACTION_BLOCK_SQL: &str = r"
            INSERT INTO compaction_blocks (
                id, tenant_id, conversation_id, summary, summary_tokens,
                original_tokens, first_message_id, last_message_id, created_at,
                first_party_only
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ";

/// A conversation's compaction blocks, oldest first.
pub(crate) const LIST_COMPACTION_BLOCKS_SQL: &str = r"
            SELECT id, tenant_id, conversation_id, summary, summary_tokens,
                   original_tokens, first_message_id, last_message_id, created_at,
                   first_party_only
            FROM compaction_blocks
            WHERE conversation_id = $1 AND tenant_id = $2
            ORDER BY created_at ASC
            ";

/// Insert a fact; `$14` is both `created_at` and `updated_at`.
pub(crate) const INSERT_USER_FACT_SQL: &str = r"
            INSERT INTO user_facts (
                id, tenant_id, user_id, agent_id, scope, kind, pillar,
                predicate_code, object, confidence, source, valid_until,
                source_msg_id, created_at, updated_at, first_party_only
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14, $15)
            ";

/// Fold a restatement into its anchor.
///
/// `COALESCE` keeps the anchor's own words when the restatement names no
/// message; the `CASE` keeps the higher confidence, so a restatement can
/// only ever raise it. The stamp only ever tightens: an anchor restated from
/// first-party-only content stays first-party-only (carnet#769).
pub(crate) const MERGE_USER_FACT_SQL: &str = r"
            UPDATE user_facts
            SET confidence = CASE WHEN confidence >= $1 THEN confidence ELSE $1 END,
                source_msg_id = COALESCE($2, source_msg_id),
                updated_at = $3,
                first_party_only = CASE WHEN $6 THEN $6 ELSE first_party_only END
            WHERE id = $4 AND tenant_id = $5
            ";

/// Read one fact back by id within its tenant.
pub(crate) const GET_USER_FACT_BY_ID_SQL: &str = r"
            SELECT id, tenant_id, user_id, agent_id, scope, kind, pillar,
                   predicate_code, object, confidence, source, valid_until,
                   source_msg_id, created_at, updated_at, first_party_only
            FROM user_facts
            WHERE id = $1 AND tenant_id = $2
            ";

/// A user's facts filtered by agent and kind, newest first.
pub(crate) const LIST_USER_FACTS_BY_AGENT_AND_KIND_SQL: &str = r"
                    SELECT * FROM user_facts
                    WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND kind = $4
                      AND (first_party_only = FALSE OR $6)
                    ORDER BY updated_at DESC
                    LIMIT $5
                    ";

/// A user's facts filtered by agent, newest first.
pub(crate) const LIST_USER_FACTS_BY_AGENT_SQL: &str = r"
                    SELECT * FROM user_facts
                    WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3
                      AND (first_party_only = FALSE OR $5)
                    ORDER BY updated_at DESC
                    LIMIT $4
                    ";

/// A user's facts filtered by kind, newest first.
pub(crate) const LIST_USER_FACTS_BY_KIND_SQL: &str = r"
                    SELECT * FROM user_facts
                    WHERE tenant_id = $1 AND user_id = $2 AND kind = $3
                      AND (first_party_only = FALSE OR $5)
                    ORDER BY updated_at DESC
                    LIMIT $4
                    ";

/// All of a user's facts, newest first.
pub(crate) const LIST_USER_FACTS_SQL: &str = r"
                    SELECT * FROM user_facts
                    WHERE tenant_id = $1 AND user_id = $2
                      AND (first_party_only = FALSE OR $4)
                    ORDER BY updated_at DESC
                    LIMIT $3
                    ";

/// A user's still-valid facts from one source, newest first.
pub(crate) const LIST_USER_FACTS_BY_SOURCE_SQL: &str = r"
            SELECT * FROM user_facts
            WHERE tenant_id = $1 AND user_id = $2 AND source = $3
              AND (valid_until IS NULL OR valid_until > $4)
              AND (first_party_only = FALSE OR $6)
            ORDER BY updated_at DESC
            LIMIT $5
            ";

/// One fact, scoped to its tenant and user.
pub(crate) const GET_USER_FACT_SQL: &str = r"
            SELECT * FROM user_facts
            WHERE id = $1 AND tenant_id = $2 AND user_id = $3
            ";

/// Delete one fact, scoped to its tenant and user.
pub(crate) const DELETE_USER_FACT_SQL: &str = r"
            DELETE FROM user_facts
            WHERE id = $1 AND tenant_id = $2 AND user_id = $3
            ";

/// Delete a user's facts carrying one exact claim: source, predicate code and
/// object all equal, other than the row `$6` names (`NULL` keeps none; no
/// fact id is empty).
pub(crate) const DELETE_FACTS_BY_CLAIM_SQL: &str = r"
            DELETE FROM user_facts
            WHERE tenant_id = $1 AND user_id = $2
              AND source = $3 AND predicate_code = $4 AND object = $5
              AND id <> COALESCE($6, '')
            ";

/// Expire a user's still-valid onboarding facts, narrowed by any of pillar,
/// creation window and predicate code. `$1` is both the new `valid_until`
/// and the new `updated_at`.
pub(crate) const EXPIRE_ONBOARDING_FACTS_SQL: &str = r"
            UPDATE user_facts
               SET valid_until = $1, updated_at = $1
             WHERE tenant_id = $2 AND user_id = $3 AND source = 'onboarding'
               AND (pillar = $4 OR $4 IS NULL)
               AND (created_at >= $5 OR $5 IS NULL)
               AND (created_at <= $6 OR $6 IS NULL)
               AND (predicate_code = $7 OR $7 IS NULL)
               AND (valid_until IS NULL OR valid_until > $1)
            ";

/// A tenant's fact totals and recency counters in one pass.
pub(crate) const COUNT_USER_FACTS_SQL: &str = r"
            SELECT
                COUNT(*)                                          AS total,
                COUNT(DISTINCT user_id)                           AS distinct_users,
                SUM(CASE WHEN updated_at >= $1 THEN 1 ELSE 0 END) AS last_24h,
                SUM(CASE WHEN updated_at >= $2 THEN 1 ELSE 0 END) AS last_7d,
                MAX(updated_at)                                   AS newest_updated_at
            FROM user_facts
            WHERE tenant_id = $3
            ";

/// A tenant's fact count per kind.
pub(crate) const COUNT_USER_FACTS_BY_KIND_SQL: &str = r"
            SELECT kind, COUNT(*) AS c
            FROM user_facts
            WHERE tenant_id = $1
            GROUP BY kind
            ";

/// Insert a note; `$8` is both `created_at` and `updated_at`.
pub(crate) const INSERT_AGENT_NOTE_SQL: &str = r"
            INSERT INTO agent_notes (
                id, tenant_id, user_id, agent_id, conversation_id,
                scope, content, created_at, updated_at, first_party_only
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, $9)
            ";

/// An agent's unsuppressed notes about a user, newest first.
///
/// Recall never sees a suppressed note again; the audit surface reads them
/// through [`LIST_AGENT_NOTES_FOR_TENANT_SQL`] so an admin can un-suppress.
pub(crate) const LIST_AGENT_NOTES_SQL: &str = r"
            SELECT * FROM agent_notes
            WHERE tenant_id = $1
              AND user_id = $2
              AND agent_id = $3
              AND suppressed = FALSE
              AND (first_party_only = FALSE OR $5)
            ORDER BY created_at DESC
            LIMIT $4
            ";

/// Every note in a tenant, suppressed ones included, newest first.
pub(crate) const LIST_AGENT_NOTES_FOR_TENANT_SQL: &str = r"
            SELECT * FROM agent_notes
            WHERE tenant_id = $1
            ORDER BY created_at DESC
            LIMIT $2
            ";

/// Flip a note's suppression.
///
/// Only a row whose state differs is touched, which makes the call
/// idempotent and lets `rows_affected` answer whether anything changed
/// without a read.
pub(crate) const SET_AGENT_NOTE_SUPPRESSED_SQL: &str = r"
            UPDATE agent_notes
            SET suppressed = $1,
                suppressed_at = CASE WHEN $1 THEN $2 ELSE NULL END,
                suppressed_by = CASE WHEN $1 THEN $3 ELSE NULL END,
                updated_at = $2
            WHERE id = $4 AND tenant_id = $5 AND suppressed != $1
            ";

/// Insert a pending followup; `$8` is both `created_at` and `updated_at`.
pub(crate) const INSERT_AGENT_FOLLOWUP_SQL: &str = r"
            INSERT INTO agent_followups (
                id, tenant_id, user_id, agent_id, conversation_id,
                content, due_at, status, created_at, updated_at, first_party_only
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8, $8, $9)
            ";

/// An agent's pending followups for a user, soonest due first.
pub(crate) const LIST_PENDING_FOLLOWUPS_SQL: &str = r"
            SELECT * FROM agent_followups
            WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND status = 'pending'
            ORDER BY due_at ASC NULLS LAST, created_at ASC
            ";

/// A tenant's pending followups, soonest due first.
pub(crate) const LIST_PENDING_FOLLOWUPS_FOR_TENANT_SQL: &str = r"
            SELECT * FROM agent_followups
            WHERE tenant_id = $1 AND status = 'pending'
            ORDER BY due_at ASC NULLS LAST, created_at ASC
            LIMIT $2
            ";

/// Deliver a pending followup; `$1` is both `delivered_at` and `updated_at`.
pub(crate) const MARK_FOLLOWUP_DELIVERED_SQL: &str = r"
            UPDATE agent_followups
            SET status = 'delivered', delivered_at = $1, updated_at = $1
            WHERE id = $2 AND tenant_id = $3 AND status = 'pending'
            ";

/// Pending followups whose due time has passed, across tenants, soonest
/// first: the scheduler's sweep.
pub(crate) const LIST_DUE_FOLLOWUPS_SQL: &str = r"
            SELECT * FROM agent_followups
            WHERE status = 'pending'
              AND due_at IS NOT NULL
              AND due_at <= $1
            ORDER BY due_at ASC
            LIMIT $2
            ";

/// Cancel a pending followup.
pub(crate) const CANCEL_FOLLOWUP_SQL: &str = r"
            UPDATE agent_followups
            SET status = 'cancelled', updated_at = $1
            WHERE id = $2 AND tenant_id = $3 AND status = 'pending'
            ";

/// The newest active session between an agent and a user.
pub(crate) const GET_ACTIVE_AGENT_SESSION_SQL: &str = r"
            SELECT * FROM agent_sessions
            WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND status = 'active'
            ORDER BY opened_at DESC
            LIMIT 1
            ";

/// Open a session; `$5` is `opened_at`, `created_at` and `updated_at`.
pub(crate) const INSERT_AGENT_SESSION_SQL: &str = r"
            INSERT INTO agent_sessions (
                id, tenant_id, user_id, agent_id, status,
                opened_at, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, 'active', $5, $5, $5)
            ";

/// Record a turn on a session; `$1` is both `last_turn_at` and `updated_at`.
pub(crate) const TOUCH_AGENT_SESSION_SQL: &str = r"
            UPDATE agent_sessions
            SET last_turn_at = $1, updated_at = $1
            WHERE id = $2 AND tenant_id = $3
            ";

/// Emit the whole `HarnessMemoryRepository` implementation for one backend type.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_harness_memory_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl HarnessMemoryRepository for $ty {
            async fn insert_compaction_block(
                &self,
                params: &InsertCompactionBlockParams<'_>,
            ) -> AppResult<CompactionBlock> {
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();
                sqlx::query(INSERT_COMPACTION_BLOCK_SQL)
                    .bind(&id)
                    .bind(params.tenant_id.to_string())
                    .bind(params.conversation_id)
                    .bind(params.summary)
                    .bind(params.summary_tokens)
                    .bind(params.original_tokens)
                    .bind(params.first_message_id)
                    .bind(params.last_message_id)
                    .bind(now)
                    .bind(params.transport_policy.is_first_party_only())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert compaction block: {e}"))
                    })?;

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
                    transport_policy: params.transport_policy,
                })
            }

            async fn list_compaction_blocks(
                &self,
                conversation_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<CompactionBlock>> {
                let rows = sqlx::query(LIST_COMPACTION_BLOCKS_SQL)
                    .bind(conversation_id)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list compaction blocks: {e}"))
                    })?;

                rows.iter().map(compaction_block_from_row).collect()
            }

            async fn upsert_user_fact(
                &self,
                params: &UpsertUserFactParams<'_>,
            ) -> AppResult<UserFact> {
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();

                sqlx::query(INSERT_USER_FACT_SQL)
                    .bind(&id)
                    .bind(params.tenant_id.to_string())
                    .bind(params.user_id)
                    .bind(params.agent_id)
                    .bind(params.scope.as_str())
                    .bind(params.kind.as_str())
                    .bind(params.pillar.map(Pillar::as_str))
                    .bind(params.predicate_code.as_str())
                    .bind(params.object)
                    .bind(params.confidence)
                    .bind(params.source.as_str())
                    .bind(params.valid_until)
                    .bind(params.source_msg_id)
                    .bind(now)
                    .bind(params.transport_policy.is_first_party_only())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to upsert user fact: {e}")))?;

                Ok(UserFact {
                    id,
                    tenant_id: params.tenant_id.to_string(),
                    user_id: params.user_id.to_owned(),
                    agent_id: params.agent_id.map(ToOwned::to_owned),
                    scope: params.scope,
                    kind: params.kind,
                    pillar: params.pillar,
                    predicate_code: params.predicate_code,
                    object: params.object.to_owned(),
                    confidence: params.confidence,
                    source: params.source,
                    valid_until: params.valid_until,
                    source_msg_id: params.source_msg_id.map(ToOwned::to_owned),
                    created_at: now,
                    updated_at: now,
                    transport_policy: params.transport_policy,
                })
            }

            async fn merge_user_fact(
                &self,
                params: &MergeUserFactParams<'_>,
            ) -> AppResult<Option<UserFact>> {
                let tenant = params.tenant_id.to_string();
                sqlx::query(MERGE_USER_FACT_SQL)
                    .bind(params.confidence)
                    .bind(params.source_msg_id)
                    .bind(Utc::now())
                    .bind(params.fact_id)
                    .bind(&tenant)
                    .bind(params.transport_policy.is_first_party_only())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to merge user fact: {e}")))?;

                let row = sqlx::query(GET_USER_FACT_BY_ID_SQL)
                    .bind(params.fact_id)
                    .bind(&tenant)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read merged user fact: {e}"))
                    })?;

                row.as_ref().map(user_fact_from_row).transpose()
            }

            async fn list_user_facts(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                agent_id: Option<&str>,
                kind: Option<FactKind>,
                limit: i64,
                readable: TransportPolicy,
            ) -> AppResult<Vec<UserFact>> {
                let stamped = readable.is_first_party_only();
                // One statement per combination of the optional filters: a
                // NULL-tolerant bind would cost every call the two extra
                // comparisons on the hot recall path.
                let tenant = tenant_id.to_string();
                let rows = match (agent_id, kind) {
                    (Some(cid), Some(k)) => {
                        sqlx::query(LIST_USER_FACTS_BY_AGENT_AND_KIND_SQL)
                            .bind(&tenant)
                            .bind(user_id)
                            .bind(cid)
                            .bind(k.as_str())
                            .bind(limit)
                            .bind(stamped)
                            .fetch_all(self.pool())
                            .await
                    }
                    (Some(cid), None) => {
                        sqlx::query(LIST_USER_FACTS_BY_AGENT_SQL)
                            .bind(&tenant)
                            .bind(user_id)
                            .bind(cid)
                            .bind(limit)
                            .bind(stamped)
                            .fetch_all(self.pool())
                            .await
                    }
                    (None, Some(k)) => {
                        sqlx::query(LIST_USER_FACTS_BY_KIND_SQL)
                            .bind(&tenant)
                            .bind(user_id)
                            .bind(k.as_str())
                            .bind(limit)
                            .bind(stamped)
                            .fetch_all(self.pool())
                            .await
                    }
                    (None, None) => {
                        sqlx::query(LIST_USER_FACTS_SQL)
                            .bind(&tenant)
                            .bind(user_id)
                            .bind(limit)
                            .bind(stamped)
                            .fetch_all(self.pool())
                            .await
                    }
                }
                .map_err(|e| AppError::database(format!("Failed to list user facts: {e}")))?;

                rows.iter().map(user_fact_from_row).collect()
            }

            async fn list_user_facts_by_source(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                source: FactSource,
                limit: i64,
                readable: TransportPolicy,
            ) -> AppResult<Vec<UserFact>> {
                let rows = sqlx::query(LIST_USER_FACTS_BY_SOURCE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id)
                    .bind(source.as_str())
                    .bind(Utc::now())
                    .bind(limit)
                    .bind(readable.is_first_party_only())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list user facts by source: {e}"))
                    })?;

                rows.iter().map(user_fact_from_row).collect()
            }

            async fn get_user_fact(
                &self,
                fact_id: &str,
                tenant_id: TenantId,
                user_id: &str,
            ) -> AppResult<Option<UserFact>> {
                let row = sqlx::query(GET_USER_FACT_SQL)
                    .bind(fact_id)
                    .bind(tenant_id.to_string())
                    .bind(user_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get user fact: {e}")))?;

                row.as_ref().map(user_fact_from_row).transpose()
            }

            async fn delete_user_fact(
                &self,
                fact_id: &str,
                tenant_id: TenantId,
                user_id: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(DELETE_USER_FACT_SQL)
                    .bind(fact_id)
                    .bind(tenant_id.to_string())
                    .bind(user_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to delete user fact: {e}")))?;
                Ok(result.rows_affected() > 0)
            }

            async fn delete_facts_by_claim(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                source: FactSource,
                predicate_code: PredicateCode,
                object: &str,
                keep_id: Option<&str>,
            ) -> AppResult<u64> {
                let result = sqlx::query(DELETE_FACTS_BY_CLAIM_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id)
                    .bind(source.as_str())
                    .bind(predicate_code.as_str())
                    .bind(object)
                    .bind(keep_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete facts by claim: {e}"))
                    })?;
                Ok(result.rows_affected())
            }

            async fn expire_onboarding_facts(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                pillar: Option<Pillar>,
                created_after: Option<DateTime<Utc>>,
                created_before: Option<DateTime<Utc>>,
                predicate_code: Option<PredicateCode>,
            ) -> AppResult<u64> {
                let result = sqlx::query(EXPIRE_ONBOARDING_FACTS_SQL)
                    .bind(Utc::now())
                    .bind(tenant_id.to_string())
                    .bind(user_id)
                    .bind(pillar.map(Pillar::as_str))
                    .bind(created_after)
                    .bind(created_before)
                    .bind(predicate_code.map(PredicateCode::as_str))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to expire onboarding facts: {e}"))
                    })?;
                Ok(result.rows_affected())
            }

            async fn count_user_facts_metrics(
                &self,
                tenant_id: TenantId,
            ) -> AppResult<UserFactMetrics> {
                let tenant = tenant_id.to_string();
                let now = Utc::now();
                let cutoff_24h = now - chrono::Duration::hours(24);
                let cutoff_7d = now - chrono::Duration::days(7);

                let aggregates = sqlx::query(COUNT_USER_FACTS_SQL)
                    .bind(cutoff_24h)
                    .bind(cutoff_7d)
                    .bind(&tenant)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to count user facts: {e}")))?;

                let kind_rows = sqlx::query(COUNT_USER_FACTS_BY_KIND_SQL)
                    .bind(&tenant)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to group user facts by kind: {e}"))
                    })?;

                user_fact_metrics_from_rows(&aggregates, &kind_rows)
            }

            async fn insert_agent_note(
                &self,
                params: &InsertAgentNoteParams<'_>,
            ) -> AppResult<AgentNote> {
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();

                sqlx::query(INSERT_AGENT_NOTE_SQL)
                    .bind(&id)
                    .bind(params.tenant_id.to_string())
                    .bind(params.user_id)
                    .bind(params.agent_id)
                    .bind(params.conversation_id)
                    .bind(params.scope.as_str())
                    .bind(params.content)
                    .bind(now)
                    .bind(params.transport_policy.is_first_party_only())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to insert coach note: {e}")))?;

                Ok(AgentNote {
                    id,
                    tenant_id: params.tenant_id.to_string(),
                    user_id: params.user_id.to_owned(),
                    agent_id: params.agent_id.to_owned(),
                    conversation_id: params.conversation_id.map(ToOwned::to_owned),
                    scope: params.scope,
                    content: params.content.to_owned(),
                    created_at: now,
                    updated_at: now,
                    suppressed: false,
                    transport_policy: params.transport_policy,
                })
            }

            async fn list_agent_notes(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                agent_id: &str,
                limit: i64,
                readable: TransportPolicy,
            ) -> AppResult<Vec<AgentNote>> {
                let rows = sqlx::query(LIST_AGENT_NOTES_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id)
                    .bind(agent_id)
                    .bind(limit)
                    .bind(readable.is_first_party_only())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list coach notes: {e}")))?;

                rows.iter().map(agent_note_from_row).collect()
            }

            async fn list_agent_notes_for_tenant(
                &self,
                tenant_id: TenantId,
                limit: i64,
            ) -> AppResult<Vec<AgentNote>> {
                let rows = sqlx::query(LIST_AGENT_NOTES_FOR_TENANT_SQL)
                    .bind(tenant_id.to_string())
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list coach notes for tenant: {e}"))
                    })?;

                rows.iter().map(agent_note_from_row).collect()
            }

            async fn set_agent_note_suppressed(
                &self,
                note_id: &str,
                tenant_id: TenantId,
                suppressed: bool,
                actor: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(SET_AGENT_NOTE_SUPPRESSED_SQL)
                    .bind(suppressed)
                    .bind(Utc::now())
                    .bind(actor)
                    .bind(note_id)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set coach note suppressed: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn insert_agent_followup(
                &self,
                params: &InsertAgentFollowupParams<'_>,
            ) -> AppResult<AgentFollowup> {
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();

                sqlx::query(INSERT_AGENT_FOLLOWUP_SQL)
                    .bind(&id)
                    .bind(params.tenant_id.to_string())
                    .bind(params.user_id)
                    .bind(params.agent_id)
                    .bind(params.conversation_id)
                    .bind(params.content)
                    .bind(params.due_at)
                    .bind(now)
                    .bind(params.transport_policy.is_first_party_only())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert coach followup: {e}"))
                    })?;

                Ok(AgentFollowup {
                    id,
                    tenant_id: params.tenant_id.to_string(),
                    user_id: params.user_id.to_owned(),
                    agent_id: params.agent_id.to_owned(),
                    conversation_id: params.conversation_id.map(ToOwned::to_owned),
                    content: params.content.to_owned(),
                    due_at: params.due_at,
                    status: FollowupStatus::Pending,
                    created_at: now,
                    updated_at: now,
                    delivered_at: None,
                    transport_policy: params.transport_policy,
                })
            }

            async fn list_pending_followups(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                agent_id: &str,
            ) -> AppResult<Vec<AgentFollowup>> {
                let rows = sqlx::query(LIST_PENDING_FOLLOWUPS_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id)
                    .bind(agent_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list pending followups: {e}"))
                    })?;

                rows.iter().map(agent_followup_from_row).collect()
            }

            async fn list_pending_followups_for_tenant(
                &self,
                tenant_id: TenantId,
                limit: i64,
            ) -> AppResult<Vec<AgentFollowup>> {
                let rows = sqlx::query(LIST_PENDING_FOLLOWUPS_FOR_TENANT_SQL)
                    .bind(tenant_id.to_string())
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list pending followups for tenant: {e}"
                        ))
                    })?;

                rows.iter().map(agent_followup_from_row).collect()
            }

            async fn mark_followup_delivered(
                &self,
                followup_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(MARK_FOLLOWUP_DELIVERED_SQL)
                    .bind(Utc::now())
                    .bind(followup_id)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to mark followup delivered: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn list_due_followups(
                &self,
                now: DateTime<Utc>,
                limit: i64,
            ) -> AppResult<Vec<AgentFollowup>> {
                let rows = sqlx::query(LIST_DUE_FOLLOWUPS_SQL)
                    .bind(now)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list due followups: {e}"))
                    })?;

                rows.iter().map(agent_followup_from_row).collect()
            }

            async fn cancel_followup(
                &self,
                followup_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(CANCEL_FOLLOWUP_SQL)
                    .bind(Utc::now())
                    .bind(followup_id)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to cancel followup: {e}")))?;

                Ok(result.rows_affected() > 0)
            }

            async fn get_or_open_agent_session(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                agent_id: &str,
            ) -> AppResult<AgentSession> {
                let tenant = tenant_id.to_string();
                if let Some(row) = sqlx::query(GET_ACTIVE_AGENT_SESSION_SQL)
                    .bind(&tenant)
                    .bind(user_id)
                    .bind(agent_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get active coach session: {e}"))
                    })?
                {
                    return agent_session_from_row(&row);
                }

                let id = Uuid::new_v4().to_string();
                let now = Utc::now();
                sqlx::query(INSERT_AGENT_SESSION_SQL)
                    .bind(&id)
                    .bind(&tenant)
                    .bind(user_id)
                    .bind(agent_id)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to open coach session: {e}"))
                    })?;

                Ok(AgentSession {
                    id,
                    tenant_id: tenant,
                    user_id: user_id.to_owned(),
                    agent_id: agent_id.to_owned(),
                    status: SessionStatus::Active,
                    opened_at: now,
                    last_turn_at: None,
                    archived_at: None,
                    created_at: now,
                    updated_at: now,
                })
            }

            async fn touch_agent_session(
                &self,
                session_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<()> {
                sqlx::query(TOUCH_AGENT_SESSION_SQL)
                    .bind(Utc::now())
                    .bind(session_id)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to touch coach session: {e}"))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_harness_memory_repository;
