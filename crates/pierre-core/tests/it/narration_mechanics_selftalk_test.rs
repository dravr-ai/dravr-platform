// ABOUTME: Output-mechanics self-talk must scrub; running-splits coaching must survive
// ABOUTME: Pins the 2026-08-23 «Good, real newlines. Let me fix the split.» leak

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Live incident 2026-08-23 (Telegram group): the winning chart reply OPENED
//! with English self-talk about message formatting — «Good, real newlines.
//! Let me fix the split.» — delivered verbatim before the French answer.
//! Neither phrasing was in the narration vocabulary. The additions must catch
//! this class WITHOUT eating real coaching: "split" is running vocabulary
//! (interval splits), which is why only the full observed phrase is listed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::narration::scrub_internal_narration;

/// The exact delivered leak: both self-talk sentences drop, the answer stays.
#[test]
fn formatting_selftalk_preamble_is_scrubbed() {
    let reply = "Good, real newlines. Let me fix the split. Voici le graph à jour, \
                 recalculé sur tes vraies données 👇";
    let scrub = scrub_internal_narration(reply);
    assert!(
        scrub.removed >= 1,
        "the mechanics preamble must be recognized as narration"
    );
    assert!(
        !scrub.cleaned.contains("real newlines") && !scrub.cleaned.contains("fix the split"),
        "no self-talk may survive, got: {:?}",
        scrub.cleaned
    );
    assert!(
        scrub.cleaned.contains("Voici le graph"),
        "the actual answer must survive the scrub, got: {:?}",
        scrub.cleaned
    );
}

/// Interval-split coaching is NOT narration — the vocabulary deliberately
/// lists only the full observed phrase, never "fix the split" alone.
#[test]
fn running_splits_coaching_survives() {
    for reply in [
        "On va fixer ton split au 5k: vise 4:45/km sur la première moitié.",
        "Your second split was faster — negative split, exactly what we wanted.",
        "Let's fix the split strategy for Sunday: even pacing until 30k.",
    ] {
        let scrub = scrub_internal_narration(reply);
        assert_eq!(
            scrub.removed, 0,
            "legitimate split coaching must never be scrubbed: {reply:?}"
        );
    }
}
