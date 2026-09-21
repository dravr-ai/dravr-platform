// ABOUTME: PostgreSQL-backed LlmUsageRepository, emitted from the shared implementation in repositories/llm_usage.rs
// ABOUTME: turn_id is a native uuid column here and a row's UTC day is its TIMESTAMPTZ rendered at UTC
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::usage::{InsertLlmUsage, LlmUsageAggregateRow, LlmUsageDailyRow};
use pierre_core::models::{ConversationTurnId, LlmUsageRecord, TenantId, TURN_SUMMARY_CALL_TYPE};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::llm_usage::{
    aggregate_from_row, call_sequence_column, daily_from_row, impl_llm_usage_repository,
    llm_usage_column_error, llm_usage_daily_series_sql, llm_usage_token_sums, recorded_at,
    since_bound, COUNT_LLM_CALLS_SINCE_SQL, INSERT_LLM_USAGE_SQL, LLM_USAGE_AGGREGATES_BY_USER_SQL,
    LLM_USAGE_AGGREGATES_SQL, LLM_USAGE_BY_TURN_SQL, RECENT_LLM_CALLS_SQL,
    SUM_COST_USD_FOR_TENANT_PERIOD_SQL, SUM_LLM_USAGE_SINCE_SQL, TENANT_TOOL_CALLS_SINCE_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::LlmUsageRepository;

impl_llm_usage_repository!(
    PostgresDatabase,
    PgRow,
    NativeUuid,
    day = "TO_CHAR(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD')"
);
