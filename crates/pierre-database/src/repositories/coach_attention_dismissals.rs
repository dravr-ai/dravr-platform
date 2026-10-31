// ABOUTME: Repository trait, statements and shared body for a coach's "Traité" on Coach Home attention reasons
// ABOUTME: Tenant-scoped rows keyed by (tenant, coach, athlete, reason) with the signal fingerprint; emitted per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach attention dismissals (carnet#1252).
//!
//! When a coach marks one of an athlete's attention reasons handled, the
//! reason's id and the fingerprint of the signal it was raised on are kept
//! here. The queue reads them back and shows the reason as handled while its
//! fingerprint is unchanged; a changed signal brings it back without anything
//! being deleted. Restoring a reason deletes its row.
//!
//! The table is tenant-scoped: `tenant_id` is the athlete's roster tenant,
//! and every statement filters on it together with the coach.

use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// One dismissed reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionDismissal {
    /// The athlete the reason is about.
    pub athlete_user_id: Uuid,
    /// The reason's id on that athlete.
    pub reason_id: String,
    /// The signal it was dismissed on.
    pub fingerprint: String,
    /// When the coach dismissed it.
    pub dismissed_at: DateTime<Utc>,
}

/// Which reason a dismissal names.
#[derive(Debug, Clone, Copy)]
pub struct AttentionDismissalKey<'reason> {
    /// The athlete's roster tenant.
    pub tenant: TenantId,
    /// The coach.
    pub coach_user_id: Uuid,
    /// The athlete.
    pub athlete_user_id: Uuid,
    /// The reason's id on that athlete.
    pub reason_id: &'reason str,
}

/// Read and write a coach's dismissed attention reasons.
#[async_trait]
pub trait CoachAttentionDismissalRepository: Send + Sync {
    /// Mark the reason handled for `fingerprint`, replacing any earlier
    /// dismissal of the same reason.
    async fn dismiss_attention(
        &self,
        key: AttentionDismissalKey<'_>,
        fingerprint: &str,
        dismissed_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Undo a dismissal. Returns whether there was one.
    async fn restore_attention(&self, key: AttentionDismissalKey<'_>) -> AppResult<bool>;

    /// Every reason the coach dismissed for the athletes of `tenant`.
    async fn list_attention_dismissals(
        &self,
        tenant: TenantId,
        coach_user_id: Uuid,
    ) -> AppResult<Vec<AttentionDismissal>>;
}

/// Insert a dismissal, or move an existing one to the new signal.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends. The user ids bind
/// through the backend's uuid codec (see [`super::uuid_columns`]); the
/// tenant is `TEXT` on both.
pub(crate) const DISMISS_ATTENTION_SQL: &str = r"
            INSERT INTO coach_attention_dismissals
                (id, tenant_id, coach_user_id, athlete_user_id, reason_id, fingerprint, dismissed_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (tenant_id, coach_user_id, athlete_user_id, reason_id) DO UPDATE SET
                fingerprint = EXCLUDED.fingerprint,
                dismissed_at = EXCLUDED.dismissed_at
            ";

/// Delete one dismissal.
pub(crate) const RESTORE_ATTENTION_SQL: &str = r"
            DELETE FROM coach_attention_dismissals
            WHERE tenant_id = $1 AND coach_user_id = $2 AND athlete_user_id = $3 AND reason_id = $4
            ";

/// The coach's dismissals under one tenant.
pub(crate) const LIST_ATTENTION_DISMISSALS_SQL: &str = r"
            SELECT athlete_user_id, reason_id, fingerprint, dismissed_at
            FROM coach_attention_dismissals
            WHERE tenant_id = $1 AND coach_user_id = $2
            ";

/// The error for a read or write of this table that failed.
pub(crate) fn dismissal_error(action: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to {action} coach attention dismissal: {e}"))
}

/// Emit the whole [`CoachAttentionDismissalRepository`] implementation for
/// one backend type. `$ids` is the codec in [`super::uuid_columns`] for how
/// that backend's user id columns bind and read.
macro_rules! impl_coach_attention_dismissal_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl CoachAttentionDismissalRepository for $ty {
            async fn dismiss_attention(
                &self,
                key: AttentionDismissalKey<'_>,
                fingerprint: &str,
                dismissed_at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(DISMISS_ATTENTION_SQL)
                    .bind($ids::bind(Uuid::new_v4()))
                    .bind(key.tenant.to_string())
                    .bind($ids::bind(key.coach_user_id))
                    .bind($ids::bind(key.athlete_user_id))
                    .bind(key.reason_id)
                    .bind(fingerprint)
                    .bind(dismissed_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| dismissal_error("store", e))?;
                Ok(())
            }

            async fn restore_attention(&self, key: AttentionDismissalKey<'_>) -> AppResult<bool> {
                let done = sqlx::query(RESTORE_ATTENTION_SQL)
                    .bind(key.tenant.to_string())
                    .bind($ids::bind(key.coach_user_id))
                    .bind($ids::bind(key.athlete_user_id))
                    .bind(key.reason_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| dismissal_error("delete", e))?;
                Ok(done.rows_affected() > 0)
            }

            async fn list_attention_dismissals(
                &self,
                tenant: TenantId,
                coach_user_id: Uuid,
            ) -> AppResult<Vec<AttentionDismissal>> {
                let rows = sqlx::query(LIST_ATTENTION_DISMISSALS_SQL)
                    .bind(tenant.to_string())
                    .bind($ids::bind(coach_user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| dismissal_error("read", e))?;
                rows.iter()
                    .map(|row| {
                        Ok(AttentionDismissal {
                            athlete_user_id: $ids::read(row, "athlete_user_id")?,
                            reason_id: row
                                .try_get("reason_id")
                                .map_err(|e| dismissal_error("decode", e))?,
                            fingerprint: row
                                .try_get("fingerprint")
                                .map_err(|e| dismissal_error("decode", e))?,
                            dismissed_at: row
                                .try_get("dismissed_at")
                                .map_err(|e| dismissal_error("decode", e))?,
                        })
                    })
                    .collect()
            }
        }
    };
}
pub(crate) use impl_coach_attention_dismissal_repository;
