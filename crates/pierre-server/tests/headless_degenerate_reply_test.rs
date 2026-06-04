// ABOUTME: Guards the contract the Copilot-ACP headless tool loop relies on to detect a degenerate turn
// ABOUTME: A parroted tool-result echo must strip to empty so run_headless_tool_loop retries instead of leaking it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Degenerate headless-reply detection contract.
//!
//! `run_headless_tool_loop` in `pierre-tool-runtime` treats a turn as
//! degenerate — and retries `converse()` once — when
//! [`strip_simulation_artifacts`] reduces the model's reply to nothing.
//! That happens in two intermittent Copilot-ACP failure modes:
//!
//! 1. The model returns empty content outright.
//! 2. The model parrots the injected tool-result turn verbatim (the
//!    `format_tool_results_as_text` preamble plus `<tool_result>` blocks)
//!    instead of reasoning over it — observed leaking raw training-history
//!    JSON into a coach reply (dev 2026-06-04).
//!
//! These tests pin the linchpin of that fix: a pure parrot strips to empty
//! (so the loop retries), a real answer survives untouched, and a partial
//! echo keeps its prose while shedding the scaffolding. If embacle ever
//! changes the echo format, the degenerate check would silently stop
//! firing — these assertions catch that.

use pierre_tool_runtime::tool_execution::strip_simulation_artifacts;

/// A pure parroted tool-result echo — preamble + a single `<tool_result>`
/// block and nothing else — must strip to empty, marking the turn
/// degenerate so the headless loop retries rather than leaking the JSON.
#[test]
fn parroted_tool_result_echo_strips_to_empty() {
    let echo = "Here are the results from the tools you requested:\n\n\
                <tool_result name=\"get_training_history\">\n\
                {\"from\":\"2026-05-07\",\"to\":\"2026-06-04\",\"days\":[{\"ctl\":77.8,\"atl\":94.7}]}\n\
                </tool_result>";

    assert!(
        strip_simulation_artifacts(echo).is_empty(),
        "a pure parroted tool-result echo must strip to empty so the headless loop treats the turn as degenerate and retries"
    );
}

/// A genuine synthesized answer carries no simulation scaffolding, so it
/// survives stripping unchanged and is never mistaken for degenerate.
#[test]
fn synthesized_answer_survives_strip() {
    let answer = "Your CTL is trending up steadily — solid base this block. \
                  Keep the long_run_z2 on Sunday and we'll add a vo2_5x3 midweek.";

    assert_eq!(strip_simulation_artifacts(answer), answer);
    assert!(!strip_simulation_artifacts(answer).is_empty());
}

/// A reply that mixes real prose with a trailing echoed `<tool_result>`
/// block keeps the prose and sheds only the scaffolding — so it is NOT
/// degenerate and is returned (cleaned) without a retry.
#[test]
fn partial_echo_keeps_prose_and_is_not_degenerate() {
    let mixed = "Your training load looks high this week — back off intensity.\n\
                 <tool_result name=\"get_training_history\">{\"ctl\":108.5}</tool_result>";

    let cleaned = strip_simulation_artifacts(mixed);
    assert_eq!(
        cleaned,
        "Your training load looks high this week — back off intensity."
    );
    assert!(
        !cleaned.is_empty(),
        "a reply with real prose must not be treated as degenerate"
    );
}
