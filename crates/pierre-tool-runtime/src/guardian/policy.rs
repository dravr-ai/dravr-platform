// ABOUTME: Guardian policy — mode (off/observe/enforce), per-turn budgets, egress allowlist, taint-rule severities.
// ABOUTME: Loaded from GUARDIAN_* env vars; ENFORCE-by-default (a guard that blocks nothing protects nothing).

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guardian policy and its configuration.
//!
//! The effective policy is resolved per snapshot by
//! [`super::config::GuardianConfigRegistry`] as compiled-in defaults ← the
//! persisted `system_settings.guardian_config` document ← `GUARDIAN_*` env
//! overrides ([`GuardianEnvOverrides`] — env wins per field, so a fleet-wide
//! env pin beats a runtime admin edit). The default is `enforce`: denials are
//! applied, with `observe` (compute and log would-be denials, never block)
//! available as the telemetry and rollback posture. The mode only changes how
//! a decision is *applied*, never the decision logic, so observe and enforce
//! exercise the same code path.

use std::collections::HashSet;

use pierre_config::guardian::GuardianSettings;
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tracing::error;
use uuid::Uuid;

/// How Guardian decisions are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardianMode {
    /// Kill switch: compute the decision for debug visibility, never block.
    Off,
    /// Compute + `warn!`-log would-be denials, but never block — the
    /// telemetry and rollback posture.
    Observe,
    /// Default: apply denials — a blocked tool returns an in-band Guardian error.
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
        if trimmed.eq_ignore_ascii_case("none") {
            return Self::None;
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

    /// Whether `raw` parses without silently dropping anything: the keywords
    /// `all` / `none`, or a comma-separated list where **every** entry is a
    /// valid UUID. The lenient [`Self::parse`] drops bad entries (fail-closed
    /// but silent); [`validate_settings`] uses this strict form to fail the boot on
    /// a typo'd tenant id instead.
    #[must_use]
    pub fn parses_strictly(raw: &str) -> bool {
        let trimmed = raw.trim();
        trimmed.eq_ignore_ascii_case("all")
            || trimmed.eq_ignore_ascii_case("none")
            || (!trimmed.is_empty()
                && trimmed
                    .split(',')
                    .all(|s| Uuid::parse_str(s.trim()).is_ok()))
    }
}

/// JSON form: the string `"none"` / `"all"`, or an array of tenant UUIDs.
/// An empty array normalizes to [`ExternalSendAllowlist::None`].
impl Serialize for ExternalSendAllowlist {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::None => serializer.serialize_str("none"),
            Self::All => serializer.serialize_str("all"),
            Self::Only(set) => {
                // Sorted for a deterministic wire form (HashSet order is not).
                let mut ids: Vec<String> = set.iter().map(Uuid::to_string).collect();
                ids.sort_unstable();
                ids.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for ExternalSendAllowlist {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Keyword(String),
            Tenants(Vec<Uuid>),
        }
        match Repr::deserialize(deserializer)? {
            Repr::Keyword(k) if k.eq_ignore_ascii_case("none") => Ok(Self::None),
            Repr::Keyword(k) if k.eq_ignore_ascii_case("all") => Ok(Self::All),
            Repr::Keyword(k) => Err(D::Error::custom(format!(
                "external_send must be \"none\", \"all\", or an array of tenant UUIDs; got \"{k}\""
            ))),
            Repr::Tenants(ids) if ids.is_empty() => Ok(Self::None),
            Repr::Tenants(ids) => Ok(Self::Only(ids.into_iter().collect())),
        }
    }
}

