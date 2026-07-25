// ABOUTME: A withheld reply must still extract from the athlete's own message, not orphan the turn
// ABOUTME: Structural + content assertions on the leak_replaced branch and its transcript marker

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! When the identity-leak detector withholds a reply, the withheld text must
//! never reach the fact store — a leaked narration minted as a fact re-enters
//! every future prompt bundle (reinforcement loop, 2026-07-10). The original
//! gate achieved that by dropping *all* background learning for the turn, which
//! also dropped the athlete's own message.
//!
//! That is what stalls a guided profile walk. The athlete answers, the coach's
//! reply is withheld, no fact is extracted, coverage never flips, and the next
//! turn asks the same question — indefinitely, since every recorded withhold to
//! date is on the coach this flow runs against.
//!
//! The invariant now: user-side extraction runs either way, with a marker
//! standing in for the reply; only assistant-side learning (playbook advice
//! capture, whose whole purpose is learning from what the coach said) is
//! skipped. Asserted structurally because the branch spawns detached tasks
//! against a live LLM provider — the regression this guards against is a source
//! edit restoring the early `return`.

use pierre_services::memory_extraction::WITHHELD_REPLY_TRANSCRIPT_MARKER;
use std::fs;
use std::path::PathBuf;

/// The body of `spawn_turn_background_learning`, from its signature to the end
/// of the function.
fn background_learning_body() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let source = fs::read_to_string(&path).expect("read lib.rs");
    let start = source
        .find("fn spawn_turn_background_learning(")
        .expect("spawn_turn_background_learning must exist");
    let rest = &source[start..];
    let end = rest
        .find("\n}\n")
        .expect("function must have a closing brace");
    rest[..end].to_owned()
}

#[test]
fn withheld_branch_extracts_from_the_user_turn() {
    let body = background_learning_body();
    let branch_start = body
        .find("if leak_replaced {")
        .expect("the leak_replaced branch must exist");
    // Bound the slice to the branch's own closing brace, so the fall-through
    // path below (which legitimately passes the real reply) is not inspected.
    let branch_len = body[branch_start..]
        .find("\n    }\n")
        .expect("the branch must be closed");
    let branch = &body[branch_start..branch_start + branch_len];

    // The branch must do work before it returns, and that work is extraction.
    let extraction = branch
        .find("spawn_turn_extraction(")
        .expect("a withheld turn must still run extraction over the user message");
    let early_return = branch
        .find("return;")
        .expect("the branch still returns early");
    assert!(
        extraction < early_return,
        "extraction must run before the branch returns — an early `return;` here is the \
         regression that stalls the profile walk on a withheld turn"
    );

    assert!(
        branch.contains("WITHHELD_REPLY_TRANSCRIPT_MARKER"),
        "the withheld branch must pass the marker in place of the reply, never the reply itself"
    );
    assert!(
        !branch.contains("assistant_reply,"),
        "the withheld reply must not be handed to the extractor"
    );
    assert!(
        !branch.contains("spawn_turn_advice_capture("),
        "playbook advice capture learns from what the coach said, so it stays skipped"
    );
}

#[test]
fn marker_tells_the_extractor_to_use_only_the_user_turn() {
    let marker = WITHHELD_REPLY_TRANSCRIPT_MARKER;
    assert!(
        !marker.is_empty(),
        "an empty marker reads as an empty reply"
    );
    assert!(
        marker.contains("withheld"),
        "the marker must say the reply was withheld, got: {marker}"
    );
    assert!(
        marker.contains("only from the user turn"),
        "the marker must scope extraction to the user turn, got: {marker}"
    );
}
