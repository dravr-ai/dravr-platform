// ABOUTME: Database operations for LLM usage tracking and cost analysis
// ABOUTME: Records per-call token usage, aggregation queries for quota enforcement and billing insights
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::repositories::LlmUsageRepository;
use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ConversationTurnId, LlmUsageRecord, TenantId, TURN_SUMMARY_CALL_TYPE};
use sqlx::query::QueryAs;
use sqlx::sqlite::SqliteArguments;
use sqlx::{Sqlite, SqlitePool};
use uuid::Uuid;

pub use pierre_core::models::usage::{
    EmbeddingUsageRecord, InsertEmbeddingUsage, InsertLlmUsage, LlmUsageAggregateRow,
    LlmUsageDailyRow,
};

use super::Database;

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
            INSERT INTO llm_usage (id, tenant_id, user_id, conversation_id, turn_id, provider, model, prompt_tokens, completion_tokens, total_tokens, cached_tokens, call_type, tool_calls_count, tools_called, execution_time_ms, cost_usd, call_sequence, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
            ",
        )
        .bind(&id)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(params.conversation_id)
        .bind(params.turn_id.as_uuid().to_string())
        .bind(params.provider)
        .bind(params.model)
        .bind(params.prompt_tokens)
        .bind(params.completion_tokens)
        .bind(params.total_tokens)
        .bind(params.cached_tokens)
        .bind(params.call_type)
        .bind(params.tool_calls_count)
        .bind(params.tools_called)
        .bind(params.execution_time_ms)
        .bind(params.cost_usd)
        .bind(params.call_sequence)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert LLM usage: {e}")))?;

        Ok(LlmUsageRecord {
            id,
            tenant_id: params.tenant_id.to_owned(),
            user_id: params.user_id.to_owned(),
            conversation_id: params.conversation_id.map(ToOwned::to_owned),
            turn_id: params.turn_id,
            provider: params.provider.to_owned(),
            model: params.model.to_owned(),
            prompt_tokens: params.prompt_tokens,
            completion_tokens: params.completion_tokens,
            total_tokens: params.total_tokens,
            cached_tokens: params.cached_tokens,
            call_type: params.call_type.to_owned(),
            tool_calls_count: params.tool_calls_count,
            tools_called: params.tools_called.to_owned(),
            execution_time_ms: params.execution_time_ms,
            cost_usd: params.cost_usd,
            call_sequence: params.call_sequence,
            created_at: now,
        })
    }

    /// Insert a new embedding usage record (`SQLite` backend).
    pub(crate) async fn insert_embedding_usage_impl(
        &self,
        params: &InsertEmbeddingUsage<'_>,
    ) -> AppResult<EmbeddingUsageRecord> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            r"
            INSERT INTO embedding_usage (id, tenant_id, user_id, provider, model, input_tokens, cost_usd, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ",
        )
        .bind(&id)
        .bind(params.tenant_id)
        .bind(params.user_id)
        .bind(params.provider)
        .bind(params.model)
        .bind(params.input_tokens)
        .bind(params.cost_usd)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to insert embedding usage: {e}")))?;

        Ok(EmbeddingUsageRecord {
            id,
            tenant_id: params.tenant_id.to_owned(),
            user_id: params.user_id.to_owned(),
            provider: params.provider.to_owned(),
            model: params.model.to_owned(),
            input_tokens: params.input_tokens,
            cost_usd: params.cost_usd,
            created_at: now,
        })
    }

    /// Query aggregated LLM usage grouped by `provider`+`model`+`call_type` (inherent implementation).
    ///
    /// Excludes the turn-summary rows written at turn completion —
    /// those have zero tokens and would only inflate the `calls`
    /// count, so aggregates reflect real LLM activity only.
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
            WHERE tenant_id = $1 AND created_at >= $2 AND call_type != $3
            GROUP BY provider, model, call_type
            ORDER BY total_tokens DESC
            ",
        )
        .bind(tenant_id)
        .bind(since)
        .bind(TURN_SUMMARY_CALL_TYPE)
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

    /// Query daily LLM usage time series (inherent implementation).
    ///
    /// Turn-summary rows are excluded for the same reason they are
    /// excluded from [`Self::get_llm_usage_aggregates_impl`] — they
    /// carry zero tokens and would inflate `calls` counts without
    /// representing real LLM activity.
    pub(crate) async fn get_llm_usage_daily_series_impl(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageDailyRow>> {
        let rows = sqlx::query_as::<_, (String, i64, i64, i64, i64, Option<f64>)>(
            r"
            SELECT DATE(created_at) as date,
                   SUM(total_tokens) as tokens,
                   SUM(prompt_tokens) as prompt_tokens,
                   SUM(completion_tokens) as completion_tokens,
                   COUNT(*) as calls,
                   AVG(CAST(execution_time_ms AS REAL)) as avg_exec_ms
            FROM llm_usage
            WHERE tenant_id = $1 AND created_at >= $2 AND call_type != $3
            GROUP BY DATE(created_at)
            ORDER BY date ASC
            ",
        )
        .bind(tenant_id)
        .bind(since)
        .bind(TURN_SUMMARY_CALL_TYPE)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query LLM usage daily series: {e}")))?;

        Ok(rows
            .into_iter()
            .map(
                |(date, tokens, prompt_tokens, completion_tokens, calls, avg_exec_ms)| {
                    LlmUsageDailyRow {
                        date,
                        tokens,
                        prompt_tokens,
                        completion_tokens,
                        calls,
                        avg_execution_time_ms: avg_exec_ms.unwrap_or(0.0),
                    }
                },
            )
            .collect())
    }
}

