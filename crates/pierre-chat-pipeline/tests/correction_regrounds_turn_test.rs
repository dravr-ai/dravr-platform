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

use pierre_chat_pipeline::stages::claim_density::{
    previous_reply_asserted_athlete_facts, previous_reply_asserted_unsupplied_facts,
};
use pierre_llm::ChatMessage;
use pierre_services::conversation_compaction::REPLAYED_SUMMARY_PREFIX;
use pierre_services::okf::BUNDLE_HEADER;

/// The agent's actual reply from the turn Raph corrected.
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

/// During an interview the agent's question quotes the athlete's answers back.
/// Live 2026-09-30 on `/season`, «150 km (5 juin 2027) … (25–65 km)» restated
/// the athlete's own calendar; numbers the athlete supplied are not claims.
#[test]
fn an_interview_question_quoting_the_athletes_answers_asserts_nothing() {
    let messages = vec![
        ChatMessage::system("coach prompt"),
        ChatMessage::user("L'ultra de 150 km le 5 juin 2027, et des trails de 25-65 km."),
        ChatMessage::assistant(
            "Noté : 150 km (5 juin 2027) en course A, et des trails de 25–65 km autour. \
             Laquelle vient en premier ?",
        ),
        ChatMessage::user("Le VTXL en juin."),
    ];
    assert!(
        previous_reply_asserted_athlete_facts(&messages),
        "the plain count reads five numbers as a reconstruction"
    );
    assert!(
        !previous_reply_asserted_unsupplied_facts(&messages),
        "every number was the athlete's own, so the interview count sees no claim"
    );
}

/// Figures the athlete never gave still count mid-interview, and a number the
/// athlete only mentions AFTER the reply does not excuse it.
#[test]
fn an_interview_question_inventing_volume_still_asserts_facts() {
    let messages = vec![
        ChatMessage::user("Je fais surtout du trail, 25 km le dimanche."),
        ChatMessage::assistant(
            "Le mois dernier tu as couru 161 km avec 2391 m de dénivelé en 6,2h, pour des \
             sorties de 25 km. Quelle est ta course A ?",
        ),
        ChatMessage::user("C'est faux, pas 161 km ni 2391 m."),
    ];
    assert!(previous_reply_asserted_unsupplied_facts(&messages));
    assert!(!previous_reply_asserted_unsupplied_facts(&[]));
}

/// A system prompt shaped like the assembled one: instructions and today's
/// date outside the dossier, the athlete's facts fenced in `<user_fact>` by
/// the OKF bundle renderer.
fn prompt_with_dossier(facts: &[&str]) -> String {
    let mut prompt = String::from(
        "coach prompt. Today is 2027-06-05 (Saturday). Call at most 30 tools, 9 per turn.\n\
         # Pillar context for this user\n",
    );
    for fact in facts {
        prompt.push_str("<user_fact kind=\"goal\" source=\"onboarding\" confidence=\"0.90\">");
        prompt.push_str(fact);
        prompt.push_str("</user_fact>\n");
    }
    prompt
}

/// The dossier in the system prompt is context the model was handed: a walk
/// question restating a stored goal and availability claims nothing, even
/// when the athlete never typed those figures in this conversation.
#[test]
fn an_interview_question_restating_the_dossier_asserts_nothing() {
    let messages = vec![
        ChatMessage::system(prompt_with_dossier(&[
            "Training for: Ultra-Trail 150 km on 2027-06-05",
            "Can train on: Tuesdays and Thursdays, 45 min before work; long run Sunday 3 h",
        ])),
        ChatMessage::user("Salut"),
        ChatMessage::assistant(
            "Ton ultra de 150 km le 5 juin 2027, avec 45 min mardi et jeudi et 3 h le \
             dimanche : c'est toujours d'actualité ?",
        ),
        ChatMessage::user("Oui."),
    ];
    assert!(
        previous_reply_asserted_athlete_facts(&messages),
        "the plain count reads the restated calendar as a reconstruction"
    );
    assert!(!previous_reply_asserted_unsupplied_facts(&messages));
}

