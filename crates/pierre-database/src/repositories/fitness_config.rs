// ABOUTME: Repository trait definitions for the fitness configuration persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::config::FitnessConfig;
use pierre_core::errors::AppResult;

use pierre_core::models::TenantId;

/// Fitness configuration management repository
#[async_trait]
pub trait FitnessConfigRepository: Send + Sync {
    /// Save tenant-level fitness configuration
    async fn save_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String>;
    /// Save user-specific fitness configuration
    async fn save_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String>;
    /// Get tenant-level fitness configuration
    async fn get_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>>;
    /// Get user-specific fitness configuration
    async fn get_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>>;
    /// List all tenant-level fitness configuration names
    async fn list_tenant_configurations(&self, tenant_id: TenantId) -> AppResult<Vec<String>>;
    /// List all user-specific fitness configuration names
    async fn list_user_configurations(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<String>>;
    /// Delete fitness configuration (tenant or user-specific)
    async fn delete_config(
        &self,
        tenant_id: TenantId,
        user_id: Option<&str>,
        configuration_name: &str,
    ) -> AppResult<bool>;
}
