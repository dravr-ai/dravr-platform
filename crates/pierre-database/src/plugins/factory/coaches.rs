// ABOUTME: Coaches repository dispatch for the database factory
// ABOUTME: Delegates CoachesRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::plugins::CoachesRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::coaches::{
    Coach, CoachAssignment, CoachListItem, CoachVersion, CreateCoachRequest,
    CreateSystemCoachRequest, ListCoachesFilter, UpdateCoachRequest,
};
use pierre_core::models::TenantId;
use uuid::Uuid;

#[async_trait]
impl CoachesRepository for Database {
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateCoachRequest,
    ) -> AppResult<Coach> {
        match self {
            Self::SQLite(db) => db.create(user_id, tenant_id, request).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create(user_id, tenant_id, request).await,
        }
    }

    async fn get_by_id(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        match self {
            Self::SQLite(db) => db.get_by_id(coach_id, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_id(coach_id, user_id, tenant_id).await,
        }
    }

    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        filter: &ListCoachesFilter,
    ) -> AppResult<Vec<CoachListItem>> {
        match self {
            Self::SQLite(db) => db.list(user_id, tenant_id, filter).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list(user_id, tenant_id, filter).await,
        }
    }

    async fn update(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>> {
        match self {
            Self::SQLite(db) => db.update(coach_id, user_id, tenant_id, request).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update(coach_id, user_id, tenant_id, request).await,
        }
    }

    async fn delete(&self, coach_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.delete(coach_id, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete(coach_id, user_id, tenant_id).await,
        }
    }

    async fn record_usage(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.record_usage(coach_id, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.record_usage(coach_id, user_id, tenant_id).await,
        }
    }

    async fn toggle_favorite(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<bool>> {
        match self {
            Self::SQLite(db) => db.toggle_favorite(coach_id, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.toggle_favorite(coach_id, user_id, tenant_id).await,
        }
    }

    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Coach>> {
        match self {
            Self::SQLite(db) => db.search(user_id, tenant_id, query, limit, offset).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.search(user_id, tenant_id, query, limit, offset).await,
        }
    }

    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32> {
        match self {
            Self::SQLite(db) => db.count(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.count(user_id, tenant_id).await,
        }
    }

    async fn fork_coach(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        match self {
            Self::SQLite(db) => db.fork_coach(source_coach_id, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.fork_coach(source_coach_id, user_id, tenant_id).await,
        }
    }

    async fn activate_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        match self {
            Self::SQLite(db) => db.activate_coach(coach_id, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.activate_coach(coach_id, user_id, tenant_id).await,
        }
    }

    async fn deactivate_coach(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.deactivate_coach(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.deactivate_coach(user_id, tenant_id).await,
        }
    }

    async fn get_active_coach(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        match self {
            Self::SQLite(db) => db.get_active_coach(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_active_coach(user_id, tenant_id).await,
        }
    }

    async fn create_system_coach(
        &self,
        admin_user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateSystemCoachRequest,
    ) -> AppResult<Coach> {
        match self {
            Self::SQLite(db) => {
                db.create_system_coach(admin_user_id, tenant_id, request)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.create_system_coach(admin_user_id, tenant_id, request)
                    .await
            }
        }
    }

    async fn list_system_coaches(&self, tenant_id: TenantId) -> AppResult<Vec<Coach>> {
        match self {
            Self::SQLite(db) => db.list_system_coaches(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_system_coaches(tenant_id).await,
        }
    }

    async fn get_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        match self {
            Self::SQLite(db) => db.get_system_coach(coach_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_system_coach(coach_id, tenant_id).await,
        }
    }

    async fn get_system_coach_any_tenant(&self, coach_id: &str) -> AppResult<Option<Coach>> {
        match self {
            Self::SQLite(db) => db.get_system_coach_any_tenant(coach_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_system_coach_any_tenant(coach_id).await,
        }
    }

    async fn update_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>> {
        match self {
            Self::SQLite(db) => db.update_system_coach(coach_id, tenant_id, request).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_system_coach(coach_id, tenant_id, request).await,
        }
    }

    async fn delete_system_coach(&self, coach_id: &str, tenant_id: TenantId) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.delete_system_coach(coach_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_system_coach(coach_id, tenant_id).await,
        }
    }

    async fn get_user_preferences(
        &self,
        coach_id: &str,
        user_id: Uuid,
    ) -> AppResult<(bool, bool, u32, Option<DateTime<Utc>>)> {
        match self {
            Self::SQLite(db) => db.get_user_preferences(coach_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_user_preferences(coach_id, user_id).await,
        }
    }

    async fn assign_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        assigned_by: Uuid,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.assign_coach(coach_id, user_id, assigned_by).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.assign_coach(coach_id, user_id, assigned_by).await,
        }
    }

    async fn unassign_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.unassign_coach(coach_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.unassign_coach(coach_id, user_id).await,
        }
    }

    async fn list_assignments(&self, coach_id: &str) -> AppResult<Vec<CoachAssignment>> {
        match self {
            Self::SQLite(db) => db.list_assignments(coach_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_assignments(coach_id).await,
        }
    }

    async fn list_assignments_for_tenant(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAssignment>> {
        match self {
            Self::SQLite(db) => db.list_assignments_for_tenant(coach_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_assignments_for_tenant(coach_id, tenant_id).await,
        }
    }

    async fn hide_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.hide_coach(coach_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.hide_coach(coach_id, user_id).await,
        }
    }

    async fn show_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.show_coach(coach_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.show_coach(coach_id, user_id).await,
        }
    }

    async fn list_hidden_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>> {
        match self {
            Self::SQLite(db) => db.list_hidden_coaches(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_hidden_coaches(user_id, tenant_id).await,
        }
    }

    async fn create_version(
        &self,
        coach_id: &str,
        user_id: Uuid,
        change_summary: Option<&str>,
    ) -> AppResult<i32> {
        match self {
            Self::SQLite(db) => db.create_version(coach_id, user_id, change_summary).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create_version(coach_id, user_id, change_summary).await,
        }
    }

    async fn get_versions(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        limit: u32,
    ) -> AppResult<Vec<CoachVersion>> {
        match self {
            Self::SQLite(db) => db.get_versions(coach_id, tenant_id, limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_versions(coach_id, tenant_id, limit).await,
        }
    }

    async fn get_version(
        &self,
        coach_id: &str,
        version: i32,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachVersion>> {
        match self {
            Self::SQLite(db) => db.get_version(coach_id, version, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_version(coach_id, version, tenant_id).await,
        }
    }

    async fn revert_to_version(
        &self,
        coach_id: &str,
        version: i32,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        match self {
            Self::SQLite(db) => {
                db.revert_to_version(coach_id, version, user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.revert_to_version(coach_id, version, user_id, tenant_id)
                    .await
            }
        }
    }

    async fn get_current_version(&self, coach_id: &str) -> AppResult<i32> {
        match self {
            Self::SQLite(db) => db.get_current_version(coach_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_current_version(coach_id).await,
        }
    }

    async fn get_startup_query_by_system_prompt(
        &self,
        system_prompt: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<String>> {
        match self {
            Self::SQLite(db) => {
                db.get_startup_query_by_system_prompt(system_prompt, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_startup_query_by_system_prompt(system_prompt, tenant_id)
                    .await
            }
        }
    }

    async fn get_max_tool_iterations_by_system_prompt(
        &self,
        system_prompt: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<i32>> {
        match self {
            Self::SQLite(db) => {
                db.get_max_tool_iterations_by_system_prompt(system_prompt, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_max_tool_iterations_by_system_prompt(system_prompt, tenant_id)
                    .await
            }
        }
    }
}
