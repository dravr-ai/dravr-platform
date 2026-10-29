// ABOUTME: The Guardian — dispatch-time security guard that gates tool egress on taint, budget, and tenant allowlist.
// ABOUTME: Pure decide() for taint/budget/egress (mode-gated) + async tenant tool-disable (always enforced).

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Guardian
//!
//! The runtime taint/allowlist enforcement layer (the cheap, shippable half of
//! Erik Meijer's *Guardians of the Agents*: prompt injection is SQL injection —
//! untrusted data must not become executable intent). A single Guardian is
//! interposed at [`crate::protocol::executor::UniversalExecutor::execute_tool`],
//! the one chokepoint every transport (chat ReAct loop, MCP-direct, A2A, the
//! Copilot-headless `/mcp` loopback) funnels through.
//!
//! It does **not** "solve" injection — it has no symbolic dataflow, so a single
//! tainted ordinary write still goes through. What it does: cap blast radius
//! (a fully-injected turn can do at most N writes / one destructive action),
//! close the unambiguous exfiltration channel (untrusted source → external
//! send), and produce telemetry to calibrate the harder rules before they are
//! enforced. The complete fix is the Phase 3 plan-then-verify layer.
//!
//! Two kinds of gate live here:
//! - **Tool-disable** ([`caller_tool_enabled`], [`tenant_tool_enabled`]) — an
//!   operator explicitly disabled a tool, for the tenant or for the caller;
//!   this is configuration, always enforced regardless of [`GuardianMode`].
//!   Absorbed from the former chat-only `enforce_tool_allowlist` so every
//!   transport honours it.
//! - **Taint / budget / egress** ([`Guardian::decide`]) — security heuristics,
//!   gated by [`GuardianMode`]: `observe` logs would-be denials, `enforce`
//!   blocks.

pub mod config;
pub mod planner;
pub mod policy;
pub mod state;
pub mod verify;
pub mod workflow;

pub use config::{
    validate_document, GuardianConfigDocument, GuardianConfigRegistry, GuardianConfigSnapshot,
    GuardianConfigSource, GuardianFieldSource, GuardianFieldSources,
    GUARDIAN_CONFIG_SCHEMA_VERSION, GUARDIAN_CONFIG_SETTING_KEY,
};
pub use planner::{PlanDenial, StepOutput, WorkflowExecutor};
pub use policy::{
    validate_settings, ExternalSendAllowlist, GuardianEnvOverrides, GuardianMode, GuardianPolicy,
    PlanMode, TaintedDestructive,
};
pub use state::{GuardianTurns, HeadlessBlock, ReadFingerprint, TurnKey, TurnState};
pub use verify::{verify, ToolLabels, VerifyError, VerifyOutcome};
pub use workflow::{ArgValue, PlanParseError, StepId, SymRef, Workflow, WorkflowStep};

use std::sync::Arc;

use pierre_core::errors::{AppResult, ErrorCode};
use tracing::{debug, warn};
use uuid::Uuid;

use pierre_core::models::TenantId;

use crate::runtime::ToolRuntime;
use crate::security::SecurityLabels;

/// Outcome of a Guardian pre-check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The tool may execute.
    Allow,
    /// The tool is denied; the reason drives logging and (in enforce) the
    /// in-band error surfaced to the LLM.
    Deny(DenyReason),
    /// The tool needs explicit user consent before it may run
    /// (`TaintedDestructive::Confirm` on a tainted destructive call): the
    /// executor parks the call and the user resolves it with `/confirm` or
    /// `/deny`. Blocks only in enforce mode — observe/off log and proceed,
    /// exactly like a would-deny.
    ConfirmRequired,
}

impl Decision {
    /// The deny reason, if this is a denial.
    #[must_use]
    pub const fn denied(self) -> Option<DenyReason> {
        match self {
            Self::Allow | Self::ConfirmRequired => None,
            Self::Deny(reason) => Some(reason),
        }
    }
}

/// Why the Guardian denied a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// Untrusted-source content preceded a forbidden sink this turn
    /// (external send always; irreversible when the taint policy is confirm/deny).
    TaintedSink,
    /// The per-turn consequential-action budget is exhausted.
    BudgetExceeded,
    /// The egress allowlist forbids external send for this tenant.
    EgressForbidden,
}

