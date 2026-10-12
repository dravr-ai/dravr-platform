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

use pierre_llm::call_record::LlmCallRecorder;
use pierre_llm::{ChatProvider, McpServerConfig, TokenUsage, Tool};
use pierre_services::chat_stream::TurnEventSink;

use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::TenantId;

use crate::protocol::UniversalExecutor;
use crate::tool_loop_telemetry::accumulate_optional;

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
    /// the loop produces a [`pierre_llm::call_record::LlmCallRecord`] that the sink persists.
    /// When absent, the loop still accumulates cumulative usage for
    /// the returned [`ToolLoopResult`] without recording individual calls.
    pub call_recorder: Option<Arc<dyn LlmCallRecorder>>,
    /// Optional sink that persists each tool dispatch round (assistant
    /// preamble + formatted tool result) so follow-up turns can replay
    /// the grounded evidence the model already saw. Absent in callers
    /// that have no conversation to attach the rows to (e.g. one-shot
    /// non-conversational tool runs).
    pub tool_message_recorder: Option<Arc<dyn ToolMessageRecorder>>,
    /// Optional per-agent LLM sampling temperature. When `Some`, applied
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
    /// What the serving provider reported ignoring across the loop's LLM
    /// calls — a temperature it has no knob for, tools it simulated in text,
    /// an image it could not see. Deduplicated, in first-seen order: a loop
    /// re-sends the same request shape every iteration.
    pub provider_warnings: Vec<String>,
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
    /// Provider warnings seen so far, deduplicated in first-seen order.
    pub provider_warnings: Vec<String>,
}

impl ToolLoopTally {
    /// Fold one LLM call's provider warnings into the loop's list.
    pub fn note_provider_warnings(&mut self, warnings: Option<&[String]>) {
        for warning in warnings.unwrap_or_default() {
            if !self.provider_warnings.contains(warning) {
                self.provider_warnings.push(warning.clone());
            }
        }
    }

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
            provider_warnings: self.provider_warnings,
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
            provider_warnings: self.provider_warnings,
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
            provider_warnings: self.provider_warnings,
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
            provider_warnings: self.provider_warnings,
        }
    }

    /// Exit taken when the iteration budget ran out before the model answered.
    pub fn max_iterations(self, usage: Option<TokenUsage>) -> ToolLoopResult {
        ToolLoopResult {
            content: String::new(),
            usage,
            finish_reason: Some(MAX_ITERATIONS_FINISH_REASON.to_owned()),
            activity_list: self.activity_list,
            tool_calls_count: self.tool_calls_count,
            tools_called: self.tools_called,
            pending_provider_auth_required: None,
            served_without_provider: self.served_without_provider,
            guardian_denied: None,
            guardian_confirm: None,
            capability_claim_unverified: false,
            provider_warnings: self.provider_warnings,
        }
    }

    /// What the loop had done when its LLM call failed: the rounds that
    /// completed before it, with no answer of its own. The re-run on the
    /// chain's tail continues from those rounds and folds this in with
    /// [`ToolLoopResult::after_prior`], so the tools that already ran stay
    /// recorded for the turn.
    pub(crate) fn interrupted(self, usage: Option<TokenUsage>) -> ToolLoopResult {
        self.answered(String::new(), usage, None)
    }
}

/// The finish reason of a loop that spent its iteration budget.
pub(crate) const MAX_ITERATIONS_FINISH_REASON: &str = "max_iterations";

impl ToolLoopResult {
    /// `true` when the loop stopped because its iteration budget ran out.
    ///
    /// Such a turn is empty but not lost to a provider: the model kept calling
    /// tools. Re-running it on another tier would spend a second budget on the
    /// same loop, so the tail fallback leaves it to the chat pipeline.
    #[must_use]
    pub(crate) fn ran_out_of_iterations(&self) -> bool {
        self.finish_reason.as_deref() == Some(MAX_ITERATIONS_FINISH_REASON)
    }

