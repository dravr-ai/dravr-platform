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
            INSERT INTO users (id, email, display_name, password_hash, tier, tenant_id, is_active, is_admin, role, user_status, approved_by, approved_at, created_at, last_active, firebase_uid, auth_provider, analytics_consent, analytics_consent_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
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
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create user: {e}")))?;

        Ok(user.id)
    }