// ABOUTME: Usage and dashboard data types for tracking API requests, tools, LLM calls, and quotas
// ABOUTME: Pure DTOs shared between the main crate's database layer and dashboard routes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::conversation::ConversationTurnId;
use crate::models::user::UserTier;

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
    /// Prompt tokens served from the provider's context cache. Billed at
    /// the model's cache-read multiplier of its input rate. Zero when the
    /// provider does not report cache metrics.
    pub cached_tokens: i64,
    /// Prompt tokens written into the provider's context cache by this
    /// call. Billed at the model's cache-write multiplier — a premium on
    /// Anthropic, not a discount. Zero when unreported.
    pub cached_write_tokens: i64,
    /// The part of `cached_write_tokens` written to a 1-hour cache entry,
    /// billed at the model's 1-hour write multiplier (2x input on
    /// Anthropic) rather than the 5-minute one. Zero when unreported.
    pub cached_write_1h_tokens: i64,
    /// Reasoning tokens reported apart from the completion count, billed
    /// at the output rate. Zero when unreported.
    pub reasoning_tokens: i64,
    /// Type of call (e.g. "chat", "insight", "embedding")
    pub call_type: String,
    /// Number of tool calls in this interaction
    pub tool_calls_count: i64,
    /// JSON-encoded list of MCP tool names invoked during this turn
    pub tools_called: String,
    /// Execution time in milliseconds
    pub execution_time_ms: Option<i64>,
    /// USD cost of this LLM call, populated at insert time via the
    /// `pierre_services::pricing` registry over `embacle::pricing`. Zero when
    /// pricing data for the (provider, model) pair is not known.
    pub cost_usd: f64,
    /// 1-based position of this call within the owning `turn_id`. The
    /// first LLM call of a turn is `1`, the second `2`, and so on.
    /// Turn-level queries order by this column to reconstruct tool loops.
    pub call_sequence: Option<i64>,
    /// When the usage was recorded (ISO 8601)
    pub created_at: String,
    /// How an outside caller's read ended (`ok`, `error`, `refused`,
    /// [`CallOutcome::as_str`]); `None` on every other row (carnet#1543).
    pub outcome: Option<String>,
}

/// Individual request log entry with detailed information
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "sqlx-types", derive(sqlx::FromRow))]
pub struct RequestLog {
    /// Unique identifier for this log entry
    pub id: String,
    /// When the request was made
    pub timestamp: Timestamp,
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
#[derive(Debug, Serialize, Deserialize)]
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
    pub timestamp: Timestamp,
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
    /// The consent grant a delegated token's request was admitted under,
    /// whose hourly budget counts it; `None` for every other credential
    pub client_grant_id: Option<String>,
}

/// An admin's per-user monthly request limit, as `user_rate_limit_overrides`
/// holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonthlyLimitOverride {
    /// No override row: the user's tier sets the monthly limit
    NotSet,
    /// A row whose monthly limit is NULL: no monthly ceiling
    Unlimited,
    /// A row capping the UTC month at this many requests
    Limit(u32),
}

impl MonthlyLimitOverride {
    /// The monthly request limit in force for a user on `tier`: this
    /// override's when one is set, else the tier's. `None` is no ceiling.
    #[must_use]
    pub const fn resolve(self, tier: &UserTier) -> Option<u32> {
        match self {
            Self::NotSet => tier.monthly_limit(),
            Self::Unlimited => None,
            Self::Limit(limit) => Some(limit),
        }
    }
}

/// A user's JWT requests since the first instant of the UTC month and the
/// admin's monthly override for them, read in one statement: everything the
/// JWT request budget needs besides the user's tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JwtMonthlyUsage {
    /// Requests counted this UTC month, before the current one
    pub used: u32,
    /// The admin's per-user monthly limit, if one is set
    pub monthly_override: MonthlyLimitOverride,
}

/// A metered credential's calls inside its own sliding rate-limit window:
/// an API key's `api_key_usage` rows, or an A2A client's `a2a_usage` rows.
///
/// Read in one statement, so the count and the oldest call describe the same
/// rows: the window frees its first slot at `oldest` plus the credential's
/// `rate_limit_window_seconds`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowUsage {
    /// Calls recorded inside the window
    pub count: u32,
    /// The earliest of those calls; `None` when the window holds none
    pub oldest: Option<Timestamp>,
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

