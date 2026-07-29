// ABOUTME: Guards the panic boundary around the claim-verification stage (stages::verification)
// ABOUTME: A panicking verifier must cost the footer, never the turn that already wrote to storage
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! On 2026-07-28 the deterministic-bounds scanner panicked on a byte-slice
//! through an accented character. The unwind reached the turn-level boundary
//! in messaging dispatch, which discarded the whole turn and replied
//! "Dravr est temporairement indisponible." — six seconds after
//! `save_training_plan` had committed the athlete's first successful plan.
//! The write survived; the sentence telling him about it did not.
//!
//! The scanner bug is fixed in `deterministic_bounds_test.rs`. These tests
//! guard the structural half: whatever the next verifier bug turns out to be,
//! it degrades to an unverified reply instead of erasing the turn.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_chat_pipeline::stages::verification::{degrade_to_unverified, ClaimVerificationOutcome};

const REPLY: &str = "Ta semaine 4 est un pic: 5 séances, dont une sortie longue de 3 h.";

#[tokio::test]
async fn a_panicking_verifier_delivers_the_reply_unverified() {
    // A panic anywhere inside the stage — the 2026-07-28 one was a string
    // slice on a non-char-boundary index, but the boundary must not care
    // which bug it is.
    let outcome = degrade_to_unverified(
        async {
            let text = "dernixxère";
            // Same shape as the production panic: a byte offset that lands
            // inside a multi-byte character.
            let _ = &text[..8];
            unreachable!("the slice above panics")
        },
        REPLY,
    )
    .await;

    assert_eq!(
        outcome.content, REPLY,
        "the athlete must still receive the reply the turn produced"
    );
    assert!(
        outcome.pending_verdicts.is_empty(),
        "a panicked verifier has no verdicts to persist"
    );
}

#[tokio::test]
async fn a_healthy_verifier_result_passes_through_untouched() {
    // The boundary must be transparent when nothing goes wrong, or it would
    // silently strip the footer it exists to protect. `pending_verdicts` is
    // opaque here (its contents come from the detector), so the pass-through
    // is pinned on the content the stage chose to return — which is NOT the
    // input reply, exactly as it would not be once a warn banner is appended.
    let verified = format!("{REPLY}\n\n---\nÀ vérifier:\n- une affirmation");
    let expected = verified.clone();
    let outcome = degrade_to_unverified(
        async move {
            ClaimVerificationOutcome {
                content: verified,
                pending_verdicts: Vec::new(),
            }
        },
        REPLY,
    )
    .await;

    assert_eq!(
        outcome.content, expected,
        "the stage's own output must reach the caller, banner included"
    );
    assert_ne!(
        outcome.content, REPLY,
        "a boundary that returned the fallback on success would look green here"
    );
}
