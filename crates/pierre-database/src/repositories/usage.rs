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
use pierre_core::models::{ApiKeyUsage, ApiKeyUsageStats, ApiKeyWindowUsage};
use pierre_core::models::{
    ConversationTurnId, JwtUsage, LlmUsageRecord, RequestLog, ToolUsage, UsageCounterRecord,
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
    ) -> AppResult<ApiKeyWindowUsage>;
    /// Get usage statistics for an API key
    async fn get_api_key_stats(
        &self,
        api_key_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<ApiKeyUsageStats>;
    /// Record JWT token usage for rate limiting and analytics
    async fn record_jwt_usage(&self, usage: &JwtUsage) -> AppResult<()>;
    /// Get current JWT usage count for rate limiting (current month)
    async fn get_jwt_current_usage(&self, user_id: Uuid) -> AppResult<u32>;
    /// JWT usage count since the start of the current UTC day, for a
    /// per-user daily request limit
    async fn get_jwt_usage_today(&self, user_id: Uuid) -> AppResult<u32>;
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

/// LLM usage tracking repository for cost analysis and quota enforcement
#[async_trait]
pub trait LlmUsageRepository: Send + Sync {
    /// Insert a new LLM usage record
    async fn insert_llm_usage(&self, params: &InsertLlmUsage<'_>) -> AppResult<LlmUsageRecord>;

    /// Query aggregated LLM usage grouped by `provider`, `model`, and `call_type`
    async fn get_llm_usage_aggregates(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageAggregateRow>>;

    /// Query daily LLM usage time series for consumption charts
    async fn get_llm_usage_daily_series(
        &self,
        tenant_id: &str,
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

    /// Get the most recent LLM usage records across all tenants (admin view)
    ///
    /// Returns the last `limit` LLM calls ordered by creation time descending.
    /// Used by the admin real-time activity dashboard.
    async fn get_recent_llm_calls_admin(&self, limit: i64) -> AppResult<Vec<LlmUsageRecord>>;

    /// Count LLM calls created since a given timestamp (admin view, cross-tenant)
    async fn count_llm_calls_since(&self, since: &str) -> AppResult<i64>;

    /// Fetch a tenant's LLM usage rows since `since` (RFC3339) that invoked at
    /// least one tool, for the tool-usage admin view. Per-tool aggregation is
    /// done in the handler (in Rust) so this stays backend-agnostic — no JSON
    /// SQL. Rows with no tools (`tools_called == "[]"`) are excluded.
    async fn get_tenant_tool_calls_since(
        &self,
        tenant_id: TenantId,
        since: &str,
    ) -> AppResult<Vec<LlmUsageRecord>>;

    /// Sum LLM usage since a given timestamp (admin view, cross-tenant).
    ///
    /// Returns a tuple of (`total_calls`, `total_tokens`) for pricing calculations.
    async fn sum_llm_usage_since(&self, since: &str) -> AppResult<(i64, i64)>;

    /// Fetch every LLM usage record attributed to the given conversation
    /// turn, ordered by `created_at` ascending.
    ///
    /// Returns an empty vector when no rows match.
    async fn find_llm_usage_by_turn_id(
        &self,
        turn_id: ConversationTurnId,
    ) -> AppResult<Vec<LlmUsageRecord>>;

    /// Sum `cost_usd` for a tenant over an inclusive period. Used by the
    /// monthly overage cron to drive Stripe Meter event reporting.
    async fn sum_cost_usd_for_tenant_period(
        &self,
        tenant_id: TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<f64>;
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
