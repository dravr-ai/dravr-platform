// ABOUTME: How a chat turn sends its tool catalogue, read from the environment once at startup
// ABOUTME: PIERRE_CHAT_DEFER_TOOLS: send a small core loaded and let the model load the rest by name
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};
use std::env;

/// The environment variable that turns deferred tool loading off.
pub const DEFER_TOOLS_VAR: &str = "PIERRE_CHAT_DEFER_TOOLS";

/// How a chat turn sends its tools, as the environment set it at startup.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatToolSettings {
    /// `PIERRE_CHAT_DEFER_TOOLS`: send every tool outside the loaded core as
    /// deferred, on a model the tool runtime names as one that reaches a
    /// deferred tool reliably (Opus today), for Anthropic's tool search to
    /// load when the model looks it up by name. Every other model is sent
    /// every tool loaded either way. On unless the variable says `off`,
    /// `false` or `0`, so a typo never turns it off.
    pub defer_tools: bool,
}

impl Default for ChatToolSettings {
    fn default() -> Self {
        Self { defer_tools: true }
    }
}

impl ChatToolSettings {
    /// Read the settings from the environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            defer_tools: env::var(DEFER_TOOLS_VAR).map_or(true, |raw| !turns_off(&raw)),
        }
    }
}

/// Whether `raw` is one of the values that switch a default-on setting off.
fn turns_off(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "off" | "false" | "0"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_explicit_off_value_turns_it_off() {
        for off in ["off", "false", "0", " OFF ", "False"] {
            assert!(turns_off(off), "{off:?} must turn it off");
        }
        for on in ["on", "true", "1", "", "of", "no thanks"] {
            assert!(!turns_off(on), "{on:?} must leave it on");
        }
    }

    #[test]
    fn it_is_on_by_default() {
        assert!(
            ChatToolSettings::default().defer_tools,
            "deferred tool loading must be on unless an operator turns it off"
        );
    }
}
