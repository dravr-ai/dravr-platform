// ABOUTME: Guardian policy — mode (observe/enforce) and the per-turn budgets.
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
//! applied, with `observe` (compute and log would-be budget denials) available as
//! the telemetry and rollback posture. An external send is refused in both. The
//! mode only changes how a decision is *applied*, never the decision logic, so
//! observe and enforce exercise the same code path.

use pierre_config::guardian::GuardianSettings;
use serde::{Deserialize, Serialize};
use tracing::error;

/// How Guardian decisions are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardianMode {
    /// Compute + `warn!`-log would-be budget denials without blocking — the
    /// telemetry and rollback posture. An external send is still refused.
    Observe,
    /// Default: apply denials — a blocked tool returns an in-band Guardian error.
    Enforce,
}

impl GuardianMode {
    /// Whether a budget `Deny` decision should actually block in this mode.
    #[must_use]
    pub const fn blocks(self) -> bool {
        matches!(self, Self::Enforce)
    }

    /// Stable lowercase label for structured logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Enforce => "enforce",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "observe" => Some(Self::Observe),
            "enforce" => Some(Self::Enforce),
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
}

impl Default for GuardianPolicy {
    fn default() -> Self {
        Self {
            // ENFORCE by default. A guard in `observe` blocks nothing, so the
            // protection only exists once denials are applied. Only the budgets
            // bite, so enforce-default is safe with the caps set for real turns.
            mode: GuardianMode::Enforce,
            // Blast-radius cap on one human message. Three covers the requests
            // athletes make in one breath (two deletes, publish then push, two
            // disconnects) and still stops a runaway turn: the tool loop would
            // otherwise reach about ten rounds of them.
            max_destructive_per_turn: 3,
            // Generous, because this bounds a runaway injected write flood rather
            // than an ordinary turn: each write call counts once, however many
            // calls a bulk ask takes. Tunable via GUARDIAN_MAX_WRITES_PER_TURN.
            max_writes_per_turn: 50,
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
            max_writes_per_turn: parse_opt(
                "GUARDIAN_MAX_WRITES_PER_TURN",
                settings.max_writes_per_turn.as_deref(),
                parse_write_budget,
            ),
        }
    }
}

/// A write budget the policy can run under. Zero is an outage switch, not a
/// budget: it would deny every write-tool dispatch on the platform.
fn parse_write_budget(raw: &str) -> Option<u16> {
    raw.trim().parse::<u16>().ok().filter(|budget| *budget > 0)
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
/// (`GUARDIAN_MODE=enfroce`, …) **fails the
/// boot loudly** instead of silently serving traffic under the laxer default —
/// the fail-fast complement to the `error!` in [`parse_opt`]. Empty vec
/// means every set variable parsed cleanly. Budget vars are validated as `u16`,
/// and the write budget must also be positive: `GUARDIAN_MAX_WRITES_PER_TURN=0`
/// would deny every write-tool dispatch. `GUARDIAN_MAX_DESTRUCTIVE_PER_TURN=0`
/// stays legal, since denying irreversible tools is a posture a super admin can
/// mean.
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
    // The mode against its parser; each budget var as u16.
    check("GUARDIAN_MODE", settings.mode.as_ref(), |v| {
        GuardianMode::parse(v.trim()).is_some()
    });
    let budget = |v: &str| v.trim().parse::<u16>().is_ok();
    check(
        "GUARDIAN_MAX_DESTRUCTIVE_PER_TURN",
        settings.max_destructive_per_turn.as_ref(),
        budget,
    );
    check(
        "GUARDIAN_MAX_WRITES_PER_TURN",
        settings.max_writes_per_turn.as_ref(),
        |v| parse_write_budget(v).is_some(),
    );
    bad
}