impl DenyReason {
    /// Stable `snake_case` label for structured logs and the `guardian_denied`
    /// machine code (never user-facing prose).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TaintedSink => "tainted_sink",
            Self::BudgetExceeded => "budget_exceeded",
            Self::EgressForbidden => "egress_forbidden",
        }
    }
}

/// The outcome of the taint/budget/egress gate at one dispatch point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateOutcome {
    /// The call may run — allowed, or a would-deny in a non-blocking mode
    /// (`observe`/`off` log the flag and execute anyway).
    Proceed,
    /// Blocked: `enforce` mode saw a denial. Carries the reason so the caller
    /// renders the transport-appropriate refusal.
    Blocked(DenyReason),
    /// Held: `enforce` mode saw a confirm-required decision. The caller parks
    /// the call for `/confirm`·`/deny` resolution and renders the
    /// transport-appropriate confirmation ask.
    ConfirmRequired,
}

/// Run the taint/budget/egress decision + atomic budget reserve for one
/// dispatch, applying the active [`GuardianMode`].
///
/// This is the SINGLE gate every dispatch point shares — the chokepoint
/// [`crate::protocol::executor::UniversalExecutor::execute_tool`], which every
/// transport's registry dispatch passes through — so a special-cased dispatch
/// path cannot silently bypass the guard. Returns the outcome plus whether budget was
/// `reserved`, so a caller that then runs the tool itself can refund on failure
/// exactly as the chokepoint does post-execution.
#[must_use]
pub fn guardian_gate(
    guardian: &Guardian,
    turns: &GuardianTurns,
    turn_key: &TurnKey,
    tool_name: &str,
    labels: SecurityLabels,
    writes_data: bool,
    tenant: Option<Uuid>,
) -> (GateOutcome, bool) {
    let mode_blocks = guardian.mode().blocks();
    // Taint provenance captured under the turn lock when the call is a
    // tainted-destructive sequence the `Log` phase lets through, so the
    // telemetry below is emitted outside the lock.
    let mut logged_taint_sources: Option<Vec<String>> = None;
    let (decision, reserved) = turns.decide_and_reserve(
        turn_key,
        tool_name,
        labels,
        writes_data,
        |turn_state| {
            let decision = guardian.decide(labels, writes_data, tenant, turn_state);
            if guardian.logs_tainted_destructive(labels, turn_state, decision) {
                logged_taint_sources = Some(turn_state.taint_sources().to_vec());
            }
            decision
        },
        // Reserve budget when the call WILL run: on an allow, and on a
        // would-deny/would-confirm in a non-blocking mode (observe/off execute
        // anyway).
        |decision| matches!(decision, Decision::Allow) || !mode_blocks,
    );
    if let Some(taint_sources) = logged_taint_sources {
        // The Phase 1 base-rate signal for `TaintedDestructive`: under `Log` the
        // call proceeds, so this line is the only record that `Confirm` would
        // have parked it. Grouped by `tool_name` and `taint_sources` in Cloud
        // Logging, it is the measurement the confirm phase is armed on.
        warn!(
            tool_name = %tool_name,
            rule = TAINTED_DESTRUCTIVE_RULE,
            phase = "log",
            taint_sources = %taint_sources.join(","),
            tenant_id = ?tenant,
            mode = guardian.mode().as_str(),
            "guardian logged tainted-destructive dispatch"
        );
    }
    match decision {
        Decision::Allow => {}
        Decision::Deny(reason) => {
            warn!(
                tool_name = %tool_name,
                reason = reason.as_str(),
                mode = guardian.mode().as_str(),
                "guardian flagged tool dispatch"
            );
            if mode_blocks {
                return (GateOutcome::Blocked(reason), reserved);
            }
        }
        Decision::ConfirmRequired => {
            warn!(
                tool_name = %tool_name,
                mode = guardian.mode().as_str(),
                "guardian requires user confirmation for tool dispatch"
            );
            if mode_blocks {
                return (GateOutcome::ConfirmRequired, reserved);
            }
        }
    }
    (GateOutcome::Proceed, reserved)
}

/// `rule` field of the `Log`-phase tainted-destructive telemetry line.
const TAINTED_DESTRUCTIVE_RULE: &str = "tainted_destructive";

