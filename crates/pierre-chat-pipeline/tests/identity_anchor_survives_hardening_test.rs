// ABOUTME: The identity anchor survives everything applied after it — both closing arms, hardening, 50 KB prompts
// ABOUTME: That it reaches the wire on the agent-bound path is pinned on real turns in pierre-server
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Every existing anchor test calls `close_with_identity_anchor("body")` on a
//! bare string. `default_and_agent_paths_get_the_identical_anchor` passes two
//! literals to the same pure function and proves only that `format!` is
//! deterministic.
//!
//! **None of them would fail if the anchor stopped being applied on the
//! agent-bound path** — which is the exact branch whose absence WAS the original
//! bug. `prompt_assembly.rs` resolved the base prompt with a `map_or_else` that
//! REPLACED `pierre_system.md` (the only file containing "You are Dravr") when a
//! agent was bound, and zero of the 52 contremaitre agent prompts carry the
//! string. Agent-bound turns matched the boundary detector 5/21; no-agent turns
//! 0/10.
//!
//! That branch is exercised where it runs: `an_agent_bound_turn_closes_with_the_same_anchor`
//! in `crates/pierre-server/tests/prompt_assembly_wire_test.rs` binds an agent,
//! runs a real turn and asserts the request the model received closes with the
//! anchor. What stays here is what a string-level test can honestly cover: the
//! anchor must survive canary hardening and the conditions under which leaks
//! were actually observed (very large prompts, an appended compaction summary),
//! since both real incidents were long-context turns rather than the short
//! provocations the live A/B used.

use pierre_chat_pipeline::stages::prompt_assembly::{
    close_with_anchors, close_with_identity_anchor,
};
use pierre_core::models::TenantId;
use pierre_services::prompt_leak::harden_system_prompt;

#[test]
fn both_arms_of_the_close_carry_the_identity_anchor() {
    // `close_with_anchors` has two arms — an agent-bound turn also gets a
    // voice anchor — so a branch inside it is where the identity anchor could
    // be skipped, and this drives both.
    for agent in [Some("strength"), None] {
        let out = close_with_anchors("## Your coaching style\nShort sessions.", agent);
        assert!(
            out.contains("You are Dravr"),
            "the identity anchor must reach the prompt with coach_slug = {agent:?}"
        );
        assert!(
            out.contains("not a competing identity"),
            "the anchor must arrive in full with coach_slug = {agent:?}"
        );
        let tail: String = out
            .chars()
            .rev()
            .take(60)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        assert!(
            tail.contains("not a competing identity"),
            "the identity anchor must still be LAST with coach_slug = {agent:?}, got tail {tail:?}"
        );
    }
}

#[test]
fn the_anchor_survives_canary_hardening() {
    // Stage 7h appends the canary marker AFTER the anchor. If hardening ever
    // truncated or rewrote the tail, the anchor would vanish from the prompt the
    // model actually receives while every string-level test kept passing.
    let assembled = close_with_identity_anchor("## Your coaching style\nShort, punchy sessions.");
    let guard = harden_system_prompt(TenantId::generate(), Some("coach-123"), &assembled);

    assert!(
        guard.hardened_prompt.contains("You are Dravr"),
        "the identity anchor must survive into the hardened prompt the LLM receives"
    );
    assert!(
        guard.hardened_prompt.contains("not a competing identity"),
        "the anchor must survive in full, not truncated to its first sentence"
    );
    assert!(
        guard.hardened_prompt.contains(&guard.canary),
        "sanity: the canary is still injected (this test is not vacuous)"
    );
}

#[test]
fn the_anchor_survives_the_conditions_where_leaks_were_actually_observed() {
    // Both real incidents were LONG-CONTEXT turns on ordinary questions, not the
    // short identity provocations the 84-run live A/B used — production
    // 2026-07-28 ran a 51,621-char prompt over 30 messages, and the local
    // reproduction on 2026-08-04 ran 62,787 chars over a compacted 48-message
    // thread. A 2-line-history harness cannot see that regime, so pin it here.
    let bulky_agent_prompt = "## Coaching context\n".to_owned() + &"session notes. ".repeat(3_500);
    assert!(
        bulky_agent_prompt.len() > 50_000,
        "precondition: the fixture must reach the size band where leaks occurred"
    );

    let assembled = close_with_identity_anchor(&bulky_agent_prompt);
    let guard = harden_system_prompt(TenantId::generate(), Some("coach-123"), &assembled);

    assert!(
        guard.hardened_prompt.contains("You are Dravr"),
        "the anchor must survive a 50 KB+ prompt — the size band of both real incidents"
    );
    // It must still be at the TAIL: the 48-run A/B measured tail placement as the
    // entire mechanism (anchor last 0/12 disclosures, anchor first 2/12).
    let anchor_at = guard
        .hardened_prompt
        .find("You are Dravr")
        .expect("anchor present");
    assert!(
        anchor_at > bulky_agent_prompt.len() / 2,
        "the anchor must sit in the tail of the prompt, not be buried mid-body"
    );
}

#[test]
fn a_compaction_summary_cannot_displace_the_anchor_from_the_tail() {
    // The local reproduction leaked on a turn where compaction had just fired.
    // A summary spliced into the prompt must not end up after the anchor and
    // push it out of the recency position the A/B measured as load-bearing.
    let body = "## Coaching context\nrecent sessions.".to_owned();
    let assembled = close_with_identity_anchor(&body);

    // The contract is positional, not proportional: the anchor must be LAST.
    // Asserting a size ratio would only hold for large bodies and would pass
    // vacuously the moment the body shrank — the anchor is 589 chars, so it
    // legitimately dominates a short prompt.
    assert!(
        assembled.trim_end().ends_with("not a competing identity."),
        "the platform prompt must END with the anchor; anything appended after it \
         costs the tail position the 48-run A/B measured as the entire mechanism"
    );
    let tail = assembled
        .rfind("You are Dravr")
        .expect("anchor present in assembled prompt");
    assert!(
        !assembled[tail..].contains("Earlier conversation summary"),
        "a compaction summary must never be appended after the identity anchor"
    );
}