#[async_trait]
impl LlmUsageRepository for Database {
    async fn insert_llm_usage(
        &self,
        params: &super::llm_usage::InsertLlmUsage<'_>,
    ) -> AppResult<LlmUsageRecord> {
        self.insert_llm_usage_impl(params).await
    }

    async fn insert_embedding_usage(
        &self,
        params: &InsertEmbeddingUsage<'_>,
    ) -> AppResult<EmbeddingUsageRecord> {
        self.insert_embedding_usage_impl(params).await
    }

    async fn get_llm_usage_aggregates(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<super::llm_usage::LlmUsageAggregateRow>> {
        self.get_llm_usage_aggregates_impl(tenant_id, since).await
    }

    async fn get_llm_usage_daily_series(
        &self,
        tenant_id: &str,
        since: &str,
    ) -> AppResult<Vec<super::llm_usage::LlmUsageDailyRow>> {
        self.get_llm_usage_daily_series_impl(tenant_id, since).await
    }

    async fn get_llm_usage_aggregates_by_user(
        &self,
        user_id: &str,
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
            WHERE user_id = $1 AND created_at >= $2 AND call_type != $3
            GROUP BY provider, model, call_type
            ORDER BY total_tokens DESC
            ",
        )
        .bind(user_id)
        .bind(since)
        .bind(TURN_SUMMARY_CALL_TYPE)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to query LLM usage aggregates by user: {e}"))
        })?;

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
                )| LlmUsageAggregateRow {
                    provider,
                    model,
                    call_type,
                    total_tokens,
                    prompt_tokens,
                    completion_tokens,
                    calls,
                },
            )
            .collect())
    }

    async fn get_llm_usage_daily_series_by_user(
        &self,
        user_id: &str,
        since: &str,
    ) -> AppResult<Vec<LlmUsageDailyRow>> {
        let rows = sqlx::query_as::<_, (String, i64, i64, i64, i64, Option<f64>)>(
            r"
            SELECT DATE(created_at) as date,
                   SUM(total_tokens) as tokens,
                   SUM(prompt_tokens) as prompt_tokens,
                   SUM(completion_tokens) as completion_tokens,
                   COUNT(*) as calls,
                   AVG(CAST(execution_time_ms AS REAL)) as avg_exec_ms
            FROM llm_usage
            WHERE user_id = $1 AND created_at >= $2 AND call_type != $3
            GROUP BY DATE(created_at)
            ORDER BY date ASC
            ",
        )
        .bind(user_id)
        .bind(since)
        .bind(TURN_SUMMARY_CALL_TYPE)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to query LLM usage daily series by user: {e}"
            ))
        })?;

        Ok(rows
            .into_iter()
            .map(
                |(date, tokens, prompt_tokens, completion_tokens, calls, avg_exec_ms)| {
                    LlmUsageDailyRow {
                        date,
                        tokens,
                        prompt_tokens,
                        completion_tokens,
                        calls,
                        avg_execution_time_ms: avg_exec_ms.unwrap_or(0.0),
                    }
                },
            )
            .collect())
    }

    async fn get_recent_llm_calls_admin(&self, limit: i64) -> AppResult<Vec<LlmUsageRecord>> {
        fetch_llm_usage_rows(
            &self.pool,
            r"
            SELECT id, tenant_id, user_id, conversation_id, turn_id, provider, model,
                   prompt_tokens, completion_tokens, total_tokens, cached_tokens, call_type,
                   tool_calls_count, tools_called, execution_time_ms, cost_usd, call_sequence, created_at
            FROM llm_usage
            ORDER BY created_at DESC
            LIMIT $1
            ",
            |q| q.bind(limit),
        )
        .await
    }

    async fn count_llm_calls_since(&self, since: &str) -> AppResult<i64> {
        let row = sqlx::query_as::<_, (i64,)>(
            r"
            SELECT COUNT(*) FROM llm_usage WHERE created_at >= $1
            ",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count LLM calls: {e}")))?;

        Ok(row.0)
    }

    async fn sum_llm_usage_since(&self, since: &str) -> AppResult<(i64, i64)> {
        let row = sqlx::query_as::<_, (i64, i64)>(
            r"
            SELECT COALESCE(COUNT(*), 0), COALESCE(SUM(total_tokens), 0)
            FROM llm_usage WHERE created_at >= $1
            ",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to sum LLM usage: {e}")))?;

        Ok(row)
    }

    async fn find_llm_usage_by_turn_id(
        &self,
        turn_id: ConversationTurnId,
    ) -> AppResult<Vec<LlmUsageRecord>> {
        let turn_id_str = turn_id.as_uuid().to_string();
        fetch_llm_usage_rows(
            &self.pool,
            r"
            SELECT id, tenant_id, user_id, conversation_id, turn_id, provider, model,
                   prompt_tokens, completion_tokens, total_tokens, cached_tokens, call_type,
                   tool_calls_count, tools_called, execution_time_ms, cost_usd, call_sequence, created_at
            FROM llm_usage
            WHERE turn_id = $1
            ORDER BY call_sequence ASC NULLS LAST, created_at ASC
            ",
            move |q| q.bind(turn_id_str),
        )
        .await
    }

    async fn sum_cost_usd_for_tenant_period(
        &self,
        tenant_id: TenantId,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<f64> {
        let total: Option<f64> = sqlx::query_scalar(
            r"
            SELECT COALESCE(SUM(cost_usd), 0.0)
            FROM llm_usage
            WHERE tenant_id = ?1
              AND created_at >= ?2
              AND created_at < ?3
            ",
        )
        .bind(tenant_id.to_string())
        .bind(start.to_rfc3339())
        .bind(end.to_rfc3339())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to sum cost_usd: {e}")))?;
        Ok(total.unwrap_or(0.0))
    }
}

