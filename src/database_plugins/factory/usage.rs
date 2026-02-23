// ABOUTME: Usage repository dispatch for the database factory
// ABOUTME: Delegates UsageRepository, UsageCounterRepository, and LlmUsageRepository calls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::api_keys::{ApiKeyUsage, ApiKeyUsageStats};
use crate::database_plugins::{LlmUsageRepository, UsageCounterRepository, UsageRepository};
use crate::errors::AppResult;
use async_trait::async_trait;
use pierre_core::models::usage::{InsertLlmUsage, LlmUsageAggregateRow, LlmUsageDailyRow};
use pierre_core::models::TenantId;
use pierre_core::models::{JwtUsage, LlmUsageRecord, RequestLog, ToolUsage, UsageCounterRecord};

#[async_trait]
impl UsageRepository for Database {
    async fn record_api_key(&self, usage: &ApiKeyUsage) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.record_api_key(usage).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.record_api_key(usage).await,
        }
    }
    async fn get_api_key_current(&self, api_key_id: &str) -> AppResult<u32> {
        match self {
            Self::SQLite(db) => db.get_api_key_current(api_key_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_api_key_current(api_key_id).await,
        }
    }
    async fn get_api_key_stats(
        &self,
        api_key_id: &str,
        start_date: chrono::DateTime<chrono::Utc>,
        end_date: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<ApiKeyUsageStats> {
        match self {
            Self::SQLite(db) => db.get_api_key_stats(api_key_id, start_date, end_date).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_api_key_stats(api_key_id, start_date, end_date).await,
        }
    }
    async fn record_jwt_usage(&self, usage: &JwtUsage) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.record_jwt_usage(usage).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.record_jwt_usage(usage).await,
        }
    }
    async fn get_jwt_current_usage(&self, user_id: uuid::Uuid) -> AppResult<u32> {
        match self {
            Self::SQLite(db) => db.get_jwt_current_usage(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_jwt_current_usage(user_id).await,
        }
    }
    async fn get_request_logs(
        &self,
        user_id: Option<uuid::Uuid>,
        api_key_id: Option<&str>,
        start_time: Option<chrono::DateTime<chrono::Utc>>,
        end_time: Option<chrono::DateTime<chrono::Utc>>,
        status_filter: Option<&str>,
        tool_filter: Option<&str>,
    ) -> AppResult<Vec<RequestLog>> {
        match self {
            Self::SQLite(db) => {
                UsageRepository::get_request_logs(
                    db,
                    user_id,
                    api_key_id,
                    start_time,
                    end_time,
                    status_filter,
                    tool_filter,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_request_logs(
                    user_id,
                    api_key_id,
                    start_time,
                    end_time,
                    status_filter,
                    tool_filter,
                )
                .await
            }
        }
    }
    async fn get_system_stats(&self, tenant_id: Option<TenantId>) -> AppResult<(u64, u64)> {
        match self {
            Self::SQLite(db) => db.get_system_stats(tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_system_stats(tenant_id).await,
        }
    }
    async fn get_top_tools_analysis(
        &self,
        user_id: uuid::Uuid,
        start_time: chrono::DateTime<chrono::Utc>,
        end_time: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Vec<ToolUsage>> {
        match self {
            Self::SQLite(db) => {
                db.get_top_tools_analysis(user_id, start_time, end_time)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_top_tools_analysis(user_id, start_time, end_time)
                    .await
            }
        }
    }
}

#[async_trait]
impl UsageCounterRepository for Database {
    async fn increment_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
        amount: i64,
    ) -> AppResult<UsageCounterRecord> {
        match self {
            Self::SQLite(db) => {
                db.increment_usage_counter_impl(tenant_id, user_id, counter_key, period, amount)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.increment_counter(tenant_id, user_id, counter_key, period, amount)
                    .await
            }
        }
    }

    async fn get_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
    ) -> AppResult<UsageCounterRecord> {
        match self {
            Self::SQLite(db) => {
                db.get_usage_counter_impl(tenant_id, user_id, counter_key, period)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_counter(tenant_id, user_id, counter_key, period)
                    .await
            }
        }
    }

    async fn delete_old_counters(&self, period_before: &str) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => db.delete_old_usage_counters_impl(period_before).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.delete_old_counters(period_before).await,
        }
    }
}

#[async_trait]
impl LlmUsageRepository for Database {
    async fn insert_llm_usage(&self, params: &InsertLlmUsage<'_>) -> AppResult<LlmUsageRecord> {
        match self {
            Self::SQLite(db) => db.insert_llm_usage_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.insert_llm_usage(params).await,
        }
    }

    async fn get_llm_usage_aggregates(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageAggregateRow>> {
        match self {
            Self::SQLite(db) => db.get_llm_usage_aggregates_impl(tenant_id, since).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_llm_usage_aggregates(tenant_id, since).await,
        }
    }

    async fn get_llm_usage_daily_series(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageDailyRow>> {
        match self {
            Self::SQLite(db) => db.get_llm_usage_daily_series_impl(tenant_id, since).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_llm_usage_daily_series(tenant_id, since).await,
        }
    }
}