/// A figure is compared by value: a reformatted date, thousands separator or
/// decimal mark is the same figure the athlete gave.
#[test]
fn a_reformatted_figure_is_still_the_athletes() {
    let messages = vec![
        ChatMessage::user("Le 05/06/2027, 2\u{202f}391 m de D+ en 6,2 h, et 1 200 km par an."),
        ChatMessage::assistant(
            "Donc le 2027-06-05 : 2391 m de dénivelé, 6.2 h d'effort, 1200 km cette année.",
        ),
    ];
    assert!(!previous_reply_asserted_unsupplied_facts(&messages));

    // Same reply, figures the athlete never gave: all four count.
    let invented = vec![
        ChatMessage::user("Je vise l'ultra."),
        ChatMessage::assistant(
            "Donc le 2027-06-05 : 2391 m de dénivelé, 6.2 h d'effort, 1200 km cette année.",
        ),
    ];
    assert!(previous_reply_asserted_unsupplied_facts(&invented));
}

/// An earlier assistant turn does not vouch for a figure: an ungrounded claim
/// repeated is still ungrounded.
#[test]
fn an_earlier_unsupported_claim_does_not_launder_itself() {
    let messages = vec![
        ChatMessage::system("coach prompt"),
        ChatMessage::user("Je fais du trail."),
        ChatMessage::assistant("Le mois dernier : 161 km, 2391 m, 6,2 h."),
        ChatMessage::user("Et donc ?"),
        ChatMessage::assistant("Comme dit : 161 km, 2391 m de dénivelé en 6,2 h."),
    ];
    assert!(previous_reply_asserted_unsupplied_facts(&messages));
}

/// Only the dossier's facts are the athlete's. Today's date and the
/// instruction limits sit in the same system prompt, and small invented
/// figures that happen to match them — 30, 9, a 5/6 date — are still claims.
#[test]
fn the_prompt_outside_the_dossier_supplies_nothing() {
    let messages = vec![
        ChatMessage::system(prompt_with_dossier(&["Training for: a first trail"])),
        ChatMessage::user("Je veux faire du trail."),
        ChatMessage::assistant("Tu fais déjà 30 km, 9 sorties, et ta course est le 5/6/2027."),
        ChatMessage::user("C'est faux."),
    ];
    assert!(
        previous_reply_asserted_unsupplied_facts(&messages),
        "30, 9 and the date are in the prompt's instructions and clock, not the athlete's data"
    );
}

/// Ambiguous groupings read as separate numbers: «semaine 3 150 km» is a week
/// and a distance, «10,12,15 km» is a list. Each alone makes three figures.
#[test]
fn ambiguous_groupings_count_as_separate_figures() {
    for reply in ["Semaine 3 150 km, en 5 h.", "Tes sorties : 10,12,15 km."] {
        let messages = vec![
            ChatMessage::user("Je fais du trail."),
            ChatMessage::assistant(reply),
        ];
        assert!(
            previous_reply_asserted_unsupplied_facts(&messages),
            "{reply:?} carries three figures"
        );
    }

    // A decimal standing alone is one figure; a list is not a decimal.
    let decimal = vec![
        ChatMessage::user("Je fais du trail."),
        ChatMessage::assistant("Environ 6,2 h et 150 km."),
    ];
    assert!(!previous_reply_asserted_unsupplied_facts(&decimal));
}

/// A month name is a date only next to a day. «may» and «sept» as words
/// supply no number, so a reply's 5 and 9 stay unsupplied.
#[test]
fn a_month_word_alone_supplies_no_number() {
    let messages = vec![
        ChatMessage::user("I may start in sept, we'll see."),
        ChatMessage::assistant("So 5 rides, 9 km each, 12 hours a week?"),
    ];
    assert!(previous_reply_asserted_unsupplied_facts(&messages));

    // Next to a day it is a date, and the date is supplied whole.
    let dated = vec![
        ChatMessage::user("My race is on June 5, 2027 and the other on 12 sept."),
        ChatMessage::assistant("Race A on 2027-06-05, race B on 12/09, and 5/6 again."),
    ];
    assert!(!previous_reply_asserted_unsupplied_facts(&dated));
}

