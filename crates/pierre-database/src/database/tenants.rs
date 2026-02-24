// ABOUTME: Tenant management database operations and repository trait implementation
// ABOUTME: Handles tenant CRUD, OAuth credentials, and user-tenant role queries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::repositories::TenantRepository;
use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::models::{OAuthApp, Tenant, TenantOAuthCredentials};
use uuid::Uuid;

#[async_trait]
impl TenantRepository for Database {
    async fn create(&self, tenant: &Tenant) -> AppResult<()> {
        Self::create_tenant_impl(self, tenant).await
    }
    async fn get_by_id(&self, tenant_id: TenantId) -> AppResult<Tenant> {
        Self::get_tenant_by_id_impl(self, tenant_id).await
    }
    async fn get_by_slug(&self, slug: &str) -> AppResult<Tenant> {
        Self::get_tenant_by_slug_impl(self, slug).await
    }
    async fn list_for_user(&self, user_id: Uuid) -> AppResult<Vec<Tenant>> {
        Self::list_tenants_for_user_impl(self, user_id).await
    }
    async fn store_oauth_credentials(&self, credentials: &TenantOAuthCredentials) -> AppResult<()> {
        Self::store_tenant_oauth_credentials_impl(self, credentials).await
    }
    async fn get_oauth_providers(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<TenantOAuthCredentials>> {
        Self::get_tenant_oauth_providers_impl(self, tenant_id).await
    }
    async fn get_oauth_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<TenantOAuthCredentials>> {
        Self::get_tenant_oauth_credentials_impl(self, tenant_id, provider).await
    }
    async fn create_oauth_app(&self, app: &OAuthApp) -> AppResult<()> {
        Self::create_oauth_app_impl(self, app).await
    }
    async fn get_oauth_app_by_client_id(&self, client_id: &str) -> AppResult<OAuthApp> {
        Self::get_oauth_app_by_client_id_impl(self, client_id).await
    }
    async fn list_oauth_apps_for_user(&self, user_id: Uuid) -> AppResult<Vec<OAuthApp>> {
        Self::list_oauth_apps_for_user(self, user_id).await
    }
    async fn get_all(&self) -> AppResult<Vec<Tenant>> {
        Self::get_all_tenants_impl(self).await
    }
    async fn get_user_role(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<String>> {
        Self::get_user_tenant_role_impl(self, user_id, tenant_id).await
    }
}
