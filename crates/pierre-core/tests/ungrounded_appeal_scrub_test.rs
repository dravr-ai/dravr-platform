// ABOUTME: Pins that a reply produced without a fetch cannot cite data as its authority
// ABOUTME: Regression for 2026-09-02 — "Roster data confirme" on a turn where no tool ran
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The athlete had corrected the coach's weekday ordering three times. On the
//! third, the coach replied:
//!
//! > *"Roster data confirme: Date ride était bien lundi (16 km, 414 m), et Road
//! > 2 AUS mardi"*
//!
//! No tool ran on that turn. The coach took the athlete's own correction,
//! attributed it to a data lookup it had not performed, and handed it back as
//! confirmation. Claiming data confirmation without a fetch is worse than
//! abstaining — it converts a correction into evidence against the person
//! making it (registre#202).
//!
//! This scrub runs **only** on an ungrounded turn. The identical sentence after
//! a real fetch is simply true, and must survive untouched.

use pierre_core::narration::scrub_ungrounded_data_appeals;

#[test]
fn the_incident_sentence_is_removed() {
    let reply = "Roster data confirme: Date ride était bien lundi. Mardi reste ton pic.";
    let scrub = scrub_ungrounded_data_appeals(reply);

    assert!(scrub.fired(), "the appeal must be caught: {scrub:?}");
    assert!(
        !scrub.cleaned.contains("Roster data confirme"),
        "the appeal to a lookup that never happened must not survive: {}",
        scrub.cleaned
    );
    assert!(
        scrub.cleaned.contains("Mardi reste ton pic"),
        "the rest of the reply is untouched — the coach may still answer, it \
         just cannot cite evidence it does not have: {}",
        scrub.cleaned
    );
}

#[test]
fn the_appeal_is_caught_in_every_shipped_locale() {
    for reply in [
        "Les données confirment que tu as roulé 161 km.",
        "The data confirms you rode 161 km.",
        "Los datos confirman que hiciste 161 km.",
        "Die Daten bestätigen, dass du 161 km gefahren bist.",
        "Os dados confirmam que pedalaste 161 km.",
    ] {
        let scrub = scrub_ungrounded_data_appeals(reply);
        assert!(
            scrub.fired(),
            "a five-locale platform cannot police only one of them: {reply:?}"
        );
    }
}

/// A coach answering from what it was given, without claiming a lookup, is
/// doing the right thing and must pass through byte-identical.
#[test]
fn an_ordinary_reply_passes_through_untouched() {
    let reply = "Ta sortie de mardi était la plus grosse de la semaine. \
                 Comment les jambes aujourd'hui?";
    let scrub = scrub_ungrounded_data_appeals(reply);

    assert!(!scrub.fired(), "nothing here appeals to a fetch");
    assert_eq!(scrub.cleaned, reply);
}

/// Hedged language is not an appeal. "D'après ce que tu me dis" cites the
/// athlete, which is exactly what an ungrounded turn should do.
#[test]
fn citing_the_athlete_rather_than_data_is_not_an_appeal() {
    let reply = "D'après ce que tu me dis, Road 2 AUS était mardi.";
    let scrub = scrub_ungrounded_data_appeals(reply);

    assert!(
        !scrub.fired(),
        "attributing a fact to the athlete is the honest move on an ungrounded \
         turn, and must not be scrubbed: {}",
        scrub.cleaned
    );
}
