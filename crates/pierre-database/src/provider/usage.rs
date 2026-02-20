// ABOUTME: Usage tracking database operations covering LLM usage, counters, and analytics
// ABOUTME: Enables UsageRepository, LlmUsageRepository, and UsageCounterRepository blanket impls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::usage::{InsertLlmUsage, LlmUsageAggregateRow, LlmUsageDailyRow};
use pierre_core::models::{
    ApiKeyUsage, ApiKeyUsageStats, JwtUsage, LlmUsageRecord, RequestLog, TenantId, ToolUsage,
    UsageCounterRecord,
};
use uuid::Uuid;

/// Usage tracking and analytics database operations
#[async_trait]
pub trait UsageDbOps: Send + Sync + Clone {
    // --- Analytics & Intelligence ---

    /// Get top tools analysis for dashboard
    async fn get_top_tools_analysis(
        &self,
        user_id: Uuid,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> AppResult<Vec<ToolUsage>>;

    // --- API Key & JWT Usage Tracking ---

    /// Record API key usage
    async fn record_api_key_usage(&self, usage: &ApiKeyUsage) -> AppResult<()>;

    /// Get current usage count for an API key
    async fn get_api_key_current_usage(&self, api_key_id: &str) -> AppResult<u32>;

    /// Get usage statistics for an API key
    async fn get_api_key_usage_stats(
        &self,
        api_key_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<ApiKeyUsageStats>;

    /// Record JWT token usage for rate limiting and analytics
    async fn record_jwt_usage(&self, usage: &JwtUsage) -> AppResult<()>;

    /// Get current JWT usage count for rate limiting (current month)
    async fn get_jwt_current_usage(&self, user_id: Uuid) -> AppResult<u32>;

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

    /// Get system statistics, optionally scoped to a tenant
    async fn get_system_stats(&self, tenant_id: Option<TenantId>) -> AppResult<(u64, u64)>;

    // --- LLM Usage Tracking ---

    /// Insert a new LLM usage record for cost analysis and quota enforcement
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

    // --- Usage Counters ---

    /// Atomically increment a usage counter via upsert
    async fn increment_usage_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
        amount: i64,
    ) -> AppResult<UsageCounterRecord>;

    /// Get the current value of a usage counter (returns 0 if not found)
    async fn get_usage_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
    ) -> AppResult<UsageCounterRecord>;

    /// Delete usage counters older than the given period cutoff
    async fn delete_old_usage_counters(&self, period_before: &str) -> AppResult<u64>;
}
