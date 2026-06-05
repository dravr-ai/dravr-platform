// ABOUTME: Layer 1 deterministic checks — substring assertions, length caps, persona keywords
// ABOUTME: Pure-Rust, runs on 100% of responses without any LLM dependency
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Deterministic Eval Layer
//!
//! Cheap, free, fast checks that gate the LLM-as-judge layer. Per the gist,
//! these run on every response; the LLM judge only runs on responses that
//! pass these checks plus a sampled subset.

use serde::{Deserialize, Serialize};

use crate::fixtures::Turn;

/// Outcome of a single deterministic check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterministicCheck {
    /// Required substring was missing from the response.
    MissingRequired {
        /// Substring that was expected but absent.
        needle: String,
    },
    /// Forbidden substring appeared in the response.
    ForbiddenPresent {
        /// Substring that was forbidden but present.
        needle: String,
    },
    /// Response exceeded the configured character cap.
    TooLong {
        /// Actual character count of the response.
        length: usize,
        /// Configured maximum character count.
        cap: usize,
    },
    /// Response was an empty string.
    Empty,
    /// All checks passed.
    Ok,
}

impl DeterministicCheck {
    /// Convenience: did this check pass?
    #[must_use]
    pub const fn passed(&self) -> bool {
        matches!(self, Self::Ok)
    }
}

/// All deterministic results for one response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeterministicReport {
    /// Per-check results, in evaluation order.
    pub checks: Vec<DeterministicCheck>,
}

impl DeterministicReport {
    /// Run every deterministic check against the given response and return
    /// the collected report. Stops accumulating once a failing check is
    /// found in any single category, but still records all categories.
    #[must_use]
    pub fn run(turn: &Turn, response: &str, max_chars: usize) -> Self {
        let mut checks = Vec::new();

        if response.is_empty() {
            checks.push(DeterministicCheck::Empty);
            return Self { checks };
        }

        if response.chars().count() > max_chars {
            checks.push(DeterministicCheck::TooLong {
                length: response.chars().count(),
                cap: max_chars,
            });
        }

        for needle in &turn.must_contain {
            if !contains_case_insensitive(response, needle) {
                checks.push(DeterministicCheck::MissingRequired {
                    needle: needle.clone(),
                });
            }
        }

        for needle in &turn.must_not_contain {
            if contains_case_insensitive(response, needle) {
                checks.push(DeterministicCheck::ForbiddenPresent {
                    needle: needle.clone(),
                });
            }
        }

        if checks.is_empty() {
            checks.push(DeterministicCheck::Ok);
        }
        Self { checks }
    }

    /// True when every check passed.
    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.checks.iter().all(DeterministicCheck::passed)
    }

    /// Count of failing checks.
    #[must_use]
    pub fn failure_count(&self) -> usize {
        self.checks.iter().filter(|c| !c.passed()).count()
    }
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}
