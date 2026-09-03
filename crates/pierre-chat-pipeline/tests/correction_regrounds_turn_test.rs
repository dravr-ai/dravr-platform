// ABOUTME: Pins the structural recovery trigger — a correction re-grounds the turn, no vocabulary
// ABOUTME: Regression for 2026-09-02, where the last five disputed turns matched no data-ask term
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Whether the Guardian repair pass ran was decided by a lowercase substring
//! list. It fired on 7 of 15 turns live on 2026-09-02 — and on **none of the
//! last five**, which were exactly the turns where the athlete was disputing
//! facts about his own data:
//!
//! | athlete | matched a term? |
//! |---|---|
//! | *"300km de dimanche? Tu parles de quoi?"* | no |
//! | *"road 2 aus etait hier, mardi. T'es melé big"* | no |
//! | *"date ride etait lundi. Ca va pas les dates"* | no |
//! | *"oui road 2 aus serait une longue"* | no |
//! | *"repose toi ton indice de mêlé est dans le tapis"* | no |
//!
//! A correction is not phrased like a question, so it never looks like a data
//! ask — yet it is the strongest available signal that grounding is wrong. The
//! replacement reads no words at all: it asks whether the reply the athlete is
//! answering asserted concrete numbers about their training.

use pierre_chat_pipeline::stages::capability_recovery::previous_reply_asserted_athlete_facts;
use pierre_llm::ChatMessage;

/// The coach's actual reply from the turn Raph corrected.
const RECONSTRUCTION: &str = "Ça donne: mardi ta grosse sortie (161 km/2391m), \
mercredi ce matin le Date ride (16 km/414m, plus léger), dimanche Roooadie \
(52 km/485m), vendredi Passion rando (26 km/895m).";

#[test]
fn a_reply_that_reconstructs_a_week_counts_as_asserting_facts() {
    let messages = vec![
        ChatMessage::system("coach prompt"),
        ChatMessage::user("tu penses quoi de ma ride d'hier"),
        ChatMessage::assistant(RECONSTRUCTION),
    ];

    assert!(
        previous_reply_asserted_athlete_facts(&messages),
        "five dated activities with distances is a reconstruction, and a \
         reconstruction built on nothing is what the athlete pushed back on"
    );
}

/// The signal is the numbers, not the language — so it holds for every locale
/// the platform ships without a translation table.
#[test]
fn the_signal_survives_translation() {
    for reply in [
        "Tuesday was your big ride: 161 km, 2391 m, 6.2h.",
        "El martes fue tu salida grande: 161 km, 2391 m, 6,2h.",
        "Dienstag war deine große Ausfahrt: 161 km, 2391 m, 6,2 Std.",
    ] {
        let messages = vec![ChatMessage::assistant(reply)];
        assert!(
            previous_reply_asserted_athlete_facts(&messages),
            "a structural signal must not depend on language: {reply:?}"
        );
    }
}

/// A social reply asserts nothing, so pushing back on it re-grounds nothing.
/// This is what keeps the trigger from firing on every turn in a chatty room.
#[test]
fn a_social_reply_does_not_arm_the_trigger() {
    for reply in [
        "Bonne idée, repos bien mérité. On se reparle demain 💪",
        "Bravo! Belle sortie.",
        "Comment les jambes aujourd'hui — lourdes ou ça va?",
    ] {
        let messages = vec![ChatMessage::assistant(reply)];
        assert!(
            !previous_reply_asserted_athlete_facts(&messages),
            "no numbers means no factual reconstruction to dispute: {reply:?}"
        );
    }
}

/// One or two numbers is a remark or a comparison, not a reconstruction.
#[test]
fn a_passing_number_is_not_a_reconstruction() {
    let messages = vec![ChatMessage::assistant(
        "Ta sortie de 161 km, c'était du solide.",
    )];

    assert!(
        !previous_reply_asserted_athlete_facts(&messages),
        "a single quoted figure is a remark, and re-grounding every one of \
         those would fire the repair pass on half the conversation"
    );
}

/// It reads the most recent assistant turn, which is the one the athlete is
/// answering — not an older one further up the window.
#[test]
fn it_reads_the_reply_the_athlete_is_answering() {
    let messages = vec![
        ChatMessage::assistant(RECONSTRUCTION),
        ChatMessage::user("road 2 aus etait hier, mardi"),
        ChatMessage::assistant("Bonne idée, on se reparle demain."),
    ];

    assert!(
        !previous_reply_asserted_athlete_facts(&messages),
        "the last assistant turn asserted nothing; an older one must not arm \
         the trigger for it"
    );
}

/// An empty turn cannot assert anything, and must not panic.
#[test]
fn no_assistant_turn_yet_is_not_an_assertion() {
    assert!(!previous_reply_asserted_athlete_facts(&[]));
    assert!(!previous_reply_asserted_athlete_facts(&[
        ChatMessage::system("coach prompt"),
        ChatMessage::user("salut"),
    ]));
}
