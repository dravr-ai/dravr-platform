// ABOUTME: Repository trait for durable per-user onboarding step state (profile-type, connect, agent, messaging)
// ABOUTME: Server-driven completion that survives device changes, replacing the web flow's localStorage-only flags
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};

/// One persisted onboarding step state for a user.
///
/// A record exists only for a step the user has reached; a step with no record is
/// still pending. `chosen_channel` is set only on the messaging-channel step (the
/// messaging app the user picked); it is `None` for every other step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingStepRecord {
    /// Step identifier (`profile_type`, `connect_provider`, `agent_proposal`,
    /// `messaging_channel`, `messaging_configure`).
    pub step_id: String,
    /// `complete` or `skipped`.
    pub status: String,
    /// The messaging channel chosen at the `messaging_channel` step, if any.
    pub chosen_channel: Option<String>,
}

/// Durable onboarding progress, keyed by `(user_id, step_id)`.
///
/// Scoped by `user_id` — the auth-token-derived isolation boundary, mirroring the
/// provider-connection onboarding gate, which also reads by `user_id` alone.
/// `tenant_id` is a best-effort audit column (the onboarding JWT may carry no
/// active tenant), never a filter.
#[async_trait]
pub trait UserOnboardingRepository: Send + Sync {
    /// Upsert a step's `status` (and, for the messaging-channel step, the chosen
    /// channel) for a user. Re-marking an existing step overwrites its status.
    async fn set_onboarding_step(
        &self,
        user_id: &str,
        step_id: &str,
        status: &str,
        chosen_channel: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<()>;

    /// Every recorded step state for a user (rows exist only for reached steps).
    async fn get_onboarding_steps(&self, user_id: &str) -> AppResult<Vec<OnboardingStepRecord>>;
}

/// Upsert one step's state, stamping `updated_at` from the engine clock.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, and every bind is a plain `&str`/`Option<&str>`, so one statement
/// serves both backends and cannot drift between them. `CURRENT_TIMESTAMP` is
/// the portable spelling of the clock both engines already used under
/// different names.
pub(crate) const UPSERT_ONBOARDING_STEP_SQL: &str = r"
            INSERT INTO user_onboarding (user_id, step_id, status, chosen_channel, tenant_id, updated_at)
            VALUES ($1, $2, $3, $4, $5, CURRENT_TIMESTAMP)
            ON CONFLICT (user_id, step_id) DO UPDATE SET
                status = excluded.status,
                chosen_channel = excluded.chosen_channel,
                tenant_id = excluded.tenant_id,
                updated_at = CURRENT_TIMESTAMP
            ";

/// Read every recorded step for a user. Scoped by `user_id` alone, which is the
/// trait's documented isolation boundary.
pub(crate) const SELECT_ONBOARDING_STEPS_SQL: &str = r"
            SELECT step_id, status, chosen_channel
            FROM user_onboarding
            WHERE user_id = $1
            ";

/// Decode a row via `try_get` (never `Row::get`, which is `try_get().unwrap()`
/// and panics on a type/NULL surprise) so a corrupt row is a recoverable error.
///
/// Every column is TEXT on both engines, so these are plain `String` decodes
/// with no INT4-as-i64 or native-uuid trap, and one generic body serves both
/// drivers.
///
/// # Errors
/// Returns a database error when a column cannot be decoded.
pub(crate) fn step_record_from_row<R>(row: &R) -> AppResult<OnboardingStepRecord>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(OnboardingStepRecord {
        step_id: row
            .try_get::<String, _>("step_id")
            .map_err(|e| AppError::database(format!("user_onboarding step_id: {e}")))?,
        status: row
            .try_get::<String, _>("status")
            .map_err(|e| AppError::database(format!("user_onboarding status: {e}")))?,
        chosen_channel: row
            .try_get::<Option<String>, _>("chosen_channel")
            .map_err(|e| AppError::database(format!("user_onboarding chosen_channel: {e}")))?,
    })
}

/// Emit the whole [`UserOnboardingRepository`] implementation for one backend.
/// The body is written once here; each backend's shell invokes it with its own
/// type, and sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_user_onboarding_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl UserOnboardingRepository for $ty {
            async fn set_onboarding_step(
                &self,
                user_id: &str,
                step_id: &str,
                status: &str,
                chosen_channel: Option<&str>,
                tenant_id: Option<&str>,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_ONBOARDING_STEP_SQL)
                    .bind(user_id)
                    .bind(step_id)
                    .bind(status)
                    .bind(chosen_channel)
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert user_onboarding: {e}"))
                    })?;

                Ok(())
            }

            async fn get_onboarding_steps(
                &self,
                user_id: &str,
            ) -> AppResult<Vec<OnboardingStepRecord>> {
                let rows = sqlx::query(SELECT_ONBOARDING_STEPS_SQL)
                    .bind(user_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read user_onboarding: {e}"))
                    })?;

                rows.iter().map(step_record_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_user_onboarding_repository;
