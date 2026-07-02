// ABOUTME: The plan-then-verify Workflow AST — a frozen, up-front tool plan with symbolic references.
// ABOUTME: SymRef placeholders carry provenance (not literal data); the verifier reasons over identity, the executor binds real outputs.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Workflow AST for the plan-then-verify layer (Phase 3 of the Guardian).
//!
//! Instead of the interleaved `ReAct` loop (where each tool *result* re-enters the
//! LLM and can drive the next call), the LLM emits the entire plan up front as a
//! [`Workflow`]: a linear list of [`WorkflowStep`]s, each a tool call whose
//! arguments are either trusted [`ArgValue::Literal`]s from the instruction
//! channel or [`ArgValue::Ref`] placeholders ([`SymRef`]) bound at runtime to a
//! prior step's output. Outputs are **never inlined as literals** — that is the
//! whole point: the [`crate::guardian::verify`] verifier reasons over the
//! reference graph (provenance), and the executor binds real values only after
//! the plan is frozen and accepted, so a malicious tool result can change a
//! *value* but never add a step, change a tool, or redirect an arg to a sink.

use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

/// Index/identity of a step. A step's `id` binds its runtime output to every
/// [`SymRef`] that names it.
pub type StepId = u32;

/// Maximum number of steps a plan may contain (S15). The verifier budgets only
/// writes/destructive, so without a hard step cap a plan of thousands of reads
/// (each a provider scrape / possible LLM enrichment) would pass verification
/// and execute — a cost-amplification vector. A real coaching plan is a handful
/// of steps; 32 is generous.
const MAX_PLAN_STEPS: usize = 32;

/// A symbolic reference to a prior step's output (optionally a sub-path within
/// it). Carries provenance only — never the literal data.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SymRef {
    /// The step whose output this refers to.
    pub step: StepId,
    /// RFC-6901 JSON pointer into that step's result (e.g. `/activities/0/name`).
    /// `None` means the whole result. v1 treats any path as equally tainted.
    #[serde(default)]
    pub path: Option<String>,
}

/// One argument value in a [`ToolCallNode`]: a constant from the trusted
/// instruction channel, or a placeholder bound at runtime to a prior output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgValue {
    /// A literal constant (trusted — it came from the instruction channel).
    Literal(Value),
    /// A placeholder bound at runtime to a prior step's (possibly tainted) output.
    Ref(SymRef),
    /// A heterogeneous array of arg values.
    Array(Vec<Self>),
    /// A nested object of arg values.
    Object(Vec<(String, Self)>),
}

impl ArgValue {
    /// Convert a raw JSON value into an [`ArgValue`]. An object with exactly the
    /// single key `$ref` becomes a [`ArgValue::Ref`]; everything else recurses
    /// into `Object`/`Array` or terminates as a `Literal`.
    fn from_value(value: Value) -> Result<Self, String> {
        match value {
            Value::Object(map) => {
                if map.len() == 1 {
                    if let Some(ref_val) = map.get("$ref") {
                        let sym: SymRef = serde_json::from_value(ref_val.clone())
                            .map_err(|e| format!("invalid $ref placeholder: {e}"))?;
                        return Ok(Self::Ref(sym));
                    }
                }
                let mut fields = Vec::with_capacity(map.len());
                for (key, val) in map {
                    fields.push((key, Self::from_value(val)?));
                }
                Ok(Self::Object(fields))
            }
            Value::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(Self::from_value(item)?);
                }
                Ok(Self::Array(out))
            }
            other => Ok(Self::Literal(other)),
        }
    }
}

impl<'de> Deserialize<'de> for ArgValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(value).map_err(D::Error::custom)
    }
}

/// A single tool call in a workflow: a tool name plus named arguments.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowStep {
    /// Binding identity; must be unique and may only be referenced by *later* steps.
    pub id: StepId,
    /// Registered tool name.
    #[serde(rename = "tool")]
    pub tool_name: String,
    /// Named arguments (deterministic order via `BTreeMap`).
    #[serde(default)]
    pub args: BTreeMap<String, ArgValue>,
}

/// A frozen, up-front plan. v1 is linear: `steps` order is execution order and a
/// step may only reference strictly-earlier steps (so the reference graph is
/// acyclic by construction).
#[derive(Debug, Clone, Deserialize)]
pub struct Workflow {
    /// The ordered steps.
    pub steps: Vec<WorkflowStep>,
}