/// The dispatch-time security guard. Holds the resolved [`GuardianPolicy`];
/// the per-turn state it reads lives in the shared [`GuardianTurns`] store.
#[derive(Debug, Clone)]
pub struct Guardian {
    policy: GuardianPolicy,
}

impl Guardian {
    /// Build a Guardian with an explicit policy.
    #[must_use]
    pub const fn new(policy: GuardianPolicy) -> Self {
        Self { policy }
    }

    /// The active mode (controls whether `decide` denials actually block).
    #[must_use]
    pub const fn mode(&self) -> GuardianMode {
        self.policy.mode
    }

    /// The resolved policy.
    #[must_use]
    pub const fn policy(&self) -> &GuardianPolicy {
        &self.policy
    }

    /// Decide whether to allow a tool call on the **taint / budget / egress**
    /// axis, given the tool's [`SecurityLabels`], whether it is a `WRITES_DATA`
    /// sink, the caller's tenant, and the current [`TurnState`].
    ///
    /// Pure and synchronous. The decision is mode-independent; the caller
    /// applies [`GuardianMode`] (observe logs, enforce blocks). The separate
    /// tool-disable gate ([`caller_tool_enabled`] at the dispatch chokepoint,
    /// [`tenant_tool_enabled`] inside a tool) is always enforced.
    #[must_use]
    pub fn decide(
        &self,
        labels: SecurityLabels,
        writes_data: bool,
        tenant: Option<Uuid>,
        turn: &TurnState,
    ) -> Decision {
        // Tier A — always-on, taint-independent, false-positive-free.
        if labels.is_irreversible()
            && turn.destructive_count() >= self.policy.max_destructive_per_turn
        {
            return Decision::Deny(DenyReason::BudgetExceeded);
        }
        if writes_data && turn.write_count() >= self.policy.max_writes_per_turn {
            return Decision::Deny(DenyReason::BudgetExceeded);
        }
        if labels.is_external_send() && !self.policy.external_send.allows(tenant) {
            return Decision::Deny(DenyReason::EgressForbidden);
        }

        // Tier B — taint -> sink (only once the turn carries untrusted content).
        if turn.is_tainted() {
            if labels.is_external_send() {
                // The exfiltration primitive: untrusted data steering an
                // outbound send. Hard-deny — near-zero false positives.
                return Decision::Deny(DenyReason::TaintedSink);
            }
            if labels.is_irreversible() {
                match self.policy.tainted_destructive {
                    TaintedDestructive::Log => {}
                    TaintedDestructive::Confirm => return Decision::ConfirmRequired,
                    TaintedDestructive::Deny => {
                        return Decision::Deny(DenyReason::TaintedSink);
                    }
                }
            }
        }

        Decision::Allow
    }

    /// Whether `decision` let through an `IRREVERSIBLE` call in a tainted turn
    /// only because the policy is [`TaintedDestructive::Log`] — the sequence
    /// `Confirm` would park and `Deny` would block. A call already denied for
    /// another reason (budget) is not counted: `Confirm` would deny it the same
    /// way, and the gate logs that denial on its own line.
    fn logs_tainted_destructive(
        &self,
        labels: SecurityLabels,
        turn: &TurnState,
        decision: Decision,
    ) -> bool {
        decision == Decision::Allow
            && self.policy.tainted_destructive == TaintedDestructive::Log
            && labels.is_irreversible()
            && turn.is_tainted()
    }
}

/// `error_code` a tenant tool-disable refusal carries: the chat pipeline reads
/// it as an ordinary tool refusal the LLM adapts to, never as a Guardian block.
pub const TENANT_DISABLED_ERROR_CODE: &str = "tool_disabled";

/// `reason` a tenant tool-disable refusal carries beside its error code.
pub const TENANT_DISABLED_REASON: &str = "tenant_disabled";

/// The message answered when `tool_name` is disabled by configuration, for the
/// tenant or for the calling user.
///
/// A nudge to pick another tool, not an incident — and one wording whether
/// the refusal is raised at the dispatch chokepoint or inside a tool that
/// resolved a tenant of its own.
#[must_use]
pub fn tenant_disabled_message(tool_name: &str) -> String {
    format!(
        "Tool '{tool_name}' is disabled for this tenant or this user. Choose a different tool or answer from what you already know."
    )
}

