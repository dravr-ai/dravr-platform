// ABOUTME: Repository trait for the new-user cohort the admin console and the daily digest read
// ABOUTME: Each account created in a window, with the first time it reached each activation milestone

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The accounts created in a window, and how far each one got.
//!
//! Two scopes, mirroring
//! [`UserRepository::get_by_status`](super::users::UserRepository::get_by_status):
//!
//! - **Platform-wide** (`tenant = None`): every account created in the window,
//!   whatever tenant it belongs to. Like `USERS_BY_STATUS_SQL`, this is the
//!   super-admin's read: the route passes `None` for a super-admin only, and
//!   refuses any other admin that acts in no tenant.
//! - **One tenant** (`tenant = Some`): the accounts that are members of that
//!   tenant (`tenant_users`), and every milestone read inside that same
//!   tenant — a provider connected, data cached, a conversation held in
//!   another tenant does not count here.
//!
//! The SQL is portable: no engine date arithmetic. A user id that is `uuid`
//! on Postgres but `TEXT` in another table is compared through
//! `CAST(u.id AS TEXT)`, which both engines accept; the per-user "first week"
//! arithmetic is left to the caller, which has both instants.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use uuid::Uuid;

/// One account created in the window, with the first time it reached each
/// activation milestone (`None` when it has not yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUserMilestones {
    /// The account.
    pub user_id: Uuid,
    /// The address it signs in with.
    pub email: String,
    /// The name it chose, if any.
    pub display_name: Option<String>,
    /// When it was created.
    pub signed_up_at: DateTime<Utc>,
    /// How it signed up: `email`, `google`, `apple`, …
    pub auth_provider: String,
    /// `pending`, `active` or `suspended`.
    pub status: String,
    /// When an operator approved it, or it was admitted.
    pub approved_at: Option<DateTime<Utc>>,
    /// When it proved its address.
    pub email_verified_at: Option<DateTime<Utc>>,
    /// When it was last seen by the server.
    pub last_active_at: Option<DateTime<Utc>>,
    /// Its first provider connection (seeded synthetic connections excluded).
    pub provider_connected_at: Option<DateTime<Utc>>,
    /// When its first provider activity was cached.
    pub first_activity_at: Option<DateTime<Utc>>,
    /// The earliest completed health sync on record. `sync_state` keeps only
    /// each provider's latest sync, so this is an upper bound on the first.
    pub first_health_sync_at: Option<DateTime<Utc>>,
    /// The first message it sent in a conversation it owns.
    pub first_turn_at: Option<DateTime<Utc>>,
}

/// One message an account in the window sent in a conversation it owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewUserTurn {
    /// Who sent it.
    pub user_id: Uuid,
    /// When.
    pub sent_at: DateTime<Utc>,
}

/// The new-user cohort, read for the admin console and the daily digest.
#[async_trait]
pub trait SignupCohortRepository: Send + Sync {
    /// Accounts created at or after `since`, newest first, at most `limit`,
    /// each with its milestones. `tenant` scopes the read as the module docs
    /// describe; `None` is the super-admin's platform-wide read.
    async fn new_user_milestones(
        &self,
        since: DateTime<Utc>,
        tenant: Option<TenantId>,
        limit: i64,
    ) -> AppResult<Vec<NewUserMilestones>>;

    /// Every message the accounts created at or after `since` sent, since
    /// then, in conversations they own — the caller counts each one's first
    /// week. Scoped like [`Self::new_user_milestones`].
    async fn new_user_turns(
        &self,
        since: DateTime<Utc>,
        tenant: Option<TenantId>,
    ) -> AppResult<Vec<NewUserTurn>>;
}

