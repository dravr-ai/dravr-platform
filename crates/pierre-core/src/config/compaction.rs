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
            // Claude Opus 4.8's real context window. The inherited 128_000 was
            // never this model's limit -- it predates the provider -- and the
            // thresholds below were fractions of a number that was wrong by 8x.
            //
            // The fractions absorb that correction rather than the trigger
            // moving: warn stays 89_600 tokens and emergency 121_600, exactly
            // where they sat before. Re-deriving the window without re-deriving
            // the fractions would have pushed warn to 700_000 -- above the
            // 603_498-token peak observed the week of 2026-08-18 -- and
            // compaction, which only became reachable when estimate_context_tokens
            // stopped charging dense JSON half price, would have stopped firing
            // again the same week it started.
            //
            // Where the trigger BELONGS is a cost question, not a correctness
            // one, and it is measurable now: it is an explicit fraction of a true
            // window rather than an artifact of a false one.
            window_tokens: 1_000_000,
            warn_threshold: 0.0896,
            emergency_threshold: 0.1216,
            // 12, not 6. The message cap is 40 and a jammed thread arrives at
            // ~90 messages, so at 6 per turn it takes ~9 turns of the athlete
            // talking to get back under — each one paying a summarization call
            // — before the prompt is the size it is supposed to be. 12 halves
            // that.
            //
            // Not higher: a block covers `summarize_oldest_n` emitted rows, and
            // everything it covers reaches later turns as a summary rather than
            // as the athlete's own words. On 2026-09-02 what kept falling out
            // of the window was his corrections, so the chunk stays small
            // enough that one block cannot swallow a whole exchange.
            summarize_oldest_n: 12,
            sliding_drop_n: 4,
            max_messages: 40,
        }
    }
}

impl CompactionConfig {
    /// Token count at which summarization becomes active.
    ///
    /// Rounded from `window_tokens * warn_threshold` to avoid `f32` precision
    /// artifacts (e.g., `1_000_000 * 0.0896f32` does not land on `89_600` exactly).
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
