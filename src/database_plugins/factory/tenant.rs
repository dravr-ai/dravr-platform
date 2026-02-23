// ABOUTME: Tenant repository dispatch for the database factory
// ABOUTME: Delegates TenantRepository, ToolSelectionRepository, LlmCredentialRepository, and FitnessConfigRepository calls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::config::fitness::FitnessConfig;
use crate::database_plugins::{
    FitnessConfigRepository, LlmCredentialRepository, TenantRepository, ToolSelectionRepository,
};
use crate::errors::AppResult;
use crate::models::{
    OAuthApp, Tenant, TenantPlan, TenantToolOverride, ToolCatalogEntry, ToolCategory,
};
use async_trait::async_trait;
use pierre_core::models::{
    LlmCredentialRecord, LlmCredentialSummary, TenantId, TenantOAuthCredentials,
};
use uuid::Uuid;

#[async_trait]
impl TenantRepository for Database {
    async fn create(&self, tenant: &Tenant) -> AppResult<()> {
        match self {
            Self::SQLite(db) => TenantRepository::create(db, tenant).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => TenantRepository::create(db, tenant).await,
        }
    }
    async fn get_by_id(&self, tenant_id: TenantId) -> AppResult<Tenant> {
        match self {
            Self::SQLite(db) => TenantRepository::get_by_id(db, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => TenantRepository::get_by_id(db, tenant_id).await,
        }
    }
    async fn get_by_slug(&self, slug: &str) -> AppResult<Tenant> {
        match self {
            Self::SQLite(db) => db.get_by_slug(slug).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_slug(slug).await,
        }
    }
    async fn list_for_user(&self, user_id: uuid::Uuid) -> AppResult<Vec<Tenant>> {
        match self {
            Self::SQLite(db) => db.list_for_user(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_for_user(user_id).await,
        }
    }
    async fn store_oauth_credentials(&self, credentials: &TenantOAuthCredentials) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_oauth_credentials(credentials).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_oauth_credentials(credentials).await,
        }
    }
    async fn get_oauth_providers(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<TenantOAuthCredentials>> {
        match self {
            Self::SQLite(db) => db.get_oauth_providers(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_oauth_providers(tenant_id).await,
        }
    }
    async fn get_oauth_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<TenantOAuthCredentials>> {
        match self {
            Self::SQLite(db) => db.get_oauth_credentials(tenant_id, provider).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_oauth_credentials(tenant_id, provider).await,
        }
    }
    async fn create_oauth_app(&self, app: &OAuthApp) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.create_oauth_app(app).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create_oauth_app(app).await,
        }
    }
    async fn get_oauth_app_by_client_id(&self, client_id: &str) -> AppResult<OAuthApp> {
        match self {
            Self::SQLite(db) => db.get_oauth_app_by_client_id(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_oauth_app_by_client_id(client_id).await,
        }
    }
    async fn list_oauth_apps_for_user(&self, user_id: uuid::Uuid) -> AppResult<Vec<OAuthApp>> {
        match self {
            Self::SQLite(db) => db.list_oauth_apps_for_user(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_oauth_apps_for_user(user_id).await,
        }
    }
    async fn get_all(&self) -> AppResult<Vec<Tenant>> {
        match self {
            Self::SQLite(db) => TenantRepository::get_all(db).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => TenantRepository::get_all(db).await,
        }
    }
    async fn get_user_role(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<String>> {
        match self {
            Self::SQLite(db) => TenantRepository::get_user_role(db, user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_user_role(user_id, tenant_id).await,
        }
    }
}

#[async_trait]
impl ToolSelectionRepository for Database {
    async fn get_tool_catalog(&self) -> AppResult<Vec<ToolCatalogEntry>> {
        match self {
            Self::SQLite(db) => db.get_tool_catalog_impl().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tool_catalog().await,
        }
    }

    async fn get_tool_catalog_entry(&self, tool_name: &str) -> AppResult<Option<ToolCatalogEntry>> {
        match self {
            Self::SQLite(db) => db.get_tool_catalog_entry_impl(tool_name).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tool_catalog_entry(tool_name).await,
        }
    }

    async fn get_tools_by_category(
        &self,
        category: ToolCategory,
    ) -> AppResult<Vec<ToolCatalogEntry>> {
        match self {
            Self::SQLite(db) => db.get_tools_by_category_impl(category).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tools_by_category(category).await,
        }
    }

    async fn get_tools_by_min_plan(&self, plan: TenantPlan) -> AppResult<Vec<ToolCatalogEntry>> {
        match self {
            Self::SQLite(db) => db.get_tools_by_min_plan_impl(plan).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tools_by_min_plan(plan).await,
        }
    }

    async fn get_overrides(&self, tenant_id: TenantId) -> AppResult<Vec<TenantToolOverride>> {
        match self {
            Self::SQLite(db) => db.get_tenant_tool_overrides_impl(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_overrides(tenant_id).await,
        }
    }

    async fn get_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> AppResult<Option<TenantToolOverride>> {
        match self {
            Self::SQLite(db) => db.get_tenant_tool_override_impl(tenant_id, tool_name).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_override(tenant_id, tool_name).await,
        }
    }

    async fn upsert_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
        is_enabled: bool,
        enabled_by_user_id: Option<Uuid>,
        reason: Option<String>,
    ) -> AppResult<TenantToolOverride> {
        match self {
            Self::SQLite(db) => {
                db.upsert_tenant_tool_override_impl(
                    tenant_id,
                    tool_name,
                    is_enabled,
                    enabled_by_user_id,
                    reason,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.upsert_override(tenant_id, tool_name, is_enabled, enabled_by_user_id, reason)
                    .await
            }
        }
    }

    async fn delete_override(&self, tenant_id: TenantId, tool_name: &str) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.delete_tenant_tool_override_impl(tenant_id, tool_name)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_override(tenant_id, tool_name).await,
        }
    }

    async fn count_enabled_tools(&self, tenant_id: TenantId) -> AppResult<usize> {
        match self {
            Self::SQLite(db) => db.count_enabled_tools_impl(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.count_enabled_tools(tenant_id).await,
        }
    }
}

#[async_trait]
impl LlmCredentialRepository for Database {
    async fn store_credentials(&self, record: &LlmCredentialRecord) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.store_credentials(record).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.store_credentials(record).await,
        }
    }

    async fn get_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<Option<LlmCredentialRecord>> {
        match self {
            Self::SQLite(db) => db.get_credentials(tenant_id, user_id, provider).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_credentials(tenant_id, user_id, provider).await,
        }
    }

    async fn list_credentials(&self, tenant_id: TenantId) -> AppResult<Vec<LlmCredentialSummary>> {
        match self {
            Self::SQLite(db) => db.list_credentials(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_credentials(tenant_id).await,
        }
    }

    async fn delete_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.delete_credentials(tenant_id, user_id, provider).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_credentials(tenant_id, user_id, provider).await,
        }
    }

    async fn get_admin_config_override(
        &self,
        config_key: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<String>> {
        match self {
            Self::SQLite(db) => db.get_admin_config_override(config_key, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_admin_config_override(config_key, tenant_id).await,
        }
    }
}

#[async_trait]
impl FitnessConfigRepository for Database {
    async fn save_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => {
                db.save_tenant_config(tenant_id, configuration_name, config)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.save_tenant_config(tenant_id, configuration_name, config)
                    .await
            }
        }
    }

    async fn save_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => {
                db.save_user_config(tenant_id, user_id, configuration_name, config)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.save_user_config(tenant_id, user_id, configuration_name, config)
                    .await
            }
        }
    }

    async fn get_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>> {
        match self {
            Self::SQLite(db) => db.get_tenant_config(tenant_id, configuration_name).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_tenant_config(tenant_id, configuration_name).await,
        }
    }

    async fn get_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>> {
        match self {
            Self::SQLite(db) => {
                db.get_user_config(tenant_id, user_id, configuration_name)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_user_config(tenant_id, user_id, configuration_name)
                    .await
            }
        }
    }

    async fn list_tenant_configurations(&self, tenant_id: TenantId) -> AppResult<Vec<String>> {
        match self {
            Self::SQLite(db) => db.list_tenant_configurations(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_tenant_configurations(tenant_id).await,
        }
    }

    async fn list_user_configurations(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<String>> {
        match self {
            Self::SQLite(db) => db.list_user_configurations(tenant_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_user_configurations(tenant_id, user_id).await,
        }
    }

    async fn delete_config(
        &self,
        tenant_id: TenantId,
        user_id: Option<&str>,
        configuration_name: &str,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.delete_config(tenant_id, user_id, configuration_name)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.delete_config(tenant_id, user_id, configuration_name)
                    .await
            }
        }
    }
}