    /// This result, continuing a turn whose earlier rounds `prior` ran on a
    /// tier that then failed.
    ///
    /// The answer and every exit signal are this result's; the tools that ran,
    /// their count, the tokens spent and the warnings seen are both results'
    /// together, earlier rounds first. The activity list and the
    /// served-without-a-provider offer are this result's when it has its own,
    /// else the earlier rounds': the tail may answer from the tool results it
    /// was handed without calling the tool again.
    #[must_use]
    pub(crate) fn after_prior(self, prior: Self) -> Self {
        let mut tools_called = prior.tools_called;
        tools_called.extend(self.tools_called);
        let mut provider_warnings = prior.provider_warnings;
        for warning in self.provider_warnings {
            if !provider_warnings.contains(&warning) {
                provider_warnings.push(warning);
            }
        }
        Self {
            usage: sum_usage(prior.usage, self.usage),
            activity_list: self.activity_list.or(prior.activity_list),
            tool_calls_count: prior.tool_calls_count.saturating_add(self.tool_calls_count),
            tools_called,
            served_without_provider: self
                .served_without_provider
                .or(prior.served_without_provider),
            provider_warnings,
            ..self
        }
    }

    /// Build the [`ToolLoopResult`] for a Guardian-rejected plan.
    ///
    /// Covers both the over-step-cap and the failed-static-verification cases.
    /// Surfaced as a `guardian_denied` so the chat pipeline renders the localized
    /// `KEY_GUARDIAN_DENIED` reply (P3) instead of the rejection flowing through
    /// post-processing as if it were model output.
    pub(crate) fn guardian_plan_denied(reason: &str) -> Self {
        Self {
            content: String::new(),
            usage: None,
            finish_reason: Some("guardian_plan_rejected".to_owned()),
            activity_list: None,
            tool_calls_count: 0,
            tools_called: Vec::new(),
            pending_provider_auth_required: None,
            served_without_provider: None,
            guardian_denied: Some(GuardianDenial {
                tool_name: "(plan)".to_owned(),
                reason: reason.to_owned(),
            }),
            guardian_confirm: None,
            capability_claim_unverified: false,
            provider_warnings: Vec::new(),
        }
    }

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
    pub fn lost_turn_error(&self, provider: &str) -> AppError {
        AppError::external_service(
            provider,
            format!(
                "empty turn: no content and no activity list after {} tool call(s)",
                self.tool_calls_count
            ),
        )
    }
}

/// Whether a tool loop's error from the chain's head re-runs the turn against
/// the chain's tail.
///
/// The tool loops converse with the chain's head directly — the headless loop
/// through its turn provider, the API loop through
/// `ChatProvider::complete_with_tools_on_head` — so embacle's fall-through
/// policy never sees their errors; this is that policy's `AppError` image —
/// the codes `ErrorKind::is_provider_fault` maps to
/// (`ResourceUnavailable` for a timeout or an unavailable model,
/// `ExternalServiceError` for a vendor fault, `AuthInvalid` for an auth
/// failure, `InternalError` for a runner bug or a missing binary) plus the
/// platform's own auth and availability codes (`ExternalAuthFailed`,
/// `AuthExpired`, `ExternalServiceUnavailable`), and a provider's rate limit
/// or spent quota (`ExternalRateLimited`, which an HTTP 402 credit exhaustion
/// maps to as well) — the next tier holds its own account. Deterministic
/// failures — a rejected request (`InvalidInput`), a config error — do not
/// reroute: a second tier would answer them the same
/// way, and the athlete would see its rejection instead of the real one. The
/// athlete's own budget (`RateLimitExceeded`, `QuotaExceeded`) is refused by
/// the ingress gate before dispatch and never reaches this predicate.
#[must_use]
pub fn reroutes_to_tail(error: &AppError) -> bool {
    matches!(
        error.code,
        ErrorCode::ExternalAuthFailed
            | ErrorCode::ExternalServiceUnavailable
            | ErrorCode::ExternalServiceError
            | ErrorCode::ExternalRateLimited
            | ErrorCode::ResourceUnavailable
            | ErrorCode::AuthInvalid
            | ErrorCode::AuthExpired
            | ErrorCode::InternalError
    )
}

/// Where a headless (Copilot) turn's outcome goes next.
///
/// The headless loop converses with the chain's head directly, outside
/// embacle's fallback walk, so neither of the chain's fall-throughs fires for
/// it — not the empty-completion one, not the provider-fault one. This is
/// where both are re-created, from [`ToolLoopResult::is_lost_turn`] and
/// [`reroutes_to_tail`]; the loop only acts on the route it is handed.
pub(crate) enum HeadlessRoute {
    /// The head answered, or left a deliberate empty reply the chat pipeline
    /// is about to replace: serve it. Boxed so a route is the size of an
    /// error, not of a whole result.
    Serve(Box<ToolLoopResult>),
    /// The head lost the turn or failed with a provider fault: re-run the turn
    /// against the chain's tail, logging this as the reason it moved.
    Reroute(AppError),
    /// A deterministic failure every tier would answer the same way: return it.
    Fail(AppError),
}

