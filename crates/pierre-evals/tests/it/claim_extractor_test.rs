// ABOUTME: External tests for the heuristic claim extractor (claim_extractor.rs)
// ABOUTME: Covers sentence splitting, category classification (incl. French), and extraction
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::claim_extractor::{
    classify_heuristic, extract_heuristic, split_sentences, ClaimSource, ExtractedClaim,
};
use pierre_memory::ClaimCategory;

#[test]
fn splits_on_sentence_boundaries() {
    let s = split_sentences("Run 5x1km. Take 2 min rest. You got this!");
    assert_eq!(s.len(), 3);
}

#[test]
fn classifies_nutrition_claim() {
    assert_eq!(
        classify_heuristic("Aim for 1.6g of protein per kg of body weight."),
        Some(ClaimCategory::Nutrition)
    );
}

#[test]
fn classifies_physiological_claim() {
    assert_eq!(
        classify_heuristic("Your VO2max sits around 58 ml/kg/min."),
        Some(ClaimCategory::Physiological)
    );
}

#[test]
fn classifies_supplement_claim() {
    assert_eq!(
        classify_heuristic("Creatine at 5g per day supports power output."),
        Some(ClaimCategory::Supplement)
    );
}

#[test]
fn rejects_non_factual_sentence() {
    assert_eq!(classify_heuristic("You're crushing it!"), None);
}

#[test]
fn french_hr_observation_classifies_as_physiological_not_nutrition() {
    // Regression: "carburant" (fuel/gasoline, used metaphorically here for
    // "refuel") contains the substring "carb", so the old contains()-based
    // classifier flagged this training-intensity observation as Nutrition.
    let claim = "Ta séance était plutôt facile à modérée: 49 min, 5,39 km, \
                 FC moy 111, donc le besoin principal ce matin, c'est surtout \
                 de remettre du carburant.";
    // Tokens: fc, moy, séance → multiple Physiological + TrainingPrescription
    // hits. Either is correct; the bug was Nutrition being the verdict.
    let result = classify_heuristic(claim);
    assert!(
        matches!(
            result,
            Some(ClaimCategory::Physiological | ClaimCategory::TrainingPrescription)
        ),
        "expected physiological/training but got {result:?}"
    );
}

/// A figure stated about one of the athlete's own sessions is routed to their
/// record in every shipped locale, even where heart-rate words would win the
/// keyword count for Physiological (carnet#1188).
#[test]
fn an_own_session_figure_routes_to_the_record_in_every_locale() {
    for sentence in [
        // en
        "Your last run, on Saturday, was 6.7 km with 173 m of climbing at an easy heart rate.",
        "Your heart rate stayed at 133 bpm for the whole 6.7 km.",
        // fr — the 2026-10-26 sentence, and the two the issue saw dropped
        "Ta dernière séance, samedi, c'était 6,7 km faciles avec 173 m de dénivelé, \
         et ta FC est restée à 133.",
        "Ta FC est restée à 133 sur toute la sortie.",
        "Dimanche, tu es passé à environ 36 km sur ta sortie longue.",
        // es
        "Tu última salida, el sábado, fueron 6,7 km con 173 m de desnivel y la FC en 133.",
        "Tu FC se mantuvo en 133 durante los 6,7 km.",
        // de
        "Deine letzte Einheit am Samstag waren 6,7 km mit 173 m und einer HF von 133.",
        "Dein Puls blieb bei 133 auf den 6,7 km.",
        // pt
        "O teu último treino, no sábado, foram 6,7 km com 173 m e FC de 133.",
        "A tua FC ficou em 133 nos 6,7 km.",
    ] {
        assert_eq!(
            classify_heuristic(sentence),
            Some(ClaimCategory::AthleteData),
            "bug: an own-session figure went to the judge instead of the record: {sentence}"
        );
    }
}

