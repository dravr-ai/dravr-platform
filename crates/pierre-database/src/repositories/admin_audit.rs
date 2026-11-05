// ABOUTME: Repository trait for the admin audit trail (audit_events): one row per operator write, read newest first
// ABOUTME: Shared body for both backends; the actor is the only id column, a target lives in the metadata

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The admin audit trail.
//!
//! Every write an operator makes through the admin API — the console, or
//! `pierre-cli` over the same routes — and every database write `pierre-cli`
//! makes without the API is one row of `audit_events` (carnet#1391). The table
//! predates its writer; both schemas carry it with the same columns.
//!
//! `user_id` is the **actor**, the operator who acted, and it is
//! `ON DELETE SET NULL`: deleting an operator account keeps the trail of what
//! was done and drops who did it. The target of an action (a user, an email, a
//! token) lives in `metadata`. A deleted account's rows in that metadata are
//! removed by the account deletion itself (`user_references`), so the trail
//! never outlives the deletion policy.

use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How an audited action ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    /// The write was applied (2xx).
    Success,
    /// The server refused it and changed nothing: permission, a missing
    /// target, a conflict, invalid input (4xx).
    Refused,
    /// It failed on the server's side (5xx); part of it may have applied.
    Failed,
}

impl AuditOutcome {
    /// The outcome an HTTP status reports.
    #[must_use]
    pub const fn from_status(status: u16) -> Self {
        match status {
            200..=399 => Self::Success,
            400..=499 => Self::Refused,
            _ => Self::Failed,
        }
    }

    /// The stored form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Refused => "refused",
            Self::Failed => "failed",
        }
    }
}

/// Where an audited action came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditSource {
    /// The admin web console (a signed-in admin's session).
    Console,
    /// `pierre-cli` over the admin API (a device-login token).
    Cli,
    /// `pierre-cli` writing the database directly (`--database-url`).
    CliDatabase,
    /// A service token: a scheduled job, a partner integration.
    Service,
}

impl AuditSource {
    /// The stored form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Console => "console",
            Self::Cli => "cli",
            Self::CliDatabase => "cli_database",
            Self::Service => "service",
        }
    }
}

/// One audited operator action, as written.
#[derive(Debug, Clone)]
pub struct NewAuditEvent {
    /// What was done: a handler's action key (`user.delete`), or the method
    /// and route template when the handler named none (`DELETE /admin/...`).
    pub action: String,
    /// One line an operator reads in the trail.
    pub summary: String,
    /// How it ended.
    pub outcome: AuditOutcome,
    /// Where it came from.
    pub source: AuditSource,
    /// The operator who acted, when the credential names one.
    pub actor_user_id: Option<Uuid>,
    /// The tenant the credential acted in; `None` for a platform-wide one.
    pub tenant_id: Option<String>,
    /// The caller's address, as the edge reported it.
    pub ip_address: Option<String>,
    /// The caller's user agent.
    pub user_agent: Option<String>,
    /// Everything else: route, status, target, before and after.
    pub metadata: serde_json::Value,
}

/// One row of the trail, as read back for an operator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEventRecord {
    /// Row id.
    pub id: String,
    /// When it happened.
    pub occurred_at: DateTime<Utc>,
    /// What was done (see [`NewAuditEvent::action`]).
    pub action: String,
    /// The line an operator reads.
    pub summary: String,
    /// `success`, `refused` or `failed`.
    pub outcome: String,
    /// `console`, `cli`, `cli_database` or `service`.
    pub source: String,
    /// The operator who acted; `None` once that account is deleted, or for a
    /// service token.
    pub actor_user_id: Option<String>,
    /// The acting operator's email, while the account exists.
    pub actor_email: Option<String>,
    /// The tenant the credential acted in.
    pub tenant_id: Option<String>,
    /// Route, status, target, before and after.
    pub metadata: serde_json::Value,
}

