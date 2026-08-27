// ABOUTME: The two user writes on PostgreSQL — `create` inserts a new row, `update` asserts a whole User onto one
// ABOUTME: Split out of user.rs, which sits at the file-size ceiling; the trait impl there delegates to these

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::User;
use uuid::Uuid;

use crate::backends::shared;

use super::PostgresDatabase;

/// `PostgreSQL`'s SQLSTATE for a violated unique constraint. `create` turns one into
/// the same structured error `SQLite`'s `UNIQUE constraint failed` produces, so a
/// duplicate reads identically to a caller whichever engine is underneath.
const UNIQUE_VIOLATION: &str = "23505";

/// The structured error for a violated unique constraint on `users`, or `None` when the
/// failure was something else.
///
/// `users` carries a second unique index besides `email`: `idx_users_firebase_uid`,
/// partial over non-null `firebase_uid`. Two concurrent Firebase sign-ins for one UID can
/// both pass `find_or_create_firebase_user`'s "no user for this UID" check and race the
/// insert, and the loser collides on *that* index — so reporting every duplicate as an
/// email collision sends whoever reads the log hunting the wrong column. Postgres names
/// the constraint, so this asks it rather than guessing.
fn duplicate_error(error: &sqlx::Error) -> Option<AppError> {
    let sqlx::Error::Database(db) = error else {
        return None;
    };
    if db.code().as_deref() != Some(UNIQUE_VIOLATION) {
        return None;
    }
    let names_firebase_uid = db
        .constraint()
        .is_some_and(|name| name.contains("firebase_uid"));
    Some(if names_firebase_uid {
        AppError::invalid_input("Firebase account already linked to another user")
    } else {
        AppError::invalid_input("Email already in use by another user")
    })
}

impl PostgresDatabase {
    pub(super) async fn create_user_row(&self, user: &User) -> AppResult<Uuid> {
        sqlx::query(
            r"
            INSERT INTO users (id, email, display_name, password_hash, tier, tenant_id, is_active, is_admin, role, user_status, approved_by, approved_at, created_at, last_active, firebase_uid, auth_provider, analytics_consent, analytics_consent_at, locale, coaching_persona, manages_roster, timezone, theme)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)
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
        .bind(user.analytics_consent)
        .bind(user.analytics_consent_at)
        .bind(&user.locale)
        .bind(user.coaching_persona.as_str())
        .bind(user.manages_roster)
        .bind(&user.timezone)
        .bind(&user.theme)
        .execute(&self.pool)
        .await
        .map_err(|e| duplicate_error(&e).unwrap_or_else(|| {
            AppError::database(format!("Failed to create user: {e}"))
        }))?;

        Ok(user.id)
    }

    pub(super) async fn update_user_row(&self, user: &User) -> AppResult<()> {
        let result = sqlx::query(
            r"
            UPDATE users SET
                display_name = $2,
                password_hash = $3,
                tier = $4,
                is_active = $5,
                user_status = $6,
                is_admin = $7,
                role = $8,
                approved_by = $9,
                approved_at = $10,
                firebase_uid = $11,
                auth_provider = $12,
                analytics_consent = $13,
                analytics_consent_at = $14,
                locale = $15,
                coaching_persona = $16,
                manages_roster = $17,
                timezone = $18,
                theme = $19,
                last_active = NOW()
            WHERE id = $1
            ",
        )
        .bind(user.id)
        .bind(&user.display_name)
        .bind(&user.password_hash)
        .bind(shared::enums::user_tier_to_str(&user.tier))
        .bind(user.is_active)
        .bind(shared::enums::user_status_to_str(&user.user_status))
        .bind(user.is_admin)
        .bind(shared::enums::user_role_to_str(&user.role))
        .bind(user.approved_by)
        .bind(user.approved_at)
        .bind(&user.firebase_uid)
        .bind(&user.auth_provider)
        .bind(user.analytics_consent)
        .bind(user.analytics_consent_at)
        .bind(&user.locale)
        .bind(user.coaching_persona.as_str())
        .bind(user.manages_roster)
        .bind(&user.timezone)
        .bind(&user.theme)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update user: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!("User {}", user.id)));
        }
        Ok(())
    }
}
