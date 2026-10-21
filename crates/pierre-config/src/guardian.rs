// ABOUTME: The GUARDIAN_* tool-dispatch policy overrides, captured from the environment once at startup
// ABOUTME: Raw values only: the tool runtime parses and validates them into the Guardian policy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};
use std::env;

/// The operator's `GUARDIAN_*` overrides as the environment held them at
/// startup, each `None` when its variable is unset.
///
/// The values are kept as written: `pierre-tool-runtime`'s
/// `GuardianEnvOverrides::from_settings` parses them (and `validate_settings`
/// fails the boot on one it does not recognize), so this crate carries the
/// operator's input without knowing the policy's types.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GuardianSettings {
    /// `GUARDIAN_MODE`: `off` | `observe` | `enforce`
    pub mode: Option<String>,
    /// `GUARDIAN_MAX_DESTRUCTIVE_PER_TURN`
    pub max_destructive_per_turn: Option<String>,
    /// `GUARDIAN_MAX_WRITES_PER_TURN`
    pub max_writes_per_turn: Option<String>,
    /// `GUARDIAN_EXTERNAL_SEND_TENANTS`: `none` | `all` | comma-separated tenant UUIDs
    pub external_send_tenants: Option<String>,
    /// `GUARDIAN_TAINTED_DESTRUCTIVE`: `log` | `confirm` | `deny`
    pub tainted_destructive: Option<String>,
    /// `GUARDIAN_PLAN_MODE`: `off` | `enforce`
    pub plan_mode: Option<String>,
}

impl GuardianSettings {
    /// Capture the currently-set `GUARDIAN_*` variables: the one reader of
    /// them in the environment.
    #[must_use]
    pub fn from_env() -> Self {
        let read = |name: &str| env::var(name).ok();
        Self {
            mode: read("GUARDIAN_MODE"),
            max_destructive_per_turn: read("GUARDIAN_MAX_DESTRUCTIVE_PER_TURN"),
            max_writes_per_turn: read("GUARDIAN_MAX_WRITES_PER_TURN"),
            external_send_tenants: read("GUARDIAN_EXTERNAL_SEND_TENANTS"),
            tainted_destructive: read("GUARDIAN_TAINTED_DESTRUCTIVE"),
            plan_mode: read("GUARDIAN_PLAN_MODE"),
        }
    }
}
