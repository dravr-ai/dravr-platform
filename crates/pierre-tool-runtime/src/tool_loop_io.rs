// ABOUTME: What a tool loop takes in and what it hands back — params, result, and the tally
// ABOUTME: that turns a loop's running accounting into one of the result's five exit shapes

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Inputs and outputs of a tool loop.
//!
//! Every loop strategy in [`crate::tool_execution`] reads the same
//! [`ToolLoopParams`] and returns the same [`ToolLoopResult`], and the chat
//! pipeline reads that result without knowing which strategy produced it.
//!
//! `ToolLoopTally` is the running half of that contract: the activity list,
//! reconnect offer, tool count and tool names a loop accumulates as it runs.
//! Each of its constructors stamps one exit shape — an auth trip, a Guardian
//! block, a Guardian confirmation, an answer, or an exhausted iteration budget
//! — onto that accounting, so the five shapes are written once instead of once
//! per loop.

use std::sync::Arc;

use pierre_llm::{ChatProvider, McpServerConfig, TokenUsage, Tool};
use pierre_services::chat_stream::TurnEventSink;

use pierre_core::errors::AppError;
use pierre_core::models::TenantId;

use crate::llm_call_record::LlmCallRecorder;
use crate::protocol::UniversalExecutor;

/// One round of tool dispatch as the in-memory loop sees it.
///
/// Carries an optional preamble of assistant text that accompanied the tool
/// request, plus the formatted tool result text. Persisting both lets a
/// follow-up turn replay the same [`pierre_llm::ChatMessage`] sequence the in-memory
/// loop produced and gives the model the same evidence base when answering
/// grounded follow-ups.
#[derive(Debug, Clone)]
pub struct ToolRoundRecord {
    /// Assistant text emitted alongside the tool call. Empty when the
    /// model returned only a tool call (no preamble). The replay path
    /// inserts this as `ChatMessage::assistant(content)`.
    pub assistant_text: String,
    /// Formatted tool result the loop pushes back to the model as a user
    /// turn (e.g. `"[Tool Result for get_activities]: ..."`). Always set.
    pub tool_result_text: String,
}

/// Sink that receives one [`ToolRoundRecord`] per tool dispatch round.
///
/// Implementations persist the round so subsequent turns rebuild the same
/// `Vec<ChatMessage>` that the in-memory loop produced. Without this, the
/// next turn sees only the final assistant text and the model — having no
/// trace of the tool result it consumed — defaults to refusing grounded
/// follow-ups ("I don't have access to your Strava data").
///
/// Invocations happen on the async runtime but the sink method itself is
/// synchronous; implementers spawn a task or push to a channel if the
/// underlying persistence is async.
pub trait ToolMessageRecorder: Send + Sync {
    /// Record a completed tool dispatch round.
    fn record(&self, record: ToolRoundRecord);
}

/// Parameters for the multi-turn tool execution loop
pub struct ToolLoopParams<'a> {
    /// LLM provider to use for completions
    pub provider: &'a ChatProvider,
    /// MCP executor for running tool calls (Arc for sharing with SDK tool handler closures)
    pub executor: Arc<UniversalExecutor>,
    /// Tool definitions available for function calling
    pub tools: &'a Tool,
    /// Model identifier for the LLM request
    pub model: &'a str,
    /// User ID for tool execution context
    pub user_id: &'a str,
    /// Tenant ID for multi-tenant isolation
    pub tenant_id: TenantId,
    /// Maximum number of tool-calling iterations before forcing a response
    pub max_iterations: usize,
    /// Optional per-LLM-call sink. When set, each provider call inside
    /// the loop produces a [`crate::llm_call_record::LlmCallRecord`] that the sink persists.
    /// When absent, the loop still accumulates cumulative usage for
    /// the returned [`ToolLoopResult`] without recording individual calls.
    pub call_recorder: Option<Arc<dyn LlmCallRecorder>>,
    /// Optional sink that persists each tool dispatch round (assistant
    /// preamble + formatted tool result) so follow-up turns can replay
    /// the grounded evidence the model already saw. Absent in callers
    /// that have no conversation to attach the rows to (e.g. one-shot
    /// non-conversational tool runs).
    pub tool_message_recorder: Option<Arc<dyn ToolMessageRecorder>>,
    /// Optional per-coach LLM sampling temperature. When `Some`, applied
    /// to every `ChatRequest` in the loop via `with_temperature`. When
    /// `None`, the provider/server default is used.
    pub temperature: Option<f32>,
    /// Optional sink for token-level streaming events. When set on the
    /// headless tool loop branch, the loop calls Copilot's
    /// `converse_stream()` instead of `converse()` and forwards each
    /// observed text delta and tool-call snapshot through the sink.
    /// The sink is a [`pierre_services::chat_stream::TurnEventSink`]
    /// — see that type for the event shape.
    pub stream_sink: Option<TurnEventSink>,
    /// MCP servers exposed to an ACP-managed provider (Copilot Headless) so
    /// the model can call Dravr tools natively over the Agent Client Protocol
    /// instead of text-based `<tool_call>` simulation. Only the headless tool
    /// loop forwards these into the ACP `session/new`; other loops ignore
    /// them. Empty for providers without SDK tool calling.
    pub mcp_servers: Vec<McpServerConfig>,
}

