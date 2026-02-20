// ABOUTME: Tenant database operations covering tenant CRUD, LLM credentials, fitness config, tool selection
// ABOUTME: Enables TenantRepository, LlmCredentialRepository, FitnessConfigRepository, ToolSelectionRepository
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::config::FitnessConfig;
use pierre_core::errors::AppResult;
use pierre_core::models::{
    LlmCredentialRecord, LlmCredentialSummary, OAuthApp, Tenant, TenantId, TenantOAuthCredentials,
    TenantPlan, TenantToolOverride, ToolCatalogEntry, ToolCategory,
};
use uuid::Uuid;

/// Tenant management, LLM credentials, fitness config, and tool selection database operations
#[async_trait]
pub trait TenantDbOps: Send + Sync + Clone {
    // --- Multi-Tenant Management ---

    /// Create a new tenant
    async fn create_tenant(&self, tenant: &Tenant) -> AppResult<()>;

    /// Get tenant by ID
    async fn get_tenant_by_id(&self, tenant_id: TenantId) -> AppResult<Tenant>;

    /// Get tenant by slug
    async fn get_tenant_by_slug(&self, slug: &str) -> AppResult<Tenant>;

    /// List tenants for a user
    async fn list_tenants_for_user(&self, user_id: Uuid) -> AppResult<Vec<Tenant>>;

    /// Get all tenants for key rotation check
    async fn get_all_tenants(&self) -> AppResult<Vec<Tenant>>;

    // --- Tenant OAuth Credentials ---

    /// Store tenant OAuth credentials
    async fn store_tenant_oauth_credentials(
        &self,
        credentials: &TenantOAuthCredentials,
    ) -> AppResult<()>;

    /// Get tenant OAuth providers
    async fn get_tenant_oauth_providers(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<TenantOAuthCredentials>>;

    /// Get tenant OAuth credentials for specific provider
    async fn get_tenant_oauth_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<TenantOAuthCredentials>>;

    // --- OAuth App Registration ---

    /// Create OAuth application for MCP clients
    async fn create_oauth_app(&self, app: &OAuthApp) -> AppResult<()>;

    /// Get OAuth app by client ID
    async fn get_oauth_app_by_client_id(&self, client_id: &str) -> AppResult<OAuthApp>;

    /// List OAuth apps for a user
    async fn list_oauth_apps_for_user(&self, user_id: Uuid) -> AppResult<Vec<OAuthApp>>;

    /// Get user role for a specific tenant
    async fn get_user_tenant_role(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<String>>;

    // --- LLM Credentials Management ---

    /// Store LLM credentials (user-specific or tenant-level)
    async fn store_llm_credentials(&self, record: &LlmCredentialRecord) -> AppResult<()>;

    /// Get LLM credentials for a specific provider
    async fn get_llm_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<Option<LlmCredentialRecord>>;

    /// List all LLM credentials for a tenant (for admin UI)
    async fn list_llm_credentials(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<LlmCredentialSummary>>;

    /// Delete LLM credentials
    async fn delete_llm_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<bool>;

    /// Get admin config override value by key (for system-wide LLM API keys)
    async fn get_admin_config_override(
        &self,
        config_key: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<String>>;

    // --- Fitness Configuration Management ---

    /// Save tenant-level fitness configuration
    async fn save_tenant_fitness_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String>;

    /// Save user-specific fitness configuration
    async fn save_user_fitness_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String>;

    /// Get tenant-level fitness configuration
    async fn get_tenant_fitness_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>>;

    /// Get user-specific fitness configuration
    async fn get_user_fitness_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>>;

    /// List all tenant-level fitness configuration names
    async fn list_tenant_fitness_configurations(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<String>>;

    /// List all user-specific fitness configuration names
    async fn list_user_fitness_configurations(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<String>>;

    /// Delete fitness configuration (tenant or user-specific)
    async fn delete_fitness_config(
        &self,
        tenant_id: TenantId,
        user_id: Option<&str>,
        configuration_name: &str,
    ) -> AppResult<bool>;

    // --- Tool Selection ---

    /// Get the complete tool catalog
    async fn get_tool_catalog(&self) -> AppResult<Vec<ToolCatalogEntry>>;

    /// Get a specific tool catalog entry by name
    async fn get_tool_catalog_entry(&self, tool_name: &str) -> AppResult<Option<ToolCatalogEntry>>;

    /// Get tools filtered by category
    async fn get_tools_by_category(
        &self,
        category: ToolCategory,
    ) -> AppResult<Vec<ToolCatalogEntry>>;

    /// Get tools available for a specific plan level
    async fn get_tools_by_min_plan(&self, plan: TenantPlan) -> AppResult<Vec<ToolCatalogEntry>>;

    /// Get all tool overrides for a tenant
    async fn get_tenant_tool_overrides(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<TenantToolOverride>>;

    /// Get a specific tool override for a tenant
    async fn get_tenant_tool_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> AppResult<Option<TenantToolOverride>>;

    /// Create or update a tool override for a tenant
    async fn upsert_tenant_tool_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
        is_enabled: bool,
        enabled_by_user_id: Option<Uuid>,
        reason: Option<String>,
    ) -> AppResult<TenantToolOverride>;

    /// Delete a tool override (revert to catalog default)
    async fn delete_tenant_tool_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> AppResult<bool>;

    /// Count enabled tools for a tenant
    async fn count_enabled_tools(&self, tenant_id: TenantId) -> AppResult<usize>;
}
