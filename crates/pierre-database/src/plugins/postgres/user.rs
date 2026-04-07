// ABOUTME: PostgreSQL user and profile repository implementations
// ABOUTME: Handles user CRUD operations and profile management for PostgreSQL backend
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::{ProfileRepository, UserRepository};
use super::PostgresDatabase;
use crate::plugins::shared;
use async_trait::async_trait;
use pierre_core::constants::tiers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::models::{User, UserStatus, UserTier};
use pierre_core::pagination::{Cursor, CursorPage, PaginationParams};
use pierre_core::permissions::UserRole;
use serde_json::Value;
use sqlx::Row;
use tracing::warn;
use uuid::Uuid;

#[async_trait]
impl UserRepository for PostgresDatabase {
    async fn create(&self, user: &User) -> AppResult<Uuid> {
        sqlx::query(
            r"
            INSERT INTO users (id, email, display_name, password_hash, tier, tenant_id, is_active, is_admin, role, user_status, approved_by, approved_at, created_at, last_active, firebase_uid, auth_provider)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ",
        )
        .bind(user.id)
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&user.password_hash)
        .bind(shared::enums::user_tier_to_str(&user.tier))
        .bind(None::<Option<String>>) // tenant_id is now managed via tenant_users table
        .bind(user.is_active)
        .bind(user.is_admin)
        .bind(shared::enums::user_role_to_str(&user.role))
        .bind(shared::enums::user_status_to_str(&user.user_status))
        .bind(user.approved_by)
        .bind(user.approved_at)
        .bind(user.created_at)
        .bind(user.last_active)
        .bind(&user.firebase_uid)
        .bind(&user.auth_provider)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create user: {e}")))?;

