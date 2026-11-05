// ABOUTME: Static taint-dataflow verifier for a frozen Workflow plan — rejects tainted-source→sink flows before any tool runs.
// ABOUTME: Pure Rust graph reachability over the SymRef DAG (no Z3); reuses the same SecurityLabels + GuardianPolicy as the runtime Guardian.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The plan verifier.
//!
//! Given a frozen [`Workflow`], this decides before any tool executes whether
//! the plan may run: its tools are ones this turn was offered, its references
//! name earlier steps, it fits the per-turn budgets, and an external send
//! follows no `UNTRUSTED_OUTPUT` source. It is pure forward reachability over
//! the step-reference graph — decidable, no solver — and it reads the **same**
//! [`SecurityLabels`](crate::security::SecurityLabels) and [`GuardianPolicy`]
//! the runtime Guardian uses, so there is a single source of truth for the
//! policy.
//!
//! Whether an irreversible or outbound sink may take arguments derived from an
//! untrusted source is the policy's call: only `Deny` rejects that here. `Log`
//! (the default) and `Confirm` pass it, and such a plan runs with tainted data in
//! that sink's arguments, up to the runtime Guardian. An ordinary write is not a
//! sink at all. So "verified" is no proof that nothing tainted reaches a write.

use std::collections::{HashMap, HashSet};
use std::fmt;

use uuid::Uuid;

use dravr_tronc::mcp::tool::ToolCapabilities;

use crate::guardian::policy::{GuardianPolicy, TaintedDestructive};
use crate::guardian::workflow::{referenced_steps, StepId, Workflow};
use crate::registry::ToolRegistry;
use crate::security::SecurityLabels;

/// What the verifier needs to know about each tool. Abstracted behind a trait so
/// the verifier unit-tests against a fake map and runs against the real
/// [`ToolRegistry`] in production.
pub trait ToolLabels {
    /// The tool's Guardian security classification, or `None` if unknown.
    fn labels(&self, tool: &str) -> Option<SecurityLabels>;
    /// Whether the tool is a `WRITES_DATA` sink, or `None` if unknown.
    fn writes_data(&self, tool: &str) -> Option<bool>;
    /// Whether the tool is callable during chat-mode function calling.
    fn chat_callable(&self, tool: &str) -> bool;
    /// Whether the turn this plan answers offered the tool to the model.
    fn offered(&self, tool: &str) -> bool;
}

impl ToolLabels for ToolRegistry {
    fn labels(&self, tool: &str) -> Option<SecurityLabels> {
        self.security_class(tool)
    }

    fn writes_data(&self, tool: &str) -> Option<bool> {
        self.get(tool).map(|registered| {
            registered
                .capabilities()
                .contains(ToolCapabilities::WRITES_DATA)
        })
    }

    fn chat_callable(&self, tool: &str) -> bool {
        self.chat_callable_schemas()
            .iter()
            .any(|schema| schema.name == tool)
    }

    /// The registry alone knows no turn. [`OfferedOnly`] narrows it to one.
    fn offered(&self, tool: &str) -> bool {
        self.contains(tool)
    }
}

/// The registry's labels, narrowed to the tools one turn offered the model.
///
/// A turn's offer is narrower than the registry: an agent's tool list, the seat,
/// a tool a super admin switched off. A plan is written by the same model, so a
/// name it was not offered is one the turn never agreed to run.
pub struct OfferedOnly<'turn> {
    labels: &'turn dyn ToolLabels,
    offered: HashSet<&'turn str>,
}

impl fmt::Debug for OfferedOnly<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OfferedOnly")
            .field("offered", &self.offered)
            .finish_non_exhaustive()
    }
}

impl<'turn> OfferedOnly<'turn> {
    /// `labels` as a turn that offered the tools named by `offered` reads them.
    pub fn new(labels: &'turn dyn ToolLabels, offered: impl Iterator<Item = &'turn str>) -> Self {
        Self {
            labels,
            offered: offered.collect(),
        }
    }
}

impl ToolLabels for OfferedOnly<'_> {
    fn labels(&self, tool: &str) -> Option<SecurityLabels> {
        self.labels.labels(tool)
    }

    fn writes_data(&self, tool: &str) -> Option<bool> {
        self.labels.writes_data(tool)
    }

    fn chat_callable(&self, tool: &str) -> bool {
        self.labels.chat_callable(tool)
    }

    fn offered(&self, tool: &str) -> bool {
        self.offered.contains(tool)
    }
}

/// Why a plan was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// A step names a tool not in the registry.
    UnknownTool(String),
    /// A step names a tool the LLM is not allowed to call during chat.
    NotChatCallable(String),
    /// A step names a tool the turn did not offer the model.
    NotOffered(String),
    /// A `SymRef` names a step that is not strictly earlier (or does not exist).
    DanglingReference(StepId),
    /// A consequential sink receives arguments derived from an untrusted source.
    TaintedSink {
        /// The sink step.
        sink_step: StepId,
        /// The sink tool name.
        sink_tool: String,
        /// The upstream untrusted source step.
        source_step: StepId,
        /// The upstream untrusted source tool name.
        source_tool: String,
    },
    /// An external-send step for a tenant the egress allowlist forbids.
    EgressForbidden(String),
    /// The plan exceeds a per-turn consequential-action budget.
    BudgetExceeded {
        /// `"writes"` or `"destructive"`.
        kind: &'static str,
        /// The configured limit.
        limit: u16,
    },
}

