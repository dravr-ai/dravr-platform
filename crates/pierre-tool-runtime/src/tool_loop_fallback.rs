// ABOUTME: The runtime fallback chain, re-created for the tool loops that converse with the chain's head alone
// ABOUTME: Skips a head inside a quota cooldown, starts one on a quota refusal, re-runs the turn on the tail
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What embacle's fallback chain does for a call, done here for a tool turn.
//!
//! The API and headless loops converse with the chain's head directly, never
//! through embacle's `FallbackProvider` (a tool-carrying request handed to a
//! tier that cannot honour it answers with no tools at all). So nothing the
//! chain does for a call happens for them unless it happens here:
//!
//! - **Quota cooldown.** A head that refused on quota is passed over for the
//!   window of the chain's shared cooldown book
//!   ([`pierre_llm::EmbacleProvider::chain`]). A turn that finds its head
//!   cooling goes straight to the tail without asking it; a head that answers
//!   with a quota refusal starts its window there. The book is the one the
//!   full chain and every tail chain read too, so all of them agree.
//! - **Fall-through.** A provider fault or a lost turn re-runs the whole turn
//!   on the chain's tail, routed again by the tail head's own capabilities.

use std::sync::Arc;

use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::llm::is_llm_provider_fault;
use pierre_llm::{ChatMessage, ChatProvider};

use crate::tool_execution::{run_api_tool_loop, run_headless_tool_loop, run_tool_loop};
use crate::tool_loop_io::{HeadlessRoute, ToolLoopParams, ToolLoopResult};
use crate::tool_loop_telemetry::{note_tool_loop_cooldown_skip, note_tool_loop_fallback};

/// The API loop's label on a fallback line.
const API_LOOP: &str = "api";

/// The headless loop's label on a fallback line.
const HEADLESS_LOOP: &str = "headless";

/// Why [`run_api_tool_loop`] stopped without a result.
pub enum ApiLoopError {
    /// The chain head's LLM call failed. `prior` is what the rounds before it
    /// had done — the tools that ran, the tokens spent — for the tail's re-run
    /// to carry on from.
    Head {
        /// The head's error.
        error: AppError,
        /// The completed rounds, with no answer of their own. Boxed: the
        /// error travels the loop's every `?`, the record only this one exit.
        prior: Box<ToolLoopResult>,
    },
    /// Anything else: a tool dispatch the executor could not run. Another
    /// tier would hit the same failure, so it never reroutes.
    Other(AppError),
}

impl From<AppError> for ApiLoopError {
    fn from(error: AppError) -> Self {
        Self::Other(error)
    }
}

impl ApiLoopError {
    /// The error, whichever side of the loop raised it.
    fn into_error(self) -> AppError {
        match self {
            Self::Head { error, .. } | Self::Other(error) => error,
        }
    }
}

