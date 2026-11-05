// ABOUTME: The plan-then-verify executor — runs a VERIFIED frozen Workflow, binding real outputs to SymRefs.
// ABOUTME: Each step still passes the runtime Guardian; the plan fixes which tools run and with what shape.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Workflow execution for the plan-then-verify layer.
//!
//! [`WorkflowExecutor`] runs a plan that has **already passed**
//! [`crate::guardian::verify::verify`]. It binds each step's real output to the
//! [`SymRef`](crate::guardian::workflow::SymRef)s that name it and dispatches
//! every call through [`crate::protocol::UniversalToolExecutor`] — so the runtime
//! Guardian still fires (defense in depth) and telemetry is identical to `ReAct`.
//!
//! The frozen-plan guarantee: a malicious tool *result* can change the value
//! bound to a `SymRef`, but the set of `(tool, arg-structure)` per step is fixed
//! by the verified plan — it cannot add a step, change a tool, or redirect an
//! argument to a sink.
//!
//! What that guarantee does not cover is the *value*. The verifier refuses a
//! tainted reference statically only at an external send, and at an irreversible
//! or outbound sink under the `Deny` taint policy. Under `Log` (the default) and
//! `Confirm` attacker-controlled bound data does reach those sinks' arguments,
//! and an ordinary write answers to no taint rule at all. There, the runtime
//! Guardian's line and the human's confirmation are what stand in the way.

use std::collections::HashMap;

use pierre_services::chat_stream::{TurnEventSink, TOOL_STATUS_IN_PROGRESS};
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use pierre_core::models::TenantId;

use crate::chat_catalogue;
use crate::guardian::workflow::{self, ResolveError, StepId, Workflow, WorkflowStep};
use crate::protocol::types::{
    auth_required_provider, failed_dispatch, guardian_block_reason, metadata_str,
};
use crate::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::tool_progress::{closing_status, report_call};

/// The output of one executed step.
#[derive(Debug, Clone)]
pub struct StepOutput {
    /// The step id (binding identity).
    pub step: StepId,
    /// The tool that ran.
    pub tool_name: String,
    /// The tool's result JSON, or the failure as `{"error": <text>}`: the same
    /// shape a `ReAct` loop hands the model, so a failed step's reason survives.
    pub result: Value,
    /// Whether the dispatch succeeded (a Guardian denial or tool error is `false`).
    pub success: bool,
    /// Whether this step ended the plan: the Guardian stopped it, or its
    /// provider needs re-authorizing. Its `result` is not evidence of anything.
    pub stopped_the_plan: bool,
}

/// A runtime Guardian denial that stopped a plan mid-execution (S9).
///
/// Surfaced out of [`WorkflowExecutor::run`] so the caller renders a
/// deterministic block reply instead of feeding the denied step's JSON to the
/// synthesis LLM to paraphrase (the "softened refusal" the guard exists to
/// prevent). Free of any `client-chat`-gated type so this always-compiled module
/// stays feature-independent; the chat loop maps it to a `GuardianDenial`.
#[derive(Debug, Clone)]
pub struct PlanDenial {
    /// The plan step's tool that was denied (or parked).
    pub tool_name: String,
    /// The machine reason slug (`DenyReason::as_str`), e.g. `budget_exceeded`;
    /// `confirm_required` for a parked step; [`PLAN_BINDING_FAILED`] for a step
    /// whose reference did not resolve.
    pub reason: String,
    /// Claim token when the step was PARKED for user confirmation
    /// (`TaintedDestructive::Confirm`) rather than denied — the caller renders
    /// the confirmation ask instead of the block reply.
    pub pending_id: Option<String>,
}

/// [`PlanDenial::reason`] of a step whose `$ref` could not be bound.
pub const PLAN_BINDING_FAILED: &str = "plan_binding_failed";

/// What a plan's run came to.
#[derive(Debug, Default)]
pub struct PlanRun {
    /// One entry per step dispatched, in order, the step that stopped the plan
    /// included. A step whose reference did not resolve was never dispatched.
    pub outputs: Vec<StepOutput>,
    /// Why the plan stopped early, when a block or a binding failure did it.
    pub denial: Option<PlanDenial>,
    /// Provider slug of the step whose provider needs re-authorizing, when that
    /// stopped the plan. The reply becomes the reconnect message, so a later
    /// step's side effect would be one the athlete is never told about.
    pub auth_required_provider: Option<String>,
}

/// Runs a verified [`Workflow`] against the shared executor.
#[derive(Debug)]
pub struct WorkflowExecutor<'a> {
    executor: &'a UniversalToolExecutor,
    user_id: &'a str,
    tenant_id: TenantId,
    sink: Option<&'a TurnEventSink>,
}

impl<'a> WorkflowExecutor<'a> {
    /// Build an executor for a single turn's plan.
    #[must_use]
    pub const fn new(
        executor: &'a UniversalToolExecutor,
        user_id: &'a str,
        tenant_id: TenantId,
    ) -> Self {
        Self {
            executor,
            user_id,
            tenant_id,
            sink: None,
        }
    }

    /// Report each step to the turn's sink as it runs, the way every `ReAct` loop
    /// reports its calls.
    #[must_use]
    pub const fn with_sink(mut self, sink: Option<&'a TurnEventSink>) -> Self {
        self.sink = sink;
        self
    }

