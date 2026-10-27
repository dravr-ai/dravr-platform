// ABOUTME: One API tool-loop iteration's call to the chain head, on the iteration model or the turn's
// ABOUTME: Picks the model, bills each call, re-asks the turn's model on a cheap answer or write, then may stay there
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
//! so each iteration asks the iteration model first. A response whose every
//! call reads — a structured call, or a `<tool_call>` block in its text — is
//! the iteration's, and the loop runs the tools as it always has. A response
//! that answers instead is billed and thrown away, and the turn's own model is
//! asked again on the same messages and tools: the reply the athlete reads is
//! always written by the turn's model, which may still call a tool itself. The
//! discarded text never enters the messages or the persisted tool rounds.
//!
//! What a write persists is something the athlete reads too — a saved plan,
//! a prescribed workout — so the iteration model never performs one either.
//! A response calling any tool that declares `WRITES_DATA`, or one the
//! registry does not hold, is discarded whole the same way, its read calls
//! included: none of its calls runs, its `llm_usage` row lists none of them,
//! and the turn's model is re-asked (carnet#1193: at iteration 0 the
//! iteration model wrote a whole plan into `save_training_plan` through a
//! `<tool_call>` block, 11,930 completion tokens over 53 s, and the athlete's
//! draft was its work).
//!
//! The turn then escalates for good — every later iteration goes straight to
//! the turn's model — on either of two signals. The iteration model tried to
//! write: the turn has reached its authoring phase, where the next iteration
//! would have it author the same content again only for it to be thrown away
//! at the same cost, or call the write a second time beside the turn model's.
//! Or the turn's model, re-asked after an answer, called a tool: it overruled
//! the iteration model's "done", so the iteration model has shown it cannot
//! judge when this turn is finished, and each later iteration-model answer
//! would be paid for and thrown away (carnet#1193: one plan-building turn ran
//! both models on all seven iterations). A turn whose iteration model only
//! ever calls tools that read, or whose re-asked turn model answers, never
//! escalates. The state lives on [`LoopSpend`], one per turn.
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

use crate::head_stream::stream_head_call;
use crate::tool_execution::parse_lenient_tool_call_blocks;
use crate::tool_loop_fallback::ApiLoopError;
use crate::tool_loop_io::{ToolLoopParams, ToolLoopTally};
use crate::tool_loop_telemetry::{
    accumulate_optional, emit_call_record, log_iteration_response, log_iteration_start,
    log_wire_shape, millis_elapsed, served_provider_name,
};

/// Which model a turn's next iteration goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IterationRoute {
    /// Tool calls on the iteration model, the answer re-asked on the turn's.
    Split,
    /// The iteration model tried to write, or the turn's model overruled its
    /// answer by calling a tool: every later iteration of this turn goes to
    /// the turn's model.
    TurnModel,
}

/// What the API loop's calls have spent so far in this turn.
#[derive(Debug)]
pub(crate) struct LoopSpend {
    /// Every call's tokens, discarded answers included: they were billed.
    pub usage: TokenUsage,
    /// Calls made so far, numbering each one's `llm_usage` row.
    calls: i64,
    /// Where this turn's next iteration goes.
    route: IterationRoute,
}

impl Default for LoopSpend {
    fn default() -> Self {
        Self {
            usage: TokenUsage::new(0, 0, 0),
            calls: 0,
            route: IterationRoute::Split,
        }
    }
}

