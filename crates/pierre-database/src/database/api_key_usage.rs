// ABOUTME: SQLite api_key_usage statements — record a call, count the current window, aggregate a period's stats
// ABOUTME: The api-key half of the SQLite UsageRepository, reached from analytics.rs; the key rows themselves are in api_keys.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::backends::ApiKeyRepository;
use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ApiKeyUsage, ApiKeyUsageStats};
use sqlx::Row;
use tracing::{debug, warn};
use uuid::Uuid;

impl Database {
    /// Record `API` key usage
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails
    pub async fn record_api_key_usage_impl(&self, usage: &ApiKeyUsage) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO api_key_usage (
                id, api_key_id, timestamp, endpoint, status_code,
                response_time_ms, request_size_bytes, response_size_bytes,
                ip_address, user_agent, error_message
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&usage.api_key_id)
        .bind(usage.timestamp)
        .bind(&usage.tool_name)
        .bind(i32::from(usage.status_code))
        .bind(
            usage
                .response_time_ms
                .map(i32::try_from)
                .transpose()
                .map_err(|e| {
                    AppError::internal(format!(
                        "Integer conversion failed for response_time_ms: {e}"
                    ))
                })?,
        )
        .bind(
            usage
                .request_size_bytes
                .map(i32::try_from)
                .transpose()
                .map_err(|e| {
                    AppError::internal(format!(
                        "Integer conversion failed for request_size_bytes: {e}"
                    ))
                })?,
        )
        .bind(
            usage
                .response_size_bytes
                .map(i32::try_from)
                .transpose()
                .map_err(|e| {
                    AppError::internal(format!(
                        "Integer conversion failed for response_size_bytes: {e}"
                    ))
                })?,
        )
        .bind(&usage.ip_address)
        .bind(&usage.user_agent)
        .bind(&usage.error_message)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to record API key usage: {e}")))?;

        Ok(())
    }

    /// Get current usage count for an `API` key (for rate limiting)
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails or the API key is not found
    pub async fn get_api_key_current_usage_impl(&self, api_key_id: &str) -> AppResult<u32> {
        // Get the API key to determine its rate limit window (system-level lookup, no user scoping)
        let api_key = self
            .get_by_id(api_key_id, None)
            .await?
            .ok_or_else(|| AppError::not_found("API key"))?;

        let window_start =
            Utc::now() - Duration::seconds(i64::from(api_key.rate_limit_window_seconds));

        let count: i32 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) FROM api_key_usage
            WHERE api_key_id = $1 AND timestamp > $2
            ",
        )
        .bind(api_key_id)
        .bind(window_start)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get API key current usage: {e}")))?;

        u32::try_from(count).map_err(|e| {
            AppError::internal(format!("Integer conversion failed for usage count: {e}"))
        })
    }

    /// Get `API` key usage statistics
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails
    pub async fn get_api_key_usage_stats(
        &self,
        api_key_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<ApiKeyUsageStats> {
        let stats = sqlx::query(
            r"
            SELECT 
                COUNT(*) as total_requests,
                COUNT(CASE WHEN status_code >= 200 AND status_code < 300 THEN 1 END) as successful_requests,
                COUNT(CASE WHEN status_code >= 400 THEN 1 END) as failed_requests,
                SUM(response_time_ms) as total_response_time,
                MAX(response_time_ms) as max_response_time,
                SUM(request_size_bytes) as total_request_bytes,
                SUM(response_size_bytes) as total_response_bytes
            FROM api_key_usage
            WHERE api_key_id = $1 AND timestamp >= $2 AND timestamp <= $3
            ",
        )
        .bind(api_key_id)
        .bind(start_date)
        .bind(end_date)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get API key usage stats: {e}")))?;

        let total_requests: i32 = stats.get(0);
        let successful_requests: i32 = stats.get(1);
        let failed_requests: i32 = stats.get(2);
        let total_response_time: Option<i64> = stats.get(3);

        // Get tool usage aggregation
        let tool_usage_stats = sqlx::query(
            r"
            SELECT endpoint,
                   COUNT(*) as tool_count,
                   AVG(response_time_ms) as avg_response_time,
                   COUNT(CASE WHEN status_code >= 200 AND status_code < 300 THEN 1 END) as success_count
            FROM api_key_usage
            WHERE api_key_id = $1 AND timestamp >= $2 AND timestamp <= $3
            GROUP BY endpoint
            ORDER BY tool_count DESC
            ",
        )
        .bind(api_key_id)
        .bind(start_date)
        .bind(end_date)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get API key tool usage stats: {e}")))?;

        let mut tool_usage = serde_json::Map::new();
        for row in tool_usage_stats {
            let tool_name: String = row.get("endpoint");
            let tool_count: i32 = row.get("tool_count");
            let avg_response_time: Option<f64> = row.get("avg_response_time");
            let success_count: i32 = row.get("success_count");

            let avg_time = avg_response_time.unwrap_or_else(|| {
                debug!(
                    api_key_id = %api_key_id,
                    tool_name = %tool_name,
                    "No average response time available for tool usage stats"
                );
                0.0
            });

            tool_usage.insert(
                tool_name,
                serde_json::json!({
                    "count": tool_count,
                    "success_count": success_count,
                    "avg_response_time_ms": avg_time,
                    "success_rate": if tool_count > 0 { f64::from(success_count) / f64::from(tool_count) } else { 0.0 }
                }),
            );
        }

        let total_time = total_response_time.map_or(0, |t| {
            u64::try_from(t).unwrap_or_else(|e| {
                warn!(
                    api_key_id = %api_key_id,
                    total_response_time = t,
                    error = %e,
                    "Failed to convert total response time for API key usage stats, using 0"
                );
                0
            })
        });

        Ok(ApiKeyUsageStats {
            api_key_id: api_key_id.to_owned(),
            period_start: start_date,
            period_end: end_date,
            total_requests: u32::try_from(total_requests).map_err(|e| {
                AppError::internal(format!("Integer conversion failed for total_requests: {e}"))
            })?,
            successful_requests: u32::try_from(successful_requests).map_err(|e| {
                AppError::internal(format!(
                    "Integer conversion failed for successful_requests: {e}"
                ))
            })?,
            failed_requests: u32::try_from(failed_requests).map_err(|e| {
                AppError::internal(format!(
                    "Integer conversion failed for failed_requests: {e}"
                ))
            })?,
            total_response_time_ms: total_time,
            tool_usage: serde_json::Value::Object(tool_usage),
        })
    }
}
