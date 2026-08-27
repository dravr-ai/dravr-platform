// ABOUTME: The users upsert on SQLite — one method that inserts a new row or asserts a whole User onto an existing one
// ABOUTME: Split out of users.rs, which sits over the file-size ceiling; the Database impl hop lives here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::User;
use uuid::Uuid;

use crate::backends::shared;

use super::super::Database;

impl Database {
    /// Create or update a user
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The email is already in use by another user
    /// - Database operation fails
    pub async fn create_user_impl(&self, user: &User) -> AppResult<Uuid> {
        // Check if user exists by email
        let existing = self.get_user_by_email_impl(&user.email).await?;
        if let Some(existing_user) = existing {
            if existing_user.id != user.id {
                return Err(AppError::invalid_input(
                    "Email already in use by another user",
                ));
            }
            // Update existing user (tokens are stored in user_oauth_tokens table)
            // NOTE: tenant_id is no longer stored on User - use tenant_users junction table
            // The caller hands a whole User and this asserts it onto the row, locale
            // included: `pierre-cli user create --force --locale en` re-points an
            // already-created dev account, which it cannot do if locale is left out.
            sqlx::query(
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
                    locale = $11,
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
            .bind(&user.locale)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to update user: {e}")))?;
        } else {
            // Insert new user (tokens are stored in user_oauth_tokens table)
            // NOTE: tenant_id is no longer stored on User - use tenant_users junction table
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
            .map_err(|e| AppError::database(format!("Failed to create user: {e}")))?;
        }

        Ok(user.id)
    }
}
