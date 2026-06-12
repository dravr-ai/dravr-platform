// ABOUTME: Per-turn tool intent pre-filter configuration from environment variables.
// ABOUTME: Gates chat tool-set narrowing (default OFF) so it ships dark and is A/B-comparable.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;

/// Default lower bound on kept tools before the selector falls back to the full
/// set rather than starve a turn.
const DEFAULT_MIN_KEEP: usize = 8;

/// Configuration for the per-turn tool intent pre-filter.
///
/// When `enabled`, the chat pipeline narrows the chat-callable tool set to the
/// tools relevant to each turn (by message intent and active coach scope) before
/// sending them to the LLM. Disabled by default so the feature ships dark and
/// can be A/B-compared via `PIERRE_TOOL_INTENT_PREFILTER_ENABLED`.
#[derive(Debug, Clone)]
pub struct ToolIntentPrefilterConfig {
    /// Whether per-turn tool narrowing is active. Defaults to `false`.
    pub enabled: bool,
    /// Lower bound on kept tools; below this the selector returns the full set.
    pub min_keep: usize,
}

impl Default for ToolIntentPrefilterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_keep: DEFAULT_MIN_KEEP,
        }
    }
}

impl ToolIntentPrefilterConfig {
    /// Load tool intent pre-filter configuration from environment variables.
    ///
    /// # Environment Variables
    ///
    /// - `PIERRE_TOOL_INTENT_PREFILTER_ENABLED`: `1` / `true` / `yes` to enable
    ///   per-turn narrowing. Defaults to off.
    /// - `PIERRE_TOOL_INTENT_PREFILTER_MIN_KEEP`: integer lower bound on kept
    ///   tools before falling back to the full set. Defaults to `8`.
    #[must_use]
    pub fn from_env() -> Self {
        let enabled = env::var("PIERRE_TOOL_INTENT_PREFILTER_ENABLED")
            .is_ok_and(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes"));
        let min_keep = env::var("PIERRE_TOOL_INTENT_PREFILTER_MIN_KEEP")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(DEFAULT_MIN_KEEP);

        Self { enabled, min_keep }
    }
}