/// How the Guardian treats an `IRREVERSIBLE` sink invoked in a tainted turn.
///
/// `Log` only (default) because deleting an agent after viewing activities is
/// plausibly legitimate — Phase 1 measures the base rate before escalating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaintedDestructive {
    /// Allow but record a would-be denial (telemetry).
    #[default]
    Log,
    /// Park the call pending explicit user consent: the executor stores it in
    /// `guardian_pending_actions` and the user resolves it with `/confirm` or
    /// `/deny` (expiry checked at resolution).
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
#[derive(Debug, Clone, Serialize)]
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
            // ENFORCE by default. A guard in `observe` blocks nothing, so the
            // protection only exists once denials are applied — shipping dark is
            // the same "install the guard before the sink" argument turned against
            // itself. What actually bites today is ONLY the budget caps: the
            // injection rules (tainted→external-send, egress) are 0-impact until
            // send tools ship, and tainted-destructive stays `Log`. So
            // enforce-default is safe with the write cap set generously below.
            mode: GuardianMode::Enforce,
            // Blast-radius cap: one destructive action per turn. A legit
            // multi-disconnect ("Strava and Garmin") is done one at a time — rare
            // and recoverable, worth the ceiling on a fully-injected turn.
            max_destructive_per_turn: 1,
            // Generous enough to clear a legit bulk turn (an agent setting goals
            // across a group) while still bounding a runaway injected write flood.
            // Tunable via GUARDIAN_MAX_WRITES_PER_TURN.
            max_writes_per_turn: 50,
            external_send: ExternalSendAllowlist::None,
            tainted_destructive: TaintedDestructive::Log,
            plan_mode: PlanMode::Off,
        }
    }
}

/// Per-field `GUARDIAN_*` overrides, parsed from the values the server's
/// configuration captured at startup ([`GuardianSettings`]).
///
/// [`super::config::GuardianConfigRegistry`] is built over them once. A field
/// is `Some` only when its variable was set **and** parses; a present-but-invalid
/// value logs at `error!` and counts as absent — the boot gate
/// [`validate_settings`] fails the process before traffic in that case, so the
/// lenient fallback only ever runs in non-server contexts.
#[derive(Debug, Clone, Default)]
pub struct GuardianEnvOverrides {
    /// `GUARDIAN_MODE`.
    pub mode: Option<GuardianMode>,
    /// `GUARDIAN_MAX_DESTRUCTIVE_PER_TURN`.
    pub max_destructive_per_turn: Option<u16>,
    /// `GUARDIAN_MAX_WRITES_PER_TURN`.
    pub max_writes_per_turn: Option<u16>,
    /// `GUARDIAN_EXTERNAL_SEND_TENANTS`.
    pub external_send: Option<ExternalSendAllowlist>,
    /// `GUARDIAN_TAINTED_DESTRUCTIVE`.
    pub tainted_destructive: Option<TaintedDestructive>,
    /// `GUARDIAN_PLAN_MODE`.
    pub plan_mode: Option<PlanMode>,
}

impl GuardianEnvOverrides {
    /// Parse the `GUARDIAN_*` values `settings` captured.
    #[must_use]
    pub fn from_settings(settings: &GuardianSettings) -> Self {
        Self {
            mode: parse_opt(
                "GUARDIAN_MODE",
                settings.mode.as_deref(),
                GuardianMode::parse,
            ),
            max_destructive_per_turn: parse_u16_opt(
                "GUARDIAN_MAX_DESTRUCTIVE_PER_TURN",
                settings.max_destructive_per_turn.as_deref(),
            ),
            max_writes_per_turn: parse_u16_opt(
                "GUARDIAN_MAX_WRITES_PER_TURN",
                settings.max_writes_per_turn.as_deref(),
            ),
            external_send: settings
                .external_send_tenants
                .as_deref()
                .map(ExternalSendAllowlist::parse),
            tainted_destructive: parse_opt(
                "GUARDIAN_TAINTED_DESTRUCTIVE",
                settings.tainted_destructive.as_deref(),
                TaintedDestructive::parse,
            ),
            plan_mode: parse_opt(
                "GUARDIAN_PLAN_MODE",
                settings.plan_mode.as_deref(),
                PlanMode::parse,
            ),
        }
    }

    /// Overlay these overrides onto `base`, returning the effective policy.
    #[must_use]
    pub fn overlay(&self, base: GuardianPolicy) -> GuardianPolicy {
        GuardianPolicy {
            mode: self.mode.unwrap_or(base.mode),
            max_destructive_per_turn: self
                .max_destructive_per_turn
                .unwrap_or(base.max_destructive_per_turn),
            max_writes_per_turn: self.max_writes_per_turn.unwrap_or(base.max_writes_per_turn),
            external_send: self.external_send.clone().unwrap_or(base.external_send),
            tainted_destructive: self.tainted_destructive.unwrap_or(base.tainted_destructive),
            plan_mode: self.plan_mode.unwrap_or(base.plan_mode),
        }
    }
}

