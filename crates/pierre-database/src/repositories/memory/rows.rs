// ABOUTME: Row decoders for the harness memory tables, generic over sqlx::Row so one parser serves both backends
// ABOUTME: Compaction blocks, user facts, agent notes, followups, sessions and the user-fact metrics aggregate
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Row decoders for the coaching harness memory repository.
//!
//! Each parser reads columns by name through [`column`], so a width, type or
//! NULL surprise surfaces as a database error naming the column instead of a
//! panic. The parent module re-exports the decoders the backend shells use.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::Pillar;
use pierre_core::transport::TransportPolicy;
use pierre_memory::{
    AgentFollowup, AgentNote, AgentSession, CompactionBlock, FactKind, FactSource, FollowupStatus,
    MemoryScope, PredicateCode, SessionStatus, UserFact, UserFactMetrics,
};

/// Clamp a signed row count to `u64`, folding impossible negatives to `0`.
/// Counts from aggregate queries are domain-guaranteed non-negative but
/// `sqlx` decodes them as `i64`.
fn i64_to_u64_saturating(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn column_error(name: &str, e: &sqlx::Error) -> AppError {
    AppError::database(format!("Failed to read {name}: {e}"))
}

/// Read one column by name via `try_get`, so a width, type or NULL surprise
/// surfaces as a recoverable error naming the column rather than a panic.
///
/// # Errors
/// Returns a database error when the column cannot be decoded.
fn column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    T: sqlx::Type<R::Database> + sqlx::Decode<'r, R::Database>,
{
    row.try_get(name).map_err(|e| column_error(name, &e))
}

/// Read a derived row's `first_party_only` stamp (carnet#769).
///
/// # Errors
/// Returns a database error when the column cannot be decoded.
fn transport_policy_column<R>(row: &R) -> AppResult<TransportPolicy>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    column(row, "first_party_only").map(TransportPolicy::from_first_party_only)
}

/// Decode one `compaction_blocks` row.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub fn compaction_block_from_row<R>(row: &R) -> AppResult<CompactionBlock>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(CompactionBlock {
        id: column(row, "id")?,
        tenant_id: column(row, "tenant_id")?,
        conversation_id: column(row, "conversation_id")?,
        summary: column(row, "summary")?,
        summary_tokens: column(row, "summary_tokens")?,
        original_tokens: column(row, "original_tokens")?,
        first_message_id: column(row, "first_message_id")?,
        last_message_id: column(row, "last_message_id")?,
        created_at: column(row, "created_at")?,
        transport_policy: transport_policy_column(row)?,
    })
}

/// Decode one `user_facts` row.
///
/// A scope or predicate code the application no longer knows is rejected
/// rather than substituted with a default; `kind` and `source` parse
/// leniently because their enums carry a catch-all.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// or an internal error for an unknown scope or predicate code.
pub fn user_fact_from_row<R>(row: &R) -> AppResult<UserFact>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    f32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let scope_str: String = column(row, "scope")?;
    let kind_str: String = column(row, "kind")?;
    let scope = MemoryScope::parse(&scope_str)
        .ok_or_else(|| AppError::internal(format!("Invalid scope in user_facts: {scope_str}")))?;
    let pillar_str: Option<String> = column(row, "pillar")?;
    let code_str: String = column(row, "predicate_code")?;
    let predicate_code = PredicateCode::parse(&code_str).ok_or_else(|| {
        AppError::internal(format!("Invalid predicate_code in user_facts: {code_str}"))
    })?;
    let source_str: String = row
        .try_get("source")
        .unwrap_or_else(|_| "conversation".into());
    Ok(UserFact {
        id: column(row, "id")?,
        tenant_id: column(row, "tenant_id")?,
        user_id: column(row, "user_id")?,
        agent_id: column(row, "agent_id")?,
        scope,
        kind: FactKind::parse_lenient(&kind_str),
        pillar: pillar_str.as_deref().and_then(Pillar::parse),
        predicate_code,
        object: column(row, "object")?,
        confidence: column(row, "confidence")?,
        source: FactSource::parse_lenient(&source_str),
        valid_until: column(row, "valid_until")?,
        source_msg_id: column(row, "source_msg_id")?,
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
        transport_policy: transport_policy_column(row)?,
    })
}

