// ABOUTME: User and profile repository dispatch for the database factory
// ABOUTME: Delegates UserRepository and ProfileRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::database_plugins::{ProfileRepository, UserRepository};
use crate::errors::AppResult;
use crate::models::{User, UserStatus};
use crate::pagination::{CursorPage, PaginationParams};
use async_trait::async_trait;
use pierre_core::models::TenantId;
use uuid::Uuid;

#[async_trait]
impl UserRepository for Database {
    async fn create(&self, user: &User) -> AppResult<uuid::Uuid> {
        match self {
            Self::SQLite(db) => UserRepository::create(db, user).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => UserRepository::create(db, user).await,
        }
    }
    async fn get(&self, user_id: uuid::Uuid, tenant_id: TenantId) -> AppResult<Option<User>> {
        match self {
            Self::SQLite(db) => db.get(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get(user_id, tenant_id).await,
        }
    }
    async fn get_global(&self, user_id: uuid::Uuid) -> AppResult<Option<User>> {
        match self {
            Self::SQLite(db) => db.get_global(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_global(user_id).await,
        }
    }
    async fn get_by_email(&self, email: &str) -> AppResult<Option<User>> {
        match self {
            Self::SQLite(db) => db.get_by_email(email).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_email(email).await,
        }
    }
    async fn get_by_email_required(&self, email: &str) -> AppResult<User> {
        match self {
            Self::SQLite(db) => db.get_by_email_required(email).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_email_required(email).await,
        }
    }
    async fn get_by_firebase_uid(&self, firebase_uid: &str) -> AppResult<Option<User>> {
        match self {
            Self::SQLite(db) => db.get_by_firebase_uid(firebase_uid).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_firebase_uid(firebase_uid).await,
        }
    }
    async fn update_last_active(&self, user_id: uuid::Uuid) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_last_active(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_last_active(user_id).await,
        }
    }
    async fn count(&self) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => db.count().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.count().await,
        }
    }
    async fn get_by_status(
        &self,
        status: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Vec<User>> {
        match self {
            Self::SQLite(db) => db.get_by_status(status, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_status(status, tenant_id).await,
        }
    }
    async fn get_first_admin_user(&self) -> AppResult<Option<User>> {
        match self {
            Self::SQLite(db) => db.get_first_admin_user().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_first_admin_user().await,
        }
    }
    async fn get_by_status_cursor(
        &self,
        status: &str,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<User>> {
        match self {
            Self::SQLite(db) => db.get_by_status_cursor(status, params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_status_cursor(status, params).await,
        }
    }
    async fn update_status(
        &self,
        user_id: uuid::Uuid,
        new_status: UserStatus,
        approved_by: Option<uuid::Uuid>,
    ) -> AppResult<User> {
        match self {
            Self::SQLite(db) => db.update_status(user_id, new_status, approved_by).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_status(user_id, new_status, approved_by).await,
        }
    }
    async fn update_tenant_id(&self, user_id: uuid::Uuid, tenant_id: TenantId) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_tenant_id(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_tenant_id(user_id, tenant_id).await,
        }
    }
    async fn update_password(&self, user_id: uuid::Uuid, password_hash: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_password(user_id, password_hash).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_password(user_id, password_hash).await,
        }
    }
    async fn update_display_name(
        &self,
        user_id: uuid::Uuid,
        display_name: &str,
    ) -> AppResult<User> {
        match self {
            Self::SQLite(db) => db.update_display_name(user_id, display_name).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_display_name(user_id, display_name).await,
        }
    }
    async fn delete(&self, user_id: uuid::Uuid) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.delete(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete(user_id).await,
        }
    }
    async fn has_synthetic_activities(&self, user_id: Uuid) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.user_has_synthetic_activities_impl(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.has_synthetic_activities(user_id).await,
        }
    }
}

#[async_trait]
impl ProfileRepository for Database {
    async fn upsert_profile(
        &self,
        user_id: uuid::Uuid,
        profile_data: serde_json::Value,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.upsert_profile(user_id, profile_data).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.upsert_profile(user_id, profile_data).await,
        }
    }

    async fn get_profile(&self, user_id: uuid::Uuid) -> AppResult<Option<serde_json::Value>> {
        match self {
            Self::SQLite(db) => db.get_profile(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_profile(user_id).await,
        }
    }

    async fn create_goal(
        &self,
        user_id: uuid::Uuid,
        goal_data: serde_json::Value,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.create_goal(user_id, goal_data).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create_goal(user_id, goal_data).await,
        }
    }

    async fn get_goals(&self, user_id: uuid::Uuid) -> AppResult<Vec<serde_json::Value>> {
        match self {
            Self::SQLite(db) => db.get_goals(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_goals(user_id).await,
        }
    }

    async fn update_goal_progress(
        &self,
        goal_id: &str,
        user_id: Uuid,
        current_value: f64,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => {
                db.update_goal_progress(goal_id, user_id, current_value)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.update_goal_progress(goal_id, user_id, current_value)
                    .await
            }
        }
    }

    async fn get_configuration(&self, user_id: &str) -> AppResult<Option<String>> {
        match self {
            Self::SQLite(db) => db.get_configuration(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_configuration(user_id).await,
        }
    }

    async fn save_configuration(&self, user_id: &str, config_json: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.save_configuration(user_id, config_json).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.save_configuration(user_id, config_json).await,
        }
    }
}
