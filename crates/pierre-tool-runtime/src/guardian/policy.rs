// ABOUTME: Guardian policy — mode (off/observe/enforce), per-turn budgets, egress allowlist, taint-rule severities.
// ABOUTME: Loaded from GUARDIAN_* env vars; observe-by-default so Phase 1 ships security-neutral.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guardian policy and its environment-driven configuration.
//!
//! The policy is read once at startup from `GUARDIAN_*` env vars. The default
//! is `observe`: the Guardian computes every decision and logs would-be denials
//! but never blocks, so the layer ships security-neutral and gathers telemetry
//! before enforcement is armed (Phase 2). The mode only changes how a decision
//! is *applied*, never the decision logic, so observe and enforce exercise the
//! same code path.

use std::collections::HashSet;
use std::env;

use tracing::error;
use uuid::Uuid;

/// How Guardian decisions are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardianMode {
    /// Kill switch: compute the decision for debug visibility, never block.
    Off,
    /// Default: compute + `warn!`-log would-be denials, but never block.
    Observe,
    /// Apply denials: a blocked tool returns an in-band Guardian error.
    Enforce,
}

impl GuardianMode {
    /// Whether a `Deny` decision should actually block in this mode.
    #[must_use]
    pub const fn blocks(self) -> bool {
        matches!(self, Self::Enforce)
    }

    /// Stable lowercase label for structured logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Observe => "observe",
            Self::Enforce => "enforce",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "observe" => Some(Self::Observe),
            "enforce" => Some(Self::Enforce),
            _ => None,
        }
    }
}

/// Which tenants may reach an `EXTERNAL_SEND` tool at all (taint-independent
/// egress allowlist; `tenant_id` is server-set and cannot be injected).
#[derive(Debug, Clone, Default)]
pub enum ExternalSendAllowlist {
    /// No tenant may reach external-send tools (default — none exist yet).
    #[default]
    None,
    /// Every tenant may (used once messaging is generally available).
    All,
    /// Only these tenants may.
    Only(HashSet<Uuid>),
}

impl ExternalSendAllowlist {
    /// Whether `tenant` is permitted to reach external-send tools.
    #[must_use]
    pub fn allows(&self, tenant: Option<Uuid>) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Only(set) => tenant.is_some_and(|t| set.contains(&t)),
        }
    }

    fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed.eq_ignore_ascii_case("all") {
            return Self::All;
        }
        let set: HashSet<Uuid> = trimmed
            .split(',')
            .filter_map(|s| Uuid::parse_str(s.trim()).ok())
            .collect();
        if set.is_empty() {
            Self::None
        } else {
            Self::Only(set)
        }
    }
}

/// How the Guardian treats an `IRREVERSIBLE` sink invoked in a tainted turn.
///
/// `Log` only (default) because deleting a coach after viewing activities is
/// plausibly legitimate — Phase 1 measures the base rate before escalating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaintedDestructive {
    /// Allow but record a would-be denial (telemetry).
    #[default]
    Log,
    /// Block, asking for confirmation (degrades to deny where no human is present).
    Confirm,
    /// Block outright.
    Deny,
}

impl TaintedDestructive {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "log" => Some(Self::Log),
            "confirm" => Some(Self::Confirm),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// Resolved Guardian policy.
#[derive(Debug, Clone)]
pub struct GuardianPolicy {
    /// How decisions are applied.
    pub mode: GuardianMode,
    /// Max `IRREVERSIBLE` executions allowed per turn (blast-radius cap).
    pub max_destructive_per_turn: u16,
    /// Max `WRITES_DATA` executions allowed per turn (blast-radius cap).
    pub max_writes_per_turn: u16,
    /// Which tenants may reach external-send tools.
    pub external_send: ExternalSendAllowlist,
    /// Severity for `IRREVERSIBLE`-after-untrusted-source.
    pub tainted_destructive: TaintedDestructive,
    /// Whether the LLM emits a verified up-front plan instead of the `ReAct` loop.
    pub plan_mode: PlanMode,
}

impl Default for GuardianPolicy {
    fn default() -> Self {
        Self {
            mode: GuardianMode::Observe,
            max_destructive_per_turn: 1,
            max_writes_per_turn: 5,
            external_send: ExternalSendAllowlist::None,
            tainted_destructive: TaintedDestructive::Log,
            plan_mode: PlanMode::Off,
        }
    }
}

impl GuardianPolicy {
    /// Load the policy from `GUARDIAN_*` env vars, falling back to the
    /// observe-safe [`Default`] for any unset or unparseable var.
    ///
    /// - `GUARDIAN_MODE` = `off` | `observe` (default) | `enforce`
    /// - `GUARDIAN_MAX_DESTRUCTIVE_PER_TURN` (default `1`)
    /// - `GUARDIAN_MAX_WRITES_PER_TURN` (default `5`)
    /// - `GUARDIAN_EXTERNAL_SEND_TENANTS` = `all` | comma-separated tenant UUIDs
    /// - `GUARDIAN_TAINTED_DESTRUCTIVE` = `log` (default) | `confirm` | `deny`
    #[must_use]
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            mode: parse_env_or_error("GUARDIAN_MODE", GuardianMode::parse, defaults.mode),
            max_destructive_per_turn: parse_u16_env(
                "GUARDIAN_MAX_DESTRUCTIVE_PER_TURN",
                defaults.max_destructive_per_turn,
            ),
            max_writes_per_turn: parse_u16_env(
                "GUARDIAN_MAX_WRITES_PER_TURN",
                defaults.max_writes_per_turn,
            ),
            external_send: env::var("GUARDIAN_EXTERNAL_SEND_TENANTS")
                .ok()
                .map_or(ExternalSendAllowlist::None, |v| {
                    ExternalSendAllowlist::parse(&v)
                }),
            tainted_destructive: parse_env_or_error(
                "GUARDIAN_TAINTED_DESTRUCTIVE",
                TaintedDestructive::parse,
                defaults.tainted_destructive,
            ),
            plan_mode: parse_env_or_error(
                "GUARDIAN_PLAN_MODE",
                PlanMode::parse,
                defaults.plan_mode,
            ),
        }
    }
}

