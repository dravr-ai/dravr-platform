// ABOUTME: Repository trait for guardian_pending_actions — parked destructive tool calls awaiting /confirm
// ABOUTME: Single-use owner-checked claims with expiry at resolution; the Guardian Confirm HITL storage seam

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

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
}

/// Result of an atomic claim attempt on a pending action.
#[derive(Debug, Clone)]
pub enum ClaimOutcome {
    /// The caller won the single-use claim; the action payload follows.
    /// Boxed: the payload dwarfs the unit variants (`large_enum_variant`).
    Claimed(Box<PendingGuardianAction>),
    /// The row exists and belongs to the caller but its TTL elapsed; it has
    /// been marked `expired`.
    Expired,
    /// No claimable row: unknown id, another user's row, or already resolved.
    /// Collapsed into one variant on purpose — distinguishing "someone
    /// else's id" from "unknown id" would let ids be probed for existence.
    NotFound,
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
                 deny_reason, status, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8)
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

/// The claimed row's payload, read back after the guarded UPDATE won.
pub(crate) const READ_CLAIMED_ACTION_SQL: &str = r"
                SELECT tenant_id, user_id, conversation_id, tool_name, arguments, deny_reason
                FROM guardian_pending_actions
                WHERE id = $1
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
    Ok(PendingGuardianAction {
        id: id.to_owned(),
        tenant_id: get("tenant_id")?,
        user_id: get("user_id")?,
        conversation_id: row
            .try_get::<Option<String>, _>("conversation_id")
            .map_err(|e| {
                AppError::database(format!("guardian_pending_actions conversation_id: {e}"))
            })?,
        tool_name: get("tool_name")?,
        arguments,
        deny_reason: get("deny_reason")?,
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
                    let row = sqlx::query(READ_CLAIMED_ACTION_SQL)
                        .bind(id)
                        .fetch_one(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to read claimed guardian_pending_action: {e}"
                            ))
                        })?;
                    return Ok(ClaimOutcome::Claimed(Box::new(action_from_row(id, &row)?)));
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
                    Ok(ClaimOutcome::Expired)
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
        }
    };
}
pub(crate) use impl_guardian_pending_actions_repository;
