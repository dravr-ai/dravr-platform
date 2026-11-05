// ABOUTME: Repository trait for guardian_pending_actions — parked destructive tool calls awaiting /confirm
// ABOUTME: Single-use owner-checked claims with expiry at resolution; the Guardian Confirm HITL storage seam

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use serde::Serialize;

/// `tool_name` a parked agent draft carries in the pending-action store.
///
/// Deliberately not a registered tool: the claim path checks for it before
/// anything is dispatched, so a confirmed draft becomes an agent rather than a
/// tool call the executor would refuse. A draft is the athlete's own request, not
/// a Guardian decision, so the super admin surface neither lists nor denies it.
pub const AGENT_PROPOSAL_ACTION: &str = "coach.create_proposal";

/// A destructive tool call the Guardian parked pending user confirmation.
///
/// `arguments` is the tool-call JSON verbatim — stored to re-dispatch on
/// `/confirm`, never echoed to the user (it can carry the very injected
/// content the taint rule fired on). Ids are uuid-simple tokens (122-bit
/// entropy); ownership is still enforced on claim.
#[derive(Debug, Clone)]
pub struct PendingGuardianAction {
    /// Opaque uuid-simple claim token, surfaced to the user in the prompt.
    pub id: String,
    /// Stringified tenant of the dispatch that was parked.
    pub tenant_id: String,
    /// Stringified user the confirmation belongs to.
    pub user_id: String,
    /// Originating conversation, when the dispatch had one (chat surfaces).
    pub conversation_id: Option<String>,
    /// Registry identifier of the parked tool.
    pub tool_name: String,
    /// Tool-call arguments JSON, re-dispatched verbatim on confirm.
    pub arguments: serde_json::Value,
    /// The Guardian deny reason that triggered the park (`tainted_sink`).
    pub deny_reason: String,
    /// The agent the parked turn answered as, when it had one bound.
    pub turn_agent_id: Option<String>,
    /// The seat the parked turn ran under, as its stored word. `None` on a row
    /// that predates the column and on a parked agent draft, which never
    /// re-dispatches.
    pub seat: Option<String>,
    /// What the parked turn had served, as `Provenance::to_stored` wrote it, so a
    /// confirmed write answers for it. Opaque here. `None` on a parked agent draft,
    /// which never re-dispatches; `/confirm` treats a call without it as having
    /// served first-party-only data.
    pub turn_provenance: Option<String>,
}

/// Result of an atomic claim attempt on a pending action.
#[derive(Debug, Clone)]
pub enum ClaimOutcome {
    /// The caller won the single-use claim; the action payload follows.
    /// Boxed: the payload dwarfs the unit variants (`large_enum_variant`).
    Claimed(Box<PendingGuardianAction>),
    /// The row belongs to the caller but its TTL elapsed; it has been marked
    /// `expired`. The payload names what lapsed. Boxed like [`Self::Claimed`].
    Expired(Box<PendingGuardianAction>),
    /// No claimable row: unknown id, another user's row, or already resolved.
    /// Collapsed into one variant on purpose — distinguishing "someone
    /// else's id" from "unknown id" would let ids be probed for existence.
    NotFound,
}

/// Which rows a super admin asks to see, by what the row means now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingActionStatus {
    /// Waiting for the athlete, inside its TTL.
    Pending,
    /// The athlete confirmed it.
    Confirmed,
    /// The athlete or a super admin denied it.
    Denied,
    /// Its TTL elapsed, whether or not a claim has marked the row yet.
    Expired,
}

impl PendingActionStatus {
    /// The wire form, which is also the row's `status` word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Denied => "denied",
            Self::Expired => "expired",
        }
    }

    /// Parse the wire form.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "pending" => Some(Self::Pending),
            "confirmed" => Some(Self::Confirmed),
            "denied" => Some(Self::Denied),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }
}