/// Shared row-fetch helper used by `get_recent_llm_calls_admin` and
/// `find_llm_usage_by_turn_id` — both select the same column list and
/// rebuild `LlmUsageRecord` the same way.
async fn fetch_llm_usage_rows<F>(
    pool: &SqlitePool,
    sql: &str,
    bind: F,
) -> AppResult<Vec<LlmUsageRecord>>
where
    F: for<'a> FnOnce(
        QueryAs<'a, Sqlite, LlmUsageRow, SqliteArguments<'a>>,
    ) -> QueryAs<'a, Sqlite, LlmUsageRow, SqliteArguments<'a>>,
{
    let query = sqlx::query_as::<_, LlmUsageRow>(sql);
    let rows = bind(query)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query LLM usage rows: {e}")))?;
    rows.into_iter().map(LlmUsageRow::into_record).collect()
}

/// Raw row projection for `llm_usage`. Parsed into [`LlmUsageRecord`] by
/// [`LlmUsageRow::into_record`]; `turn_id` is stored as TEXT in
/// `SQLite` and parsed into a [`ConversationTurnId`] here.
#[derive(sqlx::FromRow)]
struct LlmUsageRow {
    id: String,
    tenant_id: String,
    user_id: String,
    conversation_id: Option<String>,
    turn_id: String,
    provider: String,
    model: String,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    cached_tokens: i64,
    call_type: String,
    tool_calls_count: i64,
    tools_called: String,
    execution_time_ms: Option<i64>,
    cost_usd: f64,
    call_sequence: Option<i64>,
    created_at: String,
}

impl LlmUsageRow {
    fn into_record(self) -> AppResult<LlmUsageRecord> {
        let turn_uuid = Uuid::parse_str(&self.turn_id)
            .map_err(|e| AppError::database(format!("Invalid turn_id in llm_usage: {e}")))?;
        Ok(LlmUsageRecord {
            id: self.id,
            tenant_id: self.tenant_id,
            user_id: self.user_id,
            conversation_id: self.conversation_id,
            turn_id: ConversationTurnId::from_uuid(turn_uuid),
            provider: self.provider,
            model: self.model,
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            cached_tokens: self.cached_tokens,
            call_type: self.call_type,
            tool_calls_count: self.tool_calls_count,
            tools_called: self.tools_called,
            execution_time_ms: self.execution_time_ms,
            cost_usd: self.cost_usd,
            call_sequence: self.call_sequence,
            created_at: self.created_at,
        })
    }
}