/// `call_type` of a tool `POST /mcp` executed with no model in the loop.
///
/// See [`InsertLlmUsage::for_direct_tool_call`]. The row counts per-tool volume,
/// not an LLM call, so every reader that counts calls leaves it out.
pub const DIRECT_TOOL_CALL_TYPE: &str = "mcp_tool";

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
    /// when the provider did not report cache hits. Billed at the
    /// model's cache-read multiplier of its input rate.
    pub cached_tokens: i64,
    /// Prompt tokens written into the provider's context cache by this
    /// call. Billed at the model's cache-write multiplier, which is a
    /// premium (1.25x) on Anthropic rather than a discount.
    pub cached_write_tokens: i64,
    /// The part of `cached_write_tokens` written to a 1-hour cache entry,
    /// billed at the model's 1-hour write multiplier (2x input on
    /// Anthropic) rather than the 5-minute one. Zero when unreported.
    pub cached_write_1h_tokens: i64,
    /// Reasoning tokens reported apart from the completion count, billed
    /// at the output rate. Zero when the provider does not report them.
    pub reasoning_tokens: i64,
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
    /// USD cost of this call (caller must precompute via the
    /// `pierre_services::pricing` registry). Zero for unknown models.
    pub cost_usd: f64,
    /// 1-based position of this LLM call inside the owning turn.
    /// `Some(1)` for the first call, `Some(2)` for the second, etc.
    /// `None` is permitted only during backfill of pre-Phase-1 rows.
    pub call_sequence: Option<i64>,
}

impl<'a> InsertLlmUsage<'a> {
    /// A direct MCP tool call: real work, but no LLM request behind it.
    ///
    /// `POST /mcp` can execute a tool without any model in the loop, and those
    /// executions still belong in `llm_usage` because that is where per-tool
    /// volume is counted. Every token and cost field is therefore zero as a
    /// *correct* measurement, not a missing one, and the `model` column carries
    /// the tool name because there is no model to name. Naming the shape here
    /// keeps that reasoning attached to it rather than to one call site.
    #[must_use]
    pub const fn for_direct_tool_call(
        tenant_id: &'a str,
        user_id: &'a str,
        turn_id: ConversationTurnId,
        tool_name: &'a str,
        tools_called: &'a str,
        execution_time_ms: i64,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            conversation_id: None,
            turn_id,
            provider: "direct",
            model: tool_name,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            cached_write_tokens: 0,
            cached_write_1h_tokens: 0,
            reasoning_tokens: 0,
            call_type: DIRECT_TOOL_CALL_TYPE,
            tool_calls_count: 1,
            tools_called,
            execution_time_ms: Some(execution_time_ms),
            cost_usd: 0.0,
            call_sequence: None,
        }
    }

    /// The one summary row of a coaching turn an outside caller drove (an A2A
    /// agent, the athlete's API key): what the turn did as a whole, which no
    /// single LLM call's row can say.
    ///
    /// Zero tokens and cost, as every [`TURN_SUMMARY_CALL_TYPE`] row: the
    /// turn's LLM calls carry their own. The provider and model columns hold
    /// [`TURN_SUMMARY_CALL_TYPE`] because the turn has no single model to
    /// name, so no reader of the row can mistake it for a model's call.
    #[must_use]
    pub const fn for_turn_summary(
        tenant_id: &'a str,
        user_id: &'a str,
        conversation_id: Option<&'a str>,
        turn_id: ConversationTurnId,
        tools_called: &'a str,
        tool_calls_count: i64,
        execution_time_ms: i64,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            conversation_id,
            turn_id,
            provider: TURN_SUMMARY_CALL_TYPE,
            model: TURN_SUMMARY_CALL_TYPE,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            cached_write_tokens: 0,
            cached_write_1h_tokens: 0,
            reasoning_tokens: 0,
            call_type: TURN_SUMMARY_CALL_TYPE,
            tool_calls_count,
            tools_called,
            execution_time_ms: Some(execution_time_ms),
            cost_usd: 0.0,
            call_sequence: None,
        }
    }
}

/// Whose data an outside caller's read reached, and through what (carnet#1543).
///
/// The read is a client's direct tool call, or a turn of Dravr's agent it
/// drove; these are the columns of its `llm_usage` row the athlete's "who
/// read my data" feed shows.
#[derive(Debug, Clone, Copy)]
pub struct DelegatedRead<'a> {
    /// Whose data the call read: the coached athlete for a coach's read, the
    /// one group member a groups tool read with no subject resolved, the
    /// caller otherwise; `None` when the body read several members, which one
    /// row cannot name.
    pub subject_user_id: Option<&'a str>,
    /// The OAuth client the caller's token was issued to, as the credential
    /// names it; `None` when it names none.
    pub client_id: Option<&'a str>,
    /// The kind of credential the caller presented
    /// (`CredentialKind::as_str`); `None` when the call carried none.
    pub credential: Option<&'a str>,
    /// The transport the call arrived through (`Transport::as_str`).
    pub transport: &'a str,
    /// The admin whose support session made the call acting as the user;
    /// `None` for the user's own call.
    pub impersonator_user_id: Option<&'a str>,
    /// Whether the call or turn answered with a result, an error, or a
    /// refusal before the tool ran.
    pub outcome: CallOutcome,
}

