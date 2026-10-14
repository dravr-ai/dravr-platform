// ABOUTME: One API tool-loop iteration's call to the chain head, on the iteration model or the turn's
// ABOUTME: Picks the model, bills and logs each call, and re-asks the turn's model when a cheap iteration answers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Per-iteration model routing
//!
//! A turn on a head with native function calling is a loop of calls, and most
//! of them only pick the next tool: the athlete reads none of their text. On a
//! head that bills per call (the Anthropic API), those iterations can run on a
//! cheaper model than the reply, which is what
//! [`ChatProvider::tool_iteration_model`] names
//! (`PIERRE_LLM_MODEL_TOOL_ITERATION`, defaulting to Haiku on `anthropic`).
//!
//! Whether an iteration is the last one is only known once it has answered,
//! so each iteration asks the iteration model first. A response that calls a
//! tool — a structured call, or a `<tool_call>` block in its text — is the
//! iteration's, and the loop runs the tool as it always has. A response that
//! answers instead is billed and thrown away, and the turn's own model is asked
//! again on the same messages and tools: the reply the athlete reads is always
//! written by the turn's model, which may still call a tool itself. The
//! discarded text never enters the messages or the persisted tool rounds.
//!
//! A turn whose own model is the iteration model, a provider with no iteration
//! model (a Copilot or CLI head, a tenant's own key, a chain's tail) and a
//! custom provider route nothing: every call runs on the turn's model.
//!
//! [`ChatProvider::tool_iteration_model`]: pierre_llm::ChatProvider::tool_iteration_model

use std::mem;
use std::time::Instant;

use pierre_core::tokens::join_prompt_text;
use pierre_llm::call_record::{emit_call_record_with_text, CallRecordInputs};
use pierre_llm::served_tier::observe_served_tier;
use pierre_llm::{ChatMessage, ChatRequest, ChatResponseWithTools, TokenUsage};
use tracing::info;

use crate::tool_execution::parse_lenient_tool_call_blocks;
use crate::tool_loop_fallback::ApiLoopError;
use crate::tool_loop_io::{ToolLoopParams, ToolLoopTally};
use crate::tool_loop_telemetry::{
    accumulate_optional, emit_call_record, log_iteration_response, log_iteration_start,
    log_wire_shape, millis_elapsed, served_provider_name,
};

/// What the API loop's calls have spent so far in this turn.
#[derive(Debug)]
pub struct LoopSpend {
    /// Every call's tokens, discarded answers included: they were billed.
    pub usage: TokenUsage,
    /// Calls made so far, numbering each one's `llm_usage` row.
    calls: i64,
}

impl Default for LoopSpend {
    fn default() -> Self {
        Self {
            usage: TokenUsage::new(0, 0, 0),
            calls: 0,
        }
    }
}

impl LoopSpend {
    /// The tokens spent, or `None` before any call reported a figure.
    pub fn reported(&self) -> Option<TokenUsage> {
        (self.usage.total_tokens > 0).then(|| self.usage.clone())
    }

    /// Add one call's usage. The optional counts accumulate too: a loop that
    /// reads 40K tokens from cache across five iterations and reports none of
    /// it bills as if the prefix were fresh every time.
    fn add(&mut self, usage: &TokenUsage) {
        self.usage.prompt_tokens += usage.prompt_tokens;
        self.usage.completion_tokens += usage.completion_tokens;
        self.usage.total_tokens += usage.total_tokens;
        self.usage.cached_read_tokens =
            accumulate_optional(self.usage.cached_read_tokens, usage.cached_read_tokens);
        self.usage.cached_write_tokens =
            accumulate_optional(self.usage.cached_write_tokens, usage.cached_write_tokens);
        self.usage.reasoning_tokens =
            accumulate_optional(self.usage.reasoning_tokens, usage.reasoning_tokens);
    }
}

/// The response iteration `iteration` acts on: the iteration model's when it
/// calls a tool, the turn's own model's otherwise (see the module docs).
///
/// # Errors
///
/// [`ApiLoopError::Head`] when either call fails, carrying the rounds done
/// before it, so the turn re-runs on the chain's tail.
pub async fn iteration_response(
    params: &ToolLoopParams<'_>,
    llm_messages: &[ChatMessage],
    iteration: usize,
    spend: &mut LoopSpend,
    tally: &mut ToolLoopTally,
) -> Result<ChatResponseWithTools, ApiLoopError> {
    let Some(iteration_model) = params
        .provider
        .tool_iteration_model()
        .filter(|model| *model != params.model)
    else {
        return head_call(params, llm_messages, iteration, params.model, spend, tally).await;
    };
    let response = head_call(
        params,
        llm_messages,
        iteration,
        iteration_model,
        spend,
        tally,
    )
    .await?;
    if calls_a_tool(&response) {
        return Ok(response);
    }
    info!(
        iteration,
        iteration_model,
        model = params.model,
        "tool loop iteration answered on the iteration model; discarding it and asking the turn's model"
    );
    head_call(params, llm_messages, iteration, params.model, spend, tally).await
}

