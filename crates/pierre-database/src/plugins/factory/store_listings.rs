// ABOUTME: Store listings repository dispatch for the database factory
// ABOUTME: Delegates StoreListingsRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::database::store_listings::{CoachWithListing, StoreListing};
use crate::plugins::StoreListingsRepository;
use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::coaches::{Coach, CoachCategory, StoreAdminStats};
use pierre_core::models::TenantId;
use pierre_core::pagination::{CursorPage, StoreSortOrder};
use std::collections::HashMap;
use uuid::Uuid;

#[async_trait]
impl StoreListingsRepository for Database {
    async fn submit_for_review(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<StoreListing> {
        match self {
            Self::SQLite(db) => db.submit_for_review(coach_id, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.submit_for_review(coach_id, user_id, tenant_id).await,
        }
    }

    async fn get_listing(&self, coach_id: &str) -> AppResult<Option<StoreListing>> {
        match self {
            Self::SQLite(db) => db.get_listing(coach_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_listing(coach_id).await,
        }
    }

    async fn approve_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
    ) -> AppResult<CoachWithListing> {
        match self {
            Self::SQLite(db) => db.approve_coach(coach_id, tenant_id, admin_user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.approve_coach(coach_id, tenant_id, admin_user_id).await,
        }
    }

    async fn reject_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: Option<Uuid>,
        reason: &str,
    ) -> AppResult<CoachWithListing> {
        match self {
            Self::SQLite(db) => {
                db.reject_coach(coach_id, tenant_id, admin_user_id, reason)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.reject_coach(coach_id, tenant_id, admin_user_id, reason)
                    .await
            }
        }
    }

    async fn unpublish_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<CoachWithListing> {
        match self {
            Self::SQLite(db) => db.unpublish_coach(coach_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.unpublish_coach(coach_id, tenant_id).await,
        }
    }

    async fn get_pending_review_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        match self {
            Self::SQLite(db) => {
                db.get_pending_review_coaches(tenant_id, limit, offset)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_pending_review_coaches(tenant_id, limit, offset)
                    .await
            }
        }
    }

    async fn get_rejected_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        match self {
            Self::SQLite(db) => db.get_rejected_coaches(tenant_id, limit, offset).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_rejected_coaches(tenant_id, limit, offset).await,
        }
    }

    async fn get_store_admin_stats(&self, tenant_id: TenantId) -> AppResult<StoreAdminStats> {
        match self {
            Self::SQLite(db) => db.get_store_admin_stats(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_store_admin_stats(tenant_id).await,
        }
    }

    async fn get_author_email(&self, user_id: Uuid) -> AppResult<Option<String>> {
        match self {
            Self::SQLite(db) => db.get_author_email(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_author_email(user_id).await,
        }
    }

    async fn get_published_coaches(
        &self,
        category: Option<CoachCategory>,
        sort_by: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        match self {
            Self::SQLite(db) => {
                db.get_published_coaches(category, sort_by, limit, offset)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_published_coaches(category, sort_by, limit, offset)
                    .await
            }
        }
    }

    async fn get_published_coaches_cursor(
        &self,
        category: Option<CoachCategory>,
        sort_by: StoreSortOrder,
        limit: u32,
        cursor: Option<&str>,
    ) -> AppResult<CursorPage<CoachWithListing>> {
        match self {
            Self::SQLite(db) => {
                db.get_published_coaches_cursor(category, sort_by, limit, cursor)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_published_coaches_cursor(category, sort_by, limit, cursor)
                    .await
            }
        }
    }

    async fn search_published_coaches(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        match self {
            Self::SQLite(db) => db.search_published_coaches(query, limit).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.search_published_coaches(query, limit).await,
        }
    }

    async fn get_published_coach(&self, coach_id: &str) -> AppResult<Option<CoachWithListing>> {
        match self {
            Self::SQLite(db) => db.get_published_coach(coach_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_published_coach(coach_id).await,
        }
    }

    async fn get_category_counts(&self) -> AppResult<HashMap<CoachCategory, i64>> {
        match self {
            Self::SQLite(db) => db.get_category_counts().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_category_counts().await,
        }
    }

    async fn increment_install_count(&self, coach_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.increment_install_count(coach_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.increment_install_count(coach_id).await,
        }
    }

    async fn decrement_install_count(&self, coach_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.decrement_install_count(coach_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.decrement_install_count(coach_id).await,
        }
    }

    async fn install_from_store(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        match self {
            Self::SQLite(db) => {
                db.install_from_store(source_coach_id, user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.install_from_store(source_coach_id, user_id, tenant_id)
                    .await
            }
        }
    }

    async fn uninstall_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.uninstall_coach(coach_id, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.uninstall_coach(coach_id, user_id, tenant_id).await,
        }
    }

    async fn get_installed_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>> {
        match self {
            Self::SQLite(db) => db.get_installed_coaches(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_installed_coaches(user_id, tenant_id).await,
        }
    }

    async fn ensure_listing(&self, coach_id: &str, tenant_id: TenantId) -> AppResult<StoreListing> {
        match self {
            Self::SQLite(db) => db.ensure_listing(coach_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.ensure_listing(coach_id, tenant_id).await,
        }
    }
}