impl Workflow {
    /// Parse a workflow from the JSON the LLM emitted. Returns `Err` (so the
    /// caller can fall back to the `ReAct` loop) on malformed plans.
    ///
    /// # Errors
    /// Returns the serde error message when the JSON does not match the grammar.
    pub fn from_json(raw: &str) -> Result<Self, PlanParseError> {
        let workflow: Self =
            serde_json::from_str(raw).map_err(|e| PlanParseError::Unparseable(e.to_string()))?;
        if workflow.steps.len() > MAX_PLAN_STEPS {
            return Err(PlanParseError::TooManySteps {
                steps: workflow.steps.len(),
                max: MAX_PLAN_STEPS,
            });
        }
        Ok(workflow)
    }
}

/// Why [`Workflow::from_json`] rejected a model-emitted plan.
///
/// The two cases demand OPPOSITE handling, so they are distinct variants rather
/// than one opaque string: an unparseable plan is a weak-model artifact that
/// should degrade gracefully to the `ReAct` loop, but an over-cap plan is the
/// S15 cost-amplification vector and MUST fail closed (reject the turn) — never
/// silently drop to the unbudgeted `ReAct` path, which would reopen exactly what
/// the cap closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanParseError {
    /// The model did not emit parseable plan JSON — degrade to `ReAct`.
    Unparseable(String),
    /// The plan exceeds [`MAX_PLAN_STEPS`] — fail closed.
    TooManySteps {
        /// The step count the model emitted.
        steps: usize,
        /// The enforced cap.
        max: usize,
    },
}

/// The step ids referenced (transitively through arrays/objects) by an argument map.
#[must_use]
pub fn referenced_steps(args: &BTreeMap<String, ArgValue>) -> Vec<StepId> {
    let mut out = Vec::new();
    for value in args.values() {
        collect_refs(value, &mut out);
    }
    out
}

fn collect_refs(value: &ArgValue, out: &mut Vec<StepId>) {
    match value {
        ArgValue::Ref(sym) => out.push(sym.step),
        ArgValue::Array(items) => {
            for item in items {
                collect_refs(item, out);
            }
        }
        ArgValue::Object(fields) => {
            for (_, val) in fields {
                collect_refs(val, out);
            }
        }
        ArgValue::Literal(_) => {}
    }
}

/// Why runtime argument resolution failed (a bug or a plan that passed
/// verification but referenced an unbound/absent value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// A `SymRef` named a step that has not produced a binding.
    UnboundStep(StepId),
    /// A JSON pointer did not resolve within the referenced step's output.
    BadPointer(StepId, String),
}

/// Resolve a step's symbolic args into a concrete JSON object, binding each
/// [`SymRef`] to the actual output recorded in `bindings`.
///
/// # Errors
/// Returns [`ResolveError`] when a reference is unbound or its JSON pointer
/// does not resolve.
pub fn resolve_args<S: BuildHasher>(
    args: &BTreeMap<String, ArgValue>,
    bindings: &HashMap<StepId, Value, S>,
) -> Result<Value, ResolveError> {
    let mut obj = Map::new();
    for (key, value) in args {
        obj.insert(key.clone(), resolve_value(value, bindings)?);
    }
    Ok(Value::Object(obj))
}

fn resolve_value<S: BuildHasher>(
    value: &ArgValue,
    bindings: &HashMap<StepId, Value, S>,
) -> Result<Value, ResolveError> {
    match value {
        ArgValue::Literal(literal) => Ok(literal.clone()),
        ArgValue::Ref(sym) => {
            let base = bindings
                .get(&sym.step)
                .ok_or(ResolveError::UnboundStep(sym.step))?;
            sym.path.as_ref().map_or_else(
                || Ok(base.clone()),
                |pointer| {
                    base.pointer(pointer)
                        .cloned()
                        .ok_or_else(|| ResolveError::BadPointer(sym.step, pointer.clone()))
                },
            )
        }
        ArgValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve_value(item, bindings)?);
            }
            Ok(Value::Array(out))
        }
        ArgValue::Object(fields) => {
            let mut obj = Map::new();
            for (key, val) in fields {
                obj.insert(key.clone(), resolve_value(val, bindings)?);
            }
            Ok(Value::Object(obj))
        }
    }
}
