// ABOUTME: Guardian policy — the two per-turn caps the gate enforces on every turn.
// ABOUTME: Compiled-in defaults, overridden per field by the stored system_settings.guardian_config document.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guardian policy.
//!
//! The effective policy is resolved per snapshot by
//! [`super::config::GuardianConfigRegistry`] as the compiled-in defaults with
//! the stored `system_settings.guardian_config` document over them, field by
//! field. The gate always enforces: a denial blocks the call.

use serde::Serialize;

/// Resolved Guardian policy.
#[derive(Debug, Clone, Serialize)]
pub struct GuardianPolicy {
    /// Max `IRREVERSIBLE` executions allowed per turn (blast-radius cap).
    pub max_destructive_per_turn: u16,
    /// Max `WRITES_DATA` executions allowed per turn (blast-radius cap).
    pub max_writes_per_turn: u16,
}

impl Default for GuardianPolicy {
    fn default() -> Self {
        Self {
            // Blast-radius cap on one human message. Three covers the requests
            // athletes make in one breath (two deletes, publish then push, two
            // disconnects) and still stops a runaway turn: the tool loop would
            // otherwise reach about ten rounds of them.
            max_destructive_per_turn: 3,
            // Generous, because this bounds a runaway injected write flood rather
            // than an ordinary turn: each write call counts once, however many
            // calls a bulk ask takes. Tunable through the stored document.
            max_writes_per_turn: 50,
        }
    }
}