/// The routing needs the athlete's own session, not just a figure and a
/// heart-rate word: physiology, a plan and a question stay where they were.
#[test]
fn physiology_plans_and_questions_are_not_own_session_figures() {
    for sentence in [
        "Ta FC max se situe autour de 185 bpm.",
        "Your VO2max sits around 58 ml/kg/min.",
        "Samedi : sortie longue de 18 km avec ta FC sous 140.",
        "Ta prochaine séance : 10 km avec ta FC sous 140.",
        "Ta dernière séance, samedi, c'était bien 6,7 km ?",
        "Si ta FC est restée à 133 sur 10 km, c'est de l'endurance.",
    ] {
        assert_ne!(
            classify_heuristic(sentence),
            Some(ClaimCategory::AthleteData),
            "bug: routed to the record: {sentence}"
        );
    }
}

/// A "last" session that is still to come — the last one before a race, the
/// last one of this week — is a plan, in every shipped locale. These are the
/// phrasings the carnet#1188 review found reaching the record.
pub(crate) const LOOK_AHEAD_LAST_SESSIONS: [&str; 9] = [
    "Cette semaine, ta dernière séance : 12 km samedi.",
    "Your last long run before the race is 30 km on Sunday.",
    "Ta dernière sortie longue avant le marathon : 32 km.",
    "Tu última tirada antes de la carrera: 28 km el domingo.",
    "Deine letzte lange Einheit vor dem Rennen: 30 km am Sonntag.",
    "Deine letzte Einheit diese Woche: 12 km am Samstag.",
    "Tua última corrida longa antes da prova: 30 km no domingo.",
    "Teu último treino antes da maratona será de 32 km.",
    "Your last session this week: 12 km on Saturday.",
];

#[test]
fn a_last_session_still_to_come_is_not_the_athletes_record() {
    for sentence in LOOK_AHEAD_LAST_SESSIONS {
        assert_ne!(
            classify_heuristic(sentence),
            Some(ClaimCategory::AthleteData),
            "bug: a plan routed to the record: {sentence}"
        );
    }
}

/// The same "last" with a past report stays the record: a record verb or a
/// held value is past tense, whatever "before" the sentence also carries.
#[test]
fn a_reported_session_before_an_event_stays_the_record() {
    for sentence in [
        "You ran 30 km before the race, a solid last long run.",
        "Ta dernière séance avant le week-end : ta FC est restée à 133 sur 6,7 km.",
        "Antes de la carrera corriste 28 km a ritmo suave.",
    ] {
        assert_eq!(
            classify_heuristic(sentence),
            Some(ClaimCategory::AthleteData),
            "bug: a past report before an event must stay the athlete's record: {sentence}"
        );
    }
}

/// A record verb speaks for its own clause. A physiology figure in another
/// clause keeps the category it scores, so the layers that rule on physiology
/// still see it.
#[test]
fn a_record_verb_does_not_take_a_physiology_figure_in_another_clause() {
    for sentence in [
        "You ran 10 km; a VO2max of 55 is above average for your age.",
        "Tu as couru 10 km, et une VO2max de 55 est au-dessus de la moyenne.",
    ] {
        assert_eq!(
            classify_heuristic(sentence),
            Some(ClaimCategory::Physiological),
            "bug: a record verb must not take a physiology figure in another clause: {sentence}"
        );
    }
    // A heart rate is the session's own figure, in its clause or the next.
    assert_eq!(
        classify_heuristic("Tu as couru 10 km, avec une FC moyenne de 140 bpm."),
        Some(ClaimCategory::AthleteData),
        "bug: a session heart rate in the next clause stays the athlete's record"
    );
}

/// A word that is a cue in one language is an ordinary word in another:
/// Spanish `tu`/`tus` is the French subject pronoun. A French sentence with
/// `tu` and a figure is never read through the Spanish list, and English
/// "sat at" is a Saturday, not a value that held.
#[test]
fn a_homograph_does_not_route_another_languages_sentence() {
    for sentence in [
        "Tu es à environ 55 de VO2max, c'est au-dessus de la moyenne pour ton âge.",
        "Tu sais, la FC du groupe est restée à 150 sur 12 km.",
        "Tu FC est restée à 133 sur 12 km.",
        "Tu verras, samedi 12 km et ta forme suivra.",
        "Your long run Sat at 7:00 is 20 km easy.",
    ] {
        assert_ne!(
            classify_heuristic(sentence),
            Some(ClaimCategory::AthleteData),
            "bug: a homograph routed to the record: {sentence}"
        );
    }
}

