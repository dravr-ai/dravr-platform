// ABOUTME: PostgreSQL usage tracking repository implementations
// ABOUTME: Manages request usage, counters, and LLM usage metrics for billing and analytics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::{LlmUsageRepository, UsageCounterRepository, UsageRepository};
use super::PostgresDatabase;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::constants::http_status::{BAD_REQUEST, SUCCESS_MAX, SUCCESS_MIN};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::usage::{InsertLlmUsage, LlmUsageAggregateRow, LlmUsageDailyRow};
use pierre_core::models::TenantId;
use pierre_core::models::{ApiKeyUsage, ApiKeyUsageStats};
use pierre_core::models::{JwtUsage, LlmUsageRecord, RequestLog, ToolUsage, UsageCounterRecord};
use sqlx::Postgres;
use sqlx::Row;
use tracing::warn;
use uuid::Uuid;

#[async_trait]
impl UsageRepository for PostgresDatabase {
    async fn record_api_key(&self, usage: &ApiKeyUsage) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO api_key_usage (api_key_id, timestamp, endpoint, response_time_ms, status_code, 
                                     method, request_size_bytes, response_size_bytes, ip_address, user_agent)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::inet, $10)
            ",
        )
        .bind(&usage.api_key_id)
        .bind(usage.timestamp)
        .bind(&usage.tool_name)
        .bind(usage.response_time_ms.map(|x| i32::try_from(x).unwrap_or(i32::MAX)))
        .bind(i16::try_from(usage.status_code).unwrap_or(i16::MAX))
        .bind(None::<String>)
        .bind(usage.request_size_bytes.map(|x| i32::try_from(x).unwrap_or(i32::MAX)))
        .bind(usage.response_size_bytes.map(|x| i32::try_from(x).unwrap_or(i32::MAX)))
        .bind(&usage.ip_address)
        .bind(&usage.user_agent)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to record API key usage: {e}")))?;

        Ok(())
    }

    async fn get_api_key_current(&self, api_key_id: &str) -> AppResult<u32> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as count
            FROM api_key_usage
            WHERE api_key_id = $1 AND timestamp >= CURRENT_DATE
            ",
        )
        .bind(api_key_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get API key current usage: {e}")))?;

        Ok(u32::try_from(row.get::<i64, _>("count").max(0)).unwrap_or(0))
    }

    async fn get_api_key_stats(
        &self,
        api_key_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<ApiKeyUsageStats> {
        let row = sqlx::query_as::<Postgres, (i64, i64, i64, Option<i64>, Option<i64>, Option<i64>)>(
            r"
            SELECT 
                COUNT(*) as total_requests,
                COUNT(CASE WHEN status_code >= $1 AND status_code <= $2 THEN 1 END) as successful_requests,
                COUNT(CASE WHEN status_code >= $3 THEN 1 END) as failed_requests,
                SUM(response_time_ms) as total_response_time,
                SUM(request_size_bytes) as total_request_size,
                SUM(response_size_bytes) as total_response_size
            FROM api_key_usage 
            WHERE api_key_id = $4 AND timestamp >= $5 AND timestamp <= $6
            "
        )
        .bind(i32::from(SUCCESS_MIN))
        .bind(i32::from(SUCCESS_MAX))
        .bind(i32::from(BAD_REQUEST))
        .bind(api_key_id)
        .bind(start_date)
        .bind(end_date)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get API key usage stats: {e}")))?;

        // Get tool usage aggregation
        let tool_usage_stats = sqlx::query_as::<Postgres, (String, i64, Option<f64>, i64)>(
            r"
            SELECT tool_name,
                   COUNT(*) as tool_count,
                   AVG(response_time_ms) as avg_response_time,
                   COUNT(CASE WHEN status_code >= $1 AND status_code <= $2 THEN 1 END) as success_count
            FROM api_key_usage
            WHERE api_key_id = $3 AND timestamp >= $4 AND timestamp <= $5
            GROUP BY tool_name
            ORDER BY tool_count DESC
            "
        )
        .bind(i32::from(SUCCESS_MIN))
        .bind(i32::from(SUCCESS_MAX))
        .bind(api_key_id)
        .bind(start_date)
        .bind(end_date)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get tool usage stats: {e}")))?;

        let mut tool_usage = serde_json::Map::new();
        for (tool_name, tool_count, avg_response_time, success_count) in tool_usage_stats {
            tool_usage.insert(
                tool_name,
                serde_json::json!({
                    "count": tool_count,
                    "success_count": success_count,
                    "avg_response_time_ms": avg_response_time.unwrap_or(0.0),
                    "success_rate": if tool_count > 0 { 
                        f64::from(u32::try_from(success_count).unwrap_or(0)) / f64::from(u32::try_from(tool_count).unwrap_or(1))
                    } else { 0.0 }
                }),
            );
        }

        Ok(ApiKeyUsageStats {
            api_key_id: api_key_id.to_owned(),
            period_start: start_date,
            period_end: end_date,
            total_requests: u32::try_from(row.0.max(0)).unwrap_or(0),
            successful_requests: u32::try_from(row.1.max(0)).unwrap_or(0),
            failed_requests: u32::try_from(row.2.max(0)).unwrap_or(0),
            total_response_time_ms: row.3.map_or(0u64, |v| u64::try_from(v.max(0)).unwrap_or(0)),
            tool_usage: serde_json::Value::Object(tool_usage),
        })
    }

    async fn record_jwt_usage(&self, usage: &JwtUsage) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO jwt_usage (
                user_id, timestamp, endpoint, response_time_ms, status_code,
                method, request_size_bytes, response_size_bytes, 
                ip_address, user_agent
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::inet, $10)
            ",
        )
        .bind(usage.user_id)
        .bind(usage.timestamp)
        .bind(&usage.endpoint)
        .bind(
            usage
                .response_time_ms
                .map(|t| i32::try_from(t).unwrap_or(i32::MAX)),
        )
        .bind(i32::from(usage.status_code))
        .bind(&usage.method)
        .bind(
            usage
                .request_size_bytes
                .map(|s| i32::try_from(s).unwrap_or(i32::MAX)),
        )
        .bind(
            usage
                .response_size_bytes
                .map(|s| i32::try_from(s).unwrap_or(i32::MAX)),
        )
        .bind(&usage.ip_address)
        .bind(&usage.user_agent)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to record JWT usage: {e}")))?;

        Ok(())
    }

    async fn get_jwt_current_usage(&self, user_id: Uuid) -> AppResult<u32> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as count
            FROM jwt_usage
            WHERE user_id = $1 AND timestamp >= DATE_TRUNC('month', CURRENT_DATE)
            ",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get JWT current usage: {e}")))?;

        Ok(u32::try_from(row.get::<i64, _>("count").max(0)).unwrap_or(0))
    }

    async fn get_request_logs(
        &self,
        user_id: Option<Uuid>,
        api_key_id: Option<&str>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        status_filter: Option<&str>,
        tool_filter: Option<&str>,
    ) -> AppResult<Vec<RequestLog>> {
        // Build query with proper column mapping for RequestLog struct.
        // When user_id is provided, join with api_keys to scope by ownership.
        let base_query = if user_id.is_some() {
            r"SELECT
                uuid_generate_v4()::text as id,
                u.timestamp,
                u.api_key_id,
                'Unknown' as api_key_name,
                COALESCE(u.endpoint, 'unknown') as tool_name,
                u.status_code::integer as status_code,
                u.response_time_ms,
                NULL::text as error_message,
                u.request_size_bytes,
                u.response_size_bytes
              FROM api_key_usage u
              JOIN api_keys k ON u.api_key_id = k.id
              WHERE 1=1"
        } else {
            r"SELECT
                uuid_generate_v4()::text as id,
                timestamp,
                api_key_id,
                'Unknown' as api_key_name,
                COALESCE(endpoint, 'unknown') as tool_name,
                status_code::integer as status_code,
                response_time_ms,
                NULL::text as error_message,
                request_size_bytes,
                response_size_bytes
              FROM api_key_usage
              WHERE 1=1"
        };

        let mut condition_strings = Vec::new();
        let col_prefix = if user_id.is_some() { "u." } else { "" };

        let mut param_count = 0;
        if user_id.is_some() {
            param_count += 1;
            condition_strings.push(format!(" AND k.user_id = ${param_count}"));
        }
        if api_key_id.is_some() {
            param_count += 1;
            condition_strings.push(format!(" AND {col_prefix}api_key_id = ${param_count}"));
        }
        if start_time.is_some() {
            param_count += 1;
            condition_strings.push(format!(" AND {col_prefix}timestamp >= ${param_count}"));
        }
        if end_time.is_some() {
            param_count += 1;
            condition_strings.push(format!(" AND {col_prefix}timestamp <= ${param_count}"));
        }
        if status_filter.is_some() {
            param_count += 1;
            condition_strings.push(format!(
                " AND {col_prefix}status_code::text LIKE ${param_count}"
            ));
        }
        if tool_filter.is_some() {
            param_count += 1;
            condition_strings.push(format!(" AND {col_prefix}endpoint ILIKE ${param_count}"));
        }

        let full_query = format!(
            "{}{} ORDER BY {col_prefix}timestamp DESC LIMIT 1000",
            base_query,
            condition_strings.join("")
        );

        // Build query with proper parameter binding
        let mut query_builder = sqlx::query_as::<_, RequestLog>(&full_query);

        if let Some(uid) = user_id {
            query_builder = query_builder.bind(uid);
        }
        if let Some(key_id) = api_key_id {
            query_builder = query_builder.bind(key_id);
        }
        if let Some(start) = start_time {
            query_builder = query_builder.bind(start);
        }
        if let Some(end) = end_time {
            query_builder = query_builder.bind(end);
        }
        if let Some(status) = status_filter {
            query_builder = query_builder.bind(format!("{status}%"));
        }
        if let Some(tool) = tool_filter {
            query_builder = query_builder.bind(format!("%{tool}%"));
        }

        let results = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get request logs: {e}")))?;
        Ok(results)
    }

    async fn get_system_stats(&self, tenant_id: Option<TenantId>) -> AppResult<(u64, u64)> {
        let user_count_row = if let Some(tid) = tenant_id {
            sqlx::query(
                "SELECT COUNT(*) as count FROM users u INNER JOIN tenant_users tu ON u.id = tu.user_id WHERE tu.tenant_id = $1",
            )
                .bind(tid.to_string())
                .fetch_one(&self.pool)
                .await
        } else {
            sqlx::query("SELECT COUNT(*) as count FROM users")
                .fetch_one(&self.pool)
                .await
        }
        .map_err(|e| {
            AppError::database(format!("Failed to get user count for system stats: {e}"))
        })?;

        let api_key_count_row = if let Some(tid) = tenant_id {
            sqlx::query(
                "SELECT COUNT(*) as count FROM api_keys ak INNER JOIN tenant_users tu ON ak.user_id = tu.user_id WHERE ak.is_active = true AND tu.tenant_id = $1",
            )
            .bind(tid.to_string())
            .fetch_one(&self.pool)
            .await
        } else {
            sqlx::query("SELECT COUNT(*) as count FROM api_keys WHERE is_active = true")
                .fetch_one(&self.pool)
                .await
        }
        .map_err(|e| {
            AppError::database(format!("Failed to get API key count for system stats: {e}"))
        })?;

        let user_count = u64::try_from(user_count_row.get::<i64, _>("count").max(0)).unwrap_or(0);
        let api_key_count =
            u64::try_from(api_key_count_row.get::<i64, _>("count").max(0)).unwrap_or(0);

        Ok((user_count, api_key_count))
    }

    async fn get_top_tools_analysis(
        &self,
        user_id: Uuid,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> AppResult<Vec<ToolUsage>> {
        let rows = sqlx::query(
            r"
            SELECT endpoint, COUNT(*) as usage_count,
                   AVG(response_time_ms) as avg_response_time,
                   COUNT(CASE WHEN status_code < 400 THEN 1 END) as success_count,
                   COUNT(CASE WHEN status_code >= 400 THEN 1 END) as error_count
            FROM api_key_usage aku
            JOIN api_keys ak ON aku.api_key_id = ak.id
            WHERE ak.user_id = $1 AND aku.timestamp BETWEEN $2 AND $3
            GROUP BY endpoint
            ORDER BY usage_count DESC
            LIMIT 10
            ",
        )
        .bind(user_id)
        .bind(start_time)
        .bind(end_time)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get top tools analysis: {e}")))?;

        let mut tool_usage = Vec::new();
        for row in rows {
            use sqlx::Row;

            let endpoint: String = row.try_get("endpoint").unwrap_or_else(|_| "unknown".into());
            let usage_count: i64 = row.try_get("usage_count").unwrap_or(0);
            let avg_response_time: Option<f64> = row.try_get("avg_response_time").ok();
            let success_count: i64 = row.try_get("success_count").unwrap_or(0);
            let error_count: i64 = row.try_get("error_count").unwrap_or(0);

            // Log error rate for monitoring
            if error_count > 0 {
                let error_rate = f64::from(u32::try_from(error_count.max(0)).unwrap_or(0))
                    / f64::from(u32::try_from(usage_count.max(1)).unwrap_or(1));
                if error_rate > 0.1 {
                    warn!(
                        "High error rate for endpoint {}: {:.2}% ({} errors out of {} requests)",
                        endpoint,
                        error_rate * 100.0,
                        error_count,
                        usage_count
                    );
                }
            }

            tool_usage.push(ToolUsage {
                tool_name: endpoint,
                request_count: u64::try_from(usage_count.max(0)).unwrap_or(0),
                success_rate: if usage_count > 0 {
                    f64::from(u32::try_from(success_count.max(0)).unwrap_or(0))
                        / f64::from(u32::try_from(usage_count.max(1)).unwrap_or(1))
                } else {
                    0.0
                },
                average_response_time: avg_response_time.unwrap_or(0.0),
            });
        }

        Ok(tool_usage)
    }
}

