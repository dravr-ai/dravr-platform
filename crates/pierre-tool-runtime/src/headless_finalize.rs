// ABOUTME: How a Copilot headless turn ends — a text tool call handed to the text loop, a degenerate reply retried once
// ABOUTME: Records the calls the runtime ran as the turn's tool round, extracts its activity list, assembles the ToolLoopResult
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::time::Instant;

use pierre_core::errors::AppError;
use pierre_core::llm::tool_simulation;
use pierre_core::models::GUARDIAN_DENIED_FINISH_REASON;
use pierre_core::narration::is_degenerate_reply;
use pierre_llm::call_record::{emit_call_record_with_text, CallRecordInputs};
use pierre_llm::{
    ChatMessage, ChatRequest, ChatResponse, FunctionResponse, ObservedToolCall, TokenUsage,
};
use pierre_providers::ai_scope;
use pierre_services::chat_stream::{TurnEvent, TurnEventSink};
use tracing::{info, warn};

use crate::envelope_notices::merge_notice_days;
use crate::guardian::HeadlessBlock;
use crate::tool_execution::{headless_turn_key, parse_lenient_tool_call_blocks, run_cli_tool_loop};
use crate::tool_loop_io::{
    observed_tool_name, GuardianDenial, ToolLoopParams, ToolLoopResult, ToolRoundRecord,
};
use crate::tool_loop_telemetry::millis_elapsed;
use crate::tool_progress;
use crate::tool_results::extract_activity_list;

