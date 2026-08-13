// ABOUTME: Repository trait definitions for the tenants and subscriptions domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::{OAuthApp, Tenant};
use pierre_core::models::{Subscription, SubscriptionStatus, TenantId, TenantOAuthCredentials};
use uuid::Uuid;

/// Multi-tenant management repository
#[async_trait]
pub trait TenantRepository: Send + Sync {
    /// Create a new tenant
    async fn create(&self, tenant: &Tenant) -> AppResult<()>;
    /// Get tenant by ID
    async fn get_by_id(&self, tenant_id: TenantId) -> AppResult<Tenant>;
    /// Get tenant by slug
    async fn get_by_slug(&self, slug: &str) -> AppResult<Tenant>;
    /// List tenants for a user
    async fn list_for_user(&self, user_id: Uuid) -> AppResult<Vec<Tenant>>;

    /// The coach this user has selected within this tenant, if any.
    ///
    /// The single answer to "which coach is this user's?". It replaced three
    /// disagreeing ones — `users.default_coach_id`, `coach_assignments.is_active`
    /// and per-conversation overrides — where the surface that wrote and the
    /// surface that read were often different, so a user could finish onboarding
    /// on one and read as un-onboarded on another.
    ///
    /// Scoped per membership because coaches are tenant-scoped: a user in two
    /// tenants selects independently in each.
    async fn get_selected_coach(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Option<String>>;

    /// Set (or clear, with `None`) this user's selected coach in this tenant.
    ///
    /// "At most one" is structural — one column on a row that `UNIQUE(tenant_id,
    /// user_id)` already makes unique — rather than maintained by clearing every
    /// row and setting one, which could leave zero or two.
    async fn set_selected_coach(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        coach_id: Option<&str>,
    ) -> AppResult<()>;
    /// Store tenant OAuth credentials
    async fn store_oauth_credentials(&self, credentials: &TenantOAuthCredentials) -> AppResult<()>;
    /// Get tenant OAuth providers
    async fn get_oauth_providers(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<TenantOAuthCredentials>>;
    /// Get tenant OAuth credentials for specific provider
    async fn get_oauth_credentials(
        &self,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<TenantOAuthCredentials>>;
    /// Create OAuth application for MCP clients
    async fn create_oauth_app(&self, app: &OAuthApp) -> AppResult<()>;
    /// Get OAuth app by client ID
    async fn get_oauth_app_by_client_id(&self, client_id: &str) -> AppResult<OAuthApp>;
    /// List OAuth apps for a user
    async fn list_oauth_apps_for_user(&self, user_id: Uuid) -> AppResult<Vec<OAuthApp>>;
    /// Get all tenants for key rotation check
    async fn get_all(&self) -> AppResult<Vec<Tenant>>;
    /// Get user role for a specific tenant
    async fn get_user_role(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<Option<String>>;
    /// Set the tenant's billing plan (Starter / Professional / Enterprise).
    ///
    /// Called by Stripe webhook handlers when a subscription state change
    /// implies a plan-tier flip on the tenant. Owner-driven plan changes
    /// always cascade through this method so audit logging stays uniform.
    async fn set_plan(&self, tenant_id: TenantId, plan: &str) -> AppResult<Tenant>;
}

/// Provider-agnostic subscription persistence.
///
/// Webhook handlers upsert by `(provider, provider_customer_id)`;
/// read paths query by user, tenant, or `(provider, *)` identifier.
#[async_trait]
pub trait SubscriptionsRepository: Send + Sync {
    /// Insert a new subscription row or update the existing row that
    /// shares its `(provider, provider_customer_id)` key. Returns the
    /// freshly written row.
    async fn upsert_subscription(&self, subscription: &Subscription) -> AppResult<Subscription>;
    /// Look up the most recently updated subscription for a user.
    async fn get_subscription_by_user(&self, user_id: Uuid) -> AppResult<Option<Subscription>>;
    /// Look up the most recently updated subscription for a tenant.
    async fn get_subscription_by_tenant(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Option<Subscription>>;
    /// Look up a subscription by its provider-side subscription identifier.
    async fn get_subscription_by_provider_subscription_id(
        &self,
        provider: &str,
        provider_subscription_id: &str,
    ) -> AppResult<Option<Subscription>>;
    /// Look up a subscription by its provider-side customer identifier.
    async fn get_subscription_by_provider_customer_id(
        &self,
        provider: &str,
        provider_customer_id: &str,
    ) -> AppResult<Option<Subscription>>;
    /// List every subscription with the given lifecycle status.
    /// Used by admin filter views and the dunning sweep.
    async fn list_subscriptions_by_status(
        &self,
        status: SubscriptionStatus,
    ) -> AppResult<Vec<Subscription>>;

    /// Returns true when the given `(provider, event_id)` has already
    /// been processed. The webhook handler skips dispatch on `true`,
    /// making the entire pipeline safe against provider retries.
    async fn is_billing_event_processed(&self, provider: &str, event_id: &str) -> AppResult<bool>;
    /// Mark a `(provider, event_id)` as processed. Call this AFTER the
    /// event dispatch path succeeds — never before — so a partial failure
    /// allows the webhook to retry with the full body.
    async fn mark_billing_event_processed(
        &self,
        provider: &str,
        event_id: &str,
        event_type: &str,
    ) -> AppResult<()>;
}
