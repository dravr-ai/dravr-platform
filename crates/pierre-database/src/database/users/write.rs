// ABOUTME: The two user writes on SQLite — `create` inserts a new row, `update` asserts a whole User onto an existing one
// ABOUTME: Split out of users.rs, which sits over the file-size ceiling; the Database impl hops live here

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::User;
use uuid::Uuid;

use crate::backends::shared;

use super::super::Database;

/// The `SQLite` text for a violated `UNIQUE` constraint. `create` turns one into the
/// same structured error `PostgreSQL`'s unique-violation code produces, so a duplicate
/// email reads identically to a caller whichever engine is underneath.
const UNIQUE_VIOLATION: &str = "UNIQUE constraint failed";

impl Database {
    /// Insert a new user row.
    ///
    /// Insert-only on purpose: this used to look the email up first and fall
    /// through to an UPDATE, which made one method name mean two things — and
    /// meant something different again on `PostgreSQL`, whose `create` has always
    /// been a bare INSERT against a `UNIQUE` email. Callers that mean "write this
    /// User over the existing row" call [`Database::update_user_impl`].
    ///
    /// # Errors
    ///
    /// Returns `invalid_input` if the email already belongs to a user, or a
    /// database error if the operation fails.
    pub async fn create_user_impl(&self, user: &User) -> AppResult<Uuid> {
        // NOTE: tenant_id is no longer stored on User - use tenant_users junction table
        // (tokens are stored in user_oauth_tokens table)
        sqlx::query(
            r"
            INSERT INTO users (
                id, email, display_name, password_hash, tier,
                is_active, user_status, is_admin, role, approved_by, approved_at,
                created_at, last_active, firebase_uid, auth_provider,
                analytics_consent, analytics_consent_at, locale,
                coaching_persona, manages_roster
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
            ",
        )
        .bind(user.id.to_string())
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&user.password_hash)
        .bind(user.tier.as_str())
        .bind(user.is_active)
        .bind(shared::enums::user_status_to_str(&user.user_status))
        .bind(user.is_admin)
        .bind(user.role.as_str())
        .bind(user.approved_by.map(|id| id.to_string()))
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
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains(UNIQUE_VIOLATION) {
                AppError::invalid_input("Email already in use by another user")
            } else {
                AppError::database(format!("Failed to create user: {e}"))
            }
        })?;

        Ok(user.id)
    }

    /// Write a whole `User` onto its existing row, matched by id.
    ///
    /// Every mutable column is written, so the caller must hand a `User` it
    /// loaded from the store and then modified — not one it built fresh, which
    /// would blank whatever it did not fill in. Columns with a dedicated setter
    /// (`update_locale`, `set_coaching_persona`, …) stay the narrow way to change
    /// one field; this is the escape hatch for callers changing several at once:
    /// Firebase account linking, and `pierre-cli user create --force`.
    ///
    /// `email` and `created_at` are not writable here — an email change would
    /// race the unique index, and a creation date is not a mutable fact.
    ///
    /// # Errors
    ///
    /// Returns `not_found` if no row carries that id, or a database error if the
    /// operation fails.
    pub async fn update_user_impl(&self, user: &User) -> AppResult<()> {
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
                last_active = CURRENT_TIMESTAMP
            WHERE id = $1
            ",
        )
        .bind(user.id.to_string())
        .bind(&user.display_name)
        .bind(&user.password_hash)
        .bind(user.tier.as_str())
        .bind(user.is_active)
        .bind(shared::enums::user_status_to_str(&user.user_status))
        .bind(user.is_admin)
        .bind(user.role.as_str())
        .bind(user.approved_by.map(|id| id.to_string()))
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