/// What a trail read selects. Every field narrows; `None` leaves it open.
#[derive(Debug, Clone, Default)]
pub struct AuditEventFilter {
    /// Only this tenant's rows (a tenant-scoped reader always sets it).
    pub tenant_id: Option<String>,
    /// Only rows strictly older than this instant: the next page's cursor.
    pub before: Option<DateTime<Utc>>,
    /// Only actions starting with this text (`user.`, `DELETE`).
    pub action_prefix: Option<String>,
    /// Only this outcome.
    pub outcome: Option<AuditOutcome>,
    /// Only this operator's actions.
    pub actor_user_id: Option<Uuid>,
}

/// The most rows one read returns.
pub const AUDIT_PAGE_MAX: u32 = 200;

/// The admin audit trail store.
#[async_trait]
pub trait AdminAuditRepository: Send + Sync {
    /// Append one action.
    ///
    /// # Errors
    /// Returns a database error when the row cannot be written.
    async fn record(&self, event: &NewAuditEvent) -> AppResult<String>;

    /// Up to `limit` rows (capped at [`AUDIT_PAGE_MAX`]) the filter selects,
    /// newest first.
    ///
    /// # Errors
    /// Returns a database error when the read fails or a row does not decode.
    async fn list(&self, filter: &AuditEventFilter, limit: u32)
        -> AppResult<Vec<AuditEventRecord>>;
}

/// Append one row. `severity` is the column the table was born with; the
/// trail stores the outcome's severity in it (`info`, `warning`, `error`).
pub(crate) const INSERT_AUDIT_EVENT_SQL: &str = r"
            INSERT INTO audit_events
                (id, event_type, severity, message, source, result, tenant_id,
                 user_id, ip_address, user_agent, metadata, timestamp)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ";

/// The filtered page, newest first, with the actor's email while the account
/// exists. Each `$n IS NULL OR …` leaves a filter open when it binds NULL; the
/// action prefix is an [`action_prefix_pattern`], escaped with `\`.
pub(crate) const LIST_AUDIT_EVENTS_SQL: &str = r"
            SELECT a.id, a.timestamp, a.event_type, a.message, a.result, a.source,
                   a.user_id, u.email AS actor_email, a.tenant_id, a.metadata
            FROM audit_events a
            LEFT JOIN users u ON u.id = a.user_id
            WHERE ($1 IS NULL OR a.tenant_id = $1)
              AND ($2 IS NULL OR a.timestamp < $2)
              AND ($3 IS NULL OR a.event_type LIKE $3 ESCAPE '\')
              AND ($4 IS NULL OR a.result = $4)
              AND ($5 IS NULL OR a.user_id = $5)
            ORDER BY a.timestamp DESC, a.id DESC
            LIMIT $6
            ";

/// The severity column's value for an outcome.
#[must_use]
pub const fn severity_for(outcome: AuditOutcome) -> &'static str {
    match outcome {
        AuditOutcome::Success => "info",
        AuditOutcome::Refused => "warning",
        AuditOutcome::Failed => "error",
    }
}

