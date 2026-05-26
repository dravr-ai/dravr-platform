// ABOUTME: Repository trait definitions for the data source registration persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::DataSource;
use pierre_core::models::TenantId;
use uuid::Uuid;

/// Data source (device/provider) tracking repository
#[async_trait]
pub trait DataSourceRepository: Send + Sync {
    /// Upsert a data source record (insert or update on conflict)
    async fn upsert_data_source(
        &self,
        tenant_id: &TenantId,
        source: &DataSource,
    ) -> AppResult<String>;
    /// Get a data source by ID
    async fn get_data_source(&self, id: &str) -> AppResult<Option<DataSource>>;
    /// List data sources for a user
    async fn list_data_sources(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Vec<DataSource>>;
    /// List data sources for a user filtered by provider
    async fn list_data_sources_by_provider(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<Vec<DataSource>>;
    /// Delete a data source
    async fn delete_data_source(&self, id: &str) -> AppResult<()>;
}