/// What a super admin may see of a parked call.
///
/// No `arguments`, by construction: they are stored only to re-dispatch and can
/// carry the very injected content the taint rule fired on.
#[derive(Debug, Clone, Serialize)]
pub struct PendingActionSummary {
    /// The row's id.
    pub id: String,
    /// Stringified tenant of the dispatch that was parked.
    pub tenant_id: String,
    /// Stringified user the confirmation belongs to.
    pub user_id: String,
    /// Originating conversation, when the dispatch had one.
    pub conversation_id: Option<String>,
    /// Registry identifier of the parked tool.
    pub tool_name: String,
    /// The Guardian deny reason that triggered the park.
    pub deny_reason: String,
    /// What the row means now, as a [`PendingActionStatus`] word: a row past its
    /// TTL reads `expired` even before a claim marks it.
    pub status: &'static str,
    /// The agent the parked turn answered as, when it had one bound.
    pub turn_agent_id: Option<String>,
    /// The seat the parked turn ran under, as its stored word.
    pub seat: Option<String>,
    /// When the call was parked.
    pub created_at: DateTime<Utc>,
    /// When the wait ends.
    pub expires_at: DateTime<Utc>,
    /// When the row was resolved, if it has been.
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Persistent store behind the Guardian's Confirm human-in-the-loop flow.
#[async_trait]
pub trait GuardianPendingActionsRepository: Send + Sync {
    /// Park a destructive tool call until `expires_at`.
    async fn create_pending_action(
        &self,
        action: &PendingGuardianAction,
        expires_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Atomically claim a pending action for `user_id`/`tenant_id`, flipping
    /// `pending` → `resolution` (`confirmed` or `denied`). Single-use: of two
    /// concurrent claims exactly one wins. Expiry is checked here, at
    /// resolution time (the `short_links` pattern) — an elapsed row is marked
    /// `expired` and reported as [`ClaimOutcome::Expired`].
    async fn claim_pending_action(
        &self,
        id: &str,
        user_id: &str,
        tenant_id: &str,
        resolution: &str,
    ) -> AppResult<ClaimOutcome>;

    /// Delete rows whose TTL elapsed, returning how many were removed.
    ///
    /// Claims already filter expired rows, so this is storage hygiene only;
    /// it is invoked opportunistically when a new action is parked.
    async fn delete_expired_pending_actions(&self) -> AppResult<u64>;

    /// The parked calls a super admin may see, newest first: every tenant's unless
    /// `tenant_id` names one, and never an agent draft. The cross-tenant read is
    /// for global tokens only; the route enforces that.
    async fn list_pending_actions_for_super_admin(
        &self,
        tenant_id: Option<&str>,
        status: PendingActionStatus,
        limit: i64,
    ) -> AppResult<Vec<PendingActionSummary>>;

    /// Deny a call still waiting for its athlete, on a super admin's word. The same
    /// single-winner guard as a claim, so of this and a concurrent `/confirm`
    /// exactly one lands. `None` when no such row is waiting: unknown, resolved,
    /// elapsed, or an agent draft.
    async fn deny_pending_action_as_super_admin(
        &self,
        id: &str,
    ) -> AppResult<Option<PendingActionSummary>>;
}

/// Park a destructive call, pending confirmation.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends and cannot drift between
/// them. `resolved_at` binds a `DateTime<Utc>` on both: sqlx-sqlite encodes it
/// as `to_rfc3339_opts(AutoSi, false)`, which is what `to_rfc3339()` produces,
/// so the TEXT column keeps the bytes it always held while Postgres gets its
/// native `TIMESTAMPTZ`.
pub(crate) const INSERT_PENDING_ACTION_SQL: &str = r"
            INSERT INTO guardian_pending_actions
                (id, tenant_id, user_id, conversation_id, tool_name, arguments,
                 deny_reason, turn_agent_id, seat, turn_provenance, status, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'pending', $11)
            ";

/// The single-use claim: a guarded UPDATE that only one concurrent caller can
/// win. Owner + status + expiry are all in the WHERE so a stolen id, a replay,
/// and a stale row all fall through to 0 rows.
pub(crate) const CLAIM_PENDING_ACTION_SQL: &str = r"
            UPDATE guardian_pending_actions
            SET status = $1, resolved_at = $2
            WHERE id = $3 AND user_id = $4 AND tenant_id = $5
              AND status = 'pending' AND expires_at > $6
            ";

/// The row's payload, read back after a guarded UPDATE named it the caller's.
pub(crate) const READ_CLAIMED_ACTION_SQL: &str = r"
                SELECT tenant_id, user_id, conversation_id, tool_name, arguments, deny_reason,
                       turn_agent_id, seat, turn_provenance
                FROM guardian_pending_actions
                WHERE id = $1 AND user_id = $2 AND tenant_id = $3
                ";

/// Distinguish "owned but elapsed" (mark + report expired) from everything
/// else (unknown / foreign / already resolved → `NotFound`).
pub(crate) const EXPIRE_PENDING_ACTION_SQL: &str = r"
            UPDATE guardian_pending_actions
            SET status = 'expired', resolved_at = $1
            WHERE id = $2 AND user_id = $3 AND tenant_id = $4
              AND status = 'pending' AND expires_at <= $5
            ";

/// Storage hygiene: drop every elapsed row.
pub(crate) const SWEEP_PENDING_ACTIONS_SQL: &str =
    r"DELETE FROM guardian_pending_actions WHERE expires_at <= $1";

/// The columns of a [`PendingActionSummary`]. `arguments` is left out on purpose.
macro_rules! summary_columns {
    () => {
        "id, tenant_id, user_id, conversation_id, tool_name, deny_reason, status, \
         turn_agent_id, seat, created_at, expires_at, resolved_at"
    };
}

/// The super admin's view. One statement for every filter: an unnamed tenant is a
/// NULL bind, and `$3` selects by what the row means now, so a `pending` row past
/// its TTL lists as expired and never as waiting.
pub(crate) const LIST_PENDING_ACTIONS_SQL: &str = concat!(
    "SELECT ",
    summary_columns!(),
    " FROM guardian_pending_actions
      WHERE tool_name <> $1
        AND (CAST($2 AS TEXT) IS NULL OR tenant_id = $2)
        AND (($3 = 'pending' AND status = 'pending' AND expires_at > $4)
          OR ($3 = 'expired' AND (status = 'expired' OR (status = 'pending' AND expires_at <= $4)))
          OR ($3 IN ('confirmed', 'denied') AND status = $3))
      ORDER BY created_at DESC, id DESC
      LIMIT $5"
);

/// A super admin's denial: the claim's guard without the owner, and never a draft.
pub(crate) const DENY_PENDING_ACTION_SQL: &str = concat!(
    "UPDATE guardian_pending_actions
     SET status = 'denied', resolved_at = $1
     WHERE id = $2 AND status = 'pending' AND expires_at > $3 AND tool_name <> $4
     RETURNING ",
    summary_columns!()
);

/// Decode one row of the super admin's list, naming the column that cannot be decoded.
/// `created_at` and `resolved_at` are TEXT on `SQLite` and `TIMESTAMPTZ` on
/// Postgres, which `DateTime<Utc>` reads on both.
///
/// # Errors
/// Returns a database error naming the column, or on an unknown `status` word.
pub(crate) fn summary_from_row<R>(row: &R, now: i64) -> AppResult<PendingActionSummary>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |name: &str, e: sqlx::Error| {
        AppError::database(format!("guardian_pending_actions {name}: {e}"))
    };
    let stored_status: String = row.try_get("status").map_err(|e| column("status", e))?;
    let stored = PendingActionStatus::parse(&stored_status).ok_or_else(|| {
        AppError::database(format!(
            "guardian_pending_actions status: unknown word '{stored_status}'"
        ))
    })?;
    let expires_at: i64 = row
        .try_get("expires_at")
        .map_err(|e| column("expires_at", e))?;
    let status = if stored == PendingActionStatus::Pending && expires_at <= now {
        PendingActionStatus::Expired
    } else {
        stored
    };
    Ok(PendingActionSummary {
        id: row.try_get("id").map_err(|e| column("id", e))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column("tenant_id", e))?,
        user_id: row.try_get("user_id").map_err(|e| column("user_id", e))?,
        conversation_id: row
            .try_get("conversation_id")
            .map_err(|e| column("conversation_id", e))?,
        tool_name: row
            .try_get("tool_name")
            .map_err(|e| column("tool_name", e))?,
        deny_reason: row
            .try_get("deny_reason")
            .map_err(|e| column("deny_reason", e))?,
        status: status.as_str(),
        turn_agent_id: row
            .try_get("turn_agent_id")
            .map_err(|e| column("turn_agent_id", e))?,
        seat: row.try_get("seat").map_err(|e| column("seat", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column("created_at", e))?,
        expires_at: DateTime::from_timestamp(expires_at, 0).ok_or_else(|| {
            AppError::database(format!(
                "guardian_pending_actions expires_at: {expires_at} is out of range"
            ))
        })?,
        resolved_at: row
            .try_get("resolved_at")
            .map_err(|e| column("resolved_at", e))?,
    })
}

/// Extract the claimed row's payload via `try_get` (never `Row::get`, which
/// panics on a type/NULL surprise) so a corrupt row surfaces as a recoverable
/// error rather than a crash. Every column read here is TEXT by design on both
/// backends (see the migrations), so no native-UUID decode can bite.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded, or
/// when `arguments` does not parse as JSON.
pub(crate) fn action_from_row<R>(id: &str, row: &R) -> AppResult<PendingGuardianAction>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let get = |col: &str| -> AppResult<String> {
        row.try_get::<String, _>(col)
            .map_err(|e| AppError::database(format!("guardian_pending_actions {col}: {e}")))
    };
    let arguments_raw = get("arguments")?;
    let arguments = serde_json::from_str(&arguments_raw)
        .map_err(|e| AppError::database(format!("guardian_pending_actions arguments JSON: {e}")))?;
    let get_opt = |col: &str| -> AppResult<Option<String>> {
        row.try_get::<Option<String>, _>(col)
            .map_err(|e| AppError::database(format!("guardian_pending_actions {col}: {e}")))
    };
    Ok(PendingGuardianAction {
        id: id.to_owned(),
        tenant_id: get("tenant_id")?,
        user_id: get("user_id")?,
        conversation_id: get_opt("conversation_id")?,
        tool_name: get("tool_name")?,
        arguments,
        deny_reason: get("deny_reason")?,
        turn_agent_id: get_opt("turn_agent_id")?,
        seat: get_opt("seat")?,
        turn_provenance: get_opt("turn_provenance")?,
    })
}