/// Whether the plan-then-verify layer (Phase 3) is active.
///
/// `Off` keeps the interleaved `ReAct` loop (the default). `Enforce` runs the
/// verified up-front plan for API/CLI providers; the headless ACP loop owns its
/// own loop and stays on `ReAct`. (A `Shadow` telemetry-only mode was specified
/// but never implemented — `run_tool_loop` only branches on `Enforce` — so it
/// was removed rather than left as a silent no-op.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlanMode {
    /// `ReAct` only (default).
    #[default]
    Off,
    /// Run the verified plan (API/CLI providers).
    Enforce,
}

impl PlanMode {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "enforce" => Some(Self::Enforce),
            _ => None,
        }
    }

    /// Stable lowercase label for structured logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Enforce => "enforce",
        }
    }
}

fn parse_u16_env(name: &str, default: u16) -> u16 {
    env::var(name).ok().map_or(default, |raw| {
        raw.trim().parse::<u16>().unwrap_or_else(|_| {
            error!(
                var = name,
                "GUARDIAN budget value is not a valid integer; using the default"
            );
            default
        })
    })
}

/// Parse a `GUARDIAN_*` enum var, logging at **`error!`** (not silently
/// defaulting) when it is present but unrecognized — so a typo like
/// `GUARDIAN_MODE=enfroce` surfaces at ERROR severity instead of quietly
/// reverting to the laxer default (S6). Absent → default, no log.
///
/// This is the last line of defense; the strict gate is [`validate_env`], which
/// callers run at startup to fail the boot on any unrecognized value rather than
/// serve traffic under a mis-typed security posture.
fn parse_env_or_error<T>(name: &str, parse: impl Fn(&str) -> Option<T>, default: T) -> T {
    match env::var(name) {
        Ok(raw) => parse(raw.trim()).unwrap_or_else(|| {
            error!(
                var = name,
                "unrecognized GUARDIAN config value; using the safe default \
                 (check the spelling — a typo reverts to the laxer default)"
            );
            default
        }),
        Err(_) => default,
    }
}

/// Validate every present `GUARDIAN_*` variable, returning the list of
/// unrecognized `(name, value)` pairs.
///
/// Runs at server startup so a mis-typed security-posture variable
/// (`GUARDIAN_MODE=enfroce`, `GUARDIAN_TAINTED_DESTRUCTIVE=denny`, …) **fails the
/// boot loudly** instead of silently serving traffic under the laxer default —
/// the fail-fast complement to the `error!` in [`parse_env_or_error`]. Empty vec
/// means every set variable parsed cleanly. Budget vars are validated as `u16`.
#[must_use]
pub fn validate_env() -> Vec<(String, String)> {
    let mut bad = Vec::new();
    let mut check = |name: &str, recognized: bool| {
        if let Ok(raw) = env::var(name) {
            if !recognized {
                bad.push((name.to_owned(), raw));
            }
        }
    };
    // Each enum var validated against its own parser; each budget var as u16.
    check(
        "GUARDIAN_MODE",
        env::var("GUARDIAN_MODE").map_or(true, |v| GuardianMode::parse(v.trim()).is_some()),
    );
    check(
        "GUARDIAN_TAINTED_DESTRUCTIVE",
        env::var("GUARDIAN_TAINTED_DESTRUCTIVE")
            .map_or(true, |v| TaintedDestructive::parse(v.trim()).is_some()),
    );
    check(
        "GUARDIAN_PLAN_MODE",
        env::var("GUARDIAN_PLAN_MODE").map_or(true, |v| PlanMode::parse(v.trim()).is_some()),
    );
    for name in [
        "GUARDIAN_MAX_DESTRUCTIVE_PER_TURN",
        "GUARDIAN_MAX_WRITES_PER_TURN",
    ] {
        let ok = env::var(name).map_or(true, |v| v.trim().parse::<u16>().is_ok());
        check(name, ok);
    }
    bad
}
