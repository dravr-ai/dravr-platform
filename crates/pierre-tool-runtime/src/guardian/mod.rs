// ABOUTME: The Guardian — dispatch-time guard that gates tool egress on per-turn budgets and tenant tool-disable.
// ABOUTME: Pure decide() for the budgets and the external-send deny (mode-gated) + async tool-disable (always enforced).

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Guardian
//!
//! A single Guardian is interposed at
//! [`crate::protocol::executor::UniversalExecutor::execute_tool`], the one
//! chokepoint every transport (chat `ReAct` loop, MCP-direct, A2A, an ACP
//! agent's tool-host session) funnels through.
//!
//! It caps blast radius. It does not detect prompt injection: one human message
//! gets a bounded number of irreversible and data-writing calls, whatever asked
//! for them, and the next message gets a fresh budget. A one-shot MCP or A2A call
//! is its own turn, so these budgets never accumulate across such calls.
//!
//! Two kinds of gate live here:
//! - **Tool-disable** ([`caller_tool_enabled`], [`tenant_tool_enabled`]) — an
//!   operator explicitly disabled a tool, for the tenant or for the caller;
//!   this is configuration, always enforced regardless of [`GuardianMode`].
//! - **Budget / egress** ([`Guardian::decide`]) — budgets are gated by
//!   [`GuardianMode`]: `observe` logs would-be denials, `enforce` blocks. An external
//!   send is refused in both modes. Under `enforce` a turn's
//!   first block also stops it committing: the athlete's reply is settled, so
//!   later writes are refused while reads go on.

pub mod config;
pub mod impact;
pub mod policy;
pub mod state;

pub use config::{
    parse_persisted_document, validate_document, GuardianConfigDocument, GuardianConfigRegistry,
    GuardianConfigSnapshot, GuardianConfigSource, GuardianFieldSource, GuardianFieldSources,
    GuardianReloadOutcome, PersistedGuardianDocument, GUARDIAN_CONFIG_SCHEMA_VERSION,
    GUARDIAN_CONFIG_SETTING_KEY,
};
pub use impact::{high_impact_changes, GuardianImpact};
pub use policy::{validate_settings, GuardianEnvOverrides, GuardianMode, GuardianPolicy};
pub use state::{CallFate, GuardianTurns, HeadlessBlock, ReadFingerprint, TurnKey, TurnState};

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
}

/// Why the Guardian denied a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// The per-turn consequential-action budget is exhausted.
    BudgetExceeded,
    /// The tool sends data to a third party. None exist today, and one that
    /// ships stays refused until a decision about its egress replaces this.
    EgressForbidden,
    /// An earlier call in this turn was denied, so the reply the athlete gets is
    /// about that call and nothing further may commit behind it. Reads stay
    /// allowed, so the model can finish its answer.
    TurnBlocked,
}

impl DenyReason {
    /// Stable `snake_case` label for structured logs and the `guardian_denied`
    /// machine code (never user-facing prose).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BudgetExceeded => "budget_exceeded",
            Self::EgressForbidden => "egress_forbidden",
            Self::TurnBlocked => "turn_blocked",
        }
    }
}

/// The outcome of the budget/egress gate at one dispatch point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateOutcome {
    /// The call may run — allowed, or a would-deny in `observe`, which logs the
    /// flag and executes anyway.
    Proceed,
    /// Blocked: `enforce` mode saw a denial. Carries the reason so the caller
    /// renders the transport-appropriate refusal.
    Blocked(DenyReason),
}

