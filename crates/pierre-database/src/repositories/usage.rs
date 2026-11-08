// ABOUTME: Repository trait definitions for the API/LLM/usage-counter accounting + LLM credentials domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::admin::models::AdminConfigOverrideRow;
use pierre_core::errors::AppResult;

use pierre_core::models::usage::{InsertLlmUsage, LlmUsageAggregateRow, LlmUsageDailyRow};
use pierre_core::models::{ApiKeyUsage, ApiKeyUsageStats, WindowUsage};
use pierre_core::models::{
    ConversationTurnId, JwtMonthlyUsage, JwtUsage, LlmUsageRecord, RequestLog, ToolUsage,
    UsageCounterRecord,
};
use pierre_core::models::{LlmCredentialRecord, LlmCredentialSummary, TenantId};
use uuid::Uuid;

/// Usage tracking and analytics repository
#[async_trait]
pub trait UsageRepository: Send + Sync {
    /// Record API key usage
    async fn record_api_key(&self, usage: &ApiKeyUsage) -> AppResult<()>;
    /// An API key's calls after `window_start` (its sliding rate-limit
    /// window) and the earliest of them
    async fn get_api_key_window_usage(
        &self,
        api_key_id: &str,
        window_start: DateTime<Utc>,
    ) -> AppResult<WindowUsage>;
    /// Get usage statistics for an API key
    async fn get_api_key_stats(
        &self,
        api_key_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<ApiKeyUsageStats>;
    /// Record JWT token usage for rate limiting and analytics
    async fn record_jwt_usage(&self, usage: &JwtUsage) -> AppResult<()>;
    /// The user's JWT requests this UTC month and the admin's monthly
    /// override for them, read in one statement: the inputs of the JWT
    /// request budget besides the tier
    async fn get_jwt_current_usage(&self, user_id: Uuid) -> AppResult<JwtMonthlyUsage>;
    /// Get request logs with filtering options
    async fn get_request_logs(
        &self,
        user_id: Option<Uuid>,
        api_key_id: Option<&str>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        status_filter: Option<&str>,
        tool_filter: Option<&str>,
    ) -> AppResult<Vec<RequestLog>>;
    /// Get top tools analysis for dashboard
    async fn get_top_tools_analysis(
        &self,
        user_id: Uuid,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> AppResult<Vec<ToolUsage>>;
    /// How many requests the user's API keys made in the window, on every
    /// endpoint. [`Self::get_top_tools_analysis`] only reads the busiest ten.
    async fn count_api_key_requests(
        &self,
        user_id: Uuid,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> AppResult<u64>;
}

/// Usage counter repository for rate limiting and quota enforcement
#[async_trait]
pub trait UsageCounterRepository: Send + Sync {
    /// Atomically increment a usage counter via upsert
    async fn increment_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
        amount: i64,
    ) -> AppResult<UsageCounterRecord>;

    /// Add one to a counter only while it holds fewer than `limit`, creating
    /// it at one when absent. Returns whether the increment landed.
    ///
    /// The check and the write are one statement, so callers racing for the
    /// last unit under `limit` — on one instance or several sharing the
    /// database — get it once between them, and the counter never passes
    /// `limit`. A `limit` below one takes nothing.
    async fn increment_counter_below(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
        limit: i64,
    ) -> AppResult<bool>;

    /// Get the current value of a counter (returns 0 if not found)
    async fn get_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
    ) -> AppResult<UsageCounterRecord>;

    /// Delete counters older than the given period cutoff
    ///
    /// System-level housekeeping: intentionally operates across ALL tenants.
    /// Called only from background pruning tasks, not user-facing endpoints.
    async fn delete_old_counters(&self, period_before: &str) -> AppResult<u64>;
}

/// The rows an aggregate of `llm_usage` was summed over.
#[derive(Debug, Clone, Copy)]
pub enum LlmUsageScope<'scope> {
    /// Every tenant: the super-admin path.
    AllTenants,
    /// One tenant, by its id.
    Tenant(&'scope str),
    /// One user in every tenant they belong to: the super-admin path.
    User(&'scope str),
    /// One user's rows in one tenant.
    UserInTenant {
        /// The user's id.
        user_id: &'scope str,
        /// The tenant's id.
        tenant_id: &'scope str,
    },
}

impl<'scope> LlmUsageScope<'scope> {
    /// The scope as the `(tenant_id, user_id)` pair a statement filters on,
    /// each `None` for "any".
    #[must_use]
    pub const fn filters(self) -> (Option<&'scope str>, Option<&'scope str>) {
        match self {
            Self::AllTenants => (None, None),
            Self::Tenant(tenant_id) => (Some(tenant_id), None),
            Self::User(user_id) => (None, Some(user_id)),
            Self::UserInTenant { user_id, tenant_id } => (Some(tenant_id), Some(user_id)),
        }
    }
}

/// The summed calls of one aggregate row on one side of a model's prompt-size
/// threshold, from [`LlmUsageRepository::get_llm_usage_prompt_bands_since`].
#[derive(Debug, Clone)]
pub struct LlmUsagePromptBand {
    /// `true` for the calls whose prompt exceeded the threshold.
    pub long_prompt: bool,
    /// Their summed tokens.
    pub totals: LlmUsageAggregateRow,
}

/// LLM usage tracking repository for cost analysis and quota enforcement
#[async_trait]
pub trait LlmUsageRepository: Send + Sync {
    /// Insert a new LLM usage record
    async fn insert_llm_usage(&self, params: &InsertLlmUsage<'_>) -> AppResult<LlmUsageRecord>;

    /// Query aggregated LLM usage grouped by `provider`, `model`, and
    /// `call_type`, of one tenant or, with `None`, of every tenant: the
    /// super-admin path of the Analytics screen and the platform's cost today.
    /// Leaves out rows that are no LLM call.
    async fn get_llm_usage_aggregates(
        &self,
        tenant_id: Option<&str>,
        since: &str,
    ) -> AppResult<Vec<LlmUsageAggregateRow>>;

    /// Query daily LLM usage time series for consumption charts, of one tenant
    /// or, with `None`, of every tenant.
    async fn get_llm_usage_daily_series(
        &self,
        tenant_id: Option<&str>,
        since: &str,
    ) -> AppResult<Vec<LlmUsageDailyRow>>;

    /// Per-user variant of [`Self::get_llm_usage_aggregates`].
    /// Drives the admin `GET /api/admin/users/{id}/usage` endpoint
    /// so per-user billing surfaces (Usage tab, invoice preview)
    /// don't have to sum tenant aggregates client-side.
    async fn get_llm_usage_aggregates_by_user(
        &self,
        user_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageAggregateRow>>;

    /// Per-user variant of [`Self::get_llm_usage_daily_series`].
    /// Drives the admin `GET /api/admin/users/{id}/cost-timeseries`
    /// endpoint that the Usage tab uses to render daily cost gauges.
    async fn get_llm_usage_daily_series_by_user(
        &self,
        user_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageDailyRow>>;

    /// [`Self::get_llm_usage_aggregates_by_user`] for the user's rows in one tenant,
    /// which is all a tenant's admin may read of a user who also belongs elsewhere.
    async fn get_llm_usage_aggregates_by_user_in_tenant(
        &self,
        user_id: &str,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageAggregateRow>>;

    /// [`Self::get_llm_usage_daily_series_by_user`] for the user's rows in one tenant.
    async fn get_llm_usage_daily_series_by_user_in_tenant(
        &self,
        user_id: &str,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageDailyRow>>;

    /// Get the most recent LLM usage records of one tenant or, with `None`,
    /// across all tenants (admin view)
    ///
    /// Returns the last `limit` LLM calls ordered by creation time descending,
    /// leaving out rows that are no LLM call (turn summaries, direct tool calls).
    /// Used by the admin real-time activity dashboard.
    async fn get_recent_llm_calls_admin(
        &self,
        tenant_id: Option<&str>,
        limit: i64,
    ) -> AppResult<Vec<LlmUsageRecord>>;

    /// [`Self::get_recent_llm_calls_admin`] for the calls made in `[start, end)`,
    /// of one tenant or, with `None`, of every tenant, newest first: the
    /// billing export of one month.
    async fn get_llm_calls_between(
        &self,
        tenant_id: Option<&str>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<LlmUsageRecord>>;

    /// Fetch the LLM usage rows of one tenant or, with `None`, of every tenant
    /// since `since` (RFC3339) that invoked at least one tool, for the
    /// tool-usage admin view. Per-tool aggregation is done in the handler (in
    /// Rust) so this stays backend-agnostic — no JSON SQL. Rows with no tools
    /// (`tools_called == "[]"`) are excluded.
    async fn get_tool_calls_since(
        &self,
        tenant_id: Option<TenantId>,
        since: &str,
    ) -> AppResult<Vec<LlmUsageRecord>>;

    /// The calls behind one aggregate row, read in the scope the row was summed
    /// over and summed per side of `above_prompt_tokens`: at most two rows, for
    /// a model whose price depends on each call's own prompt. Pricing reads
    /// these instead of every call, which grew with the window.
    async fn get_llm_usage_prompt_bands_since(
        &self,
        scope: LlmUsageScope<'_>,
        group: &LlmUsageAggregateRow,
        since: &str,
        above_prompt_tokens: i64,
    ) -> AppResult<Vec<LlmUsagePromptBand>>;

    /// Fetch every LLM usage record attributed to the given conversation
    /// turn, ordered by `created_at` ascending.
    ///
    /// Returns an empty vector when no rows match.
    async fn find_llm_usage_by_turn_id(
        &self,
        turn_id: ConversationTurnId,
    ) -> AppResult<Vec<LlmUsageRecord>>;
}

/// LLM credential management repository
#[async_trait]
pub trait LlmCredentialRepository: Send + Sync {
    /// Store LLM credentials (user-specific or tenant-level)
    async fn store_credentials(&self, record: &LlmCredentialRecord) -> AppResult<()>;
    /// Get LLM credentials for a specific provider
    async fn get_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<Option<LlmCredentialRecord>>;
    /// List all LLM credentials for a tenant (for admin UI)
    async fn list_credentials(&self, tenant_id: TenantId) -> AppResult<Vec<LlmCredentialSummary>>;
    /// Delete LLM credentials
    async fn delete_credentials(
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

    /// List every `admin_config_overrides` row for a given category.
    /// Used by the `PricingRegistry` startup loader to populate
    /// `cat_llm_pricing` overrides on top of the compile-time table.
    async fn list_admin_config_overrides_by_category(
        &self,
        category: &str,
    ) -> AppResult<Vec<AdminConfigOverrideRow>>;
}
