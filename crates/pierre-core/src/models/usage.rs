// ABOUTME: Usage and dashboard data types for tracking API requests, tools, LLM calls, and quotas
// ABOUTME: Pure DTOs shared between the main crate's database layer and dashboard routes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::conversation::ConversationTurnId;

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
#[derive(Debug, Clone, Serialize)]
pub struct LlmUsageRecord {
    /// Unique record ID
    pub id: String,
    /// Tenant that owns this usage
    pub tenant_id: String,
    /// User who triggered the call
    pub user_id: String,
    /// Associated conversation (if from chat)
    pub conversation_id: Option<String>,
    /// Conversation-turn correlation identifier. See
    /// [`ConversationTurnId`] for semantics.
    pub turn_id: ConversationTurnId,
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
    /// Prompt tokens served from the provider's context cache. Billed
    /// at 25% of the model's input rate per Gemini/OpenAI convention.
    /// Zero when the provider does not report cache metrics.
    pub cached_tokens: i64,
    /// Type of call (e.g. "chat", "insight", "embedding")
    pub call_type: String,
    /// Number of tool calls in this interaction
    pub tool_calls_count: i64,
    /// JSON-encoded list of MCP tool names invoked during this turn
    pub tools_called: String,
    /// Execution time in milliseconds
    pub execution_time_ms: Option<i64>,
    /// USD cost of this LLM call, populated at insert time via
    /// [`pierre_llm::pricing::calculate_cost`]. Zero when pricing data
    /// for the (provider, model) pair is not known.
    pub cost_usd: f64,
    /// 1-based position of this call within the owning `turn_id`. The
    /// first LLM call of a turn is `1`, the second `2`, and so on.
    /// Turn-level queries order by this column to reconstruct tool loops.
    pub call_sequence: Option<i64>,
    /// When the usage was recorded (ISO 8601)
    pub created_at: String,
}

/// Per-turn summary record returned by the internal conversation-turn
/// endpoint. Aggregates every [`LlmUsageRecord`] that shares a
/// [`ConversationTurnId`] into a single response row.
///
/// # Fidelity note
///
/// The `llm_calls` array has one entry per row persisted under the
/// turn. The chat pipeline writes a single row per turn with token
/// counts cumulated across every internal LLM call in the tool loop,
/// so today's `llm_calls` has a single element even when the tool
/// loop made many LLM calls. The `total_*` aggregates are still
/// accurate per turn.
///
/// A future change can switch to per-LLM-call rows by subscribing a
/// `PerCallMetricsSink` to the `embacle::MetricsProvider` that wraps
/// the platform's LLM runner — the sink trait and `PerCallMetric`
/// record already ship in embacle. Doing so needs provider-construction
/// to carry the tenant/user/turn context, so it is tracked separately.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationTurnSummary {
    /// Conversation-turn correlation identifier
    pub turn_id: ConversationTurnId,
    /// Tenant that owns the turn
    pub tenant_id: String,
    /// User who triggered the turn
    pub user_id: String,
    /// Conversation the turn belongs to, if any
    pub conversation_id: Option<String>,
    /// Distinct MCP tool names invoked across every LLM call in the turn
    pub tools_called: Vec<String>,
    /// Individual LLM calls that were made for this turn, in insertion order
    pub llm_calls: Vec<ConversationTurnLlmCall>,
    /// Sum of `total_tokens` across every LLM call in the turn
    pub total_tokens: i64,
    /// Sum of `prompt_tokens` across every LLM call in the turn
    pub total_prompt_tokens: i64,
    /// Sum of `completion_tokens` across every LLM call in the turn
    pub total_completion_tokens: i64,
    /// Sum of `execution_time_ms` across every LLM call in the turn (milliseconds)
    pub total_latency_ms: i64,
    /// Earliest `created_at` among the LLM calls — approximates the turn's start
    pub first_call_at: String,
    /// Latest `created_at` among the LLM calls — approximates the turn's end
    pub last_call_at: String,
}

