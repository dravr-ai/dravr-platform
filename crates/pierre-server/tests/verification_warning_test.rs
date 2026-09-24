// ABOUTME: Tests the claim-verification caveat-banner selection (actionable_problems + warning_bullets)
// ABOUTME: Prescriptions: Unsupported suppressed, Contradicted kept; reply-sourced claims always reach the banner
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![cfg(feature = "tools-verification")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_chat_pipeline::stages::verification::{
    actionable_problems, warn_affordance, warning_bullets, FlaggedClaim, WarnAffordance,
};
use pierre_evals::{extract_heuristic, ClaimSource, ExtractedClaim, VerdictOutcome};
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};

fn claim(text: &str, category: ClaimCategory) -> ExtractedClaim {
    ExtractedClaim {
        text: text.to_owned(),
        category,
        source: ClaimSource::Reply,
    }
}

fn flagged(text: &str, contradicted: bool, source: ClaimSource) -> FlaggedClaim<'_> {
    FlaggedClaim {
        text,
        contradicted,
        source,
    }
}

fn outcome(status: ClaimStatus) -> VerdictOutcome {
    VerdictOutcome {
        status,
        evidence_strength: EvidenceStrength::None,
        confidence: 0.0,
        layer_fired: VerdictLayer::Evidence,
        explanation: String::new(),
        evidence_refs: None,
    }
}

#[test]
fn prescription_unsupported_is_suppressed_but_data_claim_kept() {
    // A training prescription with no corpus evidence is advice, not a false
    // claim — it must NOT appear in the caveat banner. A physiological data
    // claim that is unsupported still does.
    let verdicts = vec![
        (
            claim(
                "Jeu: velo Z2 75-90 min",
                ClaimCategory::TrainingPrescription,
            ),
            outcome(ClaimStatus::Unsupported),
        ),
        (
            claim("Ton CTL est passe de 74 a 88", ClaimCategory::Physiological),
            outcome(ClaimStatus::Unsupported),
        ),
    ];
    let problems = actionable_problems(&verdicts);
    let texts: Vec<&str> = problems.iter().map(|p| p.text).collect();
    assert_eq!(texts, vec!["Ton CTL est passe de 74 a 88"]);
    assert!(problems.iter().all(|p| !p.contradicted));
}

#[test]
fn contradicted_prescription_is_kept() {
    // A prescription that violated a deterministic bound (impossible load) is
    // genuinely bad advice and stays surfaced.
    let verdicts = vec![(
        claim("Cours 200 km demain", ClaimCategory::TrainingPrescription),
        outcome(ClaimStatus::Contradicted),
    )];
    let problems = actionable_problems(&verdicts);
    assert_eq!(
        problems,
        vec![flagged("Cours 200 km demain", true, ClaimSource::Reply)]
    );
}

#[test]
fn supported_and_rhetorical_claims_are_not_flagged() {
    let verdicts = vec![
        (
            claim("Adults need 7-9 hours of sleep", ClaimCategory::Recovery),
            outcome(ClaimStatus::Supported),
        ),
        (
            claim("You got this!", ClaimCategory::Recovery),
            outcome(ClaimStatus::Rhetorical),
        ),
    ];
    assert!(actionable_problems(&verdicts).is_empty());
}

#[test]
fn warning_bullets_caps_at_five_and_keeps_all_contradicted() {
    // 7 problems (4 unsupported + 3 contradicted); the cap is 5 and the three
    // bound-violations must survive it.
    let problems: Vec<FlaggedClaim<'_>> = [
        ("u1", false),
        ("u2", false),
        ("u3", false),
        ("u4", false),
        ("c1", true),
        ("c2", true),
        ("c3", true),
    ]
    .into_iter()
    .map(|(text, contradicted)| flagged(text, contradicted, ClaimSource::Reply))
    .collect();
    let bullets = warning_bullets(&problems, "short reply");
    assert_eq!(bullets.len(), 5);
    for c in ["- c1", "- c2", "- c3"] {
        assert!(bullets.iter().any(|b| b == c), "contradicted {c} dropped");
    }
}

