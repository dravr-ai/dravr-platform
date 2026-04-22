// ABOUTME: Botium-plan assertion helpers for messaging-eval integration tests
// ABOUTME: Pure functions: CitationFixture, assert_citation_grounded, assert_guardrail
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Messaging evaluation assertion helpers.
//!
//! Realizes the Botium plan's assertion-adapter approach as pure Rust
//! functions consumable from integration tests. The spike scope covers
//! the two asserters that do not depend on the per-turn observability
//! endpoint (`CITATION_GROUNDED` and `GUARDRAIL`); the correlation-ID
//! dependent asserters (tool-called, cost, latency) are implemented in
//! a later phase against `GET /internal/conversation-turn/{id}`.
//!
//! ## Citation grounding
//!
//! A coach reply is *grounded* if every numeric claim it makes about
//! the user's data matches the value pre-computed from the seeded
//! activities, within a per-metric tolerance. Two claim kinds are
//! extracted for the spike:
//!
//! - activity count — patterns like `"5 runs"`, `"10 activities"`,
//!   `"7 workouts"`
//! - distance — patterns like `"45.3 km"`, `"120 kilometers"`
//!
//! If the reply contains no extractable claims, the asserter returns
//! `Ok(())`: there is nothing to ground. Callers who want to fail on
//! missing evidence should use [`assert_guardrail`] with an expected
//! refusal string, or extend this module with a "required-claim" rule.
//!
//! ## Guardrail matching
//!
//! A fuzzy substring match: the reply must contain the expected
//! guardrail text after whitespace normalization, or the assertion
//! fails. Guardrail strings come from `dravr-contremaitre`'s
//! `messaging_strings_registry` so test fixtures track the shipping
//! copy.

#![allow(dead_code)] // helpers exist for tests that will land in a follow-up commit

use pierre_mcp_server::models::Activity;

/// Pre-computed expected metrics for a tenant's seeded activities.
///
/// Computed once from the seeded `Vec<Activity>` before the pipeline
/// runs, and passed to [`assert_citation_grounded`] to verify the
/// coach's reply cites these numbers rather than hallucinating.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CitationFixture {
    /// Number of activities in the evaluation window.
    pub activity_count: u32,
    /// Total distance in kilometers across the evaluation window.
    pub total_distance_km: f64,
}

impl CitationFixture {
    /// Derive a fixture from an in-memory slice of activities.
    ///
    /// Sums distance from every activity that reports one; activities
    /// without `distance_meters` (swims without GPS, strength sessions)
    /// contribute 0 to the total but still count toward
    /// [`CitationFixture::activity_count`].
    #[must_use]
    pub fn from_activities(activities: &[Activity]) -> Self {
        let count = u32::try_from(activities.len()).unwrap_or(u32::MAX);
        let total_meters: f64 = activities
            .iter()
            .filter_map(Activity::distance_meters)
            .sum();
        Self {
            activity_count: count,
            total_distance_km: total_meters / 1000.0,
        }
    }
}

/// Per-metric tolerance for citation grounding.
///
/// Counts are always asserted exactly — "6 runs" vs "5 runs" is never
/// a rounding problem and always a content error. Distances allow a
/// relative tolerance to absorb unit-conversion rounding (meters →
/// km, for example).
#[derive(Debug, Clone, Copy)]
pub struct Tolerances {
    /// Relative tolerance for distance claims, as a fraction
    /// (0.02 = ±2%).
    pub distance_pct: f64,
}

impl Tolerances {
    /// Tight tolerances suitable for mock-LLM integration tests where
    /// the reply text is author-controlled and should match the
    /// fixture exactly.
    #[must_use]
    pub const fn strict() -> Self {
        Self {
            distance_pct: 0.001,
        }
    }

    /// Loose tolerances suitable for real-LLM scenarios where the
    /// model rounds distances to whole kilometers.
    #[must_use]
    pub const fn loose() -> Self {
        Self { distance_pct: 0.05 }
    }
}