/// Whether `tool_name` is enabled for `tenant` (the absorbed tenant tool-disable allowlist).
///
/// Always enforced regardless of [`GuardianMode`] — it is a tenant's explicit
/// configuration, not a Guardian heuristic. For the caller's own call, the
/// dispatch chokepoint asks [`caller_tool_enabled`] instead, which also honours
/// a per-user override; this tenant-only form is for a tool acting under a
/// tenant other than the caller's, which honours that tenant's configuration.
///
/// On a lookup error the behavior splits (S3): an **uncatalogued** tool
/// (`ResourceNotFound` — e.g. a feature-flag tool deliberately absent from the
/// tenant catalog) has no per-tenant override, so it is **allowed**; a genuine
/// lookup failure on a tool we *do* dispatch **fails closed**, so a transient
/// error cannot silently defeat a tenant's explicit tool-disable. This is the
/// former chat-only `enforce_tool_allowlist`, now applied at the universal
/// chokepoint so MCP-direct / A2A / internal paths honour it too.
pub async fn tenant_tool_enabled(
    resources: &Arc<dyn ToolRuntime>,
    tenant: TenantId,
    tool_name: &str,
) -> bool {
    settle_enablement(
        resources
            .tool_selection()
            .is_tool_enabled(tenant, tool_name)
            .await,
        tenant,
        tool_name,
    )
}

/// Whether `tool_name` is enabled for `user` calling under `tenant`: the
/// tenant's decision with the user's own override above it, the precedence
/// `tools/list` and a chat turn's listings apply (carnet#406).
///
/// A tool switched off for one user is refused when they call it by name, not
/// only hidden from their listings.
///
/// Errors settle as in [`tenant_tool_enabled`].
pub async fn caller_tool_enabled(
    resources: &Arc<dyn ToolRuntime>,
    tenant: TenantId,
    user: Uuid,
    tool_name: &str,
) -> bool {
    settle_enablement(
        resources
            .tool_selection()
            .is_tool_enabled_for_user(tenant, user, tool_name)
            .await,
        tenant,
        tool_name,
    )
}