/// Strip the headless reply, retry once if the turn was degenerate, and
/// assemble the [`ToolLoopResult`].
///
/// Copilot ACP intermittently ends a turn without synthesizing an answer:
/// it returns empty content, parrots the injected tool-result turn
/// verbatim (the `format_tool_results_as_text` preamble + `<tool_result>`
/// blocks) instead of reasoning over it, or emits a dangling fragment («by
/// Dravr.», delivered to a live Telegram group on 2026-08-22). The
/// scaffolding is stripped unconditionally so a parroted echo never leaks to
/// the user; when what remains fails [`is_degenerate_reply`] the turn is
/// degenerate, so `retry_headless_turn` runs once — the failure is
/// intermittent (~5% per call) and the next turn recovers in practice.
///
/// A reply that called no tool natively but wrote `<tool_call>` blocks is not
/// degenerate: it asked for tools in the syntax the shared system prompt
/// teaches every tier, because the prompt is assembled once for the whole
/// fallback chain and the text-only tiers need it. Stripped, it read as empty,
/// the identical retry asked the same way, and the turn fell through to the
/// next tier on almost every data question (carnet#1032). The reply is handed
/// to [`run_cli_tool_loop`] instead, which runs its blocks through the executor
/// and asks again with their results, as the native path and the CLI tiers
/// already do.
///
/// Either way, drains what the ACP subprocess's tool-host session left under this
/// turn's key: the Guardian block, and the served-without-a-provider offer. Both
/// are the platform's own record of what happened inside the subprocess, which is
/// the only way this path learns of either — and the text loop's own asks carry
/// the same MCP servers, so a model that calls a tool natively there raises them
/// the same way.
///
/// The calls the runtime ran natively are recorded as the turn's tool round
/// through [`ToolLoopParams::tool_message_recorder`], as every other loop
/// records its rounds (ADR-016): without them a follow-up turn sees a grounded
/// answer with no evidence behind it. A degenerate first pass and its retry are
/// two rounds, each written from the calls it reported. The activity list a
/// `get_activities` call among them returned is extracted from the same
/// results, so a headless data answer carries the list every other loop's
/// does.
///
/// `streamed_to` is the sink the reply's prose reached, `None` when none did
/// (a turn that did not stream, or one that wrote only `<tool_call>` blocks):
/// a reply that will not be the turn's answer is taken back there.
///
/// `pub` so the integration suite can drive the production assembly — a stamped
/// store and an ACP reply in, the `ToolLoopResult` the chat pipeline reads out —
/// rather than restate it against a hand-built result and pin nothing.
///
/// # Errors
///
/// Returns an error when a degenerate first turn's single retry `converse()`
/// call fails.
pub async fn finalize_headless_turn(
    headless_response: pierre_llm::HeadlessToolResponse,
    streamed_to: Option<&TurnEventSink>,
    headless_runner: &dyn pierre_llm::HeadlessTurnProvider,
    request: &ChatRequest,
    params: &ToolLoopParams<'_>,
    prompt_text: &str,
    llm_messages: &mut Vec<ChatMessage>,
) -> Result<ToolLoopResult, AppError> {
    if headless_response.tool_calls.is_empty() {
        let text_tool_calls = parse_lenient_tool_call_blocks(&headless_response.content);
        if !text_tool_calls.is_empty() {
            info!(
                provider = %params.provider.name(),
                model = %headless_response.model,
                blocks = text_tool_calls.len(),
                "Headless reply asked for tools as text <tool_call> blocks; running the turn \
                 through the text tool loop"
            );
            take_back_streamed_reply(streamed_to);
            let first_reply = ChatResponse {
                replay_blocks: None,
                content: headless_response.content,
                model: headless_response.model,
                usage: headless_response.usage,
                finish_reason: headless_response.finish_reason,
                warnings: None,
                tool_calls: None,
            };
            // The turn's own messages, not a copy: the rounds the text loop
            // appends — the reply's calls and their results — are the
            // evidence the pipeline's claim checks read after the loop. On a
            // copy they were lost, the peer-claim check judged a start time
            // the agent read from a tool result as unsupported, and its re-ask
            // had the agent retract it (carnet#1038).
            let mut result = run_cli_tool_loop(params, llm_messages, Some(first_reply)).await?;
            // The text loop extracted the activity list from the
            // `get_activities` result its blocks ran, and it stands: a
            // headless turn shows the list the athlete's data answer refers
            // to, as every other loop does (the fold keeps it after the
            // introduction a first reply opens with, carnet#1039).
            settle_turn_signals(params, &mut result);
            return Ok(result);
        }
    }

    record_observed_round(params, &headless_response.tool_calls);
    let mut activity_list = observed_activity_list(&headless_response.tool_calls);
    let mut tool_calls_count =
        u32::try_from(headless_response.tool_calls.len()).unwrap_or(u32::MAX);
    let mut tools_called: Vec<String> = headless_response
        .tool_calls
        .iter()
        .map(observed_tool_name)
        .collect();
    let mut content = tool_simulation::strip_simulation_artifacts(&headless_response.content);
    let mut usage = headless_response.usage;
    let mut finish_reason = headless_response.finish_reason;

    if is_degenerate_reply(&content) {
        warn!(
            provider = %params.provider.name(),
            model = %params.model,
            content_len = content.len(),
            "Headless turn produced no synthesized answer (empty, parroted tool-result echo, or a dangling fragment); retrying converse once"
        );
        take_back_streamed_reply(streamed_to);
        let retry = retry_headless_turn(headless_runner, request, params, prompt_text).await?;
        content = retry.content;
        usage = retry.usage;
        finish_reason = retry.finish_reason;
        record_observed_round(params, &retry.tool_calls);
        // The retry's list is the newer read; a retry that read none keeps
        // the first pass's, as a later round without one does in every loop.
        activity_list = observed_activity_list(&retry.tool_calls).or(activity_list);
        tool_calls_count = tool_calls_count
            .saturating_add(u32::try_from(retry.tool_calls.len()).unwrap_or(u32::MAX));
        tools_called.extend(retry.tool_calls.iter().map(observed_tool_name));
    }

    let mut result = ToolLoopResult {
        content,
        usage,
        finish_reason,
        // Extracted from the results the runtime reported, the same ones
        // recorded as the turn's rounds above.
        activity_list,
        tool_calls_count,
        tools_called,
        // Set by `settle_turn_signals` below, from the chokepoint's record.
        write_attempted: false,
        // The hard auth trip stays None on this path: a loopback dispatch that
        // raises ProviderAuthRequired leaves the chokepoint as a JSON-RPC error
        // addressed to the ACP subprocess, which hands it to the model, so no
        // headless turn arrives here blanked. Which keeps the standing order
        // intact where it matters — the soft offer below is only ever read when
        // nothing harder is set.
        pending_provider_auth_required: None,
        // The soft offer is a SUCCESSFUL payload instead, so the chokepoint
        // reads it on the way past and stamps it under this turn's key; taken
        // from the shared store by `settle_turn_signals` below.
        served_without_provider: None,
        // Stamped by the chokepoint under this turn's key like the offer
        // above; taken by `settle_turn_signals` below.
        window_incomplete: false,
        envelope_days: Vec::new(),
        // #10: a block that fired at the platform chokepoint during the ACP
        // subprocess's tool calls is recovered from the shared Guardian store by
        // `settle_turn_signals` below, so the headless path surfaces the same
        // deterministic refusal every other transport does.
        guardian_denied: None,
        // Set downstream by the capability-recovery stage, which runs after
        // every loop variant returns.
        capability_claim_unverified: false,
        // The ACP session reports no parameter warnings: the subprocess owns
        // the request, so there is no platform-built request to check.
        provider_warnings: Vec::new(),
    };
    // After any degenerate-turn retry, so a retried turn keeps what its first
    // pass raised.
    settle_turn_signals(params, &mut result);
    Ok(result)
}