/// Single LLM call attributed to a conversation turn, surfaced by the
/// per-turn summary endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationTurnLlmCall {
    /// LLM provider that served the call
    pub provider: String,
    /// Model identifier
    pub model: String,
    /// Prompt token count reported by the provider
    pub prompt_tokens: i64,
    /// Completion token count reported by the provider
    pub completion_tokens: i64,
    /// Total tokens (prompt + completion)
    pub total_tokens: i64,
    /// Execution time in milliseconds
    pub latency_ms: Option<i64>,
    /// When the call was recorded (ISO 8601)
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

/// `call_type` sentinel that marks an `llm_usage` row as the
/// terminal turn-summary record rather than a per-LLM-call record.
///
/// Turn-summary rows are written once per turn with zero token
/// counts; they carry the turn-level aggregates (`tool_calls_count`,
/// `tools_called`, end-to-end `execution_time_ms`) that cannot be
/// attributed to any single LLM call. Per-call rows use the flow's
/// natural `call_type` (`"chat"`, `"insight"`, `"messaging"`,
/// `"mcp_tool"`, ...).
///
/// Aggregate queries filter this value out so call counts and token
/// sums reflect real LLM activity only; the per-turn endpoint joins
/// both shapes back together.
pub const TURN_SUMMARY_CALL_TYPE: &str = "turn_summary";

/// Input parameters for inserting a new LLM usage record
#[derive(Debug)]
pub struct InsertLlmUsage<'a> {
    /// Tenant that owns this usage
    pub tenant_id: &'a str,
    /// User who triggered the call
    pub user_id: &'a str,
    /// Associated conversation (if from chat)
    pub conversation_id: Option<&'a str>,
    /// Conversation-turn correlation identifier threaded through the
    /// entire user utterance (inbound webhook / chat request through
    /// every LLM and tool call). Queries by turn id are the per-turn
    /// observability primitive.
    pub turn_id: ConversationTurnId,
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
    /// Prompt tokens served from the provider's context cache. Zero
    /// when the provider did not report cache hits. Cached tokens are
    /// billed at 25% of the model's input rate.
    pub cached_tokens: i64,
    /// Type of call (e.g. "chat", "insight", "embedding")
    pub call_type: &'a str,
    /// Number of tool calls in this interaction
    pub tool_calls_count: i64,
    /// JSON-encoded list of MCP tool names invoked during this turn.
    /// Populated by the chat pipeline's tool dispatcher; an empty JSON
    /// array (`"[]"`) if no tools ran.
    pub tools_called: &'a str,
    /// Execution time in milliseconds
    pub execution_time_ms: Option<i64>,
    /// USD cost of this call (caller must precompute via
    /// `pierre_llm::pricing::calculate_cost`). Zero for unknown models.
    pub cost_usd: f64,
    /// 1-based position of this LLM call inside the owning turn.
    /// `Some(1)` for the first call, `Some(2)` for the second, etc.
    /// `None` is permitted only during backfill of pre-Phase-1 rows.
    pub call_sequence: Option<i64>,
}

/// Record of a single embedding API call, persisted in the
/// `embedding_usage` table. Separate from `llm_usage` so that
/// embedding volume never dilutes chat-token billing aggregates.
#[derive(Debug, Clone)]
pub struct EmbeddingUsageRecord {
    /// Unique record ID
    pub id: String,
    /// Tenant that owns the call
    pub tenant_id: String,
    /// User that triggered the call
    pub user_id: String,
    /// Embedding provider name (e.g. "gemini")
    pub provider: String,
    /// Model identifier (e.g. "text-embedding-004")
    pub model: String,
    /// Characters-based token estimate for the input text
    pub input_tokens: i64,
    /// USD cost of the embedding call
    pub cost_usd: f64,
    /// When the call was recorded (ISO 8601)
    pub created_at: String,
}

/// Insert parameters for a new `embedding_usage` row.
#[derive(Debug)]
pub struct InsertEmbeddingUsage<'a> {
    /// Tenant that owns the call
    pub tenant_id: &'a str,
    /// User that triggered the call
    pub user_id: &'a str,
    /// Embedding provider name (e.g. "gemini")
    pub provider: &'a str,
    /// Model identifier
    pub model: &'a str,
    /// Characters-based token estimate for the input text
    pub input_tokens: i64,
    /// USD cost of the embedding call
    pub cost_usd: f64,
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
