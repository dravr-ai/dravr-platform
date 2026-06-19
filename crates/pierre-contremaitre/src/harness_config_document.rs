// ABOUTME: Persisted harness-config document — compaction tunables + Tier 6 guardrails
// ABOUTME: Data types and validation only; HTTP handlers live in pierre-server::routes::admin::harness_config
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Harness Configuration Document
//!
//! Defines the JSON document persisted under `system_settings.harness_config`
//! and consumed by the [`crate::harness_config_registry::HarnessConfigRegistry`].
//! Bundles the dispatch-time tunables the harness uses — compaction
//! window / thresholds, text guardrails (blocked topics, disclaimers,
//! length cap), and the default fallback behavior when verification
//! fires.
//!
//! HTTP handlers for `GET`/`PUT /admin/settings/harness` live in
//! `pierre-server::routes::admin::harness_config` and import these types.
//! Per-tenant overrides are a Phase D follow-up — the document is
//! currently global across tenants.

use std::collections::HashMap;

use pierre_core::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};

use crate::text_guardrails::{default_locales, LocaleGuardrails};

/// `system_settings` row key the harness config document is persisted under.
pub const HARNESS_CONFIG_SETTING_KEY: &str = "harness_config";

/// Compaction tunables that the dispatch path reads from
/// `pierre_core::config::CompactionConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessCompactionConfig {
    /// Target context window size in tokens (default `128_000`).
    pub window_tokens: u32,
    /// Fraction of `window_tokens` that triggers a summarization pass.
    pub warn_threshold: f32,
    /// Fraction of `window_tokens` that triggers the sliding-window emergency fallback.
    pub emergency_threshold: f32,
    /// How many of the oldest history turns to summarize when warn fires.
    pub summarize_oldest_n: u32,
    /// How many of the oldest history turns to drop under emergency mode.
    pub sliding_drop_n: u32,
    /// Hard cap on non-system messages kept in the prompt — an
    /// estimate-independent backstop against the token heuristic under-counting
    /// dense content. Defaulted so older persisted documents still deserialize.
    #[serde(default = "default_compaction_max_messages")]
    pub max_messages: u32,
}

/// Default for [`HarnessCompactionConfig::max_messages`], also used by serde
/// when an older persisted document omits the field.
fn default_compaction_max_messages() -> u32 {
    40
}

impl Default for HarnessCompactionConfig {
    fn default() -> Self {
        Self {
            window_tokens: 128_000,
            warn_threshold: 0.70,
            emergency_threshold: 0.95,
            summarize_oldest_n: 6,
            sliding_drop_n: 4,
            max_messages: default_compaction_max_messages(),
        }
    }
}

/// Tier 6 text guardrail tunables — locale-aware disclaimer rules,
/// blocked-topic list, length cap.
///
/// The persisted document carries a `locales` map keyed by BCP-47 short
/// code (`en`, `fr`, `es`, `de`, `pt`). Each entry holds the trigger
/// list and disclaimer text in that locale. At dispatch time the chat
/// pipeline resolves the active turn locale; if absent from this map
/// it falls back to `en`; if both are absent, no disclaimer is prepended
/// (the response passes through). This guarantees a wrong-language
/// disclaimer can never leak.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessGuardrailsConfig {
    /// Maximum character length for an outbound coach response. `0` disables the cap.
    pub max_response_chars: u32,
    /// Substrings (case-insensitive) that must not appear in any response.
    pub blocked_topics: Vec<String>,
    /// Per-locale disclaimer triggers + text. Empty map disables the
    /// disclaimer prepend across all locales (responses pass through
    /// untouched).
    pub locales: HashMap<String, LocaleGuardrails>,
}

impl Default for HarnessGuardrailsConfig {
    fn default() -> Self {
        Self {
            max_response_chars: 5_000,
            blocked_topics: Vec::new(),
            locales: default_locales(),
        }
    }
}

/// Top-level document persisted under [`HARNESS_CONFIG_SETTING_KEY`].
///
/// Round-tripped to JSON in `system_settings.value`. Versioned via the
/// `schema_version` field so future migrations can reject documents that
/// predate breaking changes without losing operator data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfigDocument {
    /// Document schema version. Bump when fields are removed or renamed.
    pub schema_version: u32,
    /// Compaction tunables.
    pub compaction: HarnessCompactionConfig,
    /// Text guardrail tunables.
    pub guardrails: HarnessGuardrailsConfig,
}

impl Default for HarnessConfigDocument {
    fn default() -> Self {
        Self {
            // schema_version bumped from 1 to 2 when guardrails moved
            // from flat `disclaimer_triggers`/`disclaimer_text` fields
            // to a per-locale `locales` map. Documents persisted under
            // schema_version 1 fail to deserialize and the registry
            // falls back to compiled-in defaults — operators who had
            // overridden the disclaimer must re-save through the admin
            // UI after upgrade.
            schema_version: 2,
            compaction: HarnessCompactionConfig::default(),
            guardrails: HarnessGuardrailsConfig::default(),
        }
    }
}

/// Reject obviously broken documents before they reach storage.
///
/// The dispatch path can survive most mistuned values, but a few are
/// load-bearing: `warn_threshold < emergency_threshold` so the two-stage
/// compactor runs in order, both thresholds must be in `(0, 1]`, and the
/// disclaimer text cannot be empty when triggers are set (an empty
/// disclaimer with triggers would silently break the medical-fallback
/// behavior).
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] when any invariant fails.
pub fn validate_document(doc: &HarnessConfigDocument) -> AppResult<()> {
    let c = &doc.compaction;
    if c.window_tokens == 0 {
        return Err(AppError::invalid_input("window_tokens must be > 0"));
    }
    if c.warn_threshold <= 0.0 || c.warn_threshold > 1.0 {
        return Err(AppError::invalid_input("warn_threshold must be in (0, 1]"));
    }
    if c.emergency_threshold <= 0.0 || c.emergency_threshold > 1.0 {
        return Err(AppError::invalid_input(
            "emergency_threshold must be in (0, 1]",
        ));
    }
    if c.warn_threshold >= c.emergency_threshold {
        return Err(AppError::invalid_input(
            "warn_threshold must be strictly less than emergency_threshold",
        ));
    }
    if c.max_messages == 0 {
        return Err(AppError::invalid_input("max_messages must be > 0"));
    }
    // Summarization needs `summarize_oldest_n` turns to compact plus 2 to keep
    // (the last user+assistant pair). If that exceeds the message cap, a thread
    // can never accumulate enough turns to summarize before the cap forces a
    // raw slide — silently disabling summarization. Guard the invariant so an
    // admin can't configure the compactor into a permanently-sliding state.
    if c.summarize_oldest_n.saturating_add(2) > c.max_messages {
        return Err(AppError::invalid_input(
            "summarize_oldest_n + 2 must be <= max_messages so summarization can run before the message cap forces a slide",
        ));
    }

    let g = &doc.guardrails;
    for (locale, lg) in &g.locales {
        if !lg.disclaimer_triggers.is_empty() && lg.disclaimer_text.trim().is_empty() {
            return Err(AppError::invalid_input(format!(
                "locale '{locale}': disclaimer_text must be non-empty when disclaimer_triggers is set"
            )));
        }
    }

    Ok(())
}