/// A tool blocked by the runtime Guardian while in `enforce` mode.
///
/// Surfaced out of the tool loop as an out-of-band signal (parallel to
/// `pending_provider_auth_required`) so the chat pipeline can render a
/// deterministic, localized "blocked for safety" reply instead of feeding the
/// raw in-band denial back to the LLM and letting it paraphrase a refusal.
/// Only ever set in `enforce` mode (the default) — `off` and `observe` log and
/// fall through to execution, so this stays `None` there.
#[derive(Debug, Clone)]
pub struct GuardianDenial {
    /// Name of the tool the Guardian blocked.
    pub tool_name: String,
    /// Machine-readable denial reason (`DenyReason::as_str`), e.g.
    /// `budget_exceeded`, `tainted_sink`, `egress_forbidden`. Used for
    /// structured logging — never shown verbatim to the user.
    pub reason: String,
}

/// Result of running the multi-turn tool execution loop
pub struct ToolLoopResult {
    /// Final text content from LLM
    pub content: String,
    /// Token usage statistics if available
    pub usage: Option<TokenUsage>,
    /// Finish reason if available
    pub finish_reason: Option<String>,
    /// Activity list from `get_activities` tool (to prepend to response)
    pub activity_list: Option<String>,
    /// Total tool calls executed across all iterations
    pub tool_calls_count: u32,
    /// Names of every MCP tool invoked during the loop, in call order.
    /// Persisted alongside the LLM usage row so per-turn observability
    /// can answer "which tools ran for this turn".
    pub tools_called: Vec<String>,
    /// Provider slug that triggered an `AppError::ProviderAuthRequired`
    /// during a tool dispatch in this loop. The chat pipeline detects this
    /// and short-circuits the turn with a deterministic re-auth reply
    /// (containing a minted hosted-login URL) instead of letting the LLM
    /// rephrase a generic refusal.
    pub pending_provider_auth_required: Option<String>,
    /// Backend key of a provider the athlete must reconnect on a turn that was
    /// ANSWERED anyway — `get_activities` read the window from the athlete's
    /// healthy connections while this one's token was dead.
    ///
    /// The soft twin of `pending_provider_auth_required`: that one means the ask
    /// went unanswered and the reply becomes the reconnect message, this one
    /// means the reply is the model's own and the reconnect offer joins it. The
    /// chat pipeline's auth-recovery stage reads the hard signal first, so a turn
    /// carrying both blanks: it did not answer what the athlete asked, and the
    /// provider that blanked it is the one to offer.
    pub served_without_provider: Option<String>,
    /// The first tool the runtime Guardian blocked this turn (`enforce` mode
    /// only). The chat pipeline detects this and short-circuits with a
    /// localized "blocked for safety" reply (`KEY_GUARDIAN_DENIED`). `None`
    /// when no tool was denied (always, in `observe`).
    pub guardian_denied: Option<GuardianDenial>,
    /// The first tool the Guardian parked pending user confirmation this turn
    /// (`TaintedDestructive::Confirm`, `enforce` mode only). The chat pipeline
    /// short-circuits with the localized confirmation prompt
    /// (`KEY_GUARDIAN_CONFIRM_PROMPT`) carrying the claim token.
    pub guardian_confirm: Option<GuardianConfirmRequest>,
    /// Set by the capability-recovery stage when the delivered reply carries a
    /// data-access claim the platform could not stand behind — either an
    /// unrefuted "I can't reach your data" or the reconnect message that
    /// replaced it. The turn then persists with
    /// `UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON` so the row never re-enters a
    /// later prompt: connection state is re-derived every turn, and replaying a
    /// moment-in-time failure is what turned one 2026-07-24 apology into an
    /// identical one 18 days later.
    pub capability_claim_unverified: bool,
}

/// A tool call the Guardian parked pending `/confirm`·`/deny` resolution.
///
/// Surfaced out of the tool loop as an out-of-band signal (parallel to
/// [`GuardianDenial`]) so the chat pipeline renders a deterministic,
/// localized confirmation ask instead of letting the LLM paraphrase it.
#[derive(Debug, Clone)]
pub struct GuardianConfirmRequest {
    /// Name of the parked tool (a static registry identifier — shown to the
    /// user so consent is meaningful; arguments are never echoed).
    pub tool_name: String,
    /// Opaque claim token resolving the parked row.
    pub pending_id: String,
}

/// The accounting a tool loop carries from iteration to iteration.
///
/// Every exit a loop can take reports the same four things — the activity list
/// it captured, the provider a window was served without, how many tool calls
/// ran and which tools they were. Holding them together lets each exit name
/// only what distinguishes it.
#[derive(Debug, Default)]
pub(crate) struct ToolLoopTally {
    /// Activity list captured from a `get_activities` result this turn.
    pub activity_list: Option<String>,
    /// Total tool calls dispatched across every iteration so far.
    pub tool_calls_count: u32,
    /// Names of the tools that actually ran, in call order.
    pub tools_called: Vec<String>,
    /// Backend key of a provider a window was served without.
    pub served_without_provider: Option<String>,
}