#[test]
fn carbohydrate_word_still_triggers_nutrition() {
    // Boundary check: legitimate Nutrition claims must still classify.
    assert_eq!(
        classify_heuristic("Aim for 60g of carbs per hour during long efforts."),
        Some(ClaimCategory::Nutrition)
    );
}

#[test]
fn french_nutrition_keywords_classify_as_nutrition() {
    assert_eq!(
        classify_heuristic("Vise 1,6 g de protéines par kg de poids corporel."),
        Some(ClaimCategory::Nutrition)
    );
}

#[test]
fn extract_heuristic_returns_multiple_claims() {
    let reply = "Aim for 1.6g of protein per kg of body weight. Your VO2max is around 58. \
                 Creatine at 5g per day helps recovery. Nice work today!";
    let claims = extract_heuristic(reply);
    assert!(claims.len() >= 3);
}

#[test]
fn extract_heuristic_empty_on_motivation() {
    let reply = "You're crushing it! Keep going! Amazing work!";
    assert_eq!(extract_heuristic(reply), []);
}

#[test]
fn a_capability_report_is_not_a_claim() {
    for report in [
        "Je n'arrive pas à afficher tes dernières sorties avant ta séance.",
        "I can't show your recent runs before your session right now.",
        "No puedo ver tus últimas sesiones de entrenamiento ahora mismo.",
        "Ich kann deine letzten Einheiten gerade nicht anzeigen, sorry.",
        "Não consigo mostrar os teus últimos treinos com a sessão agora.",
    ] {
        assert!(
            extract_heuristic(report).is_empty(),
            "bug: a capability report was extracted as a claim: {report}"
        );
    }
}

#[test]
fn a_german_sentence_that_can_see_the_data_stays_a_claim() {
    let claims = extract_heuristic("Ich kann sehen, dass deine Herzfrequenz in Zone 2 lag.");
    assert_eq!(
        claims.len(),
        1,
        "bug: a German sentence that sees the data was dropped as a capability report"
    );
}

#[test]
fn a_sentence_opening_on_cant_without_data_access_stays_a_claim() {
    let claims = extract_heuristic("I can't stress enough that protein at every meal matters.");
    assert_eq!(
        claims.len(),
        1,
        "bug: \"I can't\" without a data-access verb was dropped"
    );
    assert_eq!(
        claims[0].category,
        ClaimCategory::Nutrition,
        "bug: the kept claim lost its category"
    );
}

#[test]
fn a_question_in_parentheses_does_not_end_the_sentence() {
    let s =
        split_sentences("Dis-moi ce qui est prévu (des 400 m, des 1000 m ?) et je l'ajuste. Ok!");
    assert_eq!(
        s,
        vec![
            "Dis-moi ce qui est prévu (des 400 m, des 1000 m ?) et je l'ajuste.".to_owned(),
            "Ok!".to_owned()
        ],
        "bug: a terminator inside parentheses ended the sentence"
    );
}

#[test]
fn an_extracted_claim_prints_the_size_of_its_text_never_the_text() {
    let claim = ExtractedClaim {
        text: "sentinel-claim-7f3a".to_owned(),
        category: ClaimCategory::Nutrition,
        source: ClaimSource::Caller,
    };

    for printed in [format!("{claim:?}"), format!("{claim:#?}")] {
        assert!(
            !printed.contains("sentinel-claim-7f3a"),
            "the claim text leaked: {printed}"
        );
        assert!(
            printed.contains("ExtractedClaim"),
            "the type is named: {printed}"
        );
        assert!(
            printed.contains("text_len"),
            "the claim states its size: {printed}"
        );
    }
}