/// Platform-wide: every account created at or after `$1`, newest first, at
/// most `$2`. Super-admin only, like `USERS_BY_STATUS_SQL`.
pub(crate) const NEW_USER_MILESTONES_SQL: &str = r"
            SELECT
                u.id,
                u.email,
                u.display_name,
                u.created_at,
                u.auth_provider,
                COALESCE(u.user_status, 'active') AS user_status,
                u.approved_at,
                u.email_verified_at,
                u.last_active,
                (SELECT MIN(pc.connected_at) FROM provider_connections pc
                    WHERE pc.user_id = CAST(u.id AS TEXT)
                      AND pc.connection_type <> 'synthetic') AS provider_connected_at,
                (SELECT MIN(ca.created_at) FROM cached_activities ca
                    WHERE ca.user_id = CAST(u.id AS TEXT)) AS first_activity_at,
                (SELECT MIN(ss.last_sync_at) FROM sync_state ss
                    WHERE ss.user_id = CAST(u.id AS TEXT)
                      AND ss.last_sync_status = 'completed') AS first_health_sync_at,
                (SELECT MIN(m.created_at) FROM chat_messages m
                    INNER JOIN chat_conversations c ON c.id = m.conversation_id
                    WHERE c.user_id = u.id AND m.role = 'user') AS first_turn_at
            FROM users u
            WHERE u.created_at >= $1
            ORDER BY u.created_at DESC
            LIMIT $2
            ";

/// One tenant: the accounts created at or after `$1` that are members of
/// tenant `$3` (bound through the uuid codec), at most `$2`, every milestone
/// read in that tenant (`$4`, the same tenant as text, for the `TEXT` tenant
/// columns).
pub(crate) const NEW_USER_MILESTONES_IN_TENANT_SQL: &str = r"
            SELECT
                u.id,
                u.email,
                u.display_name,
                u.created_at,
                u.auth_provider,
                COALESCE(u.user_status, 'active') AS user_status,
                u.approved_at,
                u.email_verified_at,
                u.last_active,
                (SELECT MIN(pc.connected_at) FROM provider_connections pc
                    WHERE pc.user_id = CAST(u.id AS TEXT)
                      AND pc.tenant_id = $4
                      AND pc.connection_type <> 'synthetic') AS provider_connected_at,
                (SELECT MIN(ca.created_at) FROM cached_activities ca
                    WHERE ca.user_id = CAST(u.id AS TEXT)
                      AND ca.tenant_id = $4) AS first_activity_at,
                (SELECT MIN(ss.last_sync_at) FROM sync_state ss
                    WHERE ss.user_id = CAST(u.id AS TEXT)
                      AND ss.tenant_id = $4
                      AND ss.last_sync_status = 'completed') AS first_health_sync_at,
                (SELECT MIN(m.created_at) FROM chat_messages m
                    INNER JOIN chat_conversations c ON c.id = m.conversation_id
                    WHERE c.user_id = u.id AND c.tenant_id = $4
                      AND m.role = 'user') AS first_turn_at
            FROM users u
            INNER JOIN tenant_users tu ON tu.user_id = u.id AND tu.tenant_id = $3
            WHERE u.created_at >= $1
            ORDER BY u.created_at DESC
            LIMIT $2
            ";

/// Platform-wide: the messages the accounts created at or after `$1` sent
/// since then in conversations they own. Super-admin only.
pub(crate) const NEW_USER_TURNS_SQL: &str = r"
            SELECT c.user_id AS user_id, m.created_at AS sent_at
            FROM chat_messages m
            INNER JOIN chat_conversations c ON c.id = m.conversation_id
            INNER JOIN users u ON u.id = c.user_id
            WHERE m.role = 'user'
              AND u.created_at >= $1
              AND m.created_at >= $1
            ";

