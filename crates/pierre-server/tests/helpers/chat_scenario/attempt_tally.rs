// ABOUTME: Attempt tally for the retried live chat eval — which attempt each scenario first passed on
// ABOUTME: Renders per-scenario lines plus pass@1 beside the gating pass@N, so retries cannot hide drift
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Attempt tally for the live scenario driver.
//!
//! The live driver retries each scenario and passes it on its first green
//! attempt, so its verdict is pass@N: a scenario that needs its last draw
//! reads exactly like one that passes first time. This renders the
//! difference without touching the verdict.

/// Render the per-scenario attempt tally: one line per scenario, then a
/// summary giving pass@1 (passed with no retry) beside the gating
/// pass@`max_attempts`. A pass@1 that falls while the gate stays green is
/// drift the retries are absorbing.
///
/// `tally` pairs each scenario name with the 1-based attempt it first
/// passed on, `None` when every attempt failed.
pub(crate) fn render(tally: &[(String, Option<usize>)], max_attempts: usize) -> Vec<String> {
    let mut lines: Vec<String> = tally
        .iter()
        .map(|(name, passed_on)| {
            passed_on.map_or_else(
                || format!("{name} FAILED all {max_attempts} attempts"),
                |n| format!("{name} passed on attempt {n}/{max_attempts}"),
            )
        })
        .collect();
    let first_try = tally.iter().filter(|(_, p)| *p == Some(1)).count();
    let passed = tally.iter().filter(|(_, p)| p.is_some()).count();
    lines.push(format!(
        "pass@1 {first_try}/{total} · pass@{max_attempts} {passed}/{total}",
        total = tally.len()
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn separates_first_try_passes_from_retried_and_failed_scenarios() {
        let tally = vec![
            ("first_try".to_owned(), Some(1)),
            ("retried".to_owned(), Some(3)),
            ("broken".to_owned(), None),
        ];
        assert_eq!(
            render(&tally, 4),
            vec![
                "first_try passed on attempt 1/4".to_owned(),
                "retried passed on attempt 3/4".to_owned(),
                "broken FAILED all 4 attempts".to_owned(),
                "pass@1 1/3 · pass@4 2/3".to_owned(),
            ],
            "the tally must name each scenario's passing attempt and count only attempt-1 \
             passes toward pass@1"
        );
    }

    #[test]
    fn an_empty_run_still_reports_a_zero_summary() {
        assert_eq!(
            render(&[], 4),
            vec!["pass@1 0/0 · pass@4 0/0".to_owned()],
            "an empty tally must still emit the summary line rather than nothing"
        );
    }
}