impl VerifyError {
    /// The machine slug the rejection is logged and reported under. Stable, and
    /// free of anything the plan named: it travels on the reply's denial.
    #[must_use]
    pub const fn reason_slug(&self) -> &'static str {
        match self {
            Self::UnknownTool(_) => "plan_unknown_tool",
            Self::NotChatCallable(_) => "plan_tool_not_chat_callable",
            Self::NotOffered(_) => "plan_tool_not_offered",
            Self::DanglingReference(_) => "plan_dangling_reference",
            Self::TaintedSink { .. } => "plan_tainted_sink",
            Self::EgressForbidden(_) => "plan_egress_forbidden",
            Self::BudgetExceeded { .. } => "plan_over_budget",
        }
    }
}

/// The verifier's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// The plan is safe to execute.
    Accept,
    /// The plan must not execute; the reason drives a deterministic refusal.
    Reject(VerifyError),
}

/// Statically verify a frozen [`Workflow`] against the security policy.
///
/// Rejects (before any execution) on: an unknown / non-chat-callable tool, or
/// one the turn did not offer; a dangling/forward reference; a sink whose args
/// derive from an `UNTRUSTED_OUTPUT` source (the core dataflow guarantee) — for
/// external-send always, for irreversible and outbound writes under the `Deny`
/// taint policy; an external send outside the egress allowlist; or a plan over the per-turn write/destructive
/// budget.
#[must_use]
pub fn verify(
    workflow: &Workflow,
    labels: &dyn ToolLabels,
    policy: &GuardianPolicy,
    tenant: Option<Uuid>,
) -> VerifyOutcome {
    // Output-taint of each *seen* step id, plus its tool name (for error context).
    let mut output_tainted: HashMap<StepId, bool> = HashMap::new();
    let mut tool_of: HashMap<StepId, String> = HashMap::new();
    let mut seen: HashSet<StepId> = HashSet::new();
    // First untrusted source seen so far (for the coarse external-send backstop).
    let mut first_untrusted: Option<(StepId, String)> = None;
    let mut write_count: u16 = 0;
    let mut destructive_count: u16 = 0;

    for step in &workflow.steps {
        let tool = step.tool_name.as_str();
        let Some(class) = labels.labels(tool) else {
            return VerifyOutcome::Reject(VerifyError::UnknownTool(step.tool_name.clone()));
        };
        if !labels.chat_callable(tool) {
            return VerifyOutcome::Reject(VerifyError::NotChatCallable(step.tool_name.clone()));
        }
        if !labels.offered(tool) {
            return VerifyOutcome::Reject(VerifyError::NotOffered(step.tool_name.clone()));
        }

        // References must name strictly-earlier (already-seen) steps.
        let refs = referenced_steps(&step.args);
        for referenced in &refs {
            if !seen.contains(referenced) {
                return VerifyOutcome::Reject(VerifyError::DanglingReference(*referenced));
            }
        }

        // Inputs are tainted if any referenced step's output is tainted.
        let tainted_ref = refs
            .iter()
            .find(|referenced| output_tainted.get(*referenced).copied().unwrap_or(false));
        let arg_tainted = tainted_ref.is_some();

        // Egress allowlist (taint-independent, un-spoofable tenant).
        if class.is_external_send() && !policy.external_send.allows(tenant) {
            return VerifyOutcome::Reject(VerifyError::EgressForbidden(step.tool_name.clone()));
        }

        // External send after ANY untrusted source: the exfiltration channel.
        // Coarse turn-level rule (mirrors the runtime Guardian's Tier-B), so it
        // also catches laundering through an unmodeled side channel.
        if class.is_external_send() {
            if let Some((source_step, source_tool)) = first_untrusted.clone() {
                return VerifyOutcome::Reject(VerifyError::TaintedSink {
                    sink_step: step.id,
                    sink_tool: step.tool_name.clone(),
                    source_step,
                    source_tool,
                });
            }
        }

        // Irreversible or outbound sink with arguments derived from an untrusted
        // source:
        // the precise dataflow rule (the plan-verify value-add). Gated by
        // policy: only `Deny` rejects statically. `Confirm` deliberately
        // passes here — the runtime chokepoint parks the step for human
        // confirmation at execution time, so Confirm behaves uniformly
        // across the ReAct and planned paths; `Log` records.
        if class.answers_to_taint_policy()
            && matches!(policy.tainted_destructive, TaintedDestructive::Deny)
            && arg_tainted
        {
            let (source_step, source_tool) = tainted_ref
                .and_then(|referenced| {
                    tool_of
                        .get(referenced)
                        .map(|name| (*referenced, name.clone()))
                })
                .unwrap_or_else(|| (step.id, step.tool_name.clone()));
            return VerifyOutcome::Reject(VerifyError::TaintedSink {
                sink_step: step.id,
                sink_tool: step.tool_name.clone(),
                source_step,
                source_tool,
            });
        }

        // Tally consequential actions for the static budget.
        if labels.writes_data(tool).unwrap_or(false) {
            write_count = write_count.saturating_add(1);
        }
        if class.is_irreversible() {
            destructive_count = destructive_count.saturating_add(1);
        }

        // Record this step's output taint and identity for downstream steps.
        let out_tainted = class.is_untrusted_output() || arg_tainted;
        output_tainted.insert(step.id, out_tainted);
        tool_of.insert(step.id, step.tool_name.clone());
        if class.is_untrusted_output() && first_untrusted.is_none() {
            first_untrusted = Some((step.id, step.tool_name.clone()));
        }
        seen.insert(step.id);
    }

    if write_count > policy.max_writes_per_turn {
        return VerifyOutcome::Reject(VerifyError::BudgetExceeded {
            kind: "writes",
            limit: policy.max_writes_per_turn,
        });
    }
    if destructive_count > policy.max_destructive_per_turn {
        return VerifyOutcome::Reject(VerifyError::BudgetExceeded {
            kind: "destructive",
            limit: policy.max_destructive_per_turn,
        });
    }

    VerifyOutcome::Accept
}
