// ABOUTME: The plan-then-verify tool loop: one plan call, a static verifier, a frozen execution, a synthesis
// ABOUTME: Reached only when a super admin armed plan_mode; a verified plan's steps dispatch through the chokepoint
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The planned tool loop (Phase 3 of the Guardian).
//!
//! Where a `ReAct` loop lets each tool result drive the next call, here the
//! model emits the whole plan up front, [`crate::guardian::verify`] judges it
//! before anything runs, and [`WorkflowExecutor`] runs it frozen. Plan and
//! synthesis are plain completions every provider class serves, so this loop
//! bypasses the provider's own tool surface altogether.

use std::collections::BTreeSet;

use pierre_core::errors::AppError;
use pierre_core::llm::tool_simulation;
use pierre_core::tokens::join_prompt_text;
use pierre_llm::call_record::{emit_call_record_with_text, CallRecordInputs};
use pierre_llm::{
    ChatMessage, ChatRequest, ChatResponse, FunctionResponse, MessageRole, TokenUsage, Tool,
};
use pierre_providers::ai_scope;
use tracing::{info, warn};
use uuid::Uuid;

use crate::cli_loop::{completion_call, CliLoopCall};
use crate::guardian::{
    self, OfferedOnly, PlanDenial, PlanParseError, StepOutput, VerifyOutcome, Workflow,
    WorkflowExecutor,
};
use crate::tool_execution::{
    add_function_responses_to_messages, log_tool_response_size, run_react_tool_loop,
    strip_simulation_artifacts,
};
use crate::tool_loop_io::{
    sum_usage, GuardianConfirmRequest, GuardianDenial, ToolLoopParams, ToolLoopResult,
    ToolLoopTally, ToolRoundRecord,
};
use crate::tool_results::reconnect_offer_in_steps;

/// The wire-shape label of this loop's calls.
const PLANNED_LOOP: &str = "planned_tool_loop";
/// The `llm_usage` sequence of the plan call: the turn's first.
const PLAN_CALL_SEQUENCE: i64 = 1;
/// The `llm_usage` sequence of the synthesis: the turn's second.
const SYNTHESIS_CALL_SEQUENCE: i64 = 2;

/// Disposition of a model-emitted plan after parsing.
enum ParsedPlan {
    /// A parseable, within-cap plan — run it.
    Run(Workflow),
    /// Unparseable (weak-model artifact) — degrade to the `ReAct` loop.
    Degrade,
    /// Over the step cap (S15) — fail closed, no `ReAct` fallback.
    TooLarge,
}

/// Parse the planner's JSON and log the disposition. Extracted from
/// [`run_planned_tool_loop`] to keep that loop under the cognitive bar.
fn parse_plan(plan_json: &str, model: &str) -> ParsedPlan {
    match Workflow::from_json(plan_json) {
        Ok(workflow) => ParsedPlan::Run(workflow),
        Err(PlanParseError::Unparseable(error)) => {
            warn!(
                %error,
                model,
                "guardian plan: unparseable workflow JSON; degrading to the capability-routed ReAct loop"
            );
            ParsedPlan::Degrade
        }
        Err(PlanParseError::TooManySteps { steps, max }) => {
            warn!(
                steps,
                max,
                "guardian plan: over the step cap (S15); rejecting the turn (no ReAct fallback)"
            );
            ParsedPlan::TooLarge
        }
    }
}

