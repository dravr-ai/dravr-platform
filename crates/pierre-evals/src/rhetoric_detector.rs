// ABOUTME: The rhetoric filter of the bullshit detector — discards rhetorical / non-propositional utterances
// ABOUTME: Pure Rust, no LLM. Runs on every claim to filter motivational flourishes.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Rhetoric Detector
//!
//! The first gate in the bullshit detector pipeline. Given a sentence
//! or atomic claim extracted from a coach reply, decides whether it is:
//!
//! - **Rhetorical** — motivational ("you're crushing it!"), greeting,
//!   question to the user, or imperative with no factual predicate. Fast
//!   path, `ClaimStatus::Rhetorical`, no further layers run.
//! - **Propositional** — a factual claim that deserves verification.
//!
//! The detector is intentionally conservative: false negatives (letting a
//! rhetorical phrase through) are cheap because the deterministic-bounds
//! and evidence-retrieval layers will also decline to fire a verdict on
//! them. False positives (marking a real
//! claim as rhetorical) are worse because they bypass verification, so the
//! heuristics favour "still verify" over "drop on the floor".

/// Patterns that almost always identify rhetorical or non-propositional text.
/// These are matched case-insensitively against the *start* of the claim after
/// trimming leading punctuation and whitespace.
const RHETORICAL_PREFIXES: &[&str] = &[
    "great job",
    "well done",
    "you got this",
    "keep going",
    "keep it up",
    "crushing it",
    "you're crushing",
    "awesome work",
    "nice work",
    "amazing",
    "fantastic",
    "hi ",
    "hello ",
    "hey ",
    "good morning",
    "good afternoon",
    "good evening",
    "let me know",
    "feel free to",
    "what do you think",
    "how do you feel",
    "thanks for",
    "thank you",
    "absolutely",
    "of course",
    "sure thing",
];

/// Second-order keywords that make a sentence overwhelmingly likely to be a
/// question back to the user rather than a factual claim.
const QUESTION_MARKERS: &[&str] = &[
    "?",
    " do you ",
    " are you ",
    " have you ",
    " did you ",
    " could you ",
    " would you ",
    " can you ",
    " will you ",
];

/// Result of a rhetoric check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RhetoricVerdict {
    /// The claim is rhetorical and should skip deeper verification layers.
    Rhetorical,
    /// The claim is propositional and should pass to the deterministic-bounds layer.
    Propositional,
}

/// Classify a single extracted claim as rhetorical or propositional.
///
/// Empty or whitespace-only strings are considered rhetorical so the caller
/// never wastes an LLM call on them.
#[must_use]
pub fn classify(claim: &str) -> RhetoricVerdict {
    let trimmed = claim.trim_matches(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '.'));
    if trimmed.is_empty() {
        return RhetoricVerdict::Rhetorical;
    }

    let lower = trimmed.to_lowercase();

    if RHETORICAL_PREFIXES
        .iter()
        .any(|p| lower.starts_with(p) || lower.contains(p))
    {
        return RhetoricVerdict::Rhetorical;
    }

    if QUESTION_MARKERS.iter().any(|m| lower.contains(m)) {
        return RhetoricVerdict::Rhetorical;
    }

    // Imperatives with no factual predicate ("try this", "give it a shot")
    // tend to lack any of the category-specific nouns. The deterministic-bounds layer will catch
    // them by scoring zero on all category patterns, so we let them through
    // here — false negatives in this gate are safe by design.
    RhetoricVerdict::Propositional
}