    /// Execute the (already-verified) plan in order, binding each step's real
    /// output so later steps' `SymRef`s resolve to actual values.
    ///
    /// Stops early the moment a step is denied by the runtime Guardian (S9): a
    /// mid-plan denial must not be handed to the synthesis LLM to paraphrase. It
    /// stops too on a provider that needs re-authorizing, and on a reference
    /// that does not resolve. Steps that already ran are not undone by any of
    /// these: a write committed before the stop stays committed. Ordinary tool
    /// failures do not stop the plan; they are captured in-band on the
    /// [`StepOutput`], and a failed step binds nothing, so only a step that
    /// refers to it stops there. Its error text never flows into a later
    /// step's arguments.
    pub async fn run(&self, workflow: &Workflow) -> PlanRun {
        let mut bindings: HashMap<StepId, Value> = HashMap::new();
        let mut run = PlanRun {
            outputs: Vec::with_capacity(workflow.steps.len()),
            ..PlanRun::default()
        };

        for step in &workflow.steps {
            let parameters = match workflow::resolve_args(&step.args, &bindings) {
                Ok(parameters) => parameters,
                Err(error) => {
                    run.denial = Some(binding_failure(step, &error, &run.outputs));
                    break;
                }
            };

            let response = self.dispatch(step, parameters).await;
            let block = guardian_block(&step.tool_name, &response);
            let auth_provider = auth_required_provider(&response);
            let stopped_the_plan = block.is_some() || auth_provider.is_some();
            let (result, success) = step_result(step, response);
            if success {
                bindings.insert(step.id, result.clone());
            }
            run.outputs.push(StepOutput {
                step: step.id,
                tool_name: step.tool_name.clone(),
                result,
                success,
                stopped_the_plan,
            });

            if stopped_the_plan {
                warn!(
                    step = step.id,
                    tool = %step.tool_name,
                    reason = block.as_ref().map_or("provider_auth_required", |b| b.reason.as_str()),
                    "guardian plan: a step stopped the plan at runtime"
                );
                run.denial = block;
                run.auth_required_provider = auth_provider;
                break;
            }
        }

        run
    }

    /// Run one step through the chokepoint, reporting it to the turn's sink.
    async fn dispatch(&self, step: &WorkflowStep, parameters: Value) -> UniversalResponse {
        let request = UniversalRequest {
            tool_name: step.tool_name.clone(),
            parameters: chat_catalogue::chat_arguments(parameters),
            user_id: self.user_id.to_owned(),
            protocol: "chat".to_owned(),
            tenant_id: Some(self.tenant_id.to_string()),
        };
        let call_id = Uuid::new_v4().to_string();
        report_call(
            self.sink,
            &call_id,
            &step.tool_name,
            TOOL_STATUS_IN_PROGRESS,
        );
        let response = self
            .executor
            .execute_tool(request)
            .await
            .unwrap_or_else(|e| failed_dispatch(&e));
        report_call(
            self.sink,
            &call_id,
            &step.tool_name,
            closing_status(&response),
        );
        response
    }
}

/// The stop for a step whose reference did not resolve.
///
/// A verified plan can still reach this: the verifier sees that a step exists,
/// not what its output holds or whether it succeeded. Nothing is undone on the
/// way out, so the line says how many steps already committed.
fn binding_failure(
    step: &WorkflowStep,
    error: &ResolveError,
    outputs: &[StepOutput],
) -> PlanDenial {
    warn!(
        step = step.id,
        tool = %step.tool_name,
        error = ?error,
        steps_already_run = outputs.iter().filter(|output| output.success).count(),
        "guardian plan: a reference did not resolve; stopping the plan. Steps that already ran \
         are not rolled back"
    );
    PlanDenial {
        tool_name: step.tool_name.clone(),
        reason: PLAN_BINDING_FAILED.to_owned(),
        pending_id: None,
    }
}

/// Read a Guardian block out of a step's response ([`guardian_block_reason`]'s
/// trust rule). A parked step without a claim token renders as a denial: the
/// step stops the plan either way, and no ask could be answered.
fn guardian_block(tool_name: &str, response: &UniversalResponse) -> Option<PlanDenial> {
    let stamp = |key: &str| metadata_str(response, key).map(ToOwned::to_owned);
    let (reason, pending_id) = match guardian_block_reason(response)? {
        "guardian" => (
            stamp("guardian_reason").unwrap_or_else(|| "denied".to_owned()),
            None,
        ),
        "guardian_confirm" => ("confirm_required".to_owned(), stamp("pending_id")),
        _ => return None,
    };
    Some(PlanDenial {
        tool_name: tool_name.to_owned(),
        reason,
        pending_id,
    })
}

/// A step's result as the model is owed it, and whether it succeeded: the shape
/// `build_function_response` gives a `ReAct` call, with the failure's text kept.
fn step_result(step: &WorkflowStep, response: UniversalResponse) -> (Value, bool) {
    if response.success {
        let result = response
            .result
            .unwrap_or_else(|| json!({"status": "success"}));
        return (result, true);
    }
    let error = response.error.as_deref().unwrap_or("Unknown error");
    // A failed call's error travels only to the model: without this line a
    // super admin sees `success=false` and never the reason. Tool error strings
    // are already model-visible text, as on the `ReAct` loops.
    warn!(
        step = step.id,
        tool = %step.tool_name,
        error,
        "guardian plan: a step failed; its error goes to the synthesis"
    );
    (json!({ "error": error }), false)
}
