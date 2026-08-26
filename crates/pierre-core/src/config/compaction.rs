// ABOUTME: Tier 1 conversation compactor tuning — context-window thresholds and summarize counts
// ABOUTME: Pure config type with no service-layer dependencies; consumed by pierre-services::conversation_compaction
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tuning knobs for the Tier 1 conversation compactor.
//!
//! Lives in `pierre-core` so the harness config registry in `pierre-server`
//! and the compactor service in `pierre-services` can both name the same
//! type without creating a workspace dependency cycle. The compaction
//! *logic* (LLM call, persistence, sliding window) stays in
//! `pierre_services::conversation_compaction`.

/// Tuning for the conversation compactor.
///
/// Defaults mirror the coaching harness gist: 128K context window with a
/// 70% warn and 95% emergency threshold. Callers override per-tenant via
/// admin config in Tier 6.
#[derive(Debug, Clone, Copy)]
pub struct CompactionConfig {
    /// Target context window size in tokens. When unset we assume the
    /// conservative floor across Gemini / Groq / local models.
    pub window_tokens: u32,
    /// Fraction of the window that triggers a summarization pass.
    pub warn_threshold: f32,
    /// Fraction of the window that triggers the sliding-window emergency
    /// fallback. Must be strictly greater than `warn_threshold`.
    pub emergency_threshold: f32,
    /// How many of the oldest history turns we summarize when we trigger.
    pub summarize_oldest_n: usize,
    /// How many of the oldest history turns we drop under emergency sliding
    /// window mode. Acts as a minimum-drop floor so a barely-over thread does
    /// not re-trigger emergency on every subsequent turn.
    pub sliding_drop_n: usize,
    /// Hard cap on the number of non-system messages kept in the prompt,
    /// enforced independently of the token estimate.
    ///
    /// Budgeting reads [`crate::tokens::estimate_context_tokens`], which prices
    /// dense JSON near two characters per token instead of four. That closes
    /// the gap the flat heuristic left: through August 2026 it charged
    /// JSON-heavy prompts half price, so prompts averaging 161k real tokens
    /// estimated at ~80k, stayed under the 89.6k warn line, and never
    /// summarized — leaving this cap as the only bound on the prompt, and forty
    /// dense messages is roughly the 161k that was observed. The estimate is
    /// still a character heuristic rather than a tokenizer, so the cap stays as
    /// the estimate-independent backstop.
    pub max_messages: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            window_tokens: 128_000,
            warn_threshold: 0.70,
            emergency_threshold: 0.95,
            summarize_oldest_n: 6,
            sliding_drop_n: 4,
            max_messages: 40,
        }
    }
}

impl CompactionConfig {
    /// Token count at which summarization becomes active.
    ///
    /// Rounded from `window_tokens * warn_threshold` to avoid `f32` precision
    /// artifacts (e.g., `128_000 * 0.70f32` evaluates to 89599.999… in `f64`).
    #[must_use]
    pub fn warn_tokens(&self) -> u32 {
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let v = (f64::from(self.window_tokens) * f64::from(self.warn_threshold)).round() as u32;
        v
    }

    /// Token count at which the sliding-window fallback triggers.
    ///
    /// Rounded for the same reason as [`Self::warn_tokens`].
    #[must_use]
    pub fn emergency_tokens(&self) -> u32 {
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let v =
            (f64::from(self.window_tokens) * f64::from(self.emergency_threshold)).round() as u32;
        v
    }
}