/// `response` asks for a tool: a structured call, or a `<tool_call>` block in
/// its text that the loop executes the same way.
fn calls_a_tool(response: &ChatResponseWithTools) -> bool {
    response
        .function_calls
        .as_ref()
        .is_some_and(|calls| !calls.is_empty())
        || response
            .content
            .as_deref()
            .is_some_and(|text| !parse_lenient_tool_call_blocks(text).is_empty())
}

/// One call to the chain's head on `model`, billed and logged as its own
/// `llm_usage` row.
///
/// Goes to the head alone, outside the chain's own fallback: a head fault comes
/// back as [`ApiLoopError::Head`] for the loop's caller to re-run the turn on
/// the tail, instead of embacle handing this tool-carrying request to a tier
/// that cannot honour it.
async fn head_call(
    params: &ToolLoopParams<'_>,
    llm_messages: &[ChatMessage],
    iteration: usize,
    model: &str,
    spend: &mut LoopSpend,
    tally: &mut ToolLoopTally,
) -> Result<ChatResponseWithTools, ApiLoopError> {
    let request = {
        // Not only on the ACP path: a CLI runner that accepts them —
        // `copilot` takes `--additional-mcp-config` — executes those tools
        // itself, which is the only way it will touch them.
        let req = ChatRequest::new(llm_messages.to_vec())
            .with_model(model)
            .with_mcp_servers(params.mcp_servers.clone());
        match params.temperature {
            Some(t) => req.with_temperature(t),
            None => req,
        }
    };

    spend.calls += 1;
    let call_seq = Some(spend.calls);
    log_iteration_start(iteration, spend.calls, params, model, llm_messages.len());
    log_wire_shape("api_tool_loop", llm_messages);

    // notify: LLM call about to start. Per call so tool-loop breadth is
    // observable from Slack (paired with the completion event right after the
    // await returns).
    info!(
        target: "notify",
        event = "embacle.call_started",
        model = %model,
        "LLM call dispatching"
    );

    let call_start = Instant::now();
    let (response_result, served) = observe_served_tier(
        params
            .provider
            .complete_with_tools_on_head(&request, Some(vec![params.tools.clone()])),
    )
    .await;
    let latency_ms = millis_elapsed(call_start);
    // The call went to the head alone, so no chain observer reports a tier and
    // the call is attributed to the head itself. Kept on the served tier for a
    // provider that reports one: those tokens are that provider's bill, and the
    // log line keeps the configured tier beside it.
    let served_provider = served_provider_name(served, params.provider.name());
    let call = CallFacts {
        params,
        model,
        call_seq,
        latency_ms,
    };
    match response_result {
        Ok(response) => {
            call.record_success(served_provider, &response);
            tally.note_provider_warnings(response.warnings.as_deref());
            log_iteration_response(iteration, latency_ms, &response);
            if let Some(usage) = response.usage.as_ref() {
                spend.add(usage);
            }
            Ok(response)
        }
        Err(error) => {
            call.record_failure(llm_messages);
            Err(ApiLoopError::Head {
                error,
                prior: Box::new(mem::take(tally).interrupted(spend.reported())),
            })
        }
    }
}

/// What one head call was, for its `llm_usage` row and its notify event.
struct CallFacts<'a> {
    params: &'a ToolLoopParams<'a>,
    model: &'a str,
    call_seq: Option<i64>,
    latency_ms: i64,
}

impl CallFacts<'_> {
    /// Bill and announce a call `served_provider` answered with `response`.
    fn record_success(&self, served_provider: &'static str, response: &ChatResponseWithTools) {
        let tools_in_response = response
            .function_calls
            .as_ref()
            .map(|fcs| fcs.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default();
        emit_call_record(CallRecordInputs {
            recorder: self.params.call_recorder.as_ref(),
            provider: served_provider,
            model: &response.model,
            usage: response.usage.as_ref(),
            latency_ms: self.latency_ms,
            success: true,
            call_sequence: self.call_seq,
            tools_called: tools_in_response,
        });
        // notify: LLM call succeeded — latency lands on the Slack ping so a
        // regression in tail latency surfaces in chat.
        info!(
            target: "notify",
            event = "embacle.call_completed",
            provider = served_provider,
            model = %response.model,
            configured_provider = self.params.provider.name(),
            configured_model = %self.model,
            latency_ms = self.latency_ms,
            ok = true,
            "LLM call completed"
        );
    }

    /// Bill and announce a call that failed after `llm_messages` were sent.
    fn record_failure(&self, llm_messages: &[ChatMessage]) {
        // The prompt reached the provider before it failed, so the abandoned
        // leg is charged for what it actually sent.
        let prompt_text = join_prompt_text(llm_messages.iter().map(|m| m.content.as_str()));
        emit_call_record_with_text(
            CallRecordInputs {
                recorder: self.params.call_recorder.as_ref(),
                provider: self.params.provider.name(),
                model: self.model,
                usage: None,
                latency_ms: self.latency_ms,
                success: false,
                call_sequence: self.call_seq,
                tools_called: Vec::new(),
            },
            Some(&prompt_text),
            None,
        );
        // notify: LLM call failed — ok=false so the routing rule can amplify
        // failures even when sample_rate hides successes.
        info!(
            target: "notify",
            event = "embacle.call_completed",
            model = %self.model,
            latency_ms = self.latency_ms,
            ok = false,
            "LLM call failed"
        );
    }
}