#[async_trait]
impl UsageCounterRepository for PostgresDatabase {
    async fn increment_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
        amount: i64,
    ) -> AppResult<UsageCounterRecord> {
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO usage_counters (tenant_id, user_id, counter_key, period, value, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (tenant_id, user_id, counter_key, period)
            DO UPDATE SET value = usage_counters.value + EXCLUDED.value, updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(counter_key)
        .bind(period)
        .bind(amount)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to increment usage counter: {e}")))?;

        self.get_counter(tenant_id, user_id, counter_key, period)
            .await
    }

    async fn get_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
    ) -> AppResult<UsageCounterRecord> {
        let row: Option<(String, String, String, String, i64, String)> = sqlx::query_as(
            r"
            SELECT tenant_id, user_id, counter_key, period, value, updated_at
            FROM usage_counters
            WHERE tenant_id = $1 AND user_id = $2 AND counter_key = $3 AND period = $4
            ",
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(counter_key)
        .bind(period)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get usage counter: {e}")))?;

        match row {
            Some((tid, uid, key, per, val, updated)) => Ok(UsageCounterRecord {
                tenant_id: tid,
                user_id: uid,
                counter_key: key,
                period: per,
                value: val,
                updated_at: updated,
            }),
            None => Ok(UsageCounterRecord {
                tenant_id: tenant_id.to_owned(),
                user_id: user_id.to_owned(),
                counter_key: counter_key.to_owned(),
                period: period.to_owned(),
                value: 0,
                updated_at: String::new(),
            }),
        }
    }

    /// System-level housekeeping: intentionally cross-tenant pruning of expired counters
    async fn delete_old_counters(&self, period_before: &str) -> AppResult<u64> {
        let result = sqlx::query(
            r"
            DELETE FROM usage_counters
            WHERE period < $1
            ",
        )
        .bind(period_before)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete old usage counters: {e}")))?;

        Ok(result.rows_affected())
    }
}

