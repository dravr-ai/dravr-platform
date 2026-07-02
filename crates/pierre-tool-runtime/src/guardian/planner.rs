// ABOUTME: The plan-then-verify executor — runs a VERIFIED frozen Workflow, binding real outputs to SymRefs.
// ABOUTME: Plus the planner system prompt documenting the workflow JSON grammar the LLM emits.

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
//! argument to a sink. And the verifier already proved no tainted `SymRef`
//! reaches a sink, so even fully attacker-controlled bound data has nowhere to go.

use std::collections::HashMap;

use serde_json::{json, Value};
use tracing::warn;

use pierre_core::errors::AppError;
use pierre_core::models::TenantId;

use crate::guardian::workflow::{self, StepId, Workflow};
use crate::protocol::{UniversalRequest, UniversalToolExecutor};

/// The output of one executed step.
#[derive(Debug, Clone)]
pub struct StepOutput {
    /// The step id (binding identity).
    pub step: StepId,
    /// The tool that ran.
    pub tool_name: String,
    /// The tool's result JSON (bound to this step's `SymRef`s).
    pub result: Value,
    /// Whether the dispatch succeeded (a Guardian denial or tool error is `false`).
    pub success: bool,
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
    /// The plan step's tool that was denied.
    pub tool_name: String,
    /// The machine reason slug (`DenyReason::as_str`), e.g. `budget_exceeded`.
    pub reason: String,
}

/// Runs a verified [`Workflow`] against the shared executor.
pub struct WorkflowExecutor<'a> {
    executor: &'a UniversalToolExecutor,
    user_id: &'a str,
    tenant_id: TenantId,
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
        }
    }

    /// Execute the (already-verified) plan in order, binding each step's real
    /// output so later steps' `SymRef`s resolve to actual values.
    ///
    /// Stops early and returns the outputs so far plus a [`PlanDenial`] the
    /// moment a step is denied by the runtime Guardian (S9) — a mid-plan denial
    /// must not be handed to the synthesis LLM to paraphrase. Ordinary tool
    /// failures do not stop the plan; they are captured in-band on the
    /// [`StepOutput`].
    ///
    /// # Errors
    /// Returns [`AppError`] only when a `SymRef` cannot be resolved (a verified
    /// plan should never hit this; it indicates a binding bug).
    pub async fn run(
        &self,
        workflow: &Workflow,
    ) -> Result<(Vec<StepOutput>, Option<PlanDenial>), AppError> {
        let mut bindings: HashMap<StepId, Value> = HashMap::new();
        let mut outputs = Vec::with_capacity(workflow.steps.len());
        let mut denial: Option<PlanDenial> = None;

        for step in &workflow.steps {
            let parameters = workflow::resolve_args(&step.args, &bindings).map_err(|e| {
                AppError::invalid_input(format!(
                    "plan step {} argument resolution failed: {e:?}",
                    step.id
                ))
            })?;

            let request = UniversalRequest {
                tool_name: step.tool_name.clone(),
                parameters,
                user_id: self.user_id.to_owned(),
                protocol: "chat".to_owned(),
                tenant_id: Some(self.tenant_id.to_string()),
                progress_token: None,
                cancellation_token: None,
                progress_reporter: None,
            };

            // A runtime Guardian denial is stamped `success:false` +
            // `metadata.blocked_reason == "guardian"` (operator-set, not
            // tool-forgeable). Capture it before folding the result.
            let (result, success, guardian_reason) = match self.executor.execute_tool(request).await
            {
                Ok(response) => {
                    let reason = if response.success {
                        None
                    } else {
                        response
                            .metadata
                            .as_ref()
                            .filter(|m| {
                                m.get("blocked_reason").and_then(Value::as_str) == Some("guardian")
                            })
                            .and_then(|m| m.get("guardian_reason").and_then(Value::as_str))
                            .map(ToOwned::to_owned)
                    };
                    (
                        response.result.unwrap_or(Value::Null),
                        response.success,
                        reason,
                    )
                }
                Err(e) => (json!({ "error": e.to_string() }), false, None),
            };

            bindings.insert(step.id, result.clone());
            outputs.push(StepOutput {
                step: step.id,
                tool_name: step.tool_name.clone(),
                result,
                success,
            });

            if let Some(reason) = guardian_reason {
                warn!(
                    step = step.id,
                    tool = %step.tool_name,
                    reason = %reason,
                    "guardian denied a plan step at runtime; stopping the plan"
                );
                denial = Some(PlanDenial {
                    tool_name: step.tool_name.clone(),
                    reason,
                });
                break;
            }
        }

        Ok((outputs, denial))
    }
}

/// The planner system prompt.
///
/// Instructs the LLM to emit the whole plan up front as a single JSON workflow,
/// with `$ref` references that thread a prior tool's output into a later
/// argument instead of pasting the data.
#[must_use]
pub fn planner_system_prompt() -> String {
    "You are a planning assistant. Instead of calling tools one at a time, emit \
the ENTIRE plan up front as a single JSON object and nothing else:\n\n\
{ \"steps\": [ { \"id\": 0, \"tool\": \"<tool_name>\", \"args\": { } } ] }\n\n\
Rules:\n\
- Each step has a unique integer \"id\" and a registered \"tool\" name with its \"args\".\n\
- Steps run in order; a step may use a PRIOR step's output via a placeholder \
instead of pasting the data: { \"$ref\": { \"step\": <earlier_id>, \"path\": \"<optional JSON pointer>\" } }\n\
- Use literals for values you already know; use $ref ONLY to thread a prior \
tool's output into a later argument.\n\
- A later step may only reference an EARLIER step's id.\n\
- Output ONLY the JSON object — no prose, no markdown code fences."
        .to_owned()
}