        Ok(user.id)
    }

    async fn get(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<User>> {
        let row = sqlx::query(
            r"
            SELECT u.id, u.email, u.display_name, u.password_hash, u.tier, u.tenant_id,
                   u.is_active, u.is_admin, u.role, u.user_status, u.approved_by, u.approved_at,
                   u.created_at, u.last_active, u.firebase_uid, u.auth_provider
            FROM users u
            INNER JOIN tenant_users tu ON u.id = tu.user_id AND tu.tenant_id = $2
            WHERE u.id = $1
            ",
        )
        .bind(user_id)
        .bind(tenant_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user by ID+tenant: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                Ok(Some(User {
                    id: row.get("id"),
                    email: row.get("email"),
                    display_name: row.get("display_name"),
                    password_hash: row.get("password_hash"),
                    tier: {
                        let tier_str: String = row.get("tier");
                        match tier_str.as_str() {
                            tiers::PROFESSIONAL => UserTier::Professional,
                            tiers::ENTERPRISE => UserTier::Enterprise,
                            _ => UserTier::Starter,
                        }
                    },
                    strava_token: None, // Tokens are loaded separately
                    fitbit_token: None, // Tokens are loaded separately
                    is_active: row.get("is_active"),
                    user_status: {
                        let status_str: String = row.get("user_status");
                        shared::enums::str_to_user_status(&status_str)
                    },
                    is_admin: row.get("is_admin"),
                    role: {
                        let role_str: Option<String> = row.try_get("role").ok().flatten();
                        role_str.map_or(UserRole::User, |s| shared::enums::str_to_user_role(&s))
                    },
                    approved_by: row.get("approved_by"),
                    approved_at: row.get("approved_at"),
                    created_at: row.get("created_at"),
                    last_active: row.get("last_active"),
                    firebase_uid: row.try_get("firebase_uid").ok().flatten(),
                    auth_provider: row
                        .try_get("auth_provider")
                        .unwrap_or_else(|_| "email".to_owned()),
                }))
            },
        )
    }

    async fn get_global(&self, user_id: Uuid) -> AppResult<Option<User>> {
        let row = sqlx::query(
            r"
            SELECT id, email, display_name, password_hash, tier, tenant_id, is_active, is_admin,
                   role, user_status, approved_by, approved_at, created_at, last_active,
                   firebase_uid, auth_provider
            FROM users
            WHERE id = $1
            ",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user by ID: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                Ok(Some(User {
                    id: row.get("id"),
                    email: row.get("email"),
                    display_name: row.get("display_name"),
                    password_hash: row.get("password_hash"),
                    tier: {
                        let tier_str: String = row.get("tier");
                        match tier_str.as_str() {
                            tiers::PROFESSIONAL => UserTier::Professional,
                            tiers::ENTERPRISE => UserTier::Enterprise,
                            _ => UserTier::Starter,
                        }
                    },
                    strava_token: None,
                    fitbit_token: None,
                    is_active: row.get("is_active"),
                    user_status: {
                        let status_str: String = row.get("user_status");
                        shared::enums::str_to_user_status(&status_str)
                    },
                    is_admin: row.get("is_admin"),
                    role: {
                        let role_str: Option<String> = row.try_get("role").ok().flatten();
                        role_str.map_or(UserRole::User, |s| shared::enums::str_to_user_role(&s))
                    },
                    approved_by: row.get("approved_by"),
                    approved_at: row.get("approved_at"),
                    created_at: row.get("created_at"),
                    last_active: row.get("last_active"),
                    firebase_uid: row.try_get("firebase_uid").ok().flatten(),
                    auth_provider: row
                        .try_get("auth_provider")
                        .unwrap_or_else(|_| "email".to_owned()),
                }))
            },
        )
    }

    async fn get_by_email(&self, email: &str) -> AppResult<Option<User>> {
        let row = sqlx::query(
            r"
            SELECT id, email, display_name, password_hash, tier, tenant_id, is_active, is_admin,
                   role, user_status, approved_by, approved_at, created_at, last_active,
                   firebase_uid, auth_provider
            FROM users
            WHERE email = $1
            ",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user by email: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                Ok(Some(User {
                    id: row.get("id"),
                    email: row.get("email"),
                    display_name: row.get("display_name"),
                    password_hash: row.get("password_hash"),
                    tier: {
                        let tier_str: String = row.get("tier");
                        match tier_str.as_str() {
                            tiers::PROFESSIONAL => UserTier::Professional,
                            tiers::ENTERPRISE => UserTier::Enterprise,
                            _ => UserTier::Starter,
                        }
                    },
                    strava_token: None, // Tokens are loaded separately
                    fitbit_token: None, // Tokens are loaded separately
                    is_active: row.get("is_active"),
                    user_status: {
                        let status_str: String = row.get("user_status");
                        shared::enums::str_to_user_status(&status_str)
                    },
                    is_admin: row.get("is_admin"),
                    role: {
                        let role_str: Option<String> = row.try_get("role").ok().flatten();
                        role_str.map_or(UserRole::User, |s| shared::enums::str_to_user_role(&s))
                    },
                    approved_by: row.get("approved_by"),
                    approved_at: row.get("approved_at"),
                    created_at: row.get("created_at"),
                    last_active: row.get("last_active"),
                    firebase_uid: row.try_get("firebase_uid").ok().flatten(),
                    auth_provider: row
                        .try_get("auth_provider")
                        .unwrap_or_else(|_| "email".to_owned()),
                }))
            },
        )
    }

    async fn get_by_email_required(&self, email: &str) -> AppResult<User> {
        self.get_by_email(email)
            .await?
            .ok_or_else(|| AppError::not_found(format!("User with email {email}")))
    }

    async fn get_first_admin_user(&self) -> AppResult<Option<User>> {
        let row = sqlx::query(
            r"
            SELECT id, email, display_name, password_hash, tier, tenant_id, is_active, is_admin,
                   role, user_status, approved_by, approved_at, created_at, last_active,
                   firebase_uid, auth_provider
            FROM users
            WHERE is_admin = true
            ORDER BY created_at ASC
            LIMIT 1
            ",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get first admin user: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                Ok(Some(User {
                    id: row.get("id"),
                    email: row.get("email"),
                    display_name: row.get("display_name"),
                    password_hash: row.get("password_hash"),
                    tier: {
                        let tier_str: String = row.get("tier");
                        match tier_str.as_str() {
                            tiers::PROFESSIONAL => UserTier::Professional,
                            tiers::ENTERPRISE => UserTier::Enterprise,
                            _ => UserTier::Starter,
                        }
                    },
                    strava_token: None,
                    fitbit_token: None,
                    is_active: row.get("is_active"),
                    user_status: {
                        let status_str: String = row.get("user_status");
                        shared::enums::str_to_user_status(&status_str)
                    },
                    is_admin: row.get("is_admin"),
                    role: {
                        let role_str: Option<String> = row.try_get("role").ok().flatten();
                        role_str.map_or(UserRole::User, |s| shared::enums::str_to_user_role(&s))
                    },
                    approved_by: row.get("approved_by"),
                    approved_at: row.get("approved_at"),
                    created_at: row.get("created_at"),
                    last_active: row.get("last_active"),
                    firebase_uid: row.try_get("firebase_uid").ok().flatten(),
                    auth_provider: row
                        .try_get("auth_provider")
                        .unwrap_or_else(|_| "email".to_owned()),
                }))
            },
        )
    }

    async fn get_by_firebase_uid(&self, firebase_uid: &str) -> AppResult<Option<User>> {
        let row = sqlx::query(
            r"
            SELECT id, email, display_name, password_hash, tier, tenant_id, is_active, is_admin,
                   role, user_status, approved_by, approved_at, created_at, last_active,
                   firebase_uid, auth_provider
            FROM users
            WHERE firebase_uid = $1
            ",
        )
        .bind(firebase_uid)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user by firebase_uid: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                Ok(Some(User {
                    id: row.get("id"),
                    email: row.get("email"),
                    display_name: row.get("display_name"),
                    password_hash: row.get("password_hash"),
                    tier: {
                        let tier_str: String = row.get("tier");
                        match tier_str.as_str() {
                            tiers::PROFESSIONAL => UserTier::Professional,
                            tiers::ENTERPRISE => UserTier::Enterprise,
                            _ => UserTier::Starter,
                        }
                    },
                    strava_token: None, // Tokens are loaded separately
                    fitbit_token: None, // Tokens are loaded separately
                    is_active: row.get("is_active"),
                    user_status: {
                        let status_str: String = row.get("user_status");
                        shared::enums::str_to_user_status(&status_str)
                    },
                    is_admin: row.get("is_admin"),
                    role: {
                        let role_str: Option<String> = row.try_get("role").ok().flatten();
                        role_str.map_or(UserRole::User, |s| shared::enums::str_to_user_role(&s))
                    },
                    approved_by: row.get("approved_by"),
                    approved_at: row.get("approved_at"),
                    created_at: row.get("created_at"),
                    last_active: row.get("last_active"),
                    firebase_uid: row.try_get("firebase_uid").ok().flatten(),
                    auth_provider: row
                        .try_get("auth_provider")
                        .unwrap_or_else(|_| "email".to_owned()),
                }))
            },
        )
    }

    async fn update_last_active(&self, user_id: Uuid) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE users
            SET last_active = CURRENT_TIMESTAMP
            WHERE id = $1
            ",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update last active timestamp: {e}")))?;

        Ok(())
    }

    async fn count(&self) -> AppResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user count: {e}")))?;

        Ok(row.get("count"))
    }

    async fn get_by_status(
        &self,
        status: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<User>> {
        // Query users by status from PostgreSQL
        let status_enum = match status {
            "active" => "active",
            "pending" => "pending",
            "suspended" => "suspended",
            _ => {
                return Err(AppError::invalid_input(format!(
                    "Invalid user status: {status}"
                )))
            }
        };

        // Pending users have no tenant_users entry (assigned on approval),
        // so skip the tenant join for pending status to avoid excluding them.
        let rows = if let (Some(tid), false) = (&tenant_id, status_enum == "pending") {
            sqlx::query(
                r"
                SELECT u.id, u.email, u.display_name, u.password_hash, u.tier, u.tenant_id,
                       u.is_active, u.is_admin, u.role,
                       COALESCE(u.user_status, 'active') as user_status,
                       u.approved_by, u.approved_at, u.created_at, u.last_active,
                       u.firebase_uid, u.auth_provider
                FROM users u
                INNER JOIN tenant_users tu ON u.id = tu.user_id AND tu.tenant_id = $2
                WHERE COALESCE(u.user_status, 'active') = $1
                ORDER BY u.created_at DESC
                ",
            )
            .bind(status_enum)
            .bind(tid.0)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                SELECT id, email, display_name, password_hash, tier, tenant_id, is_active, is_admin,
                       role, COALESCE(user_status, 'active') as user_status, approved_by, approved_at,
                       created_at, last_active, firebase_uid, auth_provider
                FROM users
                WHERE COALESCE(user_status, 'active') = $1
                ORDER BY created_at DESC
                ",
            )
            .bind(status_enum)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to get users by status: {e}")))?;

        let mut users = Vec::new();
        for row in rows {
            let user_status_str: String = row.get("user_status");
            let user_status = match user_status_str.as_str() {
                "pending" => UserStatus::Pending,
                "suspended" => UserStatus::Suspended,
                _ => UserStatus::Active,
            };

            users.push(User {
                id: row.get("id"),
                email: row.get("email"),
                display_name: row.get("display_name"),
                password_hash: row.get("password_hash"),
                tier: {
                    let tier_str: String = row.get("tier");
                    match tier_str.as_str() {
                        tiers::PROFESSIONAL => UserTier::Professional,
                        tiers::ENTERPRISE => UserTier::Enterprise,
                        _ => UserTier::Starter,
                    }
                },
                strava_token: None,
                fitbit_token: None,
                is_active: row.get("is_active"),
                user_status,
                is_admin: row.try_get("is_admin").unwrap_or_else(|e| {
                    tracing::warn!("is_admin column missing or invalid, defaulting to false: {e}");
                    false
                }),
                role: {
                    let role_str: Option<String> = row.try_get("role").ok().flatten();
                    role_str.map_or(UserRole::User, |s| shared::enums::str_to_user_role(&s))
                },
                approved_by: row.get("approved_by"),
                approved_at: row.get("approved_at"),
                created_at: row.get("created_at"),
                last_active: row.get("last_active"),
                firebase_uid: row.try_get("firebase_uid").ok().flatten(),
                auth_provider: row
                    .try_get("auth_provider")
                    .unwrap_or_else(|_| "email".to_owned()),
            });
        }

        Ok(users)
    }

    async fn get_by_status_cursor(
        &self,
        status: &str,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<User>> {
        const QUERY_WITH_CURSOR: &str = r"
            SELECT id, email, display_name, password_hash, tier, tenant_id, is_active, is_admin,
                   COALESCE(user_status, 'active') as user_status, approved_by, approved_at,
                   created_at, last_active, firebase_uid, auth_provider
            FROM users
            WHERE COALESCE(user_status, 'active') = $1
              AND (created_at < $2 OR (created_at = $2 AND id::text < $3))
            ORDER BY created_at DESC, id DESC
            LIMIT $4
        ";

        const QUERY_WITHOUT_CURSOR: &str = r"
            SELECT id, email, display_name, password_hash, tier, tenant_id, is_active, is_admin,
                   COALESCE(user_status, 'active') as user_status, approved_by, approved_at, created_at, last_active
            FROM users
            WHERE COALESCE(user_status, 'active') = $1
            ORDER BY created_at DESC, id DESC
            LIMIT $2
        ";

        // Validate status
        let status_enum = match status {
            "active" => "active",
            "pending" => "pending",
            "suspended" => "suspended",
            _ => {
                return Err(AppError::invalid_input(format!(
                    "Invalid user status: {status}"
                )))
            }
        };

        // Fetch one more than requested to determine if there are more items
        let fetch_limit = params.limit + 1;

        // Convert to i64 for SQL LIMIT clause (pagination limits are always reasonable)
        let fetch_limit_i64 = i64::try_from(fetch_limit).map_err(|e| {
            warn!(
                fetch_limit = fetch_limit,
                max_allowed = i64::MAX,
                error = %e,
                "Pagination limit conversion failed"
            );
            AppError::invalid_input(format!("Pagination limit too large: {fetch_limit}"))
        })?;

        // Execute query with appropriate parameters
        let rows = if let Some(ref cursor) = params.cursor {
            let (timestamp, id) = cursor
                .decode()
                .ok_or_else(|| AppError::invalid_input("Invalid cursor format"))?;

            sqlx::query(QUERY_WITH_CURSOR)
                .bind(status_enum)
                .bind(timestamp)
                .bind(id)
                .bind(fetch_limit_i64)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to get users by status with cursor: {e}"))
                })?
        } else {
            sqlx::query(QUERY_WITHOUT_CURSOR)
                .bind(status_enum)
                .bind(fetch_limit_i64)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to get users by status: {e}")))?
        };

        // Parse rows into User structs
        let mut users: Vec<User> = rows
            .iter()
            .map(Self::parse_user_from_row)
            .collect::<AppResult<Vec<_>>>()?;

        // Determine if there are more items
        let has_more = users.len() > params.limit;
        if has_more {
            users.pop(); // Remove the extra item we fetched
        }

        // Generate next cursor from the last item
        let next_cursor = if has_more {
            users
                .last()
                .map(|last_user| Cursor::new(last_user.created_at, &last_user.id.to_string()))
        } else {
            None
        };

        Ok(CursorPage::new(users, next_cursor, None, has_more))
    }

    async fn update_status(
        &self,
        user_id: Uuid,
        new_status: UserStatus,
        approved_by: Option<Uuid>,
    ) -> AppResult<User> {
        let status_str = shared::enums::user_status_to_str(&new_status);

        // Only set approved_by when activating a user and an approver UUID is provided
        let approved_by_uuid = if new_status == UserStatus::Active {
            approved_by
        } else {
            None
        };

        let approved_at = if new_status == UserStatus::Active {
            Some(chrono::Utc::now())
        } else {
            None
        };

        // Update user status
        // approved_by is UUID in PG — bind as Uuid directly
        sqlx::query(
            r"
            UPDATE users
            SET user_status = $1, approved_by = $2, approved_at = $3
            WHERE id = $4
            ",
        )
        .bind(status_str)
        .bind(approved_by_uuid)
        .bind(approved_at)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update user status: {e}")))?;

        // Return updated user
        self.get_global(user_id)
            .await?
            .ok_or_else(|| AppError::not_found("User after status update"))
    }

    async fn update_tenant_id(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<()> {
        let result = sqlx::query(
            r"
            UPDATE users
            SET tenant_id = $1
            WHERE id = $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update user tenant ID: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!("User with ID: {user_id}")));
        }

        // Upsert into tenant_users junction table (queries INNER JOIN on it)
        sqlx::query(
            r"
            INSERT INTO tenant_users (tenant_id, user_id, role, invited_at, joined_at)
            VALUES ($1, $2, 'member', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            ON CONFLICT (tenant_id, user_id) DO NOTHING
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert tenant_users entry: {e}")))?;

        Ok(())
    }

    async fn update_password(&self, user_id: Uuid, password_hash: &str) -> AppResult<()> {
        let result = sqlx::query(
            r"
            UPDATE users
            SET password_hash = $1, last_active = CURRENT_TIMESTAMP
            WHERE id = $2
            ",
        )
        .bind(password_hash)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update user password: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!("User with ID: {user_id}")));
        }

        Ok(())
    }

    async fn update_display_name(&self, user_id: Uuid, display_name: &str) -> AppResult<User> {
        let result = sqlx::query(
            r"
            UPDATE users
            SET display_name = $1, last_active = CURRENT_TIMESTAMP
            WHERE id = $2
            ",
        )
        .bind(display_name)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update user display name: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!("User with ID: {user_id}")));
        }

        self.get_global(user_id)
            .await?
            .ok_or_else(|| AppError::not_found("User after display name update"))
    }

    async fn delete(&self, user_id: Uuid) -> AppResult<()> {
        let result = sqlx::query(
            r"
            DELETE FROM users WHERE id = $1
            ",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete user: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!("User {user_id} not found")));
        }

        Ok(())
    }

    async fn has_synthetic_activities(&self, user_id: Uuid) -> AppResult<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM synthetic_activities WHERE user_id = $1 LIMIT 1",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }
}