/// Two race dates as each shipped locale writes them — 5 June and 12
/// September 2027. Read as scattered numbers, each pair leaves four
/// unsupplied (5, 2027, 12, 2027), so a date that fails to parse fires.
const LOCALE_DATES: [(&str, &str); 7] = [
    ("fr", "le 5 juin 2027 et le 12 septembre 2027"),
    ("fr abbreviated", "le 5 juin 2027 et le 12 sept. 2027"),
    ("en", "June 5, 2027 and September 12th, 2027"),
    ("en day-first", "the 5th of June 2027 and 12 Sep 2027"),
    ("es", "el 5 de junio de 2027 y el 12 de septiembre de 2027"),
    ("pt", "em 5 de junho de 2027 e 12 de setembro de 2027"),
    ("de", "am 5. Juni 2027 und am 12. September 2027"),
];

/// A restated date matches its numeric and ISO forms in every locale, in
/// both directions: the athlete typed ISO and the agent wrote it out, or the
/// athlete wrote it out and the agent answered in ISO.
#[test]
fn dates_restated_in_every_locale_are_the_athletes() {
    for (locale, written) in LOCALE_DATES {
        let restated = vec![
            ChatMessage::user("Mes courses : 2027-06-05 et 12/09/2027, 150 km chacune."),
            ChatMessage::assistant(format!("Donc {written}, 150 km.")),
        ];
        assert!(
            !previous_reply_asserted_unsupplied_facts(&restated),
            "{locale}: «{written}» must restate the athlete's ISO and numeric dates"
        );

        let answered_in_iso = vec![
            ChatMessage::user(format!("Mes courses : {written}.")),
            ChatMessage::assistant("Noté : 2027-06-05 et 12.09.2027."),
        ];
        assert!(
            !previous_reply_asserted_unsupplied_facts(&answered_in_iso),
            "{locale}: the athlete's «{written}» must supply the ISO and dotted forms"
        );
    }
}

/// A year is four digits in 1900–2100 and does not cross a comma after a
/// day-first date: the elevation after «le 5 juin,» is its own figure, and
/// so is one no calendar year could be.
#[test]
fn a_figure_after_a_date_is_not_its_year() {
    let context = ChatMessage::user("Mes sorties : 5 juin, 12 juin, 19 juin.");
    let after_comma = vec![
        context.clone(),
        ChatMessage::assistant("Le 5 juin, 1950 m ; le 12 juin, 2050 m ; le 19 juin, 2080 m."),
    ];
    assert!(
        previous_reply_asserted_unsupplied_facts(&after_comma),
        "three elevations the athlete never gave, each after a comma"
    );

    let out_of_range = vec![
        context,
        ChatMessage::assistant("Le 5 juin 2400 m, le 12 juin 2600 m, le 19 juin 2800 m."),
    ];
    assert!(
        previous_reply_asserted_unsupplied_facts(&out_of_range),
        "2400, 2600 and 2800 are not years"
    );
}

/// The compaction summary rides in the User role but a model wrote it from
/// earlier assistant turns, so the figures in it are not the athlete's.
#[test]
fn the_compaction_summary_supplies_nothing() {
    let messages = vec![
        ChatMessage::user(format!(
            "{REPLAYED_SUMMARY_PREFIX}The coach noted 161 km, 2391 m and 6,2 h last month."
        )),
        ChatMessage::user("Je fais du trail."),
        ChatMessage::assistant("Le mois dernier : 161 km, 2391 m, 6,2 h."),
    ];
    assert!(previous_reply_asserted_unsupplied_facts(&messages));
}

/// The bundle header names the `<user_fact>` tag in prose; only the fenced
/// facts after it are the athlete's, never the text around them.
#[test]
fn only_well_formed_dossier_facts_supply_figures() {
    let prompt = format!(
        "agent persona{BUNDLE_HEADER}Call at most 30 tools, 9 per turn, for 7 h.\n\
         <user_fact kind=\"goal\" source=\"onboarding\" confidence=\"0.90\">Ultra 150 km \
         on 2027-06-05</user_fact>\n"
    );
    let invented = vec![
        ChatMessage::system(prompt.clone()),
        ChatMessage::user("Salut"),
        ChatMessage::assistant("Tu fais 30 km, 9 sorties, 7 h par semaine."),
    ];
    assert!(previous_reply_asserted_unsupplied_facts(&invented));

    let restated = vec![
        ChatMessage::system(prompt),
        ChatMessage::user("Salut"),
        ChatMessage::assistant("Ton ultra de 150 km le 5 juin 2027, et 150 km encore."),
    ];
    assert!(!previous_reply_asserted_unsupplied_facts(&restated));
}