/// One tenant: the same messages for the members of tenant `$2` (uuid codec),
/// held in that tenant's conversations (`$3`, the tenant as text).
pub(crate) const NEW_USER_TURNS_IN_TENANT_SQL: &str = r"
            SELECT c.user_id AS user_id, m.created_at AS sent_at
            FROM chat_messages m
            INNER JOIN chat_conversations c ON c.id = m.conversation_id
            INNER JOIN users u ON u.id = c.user_id
            INNER JOIN tenant_users tu ON tu.user_id = u.id AND tu.tenant_id = $2
            WHERE m.role = 'user'
              AND c.tenant_id = $3
              AND u.created_at >= $1
              AND m.created_at >= $1
            ";

/// Emit the whole [`SignupCohortRepository`] implementation for one backend
/// type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]): `TEXT` ids on `SQLite`, native
/// `uuid` on Postgres.
macro_rules! impl_signup_cohort_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl SignupCohortRepository for $ty {
            async fn new_user_milestones(
                &self,
                since: DateTime<Utc>,
                tenant: Option<TenantId>,
                limit: i64,
            ) -> AppResult<Vec<NewUserMilestones>> {
                let rows = match tenant {
                    None => {
                        sqlx::query(NEW_USER_MILESTONES_SQL)
                            .bind(since)
                            .bind(limit)
                            .fetch_all(self.pool())
                            .await
                    }
                    Some(tenant) => {
                        sqlx::query(NEW_USER_MILESTONES_IN_TENANT_SQL)
                            .bind(since)
                            .bind(limit)
                            .bind($ids::bind(tenant.as_uuid()))
                            .bind(tenant.to_string())
                            .fetch_all(self.pool())
                            .await
                    }
                }
                .map_err(|e| AppError::database(format!("Failed to read new users: {e}")))?;

                let column =
                    |e: sqlx::Error| AppError::database(format!("Invalid new-user column: {e}"));
                rows.iter()
                    .map(|row| {
                        Ok(NewUserMilestones {
                            user_id: $ids::read(row, "id")?,
                            email: row.try_get("email").map_err(column)?,
                            display_name: row.try_get("display_name").map_err(column)?,
                            signed_up_at: row.try_get("created_at").map_err(column)?,
                            auth_provider: row.try_get("auth_provider").map_err(column)?,
                            status: row.try_get("user_status").map_err(column)?,
                            approved_at: row.try_get("approved_at").map_err(column)?,
                            email_verified_at: row.try_get("email_verified_at").map_err(column)?,
                            last_active_at: row.try_get("last_active").map_err(column)?,
                            provider_connected_at: row
                                .try_get("provider_connected_at")
                                .map_err(column)?,
                            first_activity_at: row.try_get("first_activity_at").map_err(column)?,
                            first_health_sync_at: row
                                .try_get("first_health_sync_at")
                                .map_err(column)?,
                            first_turn_at: row.try_get("first_turn_at").map_err(column)?,
                        })
                    })
                    .collect()
            }

            async fn new_user_turns(
                &self,
                since: DateTime<Utc>,
                tenant: Option<TenantId>,
            ) -> AppResult<Vec<NewUserTurn>> {
                let rows = match tenant {
                    None => {
                        sqlx::query(NEW_USER_TURNS_SQL)
                            .bind(since)
                            .fetch_all(self.pool())
                            .await
                    }
                    Some(tenant) => {
                        sqlx::query(NEW_USER_TURNS_IN_TENANT_SQL)
                            .bind(since)
                            .bind($ids::bind(tenant.as_uuid()))
                            .bind(tenant.to_string())
                            .fetch_all(self.pool())
                            .await
                    }
                }
                .map_err(|e| {
                    AppError::database(format!("Failed to read new users' messages: {e}"))
                })?;

                let column = |e: sqlx::Error| {
                    AppError::database(format!("Invalid new-user message column: {e}"))
                };
                rows.iter()
                    .map(|row| {
                        Ok(NewUserTurn {
                            user_id: $ids::read(row, "user_id")?,
                            sent_at: row.try_get("sent_at").map_err(column)?,
                        })
                    })
                    .collect()
            }
        }
    };
}
pub(crate) use impl_signup_cohort_repository;