#[async_trait]
impl ProfileRepository for PostgresDatabase {
    async fn upsert_profile(&self, user_id: Uuid, profile_data: Value) -> AppResult<()> {
        let now = chrono::Utc::now();
        sqlx::query(
            r"
            INSERT INTO user_profiles (user_id, profile_data, created_at, updated_at)
            VALUES ($1, $2, $3, $3)
            ON CONFLICT (user_id)
            DO UPDATE SET profile_data = $2, updated_at = $3
            ",
        )
        .bind(user_id)
        .bind(&profile_data)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert user profile: {e}")))?;

        Ok(())
    }

    async fn get_profile(&self, user_id: Uuid) -> AppResult<Option<Value>> {
        let row = sqlx::query(
            r"
            SELECT profile_data
            FROM user_profiles
            WHERE user_id = $1
            ",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user profile: {e}")))?;

        row.map_or_else(|| Ok(None), |row| Ok(Some(row.get("profile_data"))))
    }

    async fn create_goal(&self, user_id: Uuid, goal_data: Value) -> AppResult<String> {
        let goal_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        sqlx::query(
            r"
            INSERT INTO goals (id, user_id, goal_data, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $4)
            ",
        )
        .bind(&goal_id)
        .bind(user_id)
        .bind(&goal_data)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create goal: {e}")))?;

        Ok(goal_id)
    }

    async fn get_goals(&self, user_id: Uuid) -> AppResult<Vec<Value>> {
        let rows = sqlx::query(
            r"
            SELECT goal_data
            FROM goals
            WHERE user_id = $1
            ORDER BY created_at DESC
            ",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user goals: {e}")))?;

        Ok(rows.into_iter().map(|row| row.get("goal_data")).collect())
    }

    async fn update_goal_progress(
        &self,
        goal_id: &str,
        user_id: Uuid,
        current_value: f64,
    ) -> AppResult<()> {
        // Use const to avoid clippy warning about format-like strings
        const JSON_PATH: &str = "{current_value}";
        sqlx::query(
            r"
            UPDATE goals
            SET goal_data = jsonb_set(goal_data, $3::text, $1::text::jsonb),
                updated_at = CURRENT_TIMESTAMP
            WHERE id = $2 AND user_id = $4
            ",
        )
        .bind(current_value)
        .bind(goal_id)
        .bind(JSON_PATH)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update goal progress: {e}")))?;

        Ok(())
    }

    async fn get_configuration(&self, user_id: &str) -> AppResult<Option<String>> {
        // First ensure the user_configurations table exists
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS user_configurations (
                user_id TEXT PRIMARY KEY,
                config_data TEXT NOT NULL,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create user_configurations table: {e}"))
        })?;

        let query = "SELECT config_data FROM user_configurations WHERE user_id = $1";

        let row = sqlx::query(query)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user configuration: {e}")))?;

        if let Some(row) = row {
            Ok(Some(row.try_get("config_data").map_err(|e| {
                AppError::database(format!("Failed to parse config_data column: {e}"))
            })?))
        } else {
            Ok(None)
        }
    }

    async fn save_configuration(&self, user_id: &str, config_json: &str) -> AppResult<()> {
        // First ensure the user_configurations table exists
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS user_configurations (
                user_id TEXT PRIMARY KEY,
                config_data TEXT NOT NULL,
                created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to create user_configurations table: {e}"))
        })?;

        // Insert or update configuration
        let now = chrono::Utc::now();
        let query = r"
            INSERT INTO user_configurations (user_id, config_data, created_at, updated_at)
            VALUES ($1, $2, $3, $3)
            ON CONFLICT(user_id) DO UPDATE SET
                config_data = EXCLUDED.config_data,
                updated_at = $3
        ";

        sqlx::query(query)
            .bind(user_id)
            .bind(config_json)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to save user configuration: {e}")))?;

        Ok(())
    }
}
