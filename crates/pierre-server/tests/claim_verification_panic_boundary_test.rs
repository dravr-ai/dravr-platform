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
use pierre_evals::{VerificationConfig, VerificationFallback};
use tokio::task::yield_now;

const REPLY: &str = "Ta semaine 4 est un pic: 5 séances, dont une sortie longue de 3 h.";

/// What a blocking coach replaces a reply with. The real one is localized out
/// of the messaging-strings registry; the boundary only ever calls the closure
/// the caller hands it, so its text is the caller's business.
const BLOCK_FALLBACK: &str = "Je préfère vérifier ce point avant de te répondre.";

/// A coach config with the given fallback behavior.
fn config(fallback: VerificationFallback) -> VerificationConfig {
    VerificationConfig {
        fallback_behavior: fallback,
        ..VerificationConfig::default()
    }
}

/// The panic shape the production incident had: a byte offset landing inside a
/// multi-byte character. The boundary must not care which bug it is, so every
/// test here panics the same way.
async fn panicking_stage() -> ClaimVerificationOutcome {
    // The yield keeps the panic inside the polled future. Panicking before any
    // suspension point would unwind at the call site instead, outside the
    // boundary these tests exist to exercise.
    yield_now().await;
    let text = "dernixxère";
    let _ = &text[..8];
    unreachable!("the slice above panics")
}

#[tokio::test]
async fn a_panicking_verifier_delivers_the_reply_unverified() {
    // A warning coach would only ever have appended a footer, so losing the
    // footer is the whole cost of the panic.
    let outcome = degrade_to_unverified(
        panicking_stage(),
        REPLY,
        &config(VerificationFallback::Warn),
        || BLOCK_FALLBACK.to_owned(),
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
                // This surface folds the banner into the reply, so the chip
                // list is the empty half of the affordance XOR.
                chips: Vec::new(),
            }
        },
        REPLY,
        &config(VerificationFallback::Warn),
        || BLOCK_FALLBACK.to_owned(),
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

#[tokio::test]
async fn a_blocking_coach_gets_its_block_fallback_when_the_detector_panics() {
    // A coach configured to Block does not append a caveat — its fallback
    // exists to REPLACE a reply carrying a contradicted claim ("an HR max of
    // 300 bpm", "500 g of creatine per day"). Those are the class the
    // deterministic bounds catch, and the class whose numeric scan panicked.
    // Delivering the reply unverified hands the athlete exactly the sentence
    // the coach configured the platform to withhold.
    let outcome = degrade_to_unverified(
        panicking_stage(),
        REPLY,
        &config(VerificationFallback::Block),
        || BLOCK_FALLBACK.to_owned(),
    )
    .await;

    assert_eq!(
        outcome.content, BLOCK_FALLBACK,
        "a blocking coach must not deliver an unscanned reply"
    );
    assert_ne!(
        outcome.content, REPLY,
        "the unverified reply is what this coach configured away"
    );
    assert!(
        outcome.pending_verdicts.is_empty(),
        "a panicked verifier has no verdicts to persist"
    );
}

#[tokio::test]
async fn a_silent_coach_still_gets_its_reply_after_a_panic() {
    // Silent records the verdict and shows nothing, so an unscanned reply is
    // exactly what it would have delivered anyway. Only Block changes.
    let outcome = degrade_to_unverified(
        panicking_stage(),
        REPLY,
        &config(VerificationFallback::Silent),
        || BLOCK_FALLBACK.to_owned(),
    )
    .await;

    assert_eq!(outcome.content, REPLY);
}

#[tokio::test]
async fn a_coach_with_verification_off_is_never_blocked_by_a_panic() {
    // A disabled config returns the reply untouched before it looks at a single
    // claim, so it has no block behavior to honor — replacing the reply here
    // would invent a refusal the coach never asked for.
    let disabled = VerificationConfig {
        enabled: false,
        ..config(VerificationFallback::Block)
    };

    let outcome = degrade_to_unverified(panicking_stage(), REPLY, &disabled, || {
        BLOCK_FALLBACK.to_owned()
    })
    .await;

    assert_eq!(outcome.content, REPLY);
}