#[test]
fn warning_bullets_keeps_reply_sourced_claims_in_the_lead_window() {
    // A claim extracted from the reply is always a sentence of that reply, so
    // sitting in the lead window is not a reason to hide the warning.
    let reply = "Ton CTL est passe de 74 a 88 cette semaine.";
    let problems = vec![flagged(
        "Ton CTL est passe de 74 a 88",
        true,
        ClaimSource::Reply,
    )];
    let bullets = warning_bullets(&problems, reply);
    assert_eq!(bullets, vec!["- Ton CTL est passe de 74 a 88".to_owned()]);
}

#[test]
fn warning_bullets_drops_caller_text_already_in_the_lead_window() {
    // Text that did not come from the reply but already opens it is an echo;
    // caller text absent from the lead survives.
    let reply = "Ton CTL est passe de 74 a 88 cette semaine.";
    let problems = vec![
        flagged("Ton CTL est passe de 74 a 88", false, ClaimSource::Caller),
        flagged("affirmation hors du lead", false, ClaimSource::Caller),
    ];
    let bullets = warning_bullets(&problems, reply);
    assert_eq!(bullets, vec!["- affirmation hors du lead".to_owned()]);
}

/// One flagged claim earns exactly one affordance, chosen by surface capability.
///
/// Web once shipped the caveat banner AND the chip rail for a single flagged
/// claim. `WarnAffordance` makes that unrepresentable rather than merely
/// discouraged: chips and banner are variants, so no code path can emit both.
#[test]
fn a_chip_surface_gets_chips_and_an_untouched_reply() {
    const REPLY: &str = "Ton CTL est passe de 74 a 88 cette semaine.";
    let shown = vec![flagged(
        "Ton CTL est passe de 74 a 88",
        true,
        ClaimSource::Reply,
    )];

    let affordance = warn_affordance(&shown, REPLY, true, "Attention");
    let WarnAffordance::Chips(chips) = affordance else {
        panic!("a chip-capable surface must get chips, got {affordance:?}");
    };
    assert_eq!(chips.len(), 1);
    assert_eq!(chips[0].claim, "Ton CTL est passe de 74 a 88");
    assert!(chips[0].contradicted);
}

#[test]
fn a_surface_without_chips_gets_the_banner_written_into_the_reply() {
    // The flagged claim is the reply's own opening sentence, as every claim
    // the extractor produces is a sentence of the reply.
    const REPLY: &str = "Ton CTL est passe de 74 a 88 cette semaine.";
    let shown = vec![flagged(
        "Ton CTL est passe de 74 a 88",
        true,
        ClaimSource::Reply,
    )];

    let affordance = warn_affordance(&shown, REPLY, false, "Attention");
    let WarnAffordance::Banner(text) = affordance else {
        panic!("a surface with no chip rail must get the banner, got {affordance:?}");
    };
    assert!(
        text.starts_with(REPLY),
        "the banner is appended to the reply, not a replacement: {text}"
    );
    assert!(text.contains("Attention"), "banner header missing: {text}");
    assert!(
        text.ends_with("- Ton CTL est passe de 74 a 88"),
        "the banner must list the flagged claim: {text}"
    );
}

#[test]
fn a_short_reply_keeps_its_banner_end_to_end() {
    // Real extractor output on a short messaging reply: every claim sits in
    // the lead window. The caveat must still reach a surface without chips.
    const REPLY: &str = "Your VO2max is around 95 ml/kg/min right now.";
    let claims = extract_heuristic(REPLY);
    assert!(!claims.is_empty(), "fixture must yield at least one claim");
    assert!(claims.iter().all(|c| c.source == ClaimSource::Reply));
    let verdicts: Vec<(ExtractedClaim, VerdictOutcome)> = claims
        .into_iter()
        .map(|c| (c, outcome(ClaimStatus::Contradicted)))
        .collect();
    let shown = actionable_problems(&verdicts);

    let affordance = warn_affordance(&shown, REPLY, false, "Attention");
    let WarnAffordance::Banner(text) = affordance else {
        panic!("a short reply with a flagged claim must keep its banner, got {affordance:?}");
    };
    assert!(text.contains("\n- "), "banner must list the claim: {text}");
}

#[test]
fn nothing_worth_showing_produces_neither_affordance() {
    assert_eq!(
        warn_affordance(&[], "Belle seance.", true, "Attention"),
        WarnAffordance::Silent
    );
    assert_eq!(
        warn_affordance(&[], "Belle seance.", false, "Attention"),
        WarnAffordance::Silent
    );
}