impl LoopSpend {
    /// The tokens spent, or `None` before any call reported a figure.
    pub(crate) fn reported(&self) -> Option<TokenUsage> {
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
/// calls only tools that read, the turn's own model's otherwise, and the
/// turn's own model's alone once the turn has escalated (see the module docs).
///
/// # Errors
///
/// [`ApiLoopError::Head`] when either call fails, carrying the rounds done
/// before it, so the turn re-runs on the chain's tail.
pub(crate) async fn iteration_response(
    params: &ToolLoopParams<'_>,
    llm_messages: &[ChatMessage],
    iteration: usize,
    spend: &mut LoopSpend,
    tally: &mut ToolLoopTally,
) -> Result<ChatResponseWithTools, ApiLoopError> {
    let Some(iteration_model) = params
        .provider
        .tool_iteration_model()
        .filter(|model| *model != params.model && spend.route == IterationRoute::Split)
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
    let requested = requested_tools(&response);
    if let Some(write) = first_write(params, &requested) {
        spend.route = IterationRoute::TurnModel;
        info!(
            iteration,
            iteration_model,
            model = params.model,
            tool = write,
            "the iteration model called a tool that writes; discarding it, and the rest of \
             this turn runs on the turn's model"
        );
        return head_call(params, llm_messages, iteration, params.model, spend, tally).await;
    }
    if !requested.is_empty() {
        return Ok(response);
    }
    re_ask_after_answer(
        params,
        llm_messages,
        iteration,
        iteration_model,
        spend,
        tally,
    )
    .await
}

/// The iteration model answered: discard that answer, ask the turn's model,
/// and escalate the turn when the turn's model calls a tool instead.
async fn re_ask_after_answer(
    params: &ToolLoopParams<'_>,
    llm_messages: &[ChatMessage],
    iteration: usize,
    iteration_model: &str,
    spend: &mut LoopSpend,
    tally: &mut ToolLoopTally,
) -> Result<ChatResponseWithTools, ApiLoopError> {
    info!(
        iteration,
        iteration_model,
        model = params.model,
        "tool loop iteration answered on the iteration model; discarding it and asking the turn's model"
    );
    let response = head_call(params, llm_messages, iteration, params.model, spend, tally).await?;
    if !requested_tools(&response).is_empty() {
        spend.route = IterationRoute::TurnModel;
        info!(
            iteration,
            iteration_model,
            model = params.model,
            "the turn's model overruled the iteration model by calling a tool; \
             the rest of this turn runs on the turn's model"
        );
    }
    Ok(response)
}

/// Every tool `response` asks for: its structured calls, then each
/// `<tool_call>` block in its text, which the loop executes the same way.
pub(crate) fn requested_tools(response: &ChatResponseWithTools) -> Vec<String> {
    let structured = response
        .function_calls
        .iter()
        .flatten()
        .map(|call| call.name.clone());
    let written = response
        .content
        .as_deref()
        .map(parse_lenient_tool_call_blocks)
        .unwrap_or_default()
        .into_iter()
        .map(|call| call.name);
    structured.chain(written).collect()
}

/// The first of `tools` that writes, by the `WRITES_DATA` capability each tool
/// declares — the flag the dispatch-time Guardian counts writes by and the
/// OAuth scopes derive from. A name the registry does not hold counts as a
/// write: nothing vouches that it only reads.
fn first_write<'tools>(
    params: &ToolLoopParams<'_>,
    tools: &'tools [String],
) -> Option<&'tools str> {
    use crate::guardian::ToolLabels;
    let registry = params.executor.resources.tool_registry();
    tools
        .iter()
        .map(String::as_str)
        .find(|tool| registry.writes_data(tool).unwrap_or(true))
}

/// One call to the chain's head on `model`, billed and logged as its own
/// `llm_usage` row.
///
/// Goes to the head alone, outside the chain's own fallback: a head fault comes
/// back as [`ApiLoopError::Head`] for the loop's caller to re-run the turn on
/// the tail, instead of embacle handing this tool-carrying request to a tier
/// that cannot honour it.
///
/// A call on the turn's own model streams its prose to the turn's sink when
/// the loop streams text ([`stream_head_call`]); its closing response is the
/// one the non-streaming call returns, billed and logged the same way. A call on
/// the iteration model never streams: its answer may be discarded, and the
/// athlete would have watched words that are never the reply.
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
    let tools = Some(vec![params.tools.clone()]);
    // Streamed only where the loop streams text: a sink alone means the surface
    // reads frames, which tool progress uses on every provider. The stream is
    // drained inside the observation, so the served tier and the latency both
    // cover the whole response, not the opening of the stream.
    let (response_result, served) = match params
        .stream_sink
        .as_ref()
        .filter(|_| params.streams_text() && model == params.model)
    {
        Some(sink) => {
            observe_served_tier(stream_head_call(params.provider, &request, tools, sink)).await
        }
        None => {
            observe_served_tier(params.provider.complete_with_tools_on_head(&request, tools)).await
        }
    };
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
        // The row's `tools_called` is what the admin tool-usage view counts
        // as run, and an iteration-model write is discarded unrun.
        let discarded_write = self.model != self.params.model
            && first_write(self.params, &requested_tools(response)).is_some();
        let tools_in_response = if discarded_write {
            Vec::new()
        } else {
            response
                .function_calls
                .as_ref()
                .map(|fcs| fcs.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default()
        };
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