/// Run the budget/egress decision + atomic budget reserve for one dispatch,
/// applying the active [`GuardianMode`].
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
) -> (GateOutcome, bool) {
    // External send is refused in every mode: setting a mode must not open egress.
    let mode_blocks = guardian.mode().blocks() || labels.is_external_send();
    let (decision, reserved) = turns.decide_and_reserve(
        turn_key,
        labels,
        writes_data,
        |turn_state| guardian.decide(labels, writes_data, turn_state),
        // The call runs on an allow, and on a would-deny in observe. Only a block
        // latches the turn, so an observed would-deny never does (carnet#1397).
        |decision| {
            if matches!(decision, Decision::Allow) || !mode_blocks {
                CallFate::Runs
            } else {
                CallFate::Blocked
            }
        },
    );
    if let Decision::Deny(reason) = decision {
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
    (GateOutcome::Proceed, reserved)
}

/// The dispatch-time guard. Holds the resolved [`GuardianPolicy`]; the per-turn
/// state it reads lives in the shared [`GuardianTurns`] store.
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

    /// Decide whether to allow a tool call, given the tool's [`SecurityLabels`],
    /// whether it is a `WRITES_DATA` sink, and the current [`TurnState`].
    ///
    /// Pure and synchronous. The decision is mode-independent; the caller
    /// applies [`GuardianMode`] (observe logs, enforce blocks). The separate
    /// tool-disable gate ([`caller_tool_enabled`] at the dispatch chokepoint,
    /// [`tenant_tool_enabled`] inside a tool) is always enforced.
    ///
    /// A turn already blocked is checked first, ahead of the budgets. The reply
    /// is settled by then, so the later call's refusal must not vary with what
    /// is left of a budget, and anyone grouping `budget_exceeded` must not
    /// count the calls an earlier denial stopped. Only enforce mode latches a
    /// turn, so observe never reaches this arm; a mode flipped mid-turn reads it
    /// as one more would-deny and lets the call run.
    #[must_use]
    pub fn decide(&self, labels: SecurityLabels, writes_data: bool, turn: &TurnState) -> Decision {
        // The turn stopped committing at its first block. Reads carry on so the
        // model can finish.
        if turn.is_blocked()
            && (writes_data || labels.is_irreversible() || labels.is_external_send())
        {
            return Decision::Deny(DenyReason::TurnBlocked);
        }

        if labels.is_irreversible()
            && turn.destructive_count() >= self.policy.max_destructive_per_turn
        {
            return Decision::Deny(DenyReason::BudgetExceeded);
        }
        if writes_data && turn.write_count() >= self.policy.max_writes_per_turn {
            return Decision::Deny(DenyReason::BudgetExceeded);
        }
        if labels.is_external_send() {
            return Decision::Deny(DenyReason::EgressForbidden);
        }

        Decision::Allow
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
    use super::*;

    #[test]
    fn the_destructive_cap_allows_three_calls_and_denies_the_fourth() {
        let guardian = Guardian::new(GuardianPolicy::default());
        let mut turn = TurnState::default();
        for _ in 0..3 {
            assert_eq!(
                guardian.decide(SecurityLabels::IRREVERSIBLE, false, &turn),
                Decision::Allow,
                "the default cap covers three destructive calls"
            );
            turn.reserve(SecurityLabels::IRREVERSIBLE, false);
        }
        assert_eq!(
            guardian.decide(SecurityLabels::IRREVERSIBLE, false, &turn),
            Decision::Deny(DenyReason::BudgetExceeded),
            "the fourth destructive call is over the default cap"
        );
    }

    #[test]
    fn observe_runs_an_over_budget_call_and_still_refuses_an_external_send() {
        let guardian = Guardian::new(GuardianPolicy {
            mode: GuardianMode::Observe,
            max_destructive_per_turn: 0,
            ..GuardianPolicy::default()
        });
        let turns = GuardianTurns::new();
        let key = TurnKey::new(None, "observed-turn");
        let gate = |labels| guardian_gate(&guardian, &turns, &key, "a_tool", labels, false);

        assert_eq!(
            gate(SecurityLabels::IRREVERSIBLE).0,
            GateOutcome::Proceed,
            "observe only logs a budget denial"
        );
        assert_eq!(
            gate(SecurityLabels::EXTERNAL_SEND).0,
            GateOutcome::Blocked(DenyReason::EgressForbidden),
            "a mode must not open egress"
        );
    }

    #[test]
    fn an_external_send_is_refused_in_every_turn() {
        let guardian = Guardian::new(GuardianPolicy::default());
        assert_eq!(
            guardian.decide(SecurityLabels::EXTERNAL_SEND, false, &TurnState::default()),
            Decision::Deny(DenyReason::EgressForbidden),
            "external send has no allowlist: a clean turn is refused too"
        );
    }
}
