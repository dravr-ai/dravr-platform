// ABOUTME: Repository trait definitions for the data source registration persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::NaiveDate;
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
    /// List data sources for a user
    async fn list_data_sources(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Vec<DataSource>>;
    /// The earliest day a wellness row of the user's `provider` filed under
    /// a data source marked `source` holds in `tenant_id`: a night by the day
    /// it ended, a recovery, body or daily-activity row by its date. `None`
    /// when no such row exists. Soft-deleted rows count: the source held the
    /// day.
    async fn earliest_source_day(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        source: &str,
    ) -> AppResult<Option<NaiveDate>>;
}
