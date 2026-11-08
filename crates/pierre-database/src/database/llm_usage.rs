// ABOUTME: SQLite-backed LlmUsageRepository, emitted from the shared implementation in repositories/llm_usage.rs
// ABOUTME: turn_id is a TEXT column here and a row's UTC day is DATE() of the RFC 3339 text the row holds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::usage::{InsertLlmUsage, LlmUsageAggregateRow, LlmUsageDailyRow};
use pierre_core::models::{
    ConversationTurnId, LlmUsageRecord, TenantId, DIRECT_TOOL_CALL_TYPE, TURN_SUMMARY_CALL_TYPE,
};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::llm_usage::{
    aggregate_from_row, call_sequence_column, daily_from_row, impl_llm_usage_repository,
    llm_usage_column_error, llm_usage_daily_series_sql, llm_usage_token_sums, prompt_band_from_row,
    recorded_at, since_bound, INSERT_LLM_USAGE_SQL, LLM_CALLS_BETWEEN_SQL,
    LLM_USAGE_AGGREGATES_BY_USER_IN_TENANT_SQL, LLM_USAGE_AGGREGATES_BY_USER_SQL,
    LLM_USAGE_AGGREGATES_SQL, LLM_USAGE_BY_TURN_SQL, LLM_USAGE_PROMPT_BANDS_SQL,
    RECENT_LLM_CALLS_SQL, TOOL_CALLS_SINCE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;
use crate::repositories::{LlmUsagePromptBand, LlmUsageRepository, LlmUsageScope};

impl_llm_usage_repository!(Database, SqliteRow, TextUuid, day = "DATE(created_at)");