/// Fold what the chokepoint recorded under this turn into `result`, once.
///
/// Two signals raised on the tool host's task, which the turn reads only here:
///
/// - The backend key of a provider a `get_activities` served the window WITHOUT.
///   `auth_recovery` mints a reconnect URL from it and appends the offer to the
///   agent's answer, so an ACP turn a sibling connection served hands the athlete
///   the same control every other transport does. An offer the loop already read
///   off a payload stands.
/// - #10: the turn's first Guardian block, which becomes a deterministic reply
///   (the chat pipeline renders the localized reply) instead of the model's
///   paraphrase. It replaces whatever block `result` carries, since a later refusal
///   in the same turn is only a consequence of it.
///   It stamps the finish reason too: the reply is about to become the platform's
///   text, and the row is replayed by that stamp, so a model's `stop` left on it
///   would read back as the agent's own words (carnet#1398).
/// - Whether a `get_activities` window the turn read was incomplete, so
///   capability recovery re-asks a reply that counts over it (carnet#1481).
/// - Whether a consequential call reached the gate under this turn's key, which
///   a leg re-run on the chain's tail would otherwise lose along with its list
///   of tools.
///
/// Called when the headless path assembles its result, so `is_lost_turn` reads
/// the block, and again where every loop returns: a head that faulted after its
/// block hands the turn to a tail that knows nothing of it. A turn with no token
/// recorded nothing.
pub(crate) fn settle_turn_signals(params: &ToolLoopParams<'_>, result: &mut ToolLoopResult) {
    let Some(turn_key) = headless_turn_key(params) else {
        return;
    };
    let resources = &params.executor.resources;
    if let Some(offer) = resources.reconnect_offers().take_offer(&turn_key) {
        result.served_without_provider = result.served_without_provider.take().or(Some(offer));
    }
    result.window_incomplete |= resources.incomplete_windows().take(&turn_key);
    merge_notice_days(
        &mut result.envelope_days,
        resources.envelope_notices().take(&turn_key),
    );
    let turns = resources.guardian_turns();
    result.write_attempted |= turns.write_attempted(&turn_key);
    if let Some(HeadlessBlock { tool_name, reason }) = turns.take_block(&turn_key) {
        result.guardian_denied = Some(GuardianDenial {
            tool_name,
            reason: reason.as_str().to_owned(),
            committed: turns.committed(&turn_key),
            unconfirmed: turns.unconfirmed(&turn_key),
        });
        result.finish_reason = Some(GUARDIAN_DENIED_FINISH_REASON.to_owned());
    }
}