/// Reason a citation-grounding assertion failed.
#[derive(Debug, Clone, PartialEq)]
pub enum CitationMismatch {
    /// The reply cited an activity count that did not match the
    /// fixture. Counts are always asserted exactly.
    ActivityCountWrong {
        /// Value detected in the reply.
        cited: u32,
        /// Value in the fixture.
        expected: u32,
    },
    /// The reply cited a distance outside the configured relative
    /// tolerance.
    DistanceOutOfTolerance {
        /// Value detected in the reply, in kilometers.
        cited_km: f64,
        /// Value in the fixture, in kilometers.
        expected_km: f64,
        /// Observed absolute fractional difference
        /// (`|cited - expected| / expected`).
        pct_diff: f64,
    },
}

/// Reason a guardrail assertion failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailMismatch {
    /// The (normalized) reply text that was checked.
    pub reply: String,
    /// The (normalized) expected guardrail substring.
    pub expected: String,
}

/// Regex for activity-count claims in a reply.
///
/// Captures a leading integer followed by one of a small set of
/// activity synonyms (run/ride/swim/activity/workout/session) in
/// English or French. Case-insensitive.
fn activity_count_regex() -> regex::Regex {
    // Compiled on each call for simplicity; Phase 1 can switch to
    // LazyLock if this shows up in a hot path.
    regex::Regex::new(
        r"(?i)\b(\d+)\s+(?:runs?|rides?|swims?|activities|activit[ée]s?|workouts?|sessions|s[eé]ances?)\b",
    )
    .expect("activity-count regex compiles")
}

/// Regex for distance claims in kilometers.
fn distance_km_regex() -> regex::Regex {
    regex::Regex::new(r"(?i)\b(\d+(?:[.,]\d+)?)\s*(?:km|kilometers?|kilom[èe]tres?)\b")
        .expect("distance-km regex compiles")
}

/// Normalize a reply for fuzzy comparison: lowercase, collapse runs
/// of whitespace, trim.
fn normalize(text: &str) -> String {
    text.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Verify the reply's numeric claims are grounded in the fixture.
///
/// Scans the reply for activity-count and distance claims, and
/// compares each against the fixture. Returns `Err(CitationMismatch)`
/// on the first claim that disagrees beyond the configured tolerance.
///
/// When the reply contains no extractable claims of a given kind,
/// that kind is silently skipped — this asserter validates that
/// *stated* numbers are correct, not that specific numbers must
/// appear. Pair with [`assert_guardrail`] to enforce presence.
///
/// # Errors
///
/// Returns [`CitationMismatch`] describing the first disagreement.
pub fn assert_citation_grounded(
    reply: &str,
    fixture: &CitationFixture,
    tolerances: Tolerances,
) -> Result<(), CitationMismatch> {
    for cap in activity_count_regex().captures_iter(reply) {
        let Some(n) = cap.get(1).and_then(|m| m.as_str().parse::<u32>().ok()) else {
            continue;
        };
        if n != fixture.activity_count {
            return Err(CitationMismatch::ActivityCountWrong {
                cited: n,
                expected: fixture.activity_count,
            });
        }
    }

    for cap in distance_km_regex().captures_iter(reply) {
        let Some(raw) = cap.get(1) else {
            continue;
        };
        // Accept either "." or "," as the decimal separator to match
        // French locale output.
        let Ok(cited) = raw.as_str().replace(',', ".").parse::<f64>() else {
            continue;
        };
        let expected = fixture.total_distance_km;
        if expected <= 0.0 {
            continue;
        }
        let pct_diff = ((cited - expected).abs()) / expected;
        if pct_diff > tolerances.distance_pct {
            return Err(CitationMismatch::DistanceOutOfTolerance {
                cited_km: cited,
                expected_km: expected,
                pct_diff,
            });
        }
    }

    Ok(())
}

// ── Phase 1 asserters (correlation-ID-dependent) ──────────────────
//
// These operate on the JSON returned by
// `GET /internal/conversation-turn/{turn_id}` (shape:
// [`pierre_core::models::usage::ConversationTurnSummary`]). Tests wire
// them up by POSTing a Slack event, capturing the turn_id from the
// `llm_usage` table, calling the admin endpoint, and passing the parsed
// JSON to these asserters. Operating on `Value` keeps the helpers
// usable by any consumer of the endpoint (Rust tests, future JS Botium
// asserters via an HTTP bridge, operator CLIs).

/// Reason the tool-called assertion failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolNotCalled {
    /// MCP tool name the assertion expected to see.
    pub expected: String,
    /// Tools that were actually invoked during the turn.
    pub actual: Vec<String>,
}

