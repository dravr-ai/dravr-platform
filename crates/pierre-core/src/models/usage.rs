// ABOUTME: Usage and dashboard data types for tracking API requests, tools, LLM calls, and quotas
// ABOUTME: Pure DTOs shared between the main crate's database layer and dashboard routes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single usage counter record
#[derive(Debug, Clone)]
pub struct UsageCounterRecord {
    /// Tenant that owns this counter
    pub tenant_id: String,
    /// User this counter applies to
    pub user_id: String,
    /// Counter key (e.g. `messages`, `tool_calls`, `tokens`)
    pub counter_key: String,
    /// Time period bucket (e.g. `2026-02-17`, `2026-W08`)
    pub period: String,
    /// Current counter value
    pub value: i64,
    /// Last update timestamp (ISO 8601)
    pub updated_at: String,
}

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

/// Individual request log entry with detailed information
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "sqlx-types", derive(sqlx::FromRow))]
pub struct RequestLog {
    /// Unique identifier for this log entry
    pub id: String,
    /// When the request was made
    pub timestamp: DateTime<Utc>,
    /// API key UUID used for the request
    pub api_key_id: String,
    /// Friendly name of the API key
    pub api_key_name: String,
    /// Tool/endpoint that was invoked
    pub tool_name: String,
    /// HTTP status code of the response
    pub status_code: i32,
    /// Response time in milliseconds (if available)
    pub response_time_ms: Option<i32>,
    /// Error message if the request failed
    pub error_message: Option<String>,
    /// Request payload size in bytes
    pub request_size_bytes: Option<i32>,
    /// Response payload size in bytes
    pub response_size_bytes: Option<i32>,
}

/// Usage statistics for a specific tool
#[derive(Debug, Serialize)]
pub struct ToolUsage {
    /// Name of the tool
    pub tool_name: String,
    /// Number of times the tool was called
    pub request_count: u64,
    /// Percentage of successful calls
    pub success_rate: f64,
    /// Average response time (ms)
    pub average_response_time: f64,
}

/// JWT token usage record for tracking
#[derive(Debug, Serialize, Deserialize)]
pub struct JwtUsage {
    /// Unique identifier for this usage record
    pub id: Option<i64>,
    /// ID of the user who made the request
    pub user_id: Uuid,
    /// When the request was made
    pub timestamp: DateTime<Utc>,
    /// API endpoint that was accessed
    pub endpoint: String,
    /// HTTP method used (GET, POST, etc.)
    pub method: String,
    /// HTTP status code returned
    pub status_code: u16,
    /// Response time in milliseconds
    pub response_time_ms: Option<u32>,
    /// Request payload size in bytes
    pub request_size_bytes: Option<u32>,
    /// Response payload size in bytes
    pub response_size_bytes: Option<u32>,
    /// Client IP address
    pub ip_address: Option<String>,
    /// Client user agent string
    pub user_agent: Option<String>,
}

/// Input parameters for inserting a new LLM usage record
#[derive(Debug)]
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
    /// Type of call (e.g. "chat", "insight", "embedding")
    pub call_type: &'a str,
    /// Number of tool calls in this interaction
    pub tool_calls_count: i64,
    /// Execution time in milliseconds
    pub execution_time_ms: Option<i64>,
}

/// Aggregated LLM usage row grouped by provider, model, and call type
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

/// Daily LLM usage summary
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