/// Record the calls one headless converse ran as a tool round (ADR-016).
///
/// The runtime ran every call itself, so the round is what it reported: each
/// call that carries the text handed back to the model becomes a
/// `[Tool Result for <tool>]: <text>` block, the shape the API and text loops
/// persist, so the next turn replays the same evidence whichever tier served
/// this one. The text is stored as the model received it: the loopback seam
/// already projected it the way every other path projects a tool result.
///
/// A call the transport reported without its result (the ACP adapter carries
/// only a title and a status) has no evidence to persist and is left out; a
/// converse with none is no round. No assistant preamble is recorded, because
/// the runtime reports none per call: the round's text is the final answer,
/// which the pipeline stores as the reply.
///
/// The round is about every group member the turn has read so far. The calls
/// ran on the loopback task, where no per-round scope collects what they read,
/// but they marked the turn's own provenance; the turn's set holds this
/// round's, so a member who withdraws still takes the round's rows with them,
/// and a turn that read several keeps no round (carnet#1174).
fn record_observed_round(params: &ToolLoopParams<'_>, calls: &[ObservedToolCall]) {
    let Some(recorder) = params.tool_message_recorder.as_ref() else {
        return;
    };
    let tool_result_text = observed_tool_results(calls);
    if tool_result_text.is_empty() {
        return;
    }
    recorder.record(ToolRoundRecord {
        assistant_text: String::new(),
        tool_result_text,
        members_read: ai_scope::current_provenance()
            .map_or_default(|provenance| provenance.carried_reads()),
    });
}

