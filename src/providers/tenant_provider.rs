// ABOUTME: Tenant-aware fitness provider factory for multi-tenant OAuth credential management
// ABOUTME: Routes provider requests through tenant-specific OAuth credentials and rate limiting
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database_plugins::DatabaseProvider;
use crate::errors::AppResult;
use crate::models::{Activity, Athlete, PersonalRecord, Stats};
use crate::tenant::TenantContext;
use async_trait::async_trait;

/// Tenant-aware fitness provider that wraps existing providers with tenant context
#[async_trait]
pub trait TenantFitnessProvider: Send + Sync {
    /// Authenticate using tenant-specific OAuth credentials
    async fn authenticate_tenant(
        &mut self,
        tenant_context: &TenantContext,
        provider: &str,
        database: &dyn DatabaseProvider,
    ) -> AppResult<()>;

    /// Get athlete information for the authenticated tenant user
    async fn get_athlete(&self) -> AppResult<Athlete>;

    /// Get activities for the authenticated tenant user
    async fn get_activities(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> AppResult<Vec<Activity>>;

    /// Get specific activity by ID
    async fn get_activity(&self, id: &str) -> AppResult<Activity>;

    /// Get stats for the authenticated tenant user
    async fn get_stats(&self) -> AppResult<Stats>;

    /// Get personal records for the authenticated tenant user
    async fn get_personal_records(&self) -> AppResult<Vec<PersonalRecord>>;

    /// Get provider name
    fn provider_name(&self) -> &'static str;
}