/// Decode one `agent_notes` row. `suppressed` reads as unsuppressed when the
/// column cannot be decoded, matching the migration's default.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// or an internal error for an unknown scope.
pub fn agent_note_from_row<R>(row: &R) -> AppResult<AgentNote>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let scope_str: String = column(row, "scope")?;
    let scope = MemoryScope::parse(&scope_str)
        .ok_or_else(|| AppError::internal(format!("Invalid scope in coach_notes: {scope_str}")))?;
    let suppressed: bool = row.try_get("suppressed").unwrap_or(false);
    Ok(AgentNote {
        id: column(row, "id")?,
        tenant_id: column(row, "tenant_id")?,
        user_id: column(row, "user_id")?,
        agent_id: column(row, "agent_id")?,
        conversation_id: column(row, "conversation_id")?,
        scope,
        content: column(row, "content")?,
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
        suppressed,
        transport_policy: transport_policy_column(row)?,
    })
}

/// Decode one `agent_followups` row.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// or an internal error for an unknown status.
pub fn agent_followup_from_row<R>(row: &R) -> AppResult<AgentFollowup>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let status_str: String = column(row, "status")?;
    let status = FollowupStatus::parse(&status_str).ok_or_else(|| {
        AppError::internal(format!("Invalid status in coach_followups: {status_str}"))
    })?;
    Ok(AgentFollowup {
        id: column(row, "id")?,
        tenant_id: column(row, "tenant_id")?,
        user_id: column(row, "user_id")?,
        agent_id: column(row, "agent_id")?,
        conversation_id: column(row, "conversation_id")?,
        content: column(row, "content")?,
        due_at: column(row, "due_at")?,
        status,
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
        delivered_at: column(row, "delivered_at")?,
        transport_policy: transport_policy_column(row)?,
    })
}

/// Decode one `agent_sessions` row.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// or an internal error for an unknown status.
pub fn agent_session_from_row<R>(row: &R) -> AppResult<AgentSession>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let status_str: String = column(row, "status")?;
    let status = SessionStatus::parse(&status_str).ok_or_else(|| {
        AppError::internal(format!("Invalid status in coach_sessions: {status_str}"))
    })?;
    Ok(AgentSession {
        id: column(row, "id")?,
        tenant_id: column(row, "tenant_id")?,
        user_id: column(row, "user_id")?,
        agent_id: column(row, "agent_id")?,
        status,
        opened_at: column(row, "opened_at")?,
        last_turn_at: column(row, "last_turn_at")?,
        archived_at: column(row, "archived_at")?,
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
    })
}

/// Fold the two aggregate reads of [`COUNT_USER_FACTS_SQL`] and
/// [`COUNT_USER_FACTS_BY_KIND_SQL`] into a [`UserFactMetrics`].
///
/// `COUNT(*)` is non-nullable even over an empty table; `SUM(CASE …)` and
/// `MAX(…)` are NULL when no row matches, so those decode as `Option`.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub fn user_fact_metrics_from_rows<R>(aggregates: &R, kind_rows: &[R]) -> AppResult<UserFactMetrics>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let total: i64 = column(aggregates, "total")?;
    let distinct_users: i64 = column(aggregates, "distinct_users")?;
    let last_24h: Option<i64> = column(aggregates, "last_24h")?;
    let last_7d: Option<i64> = column(aggregates, "last_7d")?;
    let newest_updated_at: Option<DateTime<Utc>> = column(aggregates, "newest_updated_at")?;

    let mut facts_by_kind: BTreeMap<String, u64> = BTreeMap::new();
    for row in kind_rows {
        let kind: String = column(row, "kind")?;
        let count: i64 = column(row, "c")?;
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