impl ToolLoopTally {
    /// Exit carrying a provider the athlete must re-authorize before the ask
    /// can be answered.
    pub fn provider_auth_required(
        self,
        usage: Option<TokenUsage>,
        provider: String,
    ) -> ToolLoopResult {
        ToolLoopResult {
            content: String::new(),
            usage,
            finish_reason: Some("provider_auth_required".to_owned()),
            activity_list: self.activity_list,
            tool_calls_count: self.tool_calls_count,
            tools_called: self.tools_called,
            pending_provider_auth_required: Some(provider),
            served_without_provider: self.served_without_provider,
            guardian_denied: None,
            guardian_confirm: None,
            capability_claim_unverified: false,
        }
    }

    /// Exit carrying the Guardian block that stopped the turn.
    pub fn guardian_denied(
        self,
        usage: Option<TokenUsage>,
        denial: GuardianDenial,
    ) -> ToolLoopResult {
        ToolLoopResult {
            content: String::new(),
            usage,
            finish_reason: Some("guardian_denied".to_owned()),
            activity_list: self.activity_list,
            tool_calls_count: self.tool_calls_count,
            tools_called: self.tools_called,
            pending_provider_auth_required: None,
            served_without_provider: self.served_without_provider,
            guardian_denied: Some(denial),
            guardian_confirm: None,
            capability_claim_unverified: false,
        }
    }

    /// Exit carrying the tool call the Guardian parked pending confirmation.
    pub fn guardian_confirm(
        self,
        usage: Option<TokenUsage>,
        confirm: GuardianConfirmRequest,
    ) -> ToolLoopResult {
        ToolLoopResult {
            content: String::new(),
            usage,
            finish_reason: Some("guardian_confirm".to_owned()),
            activity_list: self.activity_list,
            tool_calls_count: self.tool_calls_count,
            tools_called: self.tools_called,
            pending_provider_auth_required: None,
            served_without_provider: self.served_without_provider,
            guardian_denied: None,
            guardian_confirm: Some(confirm),
            capability_claim_unverified: false,
        }
    }

    /// Exit carrying the model's own answer and the finish reason it reported.
    pub fn answered(
        self,
        content: String,
        usage: Option<TokenUsage>,
        finish_reason: Option<String>,
    ) -> ToolLoopResult {
        ToolLoopResult {
            content,
            usage,
            finish_reason,
            activity_list: self.activity_list,
            tool_calls_count: self.tool_calls_count,
            tools_called: self.tools_called,
            pending_provider_auth_required: None,
            served_without_provider: self.served_without_provider,
            guardian_denied: None,
            guardian_confirm: None,
            capability_claim_unverified: false,
        }
    }

    /// Exit taken when the iteration budget ran out before the model answered.
    pub fn max_iterations(self, usage: Option<TokenUsage>) -> ToolLoopResult {
        ToolLoopResult {
            content: String::new(),
            usage,
            finish_reason: Some("max_iterations".to_owned()),
            activity_list: self.activity_list,
            tool_calls_count: self.tool_calls_count,
            tools_called: self.tools_called,
            pending_provider_auth_required: None,
            served_without_provider: self.served_without_provider,
            guardian_denied: None,
            guardian_confirm: None,
            capability_claim_unverified: false,
        }
    }
}

impl ToolLoopResult {
    /// `true` when this turn produced nothing the athlete can be shown.
    ///
    /// Mirrors the rule the messaging egress applies before it sends the
    /// lost-turn string: prose **and** list both empty. A turn carrying an
    /// activity list is a real reply — the egress renders it — so it is not lost.
    ///
    /// The short-circuit signals are excluded deliberately. Empty `content`
    /// alongside any of them is not a lost turn but a *deliberate* one: the chat
    /// pipeline is about to replace the reply with a hosted re-auth URL, a
    /// Guardian refusal, or a confirmation prompt. Falling those back to a second
    /// provider would spend money to overwrite an answer the platform has already
    /// decided to give, and would read to the athlete as the refusal not sticking.
    #[must_use]
    pub fn is_lost_turn(&self) -> bool {
        self.content.trim().is_empty()
            && self.activity_list.is_none()
            && self.pending_provider_auth_required.is_none()
            && self.guardian_denied.is_none()
            && self.guardian_confirm.is_none()
    }

    /// The error describing a lost turn, for the fallback's log line.
    ///
    /// Carries the tool-call count because that is exactly what separates "the
    /// model said nothing" from "the turn died on or after a tool batch" — the
    /// distinction embacle's own empty-turn warn cannot make, since it drops
    /// `tool_calls` on the floor.
    #[must_use]
    pub fn lost_turn_error(&self) -> AppError {
        AppError::external_service(
            "copilot_headless",
            format!(
                "empty turn: no content and no activity list after {} tool call(s)",
                self.tool_calls_count
            ),
        )
    }
}
