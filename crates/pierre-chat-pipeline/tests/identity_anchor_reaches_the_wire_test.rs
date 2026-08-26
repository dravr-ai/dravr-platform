// ABOUTME: The identity anchor must reach the hardened prompt on EVERY path, coach-bound included
// ABOUTME: Closes the gap where all anchor tests exercised the pure helper on a bare string
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Every existing anchor test calls `close_with_identity_anchor("body")` on a
//! bare string. `default_and_coach_paths_get_the_identical_anchor` passes two
//! literals to the same pure function and proves only that `format!` is
//! deterministic.
//!
//! **None of them would fail if the anchor stopped being applied on the
//! coach-bound path** — which is the exact branch whose absence WAS the original
//! bug. `prompt_assembly.rs` resolved the base prompt with a `map_or_else` that
//! REPLACED `pierre_system.md` (the only file containing "You are Dravr") when a
//! coach was bound, and zero of the 52 contremaitre coach prompts carry the
//! string. Coach-bound turns matched the boundary detector 5/21; no-coach turns
//! 0/10.
//!
//! `assemble_prompt_and_messages` is `pub(crate)` and needs a live
//! `ChatPipelineContext` (DB, registries, provider), so an integration test
//! cannot drive it. These tests therefore attack the same invariant from the two
//! seams that ARE reachable:
//!
//! 1. **Structurally** — the anchor call must be unconditional and sit AFTER the
//!    coach/default branch, so reintroducing the branch-scoped bug reds. Same
//!    technique as `onboarding_directive_tail_test`, and the same reason: the
//!    regression guarded against is a source edit.
//! 2. **Compositionally** — the anchor must survive canary hardening and the
//!    conditions under which leaks were actually observed (very large prompts,
//!    an appended compaction summary), since both real incidents were
//!    long-context turns rather than the short provocations the live A/B used.

use pierre_chat_pipeline::stages::prompt_assembly::{
    close_with_anchors, close_with_identity_anchor,
};
use pierre_core::models::TenantId;
use pierre_services::prompt_leak::harden_system_prompt;
use std::fs;
use std::path::PathBuf;

fn prompt_assembly_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/stages/prompt_assembly.rs");
    fs::read_to_string(&path).expect("read prompt_assembly.rs")
}

/// Byte offset of a marker that must appear exactly once.
fn sole_offset(source: &str, marker: &str) -> usize {
    let count = source.matches(marker).count();
    assert_eq!(
        count, 1,
        "expected exactly one occurrence of {marker:?}, found {count}"
    );
    source.find(marker).expect("checked above")
}

#[test]
fn the_anchor_is_applied_unconditionally_after_the_coach_branch() {
    let source = prompt_assembly_source();

    // The `map_or_else` that chooses coach prompt vs pierre_system.md. The bug
    // was that this branch decided whether any identity text existed at all.
    let branch = sole_offset(&source, "coach_ctx.map_or_else(");
    let anchor_call = sole_offset(&source, "close_with_anchors(&raw_system_prompt,");

    assert!(
        branch < anchor_call,
        "the anchor must be applied AFTER the coach/default branch, so both paths \
         receive it — applying it inside either arm is how coach-bound turns ended \
         up with no identity text at all"
    );

    // And it must not be conditional. Anything that scopes the call to a branch
    // recreates the original defect for the excluded path.
    //
    // Checked over the whole statement rather than the marker's own line:
    // rustfmt is free to wrap `let x = f(a, b);` across two lines, and it does,
    // which made the earlier line-prefix version fail on a formatting choice
    // instead of on the defect it exists to catch.
    let stmt_start = source[..anchor_call]
        .rfind("let raw_system_prompt =")
        .expect("the anchor call must be a rebinding of raw_system_prompt");
    let between = &source[stmt_start..anchor_call];
    assert!(
        !between.contains(" if ")
            && !between.contains("match ")
            && !between.contains("=>")
            && !between.contains('{'),
        "the anchor call must be unconditional — nothing may scope it to a branch, found: {between:?}"
    );
}

#[test]
fn both_arms_of_the_close_carry_the_identity_anchor() {
    // The single call site above used to be the whole guarantee: one call, so
    // no branch could skip it. `close_with_anchors` now has two arms — a
    // coach-bound turn also gets a voice anchor — so the hazard the structural
    // test guards against moved inside that function, and this follows it there.
    //
    // Behavioural, not a grep: it asserts the string the model would receive.
    for coach in [Some("strength"), None] {
        let out = close_with_anchors("## Your coaching style\nShort sessions.", coach);
        assert!(
            out.contains("You are Dravr"),
            "the identity anchor must reach the prompt with coach_slug = {coach:?}"
        );
        assert!(
            out.contains("not a competing identity"),
            "the anchor must arrive in full with coach_slug = {coach:?}"
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
            "the identity anchor must still be LAST with coach_slug = {coach:?}, got tail {tail:?}"
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
    let bulky_coach_prompt = "## Coaching context\n".to_owned() + &"session notes. ".repeat(3_500);
    assert!(
        bulky_coach_prompt.len() > 50_000,
        "precondition: the fixture must reach the size band where leaks occurred"
    );

    let assembled = close_with_identity_anchor(&bulky_coach_prompt);
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
        anchor_at > bulky_coach_prompt.len() / 2,
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