/// Emit the whole [`GuardianPendingActionsRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes it
/// with its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion.
macro_rules! impl_guardian_pending_actions_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl GuardianPendingActionsRepository for $ty {
            async fn create_pending_action(
                &self,
                action: &PendingGuardianAction,
                expires_at: DateTime<Utc>,
            ) -> AppResult<()> {
                // Opportunistic hygiene: the parking path is rare (a tainted
                // destructive call under Confirm), so sweeping here keeps the
                // table bounded without a background job.
                self.delete_expired_pending_actions().await?;

                let arguments = serde_json::to_string(&action.arguments).map_err(|e| {
                    AppError::internal(format!("serialize pending-action arguments: {e}"))
                })?;
                sqlx::query(INSERT_PENDING_ACTION_SQL)
                    .bind(&action.id)
                    .bind(&action.tenant_id)
                    .bind(&action.user_id)
                    .bind(&action.conversation_id)
                    .bind(&action.tool_name)
                    .bind(arguments)
                    .bind(&action.deny_reason)
                    .bind(&action.turn_agent_id)
                    .bind(&action.seat)
                    .bind(&action.turn_provenance)
                    .bind(expires_at.timestamp())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to insert guardian_pending_action: {e}"))
                    })?;
                Ok(())
            }

            async fn claim_pending_action(
                &self,
                id: &str,
                user_id: &str,
                tenant_id: &str,
                resolution: &str,
            ) -> AppResult<ClaimOutcome> {
                if !matches!(resolution, "confirmed" | "denied") {
                    return Err(AppError::invalid_input(format!(
                        "invalid pending-action resolution '{resolution}' (confirmed|denied)"
                    )));
                }
                let now = Utc::now();

                let claimed = sqlx::query(CLAIM_PENDING_ACTION_SQL)
                    .bind(resolution)
                    .bind(now)
                    .bind(id)
                    .bind(user_id)
                    .bind(tenant_id)
                    .bind(now.timestamp())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim guardian_pending_action: {e}"))
                    })?;

                if claimed.rows_affected() == 1 {
                    let action = self.read_owned_action(id, user_id, tenant_id).await?;
                    return Ok(action.map_or(ClaimOutcome::NotFound, |action| {
                        ClaimOutcome::Claimed(Box::new(action))
                    }));
                }

                let expired = sqlx::query(EXPIRE_PENDING_ACTION_SQL)
                    .bind(now)
                    .bind(id)
                    .bind(user_id)
                    .bind(tenant_id)
                    .bind(now.timestamp())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to expire guardian_pending_action: {e}"))
                    })?;

                if expired.rows_affected() == 1 {
                    // Another athlete's park sweeps every elapsed row, so this one
                    // can be gone by now, and that answers as any row no longer here.
                    let action = self.read_owned_action(id, user_id, tenant_id).await?;
                    Ok(action.map_or(ClaimOutcome::NotFound, |action| {
                        ClaimOutcome::Expired(Box::new(action))
                    }))
                } else {
                    Ok(ClaimOutcome::NotFound)
                }
            }

            async fn delete_expired_pending_actions(&self) -> AppResult<u64> {
                let result = sqlx::query(SWEEP_PENDING_ACTIONS_SQL)
                    .bind(Utc::now().timestamp())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to sweep guardian_pending_actions: {e}"))
                    })?;
                Ok(result.rows_affected())
            }

            async fn list_pending_actions_for_super_admin(
                &self,
                tenant_id: Option<&str>,
                status: PendingActionStatus,
                limit: i64,
            ) -> AppResult<Vec<PendingActionSummary>> {
                let now = Utc::now().timestamp();
                let rows = sqlx::query(LIST_PENDING_ACTIONS_SQL)
                    .bind(AGENT_PROPOSAL_ACTION)
                    .bind(tenant_id)
                    .bind(status.as_str())
                    .bind(now)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list guardian_pending_actions: {e}"))
                    })?;
                rows.iter().map(|row| summary_from_row(row, now)).collect()
            }

            async fn deny_pending_action_as_super_admin(
                &self,
                id: &str,
            ) -> AppResult<Option<PendingActionSummary>> {
                let now = Utc::now();
                let row = sqlx::query(DENY_PENDING_ACTION_SQL)
                    .bind(now)
                    .bind(id)
                    .bind(now.timestamp())
                    .bind(AGENT_PROPOSAL_ACTION)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to deny guardian_pending_action: {e}"))
                    })?;
                row.as_ref()
                    .map(|row| summary_from_row(row, now.timestamp()))
                    .transpose()
            }
        }

        impl $ty {
            /// The payload of a row a guarded UPDATE just named the caller's, if the
            /// row is still there.
            async fn read_owned_action(
                &self,
                id: &str,
                user_id: &str,
                tenant_id: &str,
            ) -> AppResult<Option<PendingGuardianAction>> {
                let row = sqlx::query(READ_CLAIMED_ACTION_SQL)
                    .bind(id)
                    .bind(user_id)
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to read claimed guardian_pending_action: {e}"
                        ))
                    })?;
                row.map(|row| action_from_row(id, &row)).transpose()
            }
        }
    };
}
pub(crate) use impl_guardian_pending_actions_repository;
