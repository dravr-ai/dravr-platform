// ABOUTME: The marker standing in for a withheld reply tells the extractor to use the user turn only
// ABOUTME: Wording only — that a withheld turn still extracts is pinned on real turns in pierre-server

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
//! That is what stalls a guided profile walk. The athlete answers, the agent's
//! reply is withheld, no fact is extracted, coverage never flips, and the next
//! turn asks the same question — indefinitely, since every recorded withhold to
//! date is on the agent this flow runs against.
//!
//! The invariant now: user-side extraction runs either way, with a marker
//! standing in for the reply; only assistant-side learning (playbook advice
//! capture, whose whole purpose is learning from what the agent said) is
//! skipped. Both passes are detached tasks that call the model, so the branch
//! is asserted on what those tasks send, by running a withheld turn, in
//! `crates/pierre-server/tests/withheld_turn_learning_pipeline_test.rs`. What
//! is asserted here is the marker's own wording.

use pierre_services::memory_extraction::WITHHELD_REPLY_TRANSCRIPT_MARKER;

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