/// Statically verify the frozen plan, returning the slug the verifier refused it
/// under (`None` = verified, execute). Judged against the tools this turn
/// offered as well as the registry. Extracted from [`run_planned_tool_loop`] to
/// keep that loop under the cognitive bar.
fn plan_rejection(workflow: &Workflow, params: &ToolLoopParams<'_>) -> Option<&'static str> {
    let resources = &params.executor.resources;
    let labels = OfferedOnly::new(
        resources.tool_registry().as_ref(),
        params
            .tools
            .function_declarations
            .iter()
            .map(|decl| decl.name.as_str()),
    );
    let outcome = guardian::verify(
        workflow,
        &labels,
        resources.guardian().policy(),
        Some(params.tenant_id.as_uuid()),
    );
    if let VerifyOutcome::Reject(reason) = outcome {
        warn!(reason = ?reason, "guardian plan: REJECTED before execution");
        return Some(reason.reason_slug());
    }
    info!(
        steps = workflow.steps.len(),
        model = params.model,
        "guardian plan: verified; executing frozen plan"
    );
    None
}

/// Map a mid-plan stop into its short-circuit [`ToolLoopResult`]: a parked step
/// (Confirm) renders the confirmation ask, a denial or a failed binding renders
/// the block reply. Extracted from [`run_planned_tool_loop`] to keep that loop
/// under the cognitive-complexity bar.
fn plan_block_result(
    block: PlanDenial,
    tally: ToolLoopTally,
    usage: Option<TokenUsage>,
) -> ToolLoopResult {
    if let Some(pending_id) = block.pending_id {
        return tally.guardian_confirm(
            usage,
            GuardianConfirmRequest {
                tool_name: block.tool_name,
                pending_id,
            },
        );
    }
    tally.guardian_denied(
        usage,
        GuardianDenial {
            tool_name: block.tool_name,
            reason: block.reason,
        },
    )
}

/// The tools of a tool catalogue, without its call format.
///
/// [`tool_simulation::generate_tool_catalog`] opens by telling the model to emit
/// `<tool_call>` blocks and closes on an example of one. On the planner's call
/// that contradicts "output ONLY the JSON object", and a model that obeys the
/// catalogue emits an unparseable plan, which degrades to `ReAct` silently: a run
/// could then never plan and still look green. Embacle has no tools-only emitter,
/// so the two instructions are cut here. Should its wording move the whole
/// catalogue goes through rather than lose the parameters, and
/// `the_planner_is_told_the_parameters_and_no_other_call_format` goes red.
fn tools_section(catalog: &str) -> &str {
    let start = catalog.find("Available tools:").unwrap_or(0);
    let end = catalog
        .rfind("Example interaction:")
        .filter(|end| *end >= start)
        .unwrap_or(catalog.len());
    catalog[start..end].trim()
}

/// The planner's lead: its grammar, then the tools it may plan with and their
/// parameters, as the API loop's native declarations tell a `ReAct` model.
fn planner_lead(planner_prompt: &str, tools: &Tool) -> String {
    if tools.function_declarations.is_empty() {
        return planner_prompt.to_owned();
    }
    let catalog = tool_simulation::generate_tool_catalog(&tools.function_declarations);
    format!("{planner_prompt}\n\n{}", tools_section(&catalog))
}

/// Build the planner call's message list by folding the planner's lead INTO the
/// existing system message.
///
/// The lead is prepended onto `messages[0]` rather than inserted as a second
/// `System` message. Index 0 carries the agent persona, and the live provider
/// keeps only the *first* system message and drops the rest — a second one
/// would take the slot and silently discard the persona for the whole plan call.
/// Merging preserves the one-system-message invariant the rest of the pipeline
/// holds. The tools the model may plan with are not in index 0: they come with
/// the lead ([`planner_lead`]).
///
/// Falls back to inserting when index 0 is not a `System` message, which the
/// chat pipeline never produces but this function does not get to assume.
fn with_planner_prompt(messages: &[ChatMessage], planner_prompt: &str) -> Vec<ChatMessage> {
    let mut out = messages.to_vec();
    match out.first_mut() {
        Some(first) if first.role == MessageRole::System => {
            let lead = format!("{planner_prompt}\n\n");
            // The stable head the message names now starts after the lead: it
            // grows by the lead's length and still covers the same bytes. The
            // lead carries this turn's offer, which the seat, the agent and the
            // admin switches move, so a plan call's cached prefix holds
            // only across turns that offer the same tools.
            first.stable_prefix_len = first.stable_prefix_len.map(|len| len + lead.len());
            first.content.insert_str(0, &lead);
        }
        _ => out.insert(0, ChatMessage::system(planner_prompt)),
    }
    out
}

/// A plain completion for the planner or the synthesis: the turn's sampling, and
/// deliberately no MCP servers. A plan's tools run from the frozen plan alone,
/// and servers on this request would hand a Copilot runner the same tools to
/// call natively, around the verifier.
fn completion_request(messages: Vec<ChatMessage>, params: &ToolLoopParams<'_>) -> ChatRequest {
    let request = ChatRequest::new(messages).with_model(params.model);
    match params.temperature {
        Some(temperature) => request.with_temperature(temperature),
        None => request,
    }
}

/// Write a served call's `llm_usage` row. The assembled prompt feeds the token
/// estimator for a provider that reports no usage, as on the text loop.
fn record_call(
    params: &ToolLoopParams<'_>,
    request: &ChatRequest,
    (response, asked): (&ChatResponse, &CliLoopCall),
    call_sequence: Option<i64>,
    tools_called: Vec<String>,
) {
    let prompt_text = join_prompt_text(request.messages.iter().map(|m| m.content.as_str()));
    emit_call_record_with_text(
        CallRecordInputs {
            recorder: params.call_recorder.as_ref(),
            provider: asked.served_provider,
            model: &response.model,
            configured_provider: params.provider.name(),
            configured_model: params.model,
            usage: response.usage.as_ref(),
            latency_ms: asked.latency_ms,
            success: true,
            call_sequence,
            tools_called,
        },
        Some(&prompt_text),
        Some(&response.content),
    );
}

/// Fold what the plan ran into the running messages and the turn's records.
///
/// One pass over [`add_function_responses_to_messages`], the fold a `ReAct` round
/// goes through, so a step's result reaches the model through the same
/// projection, the activity list is captured the same way, and the round persists
/// the same text. A step that stopped the plan is left out: it did nothing the
/// model can cite, and its JSON is exactly what a block must not hand on. Earlier
/// steps stay in, since what they did stays done.
fn fold_plan_results(
    params: &ToolLoopParams<'_>,
    llm_messages: &mut Vec<ChatMessage>,
    outputs: &[StepOutput],
    members_read: BTreeSet<Uuid>,
) -> Option<String> {
    let responses: Vec<FunctionResponse> = outputs
        .iter()
        .filter(|output| !output.stopped_the_plan)
        .map(|output| FunctionResponse {
            name: output.tool_name.clone(),
            response: output.result.clone(),
        })
        .collect();
    if responses.is_empty() {
        return None;
    }
    for response in &responses {
        log_tool_response_size(response);
    }
    let round = add_function_responses_to_messages(llm_messages, &responses);
    if let Some(recorder) = params.tool_message_recorder.as_ref() {
        recorder.record(ToolRoundRecord {
            assistant_text: String::new(),
            tool_result_text: round.combined_text,
            members_read,
        });
    }
    round.activity_list
}

/// Plan-then-verify loop (Phase 3).
///
/// The LLM emits the entire tool plan up front, a static verifier judges it
/// *before* execution, and only a verified, frozen plan runs. Because the plan is
/// fixed before any tool result is seen, a malicious result cannot inject a new
/// tool call. What the verifier does and does not refuse: [`crate::guardian::verify`].
///
/// Falls back to the capability-routed `ReAct` loop when the model does not
/// emit a parseable plan (graceful degrade for weaker models).
///
/// Records one `llm_usage` row per call, and the result's usage is the plan call
/// plus whatever followed it, on every exit. Sequence 1 is the plan and 2 the
/// synthesis. A plan that degrades records no sequence, because the `ReAct` loop
/// numbers its own calls from 1 and knows nothing of the one before it.
///
/// # Errors
/// Returns error if the LLM plan/synthesis call fails. Steps that ran before a
/// failed synthesis stay done, and their round is already recorded.
pub async fn run_planned_tool_loop(
    params: &ToolLoopParams<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> Result<ToolLoopResult, AppError> {
    // 1. Plan call — the whole plan up front, in the catalogue's `guardian_planner` grammar.
    let planner = params.executor.resources.guardian_planner_prompt();
    let lead = planner_lead(planner.trim(), params.tools);
    let plan_request = completion_request(with_planner_prompt(llm_messages, &lead), params);
    let (plan_response, plan_asked) = completion_call(
        params,
        &plan_request,
        PLANNED_LOOP,
        Some(PLAN_CALL_SEQUENCE),
    )
    .await?;
    let plan_call = (&plan_response, &plan_asked);
    let plan_usage = plan_response.usage.clone();
    let mut tally = ToolLoopTally::default();
    tally.note_provider_warnings(plan_response.warnings.as_deref());

    // 2. Parse. An unparseable plan is a weak-model artifact → degrade to ReAct.
    //    An over-cap plan is the S15 cost-amplification vector → FAIL CLOSED
    //    (reject the turn); it must NOT drop to the unbudgeted ReAct loop, which
    //    would reopen exactly what the step cap closes.
    let workflow = match parse_plan(&plan_response.content, params.model) {
        ParsedPlan::Run(workflow) => workflow,
        ParsedPlan::Degrade => {
            record_call(params, &plan_request, plan_call, None, Vec::new());
            let prior = tally.interrupted(plan_usage);
            return Ok(run_react_tool_loop(params, llm_messages)
                .await?
                .after_prior(prior));
        }
        ParsedPlan::TooLarge => {
            record_call(
                params,
                &plan_request,
                plan_call,
                Some(PLAN_CALL_SEQUENCE),
                Vec::new(),
            );
            return Ok(tally.guardian_plan_denied(plan_usage, "plan_too_large"));
        }
    };

    // 3. Verify the frozen plan before anything executes.
    if let Some(slug) = plan_rejection(&workflow, params) {
        record_call(
            params,
            &plan_request,
            plan_call,
            Some(PLAN_CALL_SEQUENCE),
            Vec::new(),
        );
        return Ok(tally.guardian_plan_denied(plan_usage, slug));
    }
    record_call(
        params,
        &plan_request,
        plan_call,
        Some(PLAN_CALL_SEQUENCE),
        workflow
            .steps
            .iter()
            .map(|step| step.tool_name.clone())
            .collect(),
    );

    // 4. Execute the verified plan (each call still passes the runtime Guardian).
    let workflow_executor =
        WorkflowExecutor::new(params.executor.as_ref(), params.user_id, params.tenant_id)
            .with_sink(params.stream_sink.as_ref());
    let (run, members_read) = ai_scope::reading_members(workflow_executor.run(&workflow)).await;
    tally.activity_list = fold_plan_results(params, llm_messages, &run.outputs, members_read);
    // Ran and succeeded, not planned: the anti-fabrication gate on agent visuals
    // reads this, and a citation met by a merely-attempted step is not evidence.
    tally.tools_called = run
        .outputs
        .iter()
        .filter(|output| output.success)
        .map(|output| output.tool_name.clone())
        .collect();
    tally.tool_calls_count = u32::try_from(run.outputs.len()).unwrap_or(u32::MAX);
    tally.served_without_provider = reconnect_offer_in_steps(&run.outputs);

    // S9: a runtime Guardian block mid-plan short-circuits with a deterministic
    // reply — the blocked step's JSON must NOT be fed to the synthesis LLM to
    // paraphrase (the exact softened-refusal the guard exists to prevent). Ahead
    // of the auth trip, as on every ReAct loop.
    if let Some(block) = run.denial {
        return Ok(plan_block_result(block, tally, plan_usage));
    }
    if let Some(provider) = run.auth_required_provider {
        return Ok(tally.provider_auth_required(plan_usage, provider));
    }

    // 5. Synthesis — the bound results are evidence in the messages (never new
    //    tool proposals); this call produces the user-facing answer.
    let synth_request = completion_request(llm_messages.clone(), params);
    let (synth_response, synth_asked) = completion_call(
        params,
        &synth_request,
        PLANNED_LOOP,
        Some(SYNTHESIS_CALL_SEQUENCE),
    )
    .await?;
    record_call(
        params,
        &synth_request,
        (&synth_response, &synth_asked),
        Some(SYNTHESIS_CALL_SEQUENCE),
        Vec::new(),
    );
    info!(
        steps = workflow.steps.len(),
        tools_called = tally.tool_calls_count,
        model = params.model,
        "guardian plan: completed with synthesis"
    );

    tally.note_provider_warnings(synth_response.warnings.as_deref());
    // A model that wrote a `<tool_call>` here gets nothing run for it: the plan
    // is over. The block is stripped so it does not reach the athlete.
    let content = strip_simulation_artifacts(&synth_response.content);
    Ok(tally.answered(
        content,
        sum_usage(plan_usage, synth_response.usage),
        synth_response.finish_reason,
    ))
}

#[cfg(test)]
mod tests {
    use pierre_llm::ChatMessage;

    use super::with_planner_prompt;

    #[test]
    fn the_planner_lead_keeps_the_stable_head_on_the_same_bytes() {
        let messages = vec![
            ChatMessage::system("head tail").with_stable_prefix_len(4),
            ChatMessage::user("hi"),
        ];
        let out = with_planner_prompt(&messages, "plan first");
        assert_eq!(
            out[0].content, "plan first\n\nhead tail",
            "bug: the planner lead must precede the system prompt"
        );
        assert_eq!(
            out[0].stable_split(),
            Some(("plan first\n\nhead", " tail")),
            "bug: the stable head must grow by the lead and cover the same bytes"
        );
    }
}