/// An enablement lookup's answer as a gate: an uncatalogued tool is allowed,
/// a genuine failure fails closed.
fn settle_enablement(lookup: AppResult<bool>, tenant: TenantId, tool_name: &str) -> bool {
    match lookup {
        Ok(enabled) => enabled,
        // Uncatalogued tool (e.g. a feature-flag tool deliberately absent from
        // `tool_catalog`): the lookup returns `ResourceNotFound`, which is not a
        // failure — such tools have no per-tenant disable, so allow (as before).
        Err(e) if e.code == ErrorCode::ResourceNotFound => {
            debug!(
                tenant_id = %tenant,
                tool_name = %tool_name,
                "guardian: tool not in tenant catalog; allowing (no per-tenant override applies)"
            );
            true
        }
        // A genuine lookup failure (DB error, etc.) on a tool we DO dispatch:
        // fail CLOSED (S3) rather than silently allow a possibly-disabled tool
        // through — a transient error must not defeat the tenant allowlist.
        Err(e) => {
            warn!(
                tenant_id = %tenant,
                tool_name = %tool_name,
                error = %e,
                "guardian tenant tool-allowlist lookup failed; failing CLOSED"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Result as IoResult, Write};
    use std::sync::Mutex;

    use tracing::subscriber::with_default;
    use tracing_subscriber::fmt::{fmt as log_fmt, MakeWriter};

    use super::*;

    /// In-memory sink for the lines one gate call logs. The `Arc` is shared by
    /// every writer the subscriber makes and by the test that reads it back.
    #[derive(Clone, Default)]
    struct LogSink(Arc<Mutex<Vec<u8>>>);

    impl Write for LogSink {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            self.0.lock().expect("sink lock").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for LogSink {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Dispatch one `IRREVERSIBLE` call through [`guardian_gate`] under the
    /// `Log` policy, in a turn tainted by `sources`, and return what it logged.
    fn log_phase_gate_output(sources: &[&str]) -> String {
        let sink = LogSink::default();
        let subscriber = log_fmt()
            .with_writer(sink.clone())
            .with_ansi(false)
            .finish();
        let guardian = Guardian::new(policy(TaintedDestructive::Log));
        let turns = GuardianTurns::new();
        let key = TurnKey::new(None, "log-phase-turn");
        for source in sources {
            turns.record_taint(&key, source, SecurityLabels::UNTRUSTED_OUTPUT);
        }
        let (outcome, _reserved) = with_default(subscriber, || {
            guardian_gate(
                &guardian,
                &turns,
                &key,
                "delete_agent",
                SecurityLabels::IRREVERSIBLE,
                false,
                None,
            )
        });
        assert_eq!(outcome, GateOutcome::Proceed);
        let bytes = sink.0.lock().expect("sink lock");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn log_phase_gate_logs_the_tainted_destructive_dispatch_it_lets_through() {
        let log = log_phase_gate_output(&["get_activities", "get_activity_intelligence"]);
        assert!(log.contains(" WARN "), "{log}");
        assert!(
            log.contains("guardian logged tainted-destructive dispatch"),
            "{log}"
        );
        // The fields the arming measurement in feature-phases.yaml groups by.
        assert!(log.contains("tool_name=delete_agent"), "{log}");
        assert!(log.contains(r#"rule="tainted_destructive""#), "{log}");
        assert!(log.contains(r#"phase="log""#), "{log}");
        assert!(
            log.contains("taint_sources=get_activities,get_activity_intelligence"),
            "{log}"
        );
    }

    #[test]
    fn log_phase_gate_logs_nothing_for_an_untainted_destructive_dispatch() {
        let log = log_phase_gate_output(&[]);
        assert!(log.is_empty(), "{log}");
    }

    fn policy(tainted_destructive: TaintedDestructive) -> GuardianPolicy {
        GuardianPolicy {
            tainted_destructive,
            ..GuardianPolicy::default()
        }
    }

    fn tainted_turn() -> TurnState {
        let mut turn = TurnState::default();
        turn.add_taint("get_activities", SecurityLabels::UNTRUSTED_OUTPUT);
        turn
    }

    #[test]
    fn log_phase_records_a_tainted_destructive_call_it_allows() {
        let guardian = Guardian::new(policy(TaintedDestructive::Log));
        let turn = tainted_turn();
        let decision = guardian.decide(SecurityLabels::IRREVERSIBLE, false, None, &turn);
        assert_eq!(decision, Decision::Allow);
        assert!(guardian.logs_tainted_destructive(SecurityLabels::IRREVERSIBLE, &turn, decision));
    }

    #[test]
    fn log_phase_ignores_an_untainted_destructive_call() {
        let guardian = Guardian::new(policy(TaintedDestructive::Log));
        let turn = TurnState::default();
        let decision = guardian.decide(SecurityLabels::IRREVERSIBLE, false, None, &turn);
        assert!(!guardian.logs_tainted_destructive(SecurityLabels::IRREVERSIBLE, &turn, decision));
    }

    #[test]
    fn log_phase_ignores_a_tainted_reversible_write() {
        let guardian = Guardian::new(policy(TaintedDestructive::Log));
        let turn = tainted_turn();
        let decision = guardian.decide(SecurityLabels::empty(), true, None, &turn);
        assert!(!guardian.logs_tainted_destructive(SecurityLabels::empty(), &turn, decision));
    }

    #[test]
    fn log_phase_does_not_count_a_call_the_budget_already_denied() {
        let guardian = Guardian::new(policy(TaintedDestructive::Log));
        let mut turn = tainted_turn();
        turn.record_success("delete_agent", SecurityLabels::IRREVERSIBLE, false);
        let decision = guardian.decide(SecurityLabels::IRREVERSIBLE, false, None, &turn);
        assert_eq!(decision, Decision::Deny(DenyReason::BudgetExceeded));
        assert!(!guardian.logs_tainted_destructive(SecurityLabels::IRREVERSIBLE, &turn, decision));
    }

    #[test]
    fn armed_phases_are_not_counted_as_log_telemetry() {
        let turn = tainted_turn();
        for armed in [TaintedDestructive::Confirm, TaintedDestructive::Deny] {
            let guardian = Guardian::new(policy(armed));
            let decision = guardian.decide(SecurityLabels::IRREVERSIBLE, false, None, &turn);
            assert_ne!(decision, Decision::Allow);
            assert!(!guardian.logs_tainted_destructive(
                SecurityLabels::IRREVERSIBLE,
                &turn,
                decision
            ));
        }
    }
}