impl HeadlessRoute {
    /// Route the head's outcome; `provider` names the head in a lost turn's
    /// error, which is what the fallback logs.
    pub(crate) fn of(outcome: Result<ToolLoopResult, AppError>, provider: &str) -> Self {
        match outcome {
            Ok(result) if result.is_lost_turn() => Self::Reroute(result.lost_turn_error(provider)),
            Ok(result) => Self::Serve(Box::new(result)),
            Err(err) if reroutes_to_tail(&err) => Self::Reroute(err),
            Err(err) => Self::Fail(err),
        }
    }
}

/// The identifier a tool call is recorded under.
///
/// The SDK transport reports the name the Copilot runtime calls the tool by,
/// which for a tool served over the turn's MCP server is
/// `<server>-<tool>` (`dravr-get_activities`); the ACP adapter sends only a
/// display title, which is then the best available identifier. Either way the
/// platform's own server prefix comes off, so the turn records the name the
/// registry, the plan card and the plan-saved check all compare against. A
/// tool from any other server keeps its prefix: it is a different tool.
pub(crate) fn observed_tool_name(call: &pierre_llm::ObservedToolCall) -> String {
    let reported = call.name.as_deref().unwrap_or(&call.title);
    reported
        .strip_prefix(PLATFORM_MCP_SERVER_NAME)
        .and_then(|rest| rest.strip_prefix('-'))
        .unwrap_or(reported)
        .to_owned()
}

/// The name the turn's MCP tool server is registered under.
///
/// It namespaces the platform's tools in the model's view, and the Copilot
/// runtime reports each one back as this name, a hyphen, and the tool's own
/// name.
pub const PLATFORM_MCP_SERVER_NAME: &str = "dravr";

/// Two optional usages summed, counting the optional counts too.
fn sum_usage(first: Option<TokenUsage>, second: Option<TokenUsage>) -> Option<TokenUsage> {
    match (first, second) {
        (None, None) => None,
        (Some(only), None) | (None, Some(only)) => Some(only),
        (Some(a), Some(b)) => {
            let mut sum = TokenUsage::new(
                a.prompt_tokens.saturating_add(b.prompt_tokens),
                a.completion_tokens.saturating_add(b.completion_tokens),
                a.total_tokens.saturating_add(b.total_tokens),
            );
            sum.cached_read_tokens =
                accumulate_optional(a.cached_read_tokens, b.cached_read_tokens);
            sum.cached_write_tokens =
                accumulate_optional(a.cached_write_tokens, b.cached_write_tokens);
            sum.reasoning_tokens = accumulate_optional(a.reasoning_tokens, b.reasoning_tokens);
            Some(sum)
        }
    }
}

#[cfg(test)]
mod tests {
    //! The tool loop's results, and the headless turn's routing as the loop acts on it.
    //!
    //! `is_lost_turn` and `reroutes_to_tail` are pinned on their own in
    //! `tests/`; these pin that [`HeadlessRoute`] actually sends what they
    //! flag to the chain's tail. Without the lost-turn arm a head that
    //! answers nothing is served as-is, and the athlete gets the empty-turn
    //! apology while a configured fallback tier sits unused (carnet#108).

    use pierre_core::errors::{AppError, ErrorCode};
    use pierre_llm::TokenUsage;

    use super::{HeadlessRoute, ToolLoopResult, ToolLoopTally};

    fn tally(tools: &[&str], activity_list: Option<&str>, warning: &str) -> ToolLoopTally {
        ToolLoopTally {
            activity_list: activity_list.map(str::to_owned),
            tool_calls_count: u32::try_from(tools.len()).unwrap_or(u32::MAX),
            tools_called: tools.iter().map(|t| (*t).to_owned()).collect(),
            served_without_provider: None,
            provider_warnings: vec![warning.to_owned()],
        }
    }

    const PROVIDER: &str = "copilot_sdk";

    /// A turn the head answered after one tool call.
    fn answered() -> ToolLoopResult {
        ToolLoopResult {
            content: "Tu as couru 42 km ce mois-ci.".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            activity_list: None,
            tool_calls_count: 1,
            tools_called: vec!["get_activities".to_owned()],
            pending_provider_auth_required: None,
            served_without_provider: None,
            guardian_denied: None,
            guardian_confirm: None,
            capability_claim_unverified: false,
            provider_warnings: Vec::new(),
        }
    }

