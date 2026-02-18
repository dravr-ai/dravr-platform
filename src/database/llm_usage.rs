// ABOUTME: Database operations for LLM usage tracking and cost analysis
// ABOUTME: Records per-call token usage, aggregation queries for quota enforcement and billing insights
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::Serialize;

use crate::errors::{AppError, AppResult};
use uuid::Uuid;

use super::Database;

/// Record of a single LLM API call for usage tracking
#[derive(Debug, Clone)]
pub struct LlmUsageRecord {
    /// Unique record ID
    pub id: String,
    /// Tenant that owns this usage
    pub tenant_id: String,
    /// User who triggered the call
    pub user_id: String,
    /// Associated conversation (if from chat)
    pub conversation_id: Option<String>,
    /// LLM provider name (e.g. "google", "openai")
    pub provider: String,
    /// Model identifier (e.g. "gemini-2.0-flash-exp")
    pub model: String,
    /// Tokens in the prompt
    pub prompt_tokens: i64,
    /// Tokens in the completion
    pub completion_tokens: i64,
    /// Total tokens (prompt + completion)
    pub total_tokens: i64,
    /// Type of call (e.g. "chat", "insight", "embedding")
    pub call_type: String,
    /// Number of tool calls in this interaction
    pub tool_calls_count: i64,
    /// Execution time in milliseconds
    pub execution_time_ms: Option<i64>,
    /// When the usage was recorded (ISO 8601)
    pub created_at: String,
}

/// Parameters for inserting a new LLM usage record
pub struct InsertLlmUsage<'a> {
    /// Tenant that owns this usage
    pub tenant_id: &'a str,
    /// User who triggered the call
    pub user_id: &'a str,
    /// Associated conversation (if from chat)
    pub conversation_id: Option<&'a str>,
    /// LLM provider name
    pub provider: &'a str,
    /// Model identifier
    pub model: &'a str,
    /// Tokens in the prompt
    pub prompt_tokens: i64,
    /// Tokens in the completion
    pub completion_tokens: i64,
    /// Total tokens (prompt + completion)
    pub total_tokens: i64,
    /// Type of call
    pub call_type: &'a str,
    /// Number of tool calls
    pub tool_calls_count: i64,
    /// Execution time in milliseconds
    pub execution_time_ms: Option<i64>,
}

/// Aggregated LLM usage row grouped by `provider`/`model`/`call_type`
#[derive(Debug, Clone, Serialize)]
pub struct LlmUsageAggregateRow {
    /// LLM provider name
    pub provider: String,
    /// Model identifier
    pub model: String,
    /// Call type (chat, insight, embedding, etc.)
    pub call_type: String,
    /// Total tokens consumed across all matching records
    pub total_tokens: i64,
    /// Total prompt tokens
    pub prompt_tokens: i64,
    /// Total completion tokens
    pub completion_tokens: i64,
    /// Number of LLM calls
    pub calls: i64,
}

/// Daily aggregated LLM usage for time series display
#[derive(Debug, Clone, Serialize)]
pub struct LlmUsageDailyRow {
    /// Date string (YYYY-MM-DD)
    pub date: String,
    /// Total tokens for this day
    pub tokens: i64,
    /// Total prompt tokens for this day
    pub prompt_tokens: i64,
    /// Total completion tokens for this day
    pub completion_tokens: i64,
    /// Number of calls for this day
    pub calls: i64,
}

/// Allowed grouping dimensions for LLM consumption aggregation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmUsageGroupBy {
    /// Group by provider name
    Provider,
    /// Group by model identifier
    Model,
    /// Group by call type
    CallType,
}

impl LlmUsageGroupBy {
    /// Parse from query string parameter
    #[must_use]
    pub fn from_str_param(s: &str) -> Option<Self> {
        match s {
            "provider" => Some(Self::Provider),
            "model" => Some(Self::Model),
            "call_type" => Some(Self::CallType),
            _ => None,
        }
    }
}

impl Database {
    /// Insert a new LLM usage record (inherent implementation)
    pub(crate) async fn insert_llm_usage_impl(
        &self,
        params: &InsertLlmUsage<'_>,
    ) -> AppResult<LlmUsageRecord> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

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
        .bind(&now)
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
            created_at: now,
        })
    }

    /// Query aggregated LLM usage grouped by `provider`+`model`+`call_type` (inherent implementation)
    pub(crate) async fn get_llm_usage_aggregates_impl(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageAggregateRow>> {
        let rows = sqlx::query_as::<_, (String, String, String, i64, i64, i64, i64)>(
            r"
            SELECT provider, model, call_type,
                   SUM(total_tokens) as total_tokens,
                   SUM(prompt_tokens) as prompt_tokens,
                   SUM(completion_tokens) as completion_tokens,
                   COUNT(*) as calls
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

    /// Query daily LLM usage time series (inherent implementation)
    pub(crate) async fn get_llm_usage_daily_series_impl(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageDailyRow>> {
        let rows = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(
            r"
            SELECT DATE(created_at) as date,
                   SUM(total_tokens) as tokens,
                   SUM(prompt_tokens) as prompt_tokens,
                   SUM(completion_tokens) as completion_tokens,
                   COUNT(*) as calls
            FROM llm_usage
            WHERE tenant_id = $1 AND created_at >= $2
            GROUP BY DATE(created_at)
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
