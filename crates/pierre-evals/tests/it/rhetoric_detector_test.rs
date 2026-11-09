// ABOUTME: External tests for the rhetoric-vs-propositional classifier (rhetoric_detector.rs)
// ABOUTME: Verifies motivational/greeting/question/empty text vs factual claims
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::rhetoric_detector::{classify, RhetoricVerdict};

#[test]
fn motivational_is_rhetorical() {
    assert_eq!(classify("You're crushing it!"), RhetoricVerdict::Rhetorical);
    assert_eq!(
        classify("Great job on that workout."),
        RhetoricVerdict::Rhetorical
    );
    assert_eq!(classify("Keep it up."), RhetoricVerdict::Rhetorical);
}

#[test]
fn greetings_are_rhetorical() {
    assert_eq!(classify("Hi there!"), RhetoricVerdict::Rhetorical);
    assert_eq!(
        classify("Good morning, champ."),
        RhetoricVerdict::Rhetorical
    );
}

#[test]
fn questions_are_rhetorical() {
    assert_eq!(
        classify("How do you feel today?"),
        RhetoricVerdict::Rhetorical
    );
    assert_eq!(
        classify("Are you ready for tomorrow?"),
        RhetoricVerdict::Rhetorical
    );
}

#[test]
fn an_ask_of_the_athlete_is_rhetorical_in_every_supported_locale() {
    for claim in [
        "Tell me your FTP.",
        "Tell me how the session went.",
        "Give me your resting heart rate.",
        "Dis-moi comment s'est passée ta séance.",
        "Donne-moi ton FTP.",
        "Dites-moi comment vous vous sentez.",
        "Dime tu FTP.",
        "Dame tu frecuencia cardíaca en reposo.",
        "Sag mir deine FTP.",
        "Nenn mir deine FTP.",
        "Diz-me o teu FTP.",
        "Dá-me o teu FTP.",
    ] {
        assert_eq!(classify(claim), RhetoricVerdict::Rhetorical, "{claim}");
    }
}

/// The tradeoff of catching asks: a sentence that opens like one but carries a
/// figure or a second clause is still checked.
#[test]
fn an_ask_carrying_a_claim_is_still_checked() {
    for claim in [
        "Tell me your FTP, since creatine doubles it.",
        "Tell me your FTP because creatine doubles it.",
        "Tell me if you can hold 300 W for an hour.",
        "Donne-moi ton FTP, car la créatine le double.",
        "Dime tu FTP porque la creatina lo duplica.",
    ] {
        assert_eq!(classify(claim), RhetoricVerdict::Propositional, "{claim}");
    }
}

/// Imperatives that prescribe are not asks: the later layers must see them.
#[test]
fn a_prescription_is_not_an_ask() {
    for claim in [
        "Eat 60 g of carbs per hour on long rides.",
        "Take creatine every day to raise your FTP.",
        "Share the load across three sessions a week.",
        "Dimension your long run to your weekly volume.",
        "Share your weekly volume across three sessions.",
        "Partage ta charge sur trois séances.",
        "Partage ton volume sur la semaine.",
    ] {
        assert_eq!(classify(claim), RhetoricVerdict::Propositional, "{claim}");
    }
}

#[test]
fn factual_claim_is_propositional() {
    assert_eq!(
        classify("Your VO2max is 58 ml/kg/min."),
        RhetoricVerdict::Propositional
    );
    assert_eq!(
        classify("Creatine at 5g per day increases power output."),
        RhetoricVerdict::Propositional
    );
}

/// A phrase only counts at the start of the sentence. These all skipped every
/// check while the match ran anywhere: `hey ` inside `they ` and `whey `.
#[test]
fn a_phrase_further_in_does_not_make_a_sentence_rhetorical() {
    for claim in [
        "They recover faster with 30 g of protein after hard sessions.",
        "A whey shake after training raises muscle protein synthesis.",
        "Creatine absolutely increases power output in repeated sprints.",
        "Your long run was, of course, 40 km at 300 W.",
    ] {
        assert_eq!(classify(claim), RhetoricVerdict::Propositional, "{claim}");
    }
}

#[test]
fn encouragement_is_rhetorical_in_every_supported_locale() {
    for claim in [
        "Bravo pour ta séance de ce matin.",
        "Bien joué pour cette semaine de récupération.",
        "Buen trabajo con la sesión de hoy.",
        "Gut gemacht bei deinem Lauf heute.",
        "Bom trabalho na sessão de hoje.",
    ] {
        assert_eq!(classify(claim), RhetoricVerdict::Rhetorical, "{claim}");
    }
}

#[test]
fn empty_is_rhetorical() {
    assert_eq!(classify(""), RhetoricVerdict::Rhetorical);
    assert_eq!(classify("   "), RhetoricVerdict::Rhetorical);
}

#[test]
fn punctuation_only_is_rhetorical() {
    assert_eq!(classify("..."), RhetoricVerdict::Rhetorical);
}

/// A courtesy alone states nothing, in every supported locale, and an address
/// after it ("Good morning, champ.") is still no claim.
#[test]
fn a_bare_interjection_is_rhetorical_in_every_supported_locale() {
    for claim in [
        "Of course!",
        "Absolutely.",
        "Thank you, Paul!",
        "Bien sûr !",
        "Merci, Marie.",
        "Por supuesto.",
        "Gracias, Ana.",
        "Selbstverständlich!",
        "Danke, Lukas.",
        "Claro que sim!",
        "Obrigado, João.",
    ] {
        assert_eq!(classify(claim), RhetoricVerdict::Rhetorical, "{claim}");
    }
}

/// A courtesy opening a claim does not excuse the claim: a second clause or a
/// figure after it sends the sentence on to be checked. These all skipped every
/// check while the opening alone decided.
#[test]
fn an_interjection_opening_a_claim_is_still_checked_in_every_supported_locale() {
    for claim in [
        "Of course, creatine doubles your FTP.",
        "Absolutely, creatine doubles your FTP.",
        "Of course creatine adds 20 W to your FTP.",
        "Bien sûr, la créatine double ta FTP.",
        "Merci, la créatine double ta FTP.",
        "Bravo pour tes 300 W, la créatine les double.",
        "Por supuesto, la creatina duplica tu FTP.",
        "Gracias, la creatina duplica tu FTP.",
        "Selbstverständlich, Kreatin verdoppelt deine FTP.",
        "Danke, Kreatin verdoppelt deine FTP.",
        "Claro que sim, a creatina duplica o teu FTP.",
        "Obrigado, a creatina duplica o teu FTP.",
    ] {
        assert_eq!(classify(claim), RhetoricVerdict::Propositional, "{claim}");
    }
}