#[async_trait]
impl LlmUsageRepository for PostgresDatabase {
    async fn insert_llm_usage(&self, params: &InsertLlmUsage<'_>) -> AppResult<LlmUsageRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO llm_usage (id, tenant_id, user_id, conversation_id, provider, model, prompt_tokens, completion_tokens, total_tokens, call_type, tool_calls_count, execution_time_ms, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ",
        )
        .bind(&id)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(params.conversation_id)
        .bind(params.provider)
        .bind(params.model)
        .bind(params.prompt_tokens)
        .bind(params.completion_tokens)
        .bind(params.total_tokens)
        .bind(params.call_type)
        .bind(params.tool_calls_count)
        .bind(params.execution_time_ms)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert LLM usage: {e}")))?;

        Ok(LlmUsageRecord {
            id,
            tenant_id: params.tenant_id.to_owned(),
            user_id: params.user_id.to_owned(),
            conversation_id: params.conversation_id.map(ToOwned::to_owned),
            provider: params.provider.to_owned(),
            model: params.model.to_owned(),
            prompt_tokens: params.prompt_tokens,
            completion_tokens: params.completion_tokens,
            total_tokens: params.total_tokens,
            call_type: params.call_type.to_owned(),
            tool_calls_count: params.tool_calls_count,
            execution_time_ms: params.execution_time_ms,
            created_at: now.to_rfc3339(),
        })
    }

    async fn get_llm_usage_aggregates(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageAggregateRow>> {
        let rows = sqlx::query_as::<_, (String, String, String, i64, i64, i64, i64)>(
            r"
            SELECT provider, model, call_type,
                   COALESCE(SUM(total_tokens), 0)::BIGINT as total_tokens,
                   COALESCE(SUM(prompt_tokens), 0)::BIGINT as prompt_tokens,
                   COALESCE(SUM(completion_tokens), 0)::BIGINT as completion_tokens,
                   COUNT(*)::BIGINT as calls
            FROM llm_usage
            WHERE tenant_id = $1 AND created_at >= $2
            GROUP BY provider, model, call_type
            ORDER BY total_tokens DESC
            ",
        )
        .bind(tenant_id)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query LLM usage aggregates: {e}")))?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    provider,
                    model,
                    call_type,
                    total_tokens,
                    prompt_tokens,
                    completion_tokens,
                    calls,
                )| {
                    LlmUsageAggregateRow {
                        provider,
                        model,
                        call_type,
                        total_tokens,
                        prompt_tokens,
                        completion_tokens,
                        calls,
                    }
                },
            )
            .collect())
    }

    async fn get_llm_usage_daily_series(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageDailyRow>> {
        let rows = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(
            r"
            SELECT TO_CHAR(created_at::DATE, 'YYYY-MM-DD') as date,
                   COALESCE(SUM(total_tokens), 0)::BIGINT as tokens,
                   COALESCE(SUM(prompt_tokens), 0)::BIGINT as prompt_tokens,
                   COALESCE(SUM(completion_tokens), 0)::BIGINT as completion_tokens,
                   COUNT(*)::BIGINT as calls
            FROM llm_usage
            WHERE tenant_id = $1 AND created_at >= $2
            GROUP BY created_at::DATE
            ORDER BY date ASC
            ",
        )
        .bind(tenant_id)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query LLM usage daily series: {e}")))?;

        Ok(rows
            .into_iter()
            .map(
                |(date, tokens, prompt_tokens, completion_tokens, calls)| LlmUsageDailyRow {
                    date,
                    tokens,
                    prompt_tokens,
                    completion_tokens,
                    calls,
                },
            )
            .collect())
    }
}