    /// The live shape: the head ran its tool and returned no prose, with
    /// `finish_reason` still "stop".
    fn lost() -> ToolLoopResult {
        ToolLoopResult {
            content: String::new(),
            ..answered()
        }
    }

    #[test]
    fn a_lost_turn_is_rerouted_to_the_tail() {
        match HeadlessRoute::of(Ok(lost()), PROVIDER) {
            HeadlessRoute::Reroute(reason) => {
                let logged = reason.to_string();
                assert!(
                    logged.contains(PROVIDER) && logged.contains("1 tool call"),
                    "the fallback's reason must name the head and its tool calls: {logged}"
                );
            }
            HeadlessRoute::Serve(_) => panic!(
                "a lost turn was served as-is: the athlete gets the empty-turn apology \
                 instead of the tail's answer"
            ),
            HeadlessRoute::Fail(err) => {
                panic!("a lost turn failed the turn instead of moving to the tail: {err}")
            }
        }
    }

    #[test]
    fn an_answered_turn_is_served_as_is() {
        match HeadlessRoute::of(Ok(answered()), PROVIDER) {
            HeadlessRoute::Serve(result) => assert_eq!(result.content, answered().content),
            HeadlessRoute::Reroute(reason) => panic!(
                "an answered turn was re-run on the tail, paying for an answer already in \
                 hand: {reason}"
            ),
            HeadlessRoute::Fail(err) => panic!("an answered turn failed: {err}"),
        }
    }

    #[test]
    fn a_provider_fault_moves_to_the_tail_and_a_rejected_request_does_not() {
        let timeout = AppError::new(
            ErrorCode::ResourceUnavailable,
            "copilot-acp: prompt timed out after 150s",
        );
        assert!(
            matches!(
                HeadlessRoute::of(Err(timeout), PROVIDER),
                HeadlessRoute::Reroute(err) if err.code == ErrorCode::ResourceUnavailable
            ),
            "a provider fault must re-run the turn on the tail, carrying its own error"
        );

        let rejected = AppError::invalid_input("the request names no model");
        assert!(
            matches!(
                HeadlessRoute::of(Err(rejected), PROVIDER),
                HeadlessRoute::Fail(err) if err.code == ErrorCode::InvalidInput
            ),
            "a rejected request fails as-is: the tail would reject it the same way"
        );
    }

    #[test]
    fn a_tail_result_keeps_what_the_head_already_did() {
        let mut head_usage = TokenUsage::new(100, 10, 110);
        head_usage.cached_read_tokens = Some(40);
        let prior = tally(&["get_activities"], Some("list"), "temperature ignored")
            .interrupted(Some(head_usage));
        let tail = tally(&["get_connection_status"], None, "temperature ignored").answered(
            "answer".to_owned(),
            Some(TokenUsage::new(50, 5, 55)),
            Some("stop".to_owned()),
        );

        let merged = tail.after_prior(prior);

        assert_eq!(merged.content, "answer");
        assert_eq!(merged.finish_reason.as_deref(), Some("stop"));
        assert_eq!(
            merged.tools_called,
            ["get_activities", "get_connection_status"],
            "the head's tools first"
        );
        assert_eq!(merged.tool_calls_count, 2);
        assert_eq!(
            merged.activity_list.as_deref(),
            Some("list"),
            "the tail answered from the list the head fetched"
        );
        assert_eq!(merged.provider_warnings, ["temperature ignored"]);
        let usage = merged.usage.expect("both legs spent tokens");
        assert_eq!(usage.prompt_tokens, 150);
        assert_eq!(usage.total_tokens, 165);
        assert_eq!(usage.cached_read_tokens, Some(40));
    }

    #[test]
    fn an_exhausted_budget_is_not_a_lost_turn_to_reroute() {
        let exhausted = ToolLoopTally::default().max_iterations(None);
        assert!(exhausted.is_lost_turn());
        assert!(exhausted.ran_out_of_iterations());
        let empty = ToolLoopTally::default().answered(String::new(), None, None);
        assert!(empty.is_lost_turn());
        assert!(!empty.ran_out_of_iterations());
    }

    #[test]
    fn an_interrupted_head_carries_no_answer() {
        let interrupted: ToolLoopResult = tally(&["get_activities"], None, "w").interrupted(None);
        assert!(interrupted.content.is_empty());
        assert!(interrupted.finish_reason.is_none());
        assert_eq!(interrupted.tools_called, ["get_activities"]);
    }
}