/// Reason a latency-budget assertion failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatencyOverBudget {
    /// Observed end-to-end latency for the turn.
    pub observed_ms: i64,
    /// Caller-supplied budget.
    pub budget_ms: u64,
}

/// Reason a token-budget assertion failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokensOverBudget {
    /// Observed total tokens for the turn.
    pub observed: i64,
    /// Caller-supplied budget.
    pub budget: u64,
}

/// Verify the turn actually invoked the named MCP tool.
///
/// Reads `tools_called` from the endpoint JSON and checks whether
/// `expected_tool` appears in the list. Catches the "plausible-sounding
/// answer without calling the tool" hallucination class.
///
/// # Errors
///
/// Returns [`ToolNotCalled`] containing the expected tool name and the
/// actual list when the expected tool is absent.
pub fn assert_tool_called(
    turn_data: &serde_json::Value,
    expected_tool: &str,
) -> Result<(), ToolNotCalled> {
    let actual: Vec<String> = turn_data
        .get("tools_called")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();

    if actual.iter().any(|name| name == expected_tool) {
        Ok(())
    } else {
        Err(ToolNotCalled {
            expected: expected_tool.to_owned(),
            actual,
        })
    }
}

/// Verify the turn's end-to-end latency stayed within `budget_ms`.
///
/// Reads `total_latency_ms` from the endpoint JSON. When the field is
/// missing or non-numeric the assertion passes (no evidence of a
/// violation to report); callers who want to enforce presence should
/// pair this with [`assert_tool_called`] which fails loudly on a
/// missing turn record.
///
/// # Errors
///
/// Returns [`LatencyOverBudget`] when the observed latency exceeds
/// `budget_ms`.
pub fn assert_latency_ms_at_most(
    turn_data: &serde_json::Value,
    budget_ms: u64,
) -> Result<(), LatencyOverBudget> {
    let Some(observed) = turn_data
        .get("total_latency_ms")
        .and_then(serde_json::Value::as_i64)
    else {
        return Ok(());
    };
    let budget_i64 = i64::try_from(budget_ms).unwrap_or(i64::MAX);
    if observed > budget_i64 {
        Err(LatencyOverBudget {
            observed_ms: observed,
            budget_ms,
        })
    } else {
        Ok(())
    }
}

/// Verify the turn's total token usage stayed within `budget`.
///
/// Reads `total_tokens` from the endpoint JSON. Used as the cost
/// proxy until per-turn cost tracking (pricing tables in embacle's
/// `PerCallMetric`) lands. Missing/non-numeric field → passes.
///
/// # Errors
///
/// Returns [`TokensOverBudget`] when the observed total exceeds
/// `budget`.
pub fn assert_tokens_at_most(
    turn_data: &serde_json::Value,
    budget: u64,
) -> Result<(), TokensOverBudget> {
    let Some(observed) = turn_data
        .get("total_tokens")
        .and_then(serde_json::Value::as_i64)
    else {
        return Ok(());
    };
    let budget_i64 = i64::try_from(budget).unwrap_or(i64::MAX);
    if observed > budget_i64 {
        Err(TokensOverBudget { observed, budget })
    } else {
        Ok(())
    }
}