/// How an outside caller's read ended (a direct tool call, an agent turn),
/// as its `llm_usage` row's `outcome` stores it: admin counts include every one, the athlete's
/// feed lists only [`Self::Ok`] (carnet#1543).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallOutcome {
    /// The tool answered with a result.
    Ok,
    /// The tool's body ran and answered with an error (an `isError` answer,
    /// or a failure it raised).
    Error,
    /// The call was answered before the tool's body ran: a scope, role,
    /// Guardian or read-subject refusal, or a failure on the way.
    Refused,
}

impl CallOutcome {
    /// The value the `outcome` column stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Refused => "refused",
        }
    }
}

/// What an outside caller's recorded read was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutsideReadKind {
    /// A tool the client called itself, over MCP or A2A.
    ToolCall,
    /// A coaching turn of Dravr's agent the caller asked for (an A2A text
    /// message, a chat message through the athlete's API key), with every
    /// tool the agent called in it.
    AgentTurn,
}

impl OutsideReadKind {
    /// The name the feed gives it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ToolCall => "tool_call",
            Self::AgentTurn => "agent_turn",
        }
    }
}

/// One recorded read an outside caller made of the athlete's own data, as
/// their "who read my data" feed lists it (carnet#1543): a tool call, or a
/// turn of Dravr's agent it drove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutsideRead {
    /// When the call was made (a turn: when it ended).
    pub read_at: Timestamp,
    /// A tool call or an agent turn.
    pub kind: OutsideReadKind,
    /// The tool the client called; `None` for an agent turn, which names
    /// its tools in [`Self::tools`].
    pub tool: Option<String>,
    /// Every tool the read ran: the one called, or the agent turn's, in
    /// call order.
    pub tools: Vec<String>,
    /// The client its credential names, when it names one.
    pub client_id: Option<String>,
    /// The kind of credential it came through (`CredentialKind::as_str`).
    pub credential: Option<String>,
    /// The transport it arrived through (`Transport::as_str`).
    pub transport: Option<String>,
    /// The admin whose support session made it, acting as the athlete.
    pub impersonator_user_id: Option<String>,
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
    /// Prompt tokens served from the provider's cache across all matching
    /// records. Summed so the read path can apply the cache discount — it
    /// recomputed cost with a hardcoded zero before, ignoring the discount
    /// even on rows that stored a real count.
    pub cached_tokens: i64,
    /// Prompt tokens written into the provider's context cache. Billed at
    /// the model's cache-write multiplier — a premium on Anthropic.
    pub cached_write_tokens: i64,
    /// The part of `cached_write_tokens` written to a 1-hour cache entry,
    /// billed at the model's 1-hour write multiplier (2x input on
    /// Anthropic) rather than the 5-minute one. Zero when unreported.
    pub cached_write_1h_tokens: i64,
    /// Reasoning tokens reported apart from the completion count, billed
    /// at the output rate.
    pub reasoning_tokens: i64,
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
    /// Mean execution time in milliseconds across calls for this day.
    /// Zero when no rows recorded `execution_time_ms`.
    pub avg_execution_time_ms: f64,
    /// Prompt tokens served from the provider's cache across all matching
    /// records. Summed so the read path can apply the cache discount — it
    /// recomputed cost with a hardcoded zero before, ignoring the discount
    /// even on rows that stored a real count.
    pub cached_tokens: i64,
    /// Prompt tokens written into the provider's context cache. Billed at
    /// the model's cache-write multiplier — a premium on Anthropic.
    pub cached_write_tokens: i64,
    /// The part of `cached_write_tokens` written to a 1-hour cache entry,
    /// billed at the model's 1-hour write multiplier (2x input on
    /// Anthropic) rather than the 5-minute one. Zero when unreported.
    pub cached_write_1h_tokens: i64,
    /// Reasoning tokens reported apart from the completion count, billed
    /// at the output rate.
    pub reasoning_tokens: i64,
}
