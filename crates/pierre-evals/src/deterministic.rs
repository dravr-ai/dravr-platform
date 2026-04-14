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

#[cfg(test)]
mod tests {
    use super::{DeterministicCheck, DeterministicReport};
    use crate::fixtures::Turn;

    fn turn(must: Vec<&str>, must_not: Vec<&str>) -> Turn {
        Turn {
            user: String::new(),
            expected_coach: String::new(),
            must_contain: must.into_iter().map(str::to_owned).collect(),
            must_not_contain: must_not.into_iter().map(str::to_owned).collect(),
        }
    }

    #[test]
    fn empty_response_fails() {
        let report = DeterministicReport::run(&turn(vec![], vec![]), "", 5_000);
        assert_eq!(report.checks, vec![DeterministicCheck::Empty]);
        assert!(!report.all_passed());
    }

    #[test]
    fn missing_required_substring_fails() {
        let t = turn(vec!["disclaimer"], vec![]);
        let report = DeterministicReport::run(&t, "your training looks fine", 5_000);
        assert!(matches!(
            report.checks[0],
            DeterministicCheck::MissingRequired { .. }
        ));
        assert!(!report.all_passed());
    }

    #[test]
    fn forbidden_substring_fails() {
        let t = turn(vec![], vec!["definitely cured"]);
        let report = DeterministicReport::run(&t, "you are definitely cured", 5_000);
        assert!(matches!(
            report.checks[0],
            DeterministicCheck::ForbiddenPresent { .. }
        ));
    }

    #[test]
    fn too_long_fails() {
        let t = turn(vec![], vec![]);
        let response = "x".repeat(10);
        let report = DeterministicReport::run(&t, &response, 5);
        assert!(matches!(
            report.checks[0],
            DeterministicCheck::TooLong { .. }
        ));
    }

    #[test]
    fn all_pass() {
        let t = turn(vec!["plan"], vec!["never"]);
        let report = DeterministicReport::run(&t, "Here is your training plan.", 5_000);
        assert_eq!(report.checks, vec![DeterministicCheck::Ok]);
        assert!(report.all_passed());
        assert_eq!(report.failure_count(), 0);
    }
}