/// Whether the plan-then-verify layer (Phase 3) is active.
///
/// `Off` keeps the interleaved `ReAct` loop (the default). `Enforce` runs the
/// verified up-front plan for EVERY provider class — the planner and synthesis
/// calls are plain completions, so SDK-tool-calling providers (Copilot ACP)
/// serve them too and their subprocess loop is bypassed while armed. (A
/// `Shadow` telemetry-only mode was specified but never implemented —
/// `run_tool_loop` only branches on `Enforce` — so it was removed rather than
/// left as a silent no-op.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

fn parse_u16_opt(name: &str, raw: Option<&str>) -> Option<u16> {
    raw.and_then(|raw| {
        raw.trim().parse::<u16>().map_or_else(
            |_| {
                error!(
                    var = name,
                    "GUARDIAN budget value is not a valid integer; ignoring the env override"
                );
                None
            },
            Some,
        )
    })
}

/// Parse a `GUARDIAN_*` enum value, logging at **`error!`** (not silently
/// ignoring) when it is present but unrecognized — so a typo like
/// `GUARDIAN_MODE=enfroce` surfaces at ERROR severity instead of quietly
/// falling back to the laxer resolved value (S6). Absent → `None`, no log.
///
/// This is the last line of defense; the strict gate is [`validate_settings`], which
/// callers run at startup to fail the boot on any unrecognized value rather than
/// serve traffic under a mis-typed security posture.
fn parse_opt<T>(name: &str, raw: Option<&str>, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
    raw.and_then(|raw| {
        let parsed = parse(raw.trim());
        if parsed.is_none() {
            error!(
                var = name,
                "unrecognized GUARDIAN config value; ignoring the env override \
                 (check the spelling — validate_settings fails the boot on this)"
            );
        }
        parsed
    })
}

/// Validate every `GUARDIAN_*` value `settings` captured, returning the list
/// of unrecognized `(name, value)` pairs.
///
/// Runs at server startup so a mis-typed security-posture variable
/// (`GUARDIAN_MODE=enfroce`, `GUARDIAN_TAINTED_DESTRUCTIVE=denny`, …) **fails the
/// boot loudly** instead of silently serving traffic under the laxer default —
/// the fail-fast complement to the `error!` in [`parse_opt`]. Empty vec
/// means every set variable parsed cleanly. Budget vars are validated as `u16`.
#[must_use]
pub fn validate_settings(settings: &GuardianSettings) -> Vec<(String, String)> {
    let mut bad = Vec::new();
    let mut check = |name: &str, raw: Option<&String>, recognized: fn(&str) -> bool| {
        if let Some(raw) = raw {
            if !recognized(raw) {
                bad.push((name.to_owned(), raw.clone()));
            }
        }
    };
    // Each enum var validated against its own parser; each budget var as u16.
    check("GUARDIAN_MODE", settings.mode.as_ref(), |v| {
        GuardianMode::parse(v.trim()).is_some()
    });
    check(
        "GUARDIAN_TAINTED_DESTRUCTIVE",
        settings.tainted_destructive.as_ref(),
        |v| TaintedDestructive::parse(v.trim()).is_some(),
    );
    check("GUARDIAN_PLAN_MODE", settings.plan_mode.as_ref(), |v| {
        PlanMode::parse(v.trim()).is_some()
    });
    // Strict here even though the lenient parse fail-closes (drops bad UUIDs →
    // `None`): a typo'd tenant id in the allowlist should fail the boot loudly,
    // not silently cut every tenant off from external send.
    check(
        "GUARDIAN_EXTERNAL_SEND_TENANTS",
        settings.external_send_tenants.as_ref(),
        ExternalSendAllowlist::parses_strictly,
    );
    let budget = |v: &str| v.trim().parse::<u16>().is_ok();
    check(
        "GUARDIAN_MAX_DESTRUCTIVE_PER_TURN",
        settings.max_destructive_per_turn.as_ref(),
        budget,
    );
    check(
        "GUARDIAN_MAX_WRITES_PER_TURN",
        settings.max_writes_per_turn.as_ref(),
        budget,
    );
    bad
}