/// The `[Tool Result for <tool>]: <text>` blocks of every call in `calls`
/// that reported its result, joined as the other loops join a round's.
fn observed_tool_results(calls: &[ObservedToolCall]) -> String {
    calls
        .iter()
        .filter_map(|call| {
            call.result
                .as_deref()
                .map(|text| format!("[Tool Result for {}]: {text}", observed_tool_name(call)))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The activity list a `get_activities` call in `calls` returned, extracted as
/// every other loop extracts it ([`extract_activity_list`]).
///
/// The SDK transport hands back each call's result as the JSON text the
/// loopback seam serialized, so it parses back to the payload the other loops
/// read. A call reported without its result (the ACP adapter), or with text
/// that is not JSON (a refusal's reason), has no list to give.
fn observed_activity_list(calls: &[ObservedToolCall]) -> Option<String> {
    let responses: Vec<FunctionResponse> = calls
        .iter()
        .filter_map(|call| {
            let response = serde_json::from_str(call.result.as_deref()?).ok()?;
            Some(FunctionResponse {
                name: observed_tool_name(call),
                response,
            })
        })
        .collect();
    extract_activity_list(&responses)
}

/// Take back the first reply's streamed prose, which will not be the turn's
/// reply: the text loop's answer to its `<tool_call>` blocks, or the retry of
/// a degenerate turn, replaces it, and neither streams. Left on screen it
/// would read as the reply while the text loop's tool steps ran under it.
///
/// Sent only to the sink the first reply's prose actually reached, which the
/// forwarder knows once it has released everything it held: a reply that wrote
/// nothing but `<tool_call>` blocks, which never cross, put nothing on screen
/// and sends no retraction.
fn take_back_streamed_reply(streamed_to: Option<&TurnEventSink>) {
    if let Some(sink) = streamed_to {
        tool_progress::deliver(sink, TurnEvent::ProseRetract);
    }
}

/// Outcome of a single degenerate-turn retry: the stripped reply plus the
/// accounting [`finalize_headless_turn`] folds into its `ToolLoopResult`.
struct HeadlessRetry {
    content: String,
    usage: Option<TokenUsage>,
    finish_reason: Option<String>,
    /// The calls the retry ran, as the runtime reported them.
    tool_calls: Vec<ObservedToolCall>,
}

/// Retry a degenerate headless turn once via non-streaming `converse()`.
///
/// Called only when the first turn produced no synthesized answer (empty or
/// a parroted tool-result echo). Records the retry as its own per-call usage
/// row and returns the stripped reply so the caller can decide whether the
/// model recovered. Non-streaming on purpose: a re-parroted echo must never
/// reach the user's stream a second time.
async fn retry_headless_turn(
    headless_runner: &dyn pierre_llm::HeadlessTurnProvider,
    request: &ChatRequest,
    params: &ToolLoopParams<'_>,
    prompt_text: &str,
) -> Result<HeadlessRetry, AppError> {
    let retry_start = Instant::now();
    let retry = headless_runner
        .converse(request)
        .await
        .map_err(AppError::from)?;
    let retry_latency_ms = millis_elapsed(retry_start);
    emit_call_record_with_text(
        CallRecordInputs {
            recorder: params.call_recorder.as_ref(),
            provider: params.provider.name(),
            model: &retry.model,
            configured_provider: params.provider.name(),
            configured_model: params.model,
            usage: retry.usage.as_ref(),
            latency_ms: retry_latency_ms,
            success: true,
            call_sequence: Some(2),
            tools_called: retry.tool_calls.iter().map(observed_tool_name).collect(),
        },
        Some(prompt_text),
        Some(&retry.content),
    );
    Ok(HeadlessRetry {
        content: tool_simulation::strip_simulation_artifacts(&retry.content),
        usage: retry.usage,
        finish_reason: retry.finish_reason,
        tool_calls: retry.tool_calls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: Option<&str>, title: &str, result: Option<&str>) -> ObservedToolCall {
        ObservedToolCall {
            id: format!("call-{title}"),
            title: title.to_owned(),
            status: "Completed".to_owned(),
            name: name.map(ToOwned::to_owned),
            arguments: None,
            result: result.map(ToOwned::to_owned),
        }
    }

    /// Each call that reported its result is one block under the platform's
    /// bare tool name, in the order the runtime ran them; a call reported
    /// without its result is left out rather than given an invented one.
    #[test]
    fn a_round_holds_the_results_the_runtime_reported() {
        let calls = [
            call(Some("dravr-get_stats"), "Get stats", Some("{\"rides\":3}")),
            call(None, "get_activities", None),
            call(Some("get_athlete"), "Get athlete", Some("Ana")),
        ];

        assert_eq!(
            observed_tool_results(&calls),
            "[Tool Result for get_stats]: {\"rides\":3}\n\n[Tool Result for get_athlete]: Ana",
            "every reported result, and only those, must reach the round in order"
        );
    }

    /// The list comes from the `get_activities` result the runtime reported,
    /// parsed back from the JSON text the loopback serialized; another tool's
    /// result, a result-less call and a non-JSON refusal give none.
    #[test]
    fn the_activity_list_comes_from_a_reported_get_activities_result() {
        let listed = call(
            Some("dravr-get_activities"),
            "Get activities",
            Some("{\"activity_list\":\"Mon 31 km\",\"count\":1}"),
        );
        assert_eq!(
            observed_activity_list(&[call(Some("get_athlete"), "Get athlete", Some("{}")), listed]),
            Some("Mon 31 km".to_owned()),
            "a reported get_activities result yields its list"
        );
        assert_eq!(
            observed_activity_list(&[
                call(None, "get_activities", None),
                call(
                    Some("dravr-get_activities"),
                    "Get activities",
                    Some("declined")
                ),
                call(
                    Some("dravr-get_stats"),
                    "Get stats",
                    Some("{\"activity_list\":\"not a listing\"}")
                ),
            ]),
            None,
            "no reported get_activities payload, no list"
        );
    }

    /// A converse that reported no result has no round to record.
    #[test]
    fn calls_without_results_make_no_round() {
        assert_eq!(
            observed_tool_results(&[call(None, "get_activities", None)]),
            "",
            "a call with no result carries no evidence"
        );
        assert_eq!(
            observed_tool_results(&[]),
            "",
            "a turn that ran no tool has no round"
        );
    }
}