/// One API-loop turn on the chain's head, re-run on the tail when the head
/// cannot finish it.
///
/// The loop converses with the head alone (see [`run_api_tool_loop`]), so the
/// chain's own fallbacks never fire for it; both are re-created here as they
/// are for the headless loop. A head inside a quota cooldown is not asked: the
/// turn goes to the tail. A provider fault ([`is_llm_provider_fault`], which covers
/// a spent quota or credit) re-runs the turn on the tail from wherever the head
/// stopped, with the tool results already gathered in `llm_messages`, and a
/// quota refusal starts the head's cooldown first. A lost turn — nothing to
/// show, the head answered empty — does the same. A loop that ran out of
/// iterations is not lost: the model kept calling tools, and a second tier
/// would spend a second budget on the same loop.
///
/// With no tail — a single provider, as an unset chain is — the head's result
/// and error come back unchanged, as they always did.
pub async fn run_api_turn(
    params: &ToolLoopParams<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> Result<ToolLoopResult, AppError> {
    if let Some(tail) = tail_past_cooling_head(params.provider) {
        note_tool_loop_cooldown_skip(params.provider, tail, API_LOOP);
        return run_on_tail(params, tail, llm_messages, None).await;
    }
    let outcome = run_api_tool_loop(params, llm_messages).await;
    if let Err(ApiLoopError::Head { error, .. }) = &outcome {
        start_cooldown_on_quota_refusal(params.provider, error);
    }
    if params.provider.fallback_tail().is_none() {
        return outcome.map_err(ApiLoopError::into_error);
    }
    match outcome {
        Ok(r) if r.is_lost_turn() && !r.ran_out_of_iterations() => {
            let error = r.lost_turn_error(params.provider.name());
            run_tail_fallback(params, llm_messages, error, Some(r), API_LOOP).await
        }
        Ok(result) => Ok(result),
        Err(ApiLoopError::Head { error, prior }) if is_llm_provider_fault(&error) => {
            run_tail_fallback(params, llm_messages, error, Some(*prior), API_LOOP).await
        }
        Err(other) => Err(other.into_error()),
    }
}

/// One headless-loop turn on the chain's head, re-run on the tail when the
/// head cannot finish it.
///
/// Converses with the chain's head directly, outside embacle's fallback walk,
/// so none of its fallbacks fires here — not the quota cooldown, not the
/// provider-fault one, not the empty-completion one. All three are re-created:
/// a cooling head is not asked, a quota refusal starts its cooldown, and a
/// fault or a lost turn ([`HeadlessRoute`]) re-runs the turn on the tail.
pub async fn run_headless_turn(
    params: &ToolLoopParams<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> Result<ToolLoopResult, AppError> {
    if let Some(tail) = tail_past_cooling_head(params.provider) {
        note_tool_loop_cooldown_skip(params.provider, tail, HEADLESS_LOOP);
        return run_on_tail(params, tail, llm_messages, None).await;
    }
    let outcome = run_headless_tool_loop(params, llm_messages).await;
    if let Err(err) = &outcome {
        start_cooldown_on_quota_refusal(params.provider, err);
    }
    match HeadlessRoute::of(outcome, params.provider.name()) {
        HeadlessRoute::Serve(result) => Ok(*result),
        HeadlessRoute::Reroute(reason) => {
            run_tail_fallback(params, llm_messages, reason, None, HEADLESS_LOOP).await
        }
        HeadlessRoute::Fail(err) => Err(err),
    }
}

/// The chain's tail, when the chain's head is inside a quota cooldown and a
/// tail exists to take the turn instead.
///
/// `None` for a provider with no tail: the last tier is always asked, as
/// embacle's chain always asks its last tier.
fn tail_past_cooling_head(provider: &ChatProvider) -> Option<&ChatProvider> {
    let tail = provider.fallback_tail()?;
    let book = provider.quota_cooldowns()?;
    book.is_cooling(provider.name()).then_some(tail)
}

/// Start the head's quota cooldown when `error` is its quota refusal.
///
/// `ExternalRateLimited` is the one code embacle's `RateLimit` — a spent
/// subscription window, a 429, an HTTP 402 credit exhaustion — reaches the
/// platform as, and the athlete's own budget never does (the ingress gate
/// refuses that before dispatch). The window is the book's own, as it is for
/// every chain over it. A provider not built as a chain has no book.
fn start_cooldown_on_quota_refusal(provider: &ChatProvider, error: &AppError) {
    if error.code != ErrorCode::ExternalRateLimited {
        return;
    }
    if let Some(book) = provider.quota_cooldowns() {
        book.start(provider.name());
    }
}

/// Re-run a turn the chain's head could not finish against the chain's tail.
///
/// When the head fails with a provider fault or loses the turn, this re-runs
/// the whole tool loop against the chain's tail (see [`run_on_tail`]) and
/// records the hop. With no tail configured the original error is returned
/// unchanged.
async fn run_tail_fallback(
    params: &ToolLoopParams<'_>,
    llm_messages: &mut Vec<ChatMessage>,
    head_err: AppError,
    prior: Option<ToolLoopResult>,
    tool_loop: &'static str,
) -> Result<ToolLoopResult, AppError> {
    let Some(tail) = params.provider.fallback_tail() else {
        return Err(head_err);
    };
    note_tool_loop_fallback(params.provider, tail, &head_err, tool_loop);
    run_on_tail(params, tail, llm_messages, prior).await
}

/// Run the turn's tool loop on `tail`, routed by the tail's own capabilities
/// (the API loop for another Anthropic account or Gemini, the headless loop
/// with its MCP bridge for Copilot, the text loop for Claude Code), so the
/// turn still produces a grounded answer. `llm_messages` carries whatever the
/// head's rounds already added, and `prior` what they did (the API loop's
/// completed rounds), folded into the tail's result so the tools that ran
/// stay recorded for the turn.
async fn run_on_tail(
    params: &ToolLoopParams<'_>,
    tail: &ChatProvider,
    llm_messages: &mut Vec<ChatMessage>,
    prior: Option<ToolLoopResult>,
) -> Result<ToolLoopResult, AppError> {
    let fallback_params = ToolLoopParams {
        provider: tail,
        executor: Arc::clone(&params.executor),
        tools: params.tools,
        // The tail picks its own model: the head's model (e.g. Copilot's
        // `claude-sonnet-5.5`) is meaningless to the Anthropic API or Gemini.
        // Mirrors `ResponsePolicy::strict().own_model_per_tier` clearing the
        // model before each hop of the embacle chain.
        model: tail.default_model(),
        user_id: params.user_id,
        tenant_id: params.tenant_id,
        max_iterations: params.max_iterations,
        call_recorder: params.call_recorder.clone(),
        tool_message_recorder: params.tool_message_recorder.clone(),
        temperature: params.temperature,
        stream_sink: params.stream_sink.clone(),
        mcp_servers: params.mcp_servers.clone(),
    };
    let result = Box::pin(run_tool_loop(&fallback_params, llm_messages)).await?;
    Ok(match prior {
        Some(prior) => result.after_prior(prior),
        None => result,
    })
}
