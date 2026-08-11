// ABOUTME: Postgres-backed GuardianPendingActionsRepository — parked destructive tool calls awaiting /confirm
// ABOUTME: Mirrors the SQLite impl (TEXT ids, epoch expiry) with $n placeholders and TIMESTAMPTZ resolved_at

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::Row;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::{ClaimOutcome, GuardianPendingActionsRepository, PendingGuardianAction};
use pierre_core::errors::{AppError, AppResult};

/// Extract the claimed row's payload via `try_get` so a corrupt row surfaces
/// as a recoverable error rather than a crash. Every column is TEXT by
/// design (see the migration), so no native-UUID decode can bite here.
fn row_to_action(id: &str, row: &PgRow) -> AppResult<PendingGuardianAction> {
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

#[async_trait]
impl GuardianPendingActionsRepository for PostgresDatabase {
    async fn create_pending_action(
        &self,
        action: &PendingGuardianAction,
        expires_at: DateTime<Utc>,
    ) -> AppResult<()> {
        self.delete_expired_pending_actions().await?;

        let arguments = serde_json::to_string(&action.arguments)
            .map_err(|e| AppError::internal(format!("serialize pending-action arguments: {e}")))?;
        sqlx::query(
            r"
            INSERT INTO guardian_pending_actions
                (id, tenant_id, user_id, conversation_id, tool_name, arguments,
                 deny_reason, status, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8)
            ",
        )
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

        let claimed = sqlx::query(
            r"
            UPDATE guardian_pending_actions
            SET status = $1, resolved_at = $2
            WHERE id = $3 AND user_id = $4 AND tenant_id = $5
              AND status = 'pending' AND expires_at > $6
            ",
        )
        .bind(resolution)
        .bind(now)
        .bind(id)
        .bind(user_id)
        .bind(tenant_id)
        .bind(now.timestamp())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to claim guardian_pending_action: {e}")))?;

        if claimed.rows_affected() == 1 {
            let row = sqlx::query(
                r"
                SELECT tenant_id, user_id, conversation_id, tool_name, arguments, deny_reason
                FROM guardian_pending_actions
                WHERE id = $1
                ",
            )
            .bind(id)
            .fetch_one(self.pool())
            .await
            .map_err(|e| {
                AppError::database(format!(
                    "Failed to read claimed guardian_pending_action: {e}"
                ))
            })?;
            return Ok(ClaimOutcome::Claimed(Box::new(row_to_action(id, &row)?)));
        }

        let expired = sqlx::query(
            r"
            UPDATE guardian_pending_actions
            SET status = 'expired', resolved_at = $1
            WHERE id = $2 AND user_id = $3 AND tenant_id = $4
              AND status = 'pending' AND expires_at <= $5
            ",
        )
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
        let result = sqlx::query(r"DELETE FROM guardian_pending_actions WHERE expires_at <= $1")
            .bind(Utc::now().timestamp())
            .execute(self.pool())
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to sweep guardian_pending_actions: {e}"))
            })?;
        Ok(result.rows_affected())
    }
}