/// Verify the reply contains the expected guardrail string after
/// whitespace normalization.
///
/// The comparison is case-insensitive and whitespace-tolerant. Use
/// for asserting that an out-of-scope or prerequisite-unmet request
/// produced the localized guardrail copy rather than an LLM
/// completion.
///
/// # Errors
///
/// Returns [`GuardrailMismatch`] containing both the normalized reply
/// and normalized expected substring when the expected text is not
/// found.
pub fn assert_guardrail(reply: &str, expected: &str) -> Result<(), GuardrailMismatch> {
    let n_reply = normalize(reply);
    let n_expected = normalize(expected);
    if n_reply.contains(&n_expected) {
        Ok(())
    } else {
        Err(GuardrailMismatch {
            reply: n_reply,
            expected: n_expected,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        assert_citation_grounded, assert_guardrail, CitationFixture, CitationMismatch, Tolerances,
    };

    const FIXTURE: CitationFixture = CitationFixture {
        activity_count: 7,
        total_distance_km: 45.3,
    };

    #[test]
    fn citation_passes_when_reply_matches_fixture_exactly() {
        let reply = "Over the past 8 weeks you logged 7 runs totaling 45.3 km. Solid base.";
        assert!(assert_citation_grounded(reply, &FIXTURE, Tolerances::strict()).is_ok());
    }

    #[test]
    fn citation_passes_for_empty_reply() {
        // No claims, nothing to ground.
        assert!(assert_citation_grounded("", &FIXTURE, Tolerances::strict()).is_ok());
    }

    #[test]
    fn citation_catches_hallucinated_activity_count() {
        let reply = "You logged 15 runs covering 45.3 km.";
        let err = assert_citation_grounded(reply, &FIXTURE, Tolerances::strict())
            .expect_err("15 ≠ 7 should fail");
        assert_eq!(
            err,
            CitationMismatch::ActivityCountWrong {
                cited: 15,
                expected: 7,
            }
        );
    }

    #[test]
    fn citation_catches_hallucinated_distance() {
        let reply = "Your 7 runs this block totaled 120 km of volume.";
        let err = assert_citation_grounded(reply, &FIXTURE, Tolerances::strict())
            .expect_err("120 km ≠ 45.3 km should fail");
        match err {
            CitationMismatch::DistanceOutOfTolerance {
                cited_km,
                expected_km,
                ..
            } => {
                assert!((cited_km - 120.0).abs() < f64::EPSILON);
                assert!((expected_km - 45.3).abs() < f64::EPSILON);
            }
            CitationMismatch::ActivityCountWrong { cited, expected } => {
                panic!("expected distance mismatch, got ActivityCountWrong(cited={cited}, expected={expected})")
            }
        }
    }

    #[test]
    fn citation_allows_rounding_within_loose_tolerance() {
        // 45.3 km ≈ 45 km well within 5% tolerance
        let reply = "You covered about 45 km in 7 runs.";
        assert!(assert_citation_grounded(reply, &FIXTURE, Tolerances::loose()).is_ok());
    }

    #[test]
    fn citation_rejects_rounding_outside_strict_tolerance() {
        let reply = "You covered about 45 km in 7 runs.";
        let err = assert_citation_grounded(reply, &FIXTURE, Tolerances::strict())
            .expect_err("0.7% diff should fail strict 0.1%");
        assert!(matches!(
            err,
            CitationMismatch::DistanceOutOfTolerance { .. }
        ));
    }

    #[test]
    fn citation_accepts_french_comma_decimal() {
        let reply = "Vos 7 courses totalisent 45,3 km.";
        assert!(assert_citation_grounded(reply, &FIXTURE, Tolerances::strict()).is_ok());
    }

    #[test]
    fn citation_accepts_french_noun_forms() {
        let reply = "Vous avez couru 7 séances pour 45,3 km.";
        assert!(assert_citation_grounded(reply, &FIXTURE, Tolerances::strict()).is_ok());
    }

    #[test]
    fn guardrail_matches_scope_refusal_en() {
        let reply = "That's outside what I can help with — I'm your fitness assistant.";
        assert!(assert_guardrail(reply, "outside what I can help with").is_ok());
    }

    #[test]
    fn guardrail_matches_with_whitespace_differences() {
        let reply = "That's\n  outside  what\tI  can   help with.";
        assert!(assert_guardrail(reply, "outside what i can help with").is_ok());
    }

    #[test]
    fn guardrail_fails_when_string_absent() {
        let reply = "Sure, let me look that up for you.";
        let err = assert_guardrail(reply, "outside what I can help with")
            .expect_err("refusal string is not present");
        assert_eq!(err.expected, "outside what i can help with");
    }

    // ── Phase 1 asserter tests ───────────────────────────────────
    //
    // Hand-crafted JSON mirrors the shape of
    // `pierre_core::models::usage::ConversationTurnSummary`. If that
    // struct grows fields, these fixtures are the canary: they compile
    // against the serialization contract, not the struct, so a field
    // rename ships a behavior change the asserters must adapt to.

    use super::{
        assert_latency_ms_at_most, assert_tokens_at_most, assert_tool_called, LatencyOverBudget,
        TokensOverBudget, ToolNotCalled,
    };
    use serde_json::json;

    fn turn_json() -> serde_json::Value {
        json!({
            "turn_id": "11111111-2222-3333-4444-555555555555",
            "tenant_id": "tenant-xyz",
            "user_id": "user-xyz",
            "tools_called": ["get_activities", "get_training_load"],
            "llm_calls": [],
            "total_tokens": 1234_i64,
            "total_prompt_tokens": 1000_i64,
            "total_completion_tokens": 234_i64,
            "total_latency_ms": 3200_i64,
            "first_call_at": "2026-04-22T18:00:00Z",
            "last_call_at": "2026-04-22T18:00:03Z"
        })
    }

    #[test]
    fn tool_called_passes_when_expected_tool_is_in_list() {
        assert!(assert_tool_called(&turn_json(), "get_activities").is_ok());
    }

    #[test]
    fn tool_called_catches_missing_invocation() {
        let err = assert_tool_called(&turn_json(), "suggest_goals")
            .expect_err("suggest_goals is not in tools_called");
        assert_eq!(
            err,
            ToolNotCalled {
                expected: "suggest_goals".to_owned(),
                actual: vec!["get_activities".to_owned(), "get_training_load".to_owned()],
            }
        );
    }

    #[test]
    fn tool_called_fails_loudly_when_array_missing() {
        // Turn record without tools_called — hallucination assertion
        // should still fail rather than silently pass.
        let turn = json!({ "turn_id": "x", "total_tokens": 0_i64 });
        let err = assert_tool_called(&turn, "get_activities")
            .expect_err("missing tools_called must not silently pass");
        assert!(err.actual.is_empty());
    }

    #[test]
    fn latency_passes_under_budget() {
        assert!(assert_latency_ms_at_most(&turn_json(), 5_000).is_ok());
    }

    #[test]
    fn latency_passes_at_exact_budget() {
        assert!(assert_latency_ms_at_most(&turn_json(), 3_200).is_ok());
    }

    #[test]
    fn latency_catches_over_budget() {
        let err =
            assert_latency_ms_at_most(&turn_json(), 2_000).expect_err("3200 ms > 2000 ms budget");
        assert_eq!(
            err,
            LatencyOverBudget {
                observed_ms: 3_200,
                budget_ms: 2_000,
            }
        );
    }

    #[test]
    fn latency_passes_when_field_missing() {
        // Absence of evidence is not evidence of budget violation —
        // callers use assert_tool_called to enforce turn presence.
        let turn = json!({ "turn_id": "x" });
        assert!(assert_latency_ms_at_most(&turn, 1_000).is_ok());
    }

    #[test]
    fn tokens_passes_under_budget() {
        assert!(assert_tokens_at_most(&turn_json(), 5_000).is_ok());
    }

    #[test]
    fn tokens_catches_over_budget() {
        let err = assert_tokens_at_most(&turn_json(), 500).expect_err("1234 tokens > 500 budget");
        assert_eq!(
            err,
            TokensOverBudget {
                observed: 1_234,
                budget: 500,
            }
        );
    }

    #[test]
    fn tokens_passes_when_field_missing() {
        let turn = json!({ "turn_id": "x" });
        assert!(assert_tokens_at_most(&turn, 100).is_ok());
    }
}