/// A `LIKE … ESCAPE '\'` pattern matching every action that starts with
/// `prefix`, its own wildcards escaped so `pre_approval.` matches itself and
/// `user_` does not match `userX`.
#[must_use]
pub fn action_prefix_pattern(prefix: &str) -> String {
    let mut pattern = String::with_capacity(prefix.len() + 1);
    for c in prefix.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

/// A database error naming the trail column that would not decode.
pub(crate) fn audit_column_error(col: &str, e: impl Display) -> AppError {
    AppError::database(format!("audit_events {col}: {e}"))
}

/// Emit the whole [`AdminAuditRepository`] implementation for one backend:
/// `$ty` the database type, `$ids` its uuid codec (`TextUuid` or
/// `NativeUuid`). The statements, the binds and the row decode are written
/// once here; the invoking shell brings `DateTime`, `Utc`, `Json`, `Row`,
/// `Uuid` and `Value` into scope.
macro_rules! impl_admin_audit_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl AdminAuditRepository for $ty {
            async fn record(&self, event: &NewAuditEvent) -> AppResult<String> {
                let id = Uuid::new_v4().to_string();
                sqlx::query(INSERT_AUDIT_EVENT_SQL)
                    .bind(&id)
                    .bind(&event.action)
                    .bind(severity_for(event.outcome))
                    .bind(&event.summary)
                    .bind(event.source.as_str())
                    .bind(event.outcome.as_str())
                    .bind(&event.tenant_id)
                    .bind($ids::bind_opt(event.actor_user_id))
                    .bind(&event.ip_address)
                    .bind(&event.user_agent)
                    .bind(Json(&event.metadata))
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record an audit event: {e}"))
                    })?;
                Ok(id)
            }

            async fn list(
                &self,
                filter: &AuditEventFilter,
                limit: u32,
            ) -> AppResult<Vec<AuditEventRecord>> {
                let rows = sqlx::query(LIST_AUDIT_EVENTS_SQL)
                    .bind(&filter.tenant_id)
                    .bind(filter.before)
                    .bind(filter.action_prefix.as_deref().map(action_prefix_pattern))
                    .bind(filter.outcome.map(AuditOutcome::as_str))
                    .bind($ids::bind_opt(filter.actor_user_id))
                    .bind(i64::from(limit.clamp(1, AUDIT_PAGE_MAX)))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read the audit trail: {e}"))
                    })?;
                rows.iter()
                    .map(|row| {
                        let text = |col: &str| -> AppResult<String> {
                            row.try_get::<String, _>(col)
                                .map_err(|e| audit_column_error(col, e))
                        };
                        let opt = |col: &str| -> AppResult<Option<String>> {
                            row.try_get::<Option<String>, _>(col)
                                .map_err(|e| audit_column_error(col, e))
                        };
                        let metadata = row
                            .try_get::<Option<Json<Value>>, _>("metadata")
                            .map_err(|e| audit_column_error("metadata", e))?
                            .map_or(Value::Null, |json| json.0);
                        Ok(AuditEventRecord {
                            id: text("id")?,
                            occurred_at: row
                                .try_get::<DateTime<Utc>, _>("timestamp")
                                .map_err(|e| audit_column_error("timestamp", e))?,
                            action: text("event_type")?,
                            summary: text("message")?,
                            outcome: text("result")?,
                            source: text("source")?,
                            actor_user_id: $ids::read_text_opt(row, "user_id")?,
                            actor_email: opt("actor_email")?,
                            tenant_id: opt("tenant_id")?,
                            metadata,
                        })
                    })
                    .collect()
            }
        }
    };
}

pub(crate) use impl_admin_audit_repository;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_follows_the_status_class() {
        for (status, outcome) in [
            (200, AuditOutcome::Success),
            (204, AuditOutcome::Success),
            (399, AuditOutcome::Success),
            (400, AuditOutcome::Refused),
            (403, AuditOutcome::Refused),
            (409, AuditOutcome::Refused),
            (499, AuditOutcome::Refused),
            (500, AuditOutcome::Failed),
        ] {
            assert_eq!(
                AuditOutcome::from_status(status),
                outcome,
                "bug: status {status} maps to the wrong outcome"
            );
        }
    }

    #[test]
    fn action_prefix_escapes_wildcards() {
        assert_eq!(
            action_prefix_pattern("user."),
            "user.%",
            "a prefix with no wildcard is itself plus `%`"
        );
        assert_eq!(
            action_prefix_pattern("pre_approval."),
            r"pre\_approval.%",
            "`_` is escaped, not dropped: the stored key keeps it"
        );
        assert_eq!(
            action_prefix_pattern(r"a%b\"),
            r"a\%b\\%",
            "`%` and the escape character itself are escaped"
        );
    }
}
