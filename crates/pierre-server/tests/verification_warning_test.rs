// ABOUTME: Tests the claim-verification caveat-banner selection (actionable_problems + warning_bullets)
// ABOUTME: Prescriptions: Unsupported is suppressed, Contradicted kept; list is severity-sorted and capped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![cfg(feature = "tools-verification")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_chat_pipeline::stages::verification::{actionable_problems, warning_bullets};
use pierre_evals::{ExtractedClaim, VerdictOutcome};
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};

fn claim(text: &str, category: ClaimCategory) -> ExtractedClaim {
    ExtractedClaim {
        text: text.to_owned(),
        category,
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
    let texts: Vec<&str> = problems.iter().map(|(t, _)| *t).collect();
    assert_eq!(texts, vec!["Ton CTL est passe de 74 a 88"]);
    assert!(problems.iter().all(|(_, contradicted)| !contradicted));
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
    assert_eq!(problems, vec![("Cours 200 km demain", true)]);
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
    let problems = vec![
        ("u1", false),
        ("u2", false),
        ("u3", false),
        ("u4", false),
        ("c1", true),
        ("c2", true),
        ("c3", true),
    ];
    let bullets = warning_bullets(&problems, "short reply");
    assert_eq!(bullets.len(), 5);
    for c in ["- c1", "- c2", "- c3"] {
        assert!(bullets.iter().any(|b| b == c), "contradicted {c} dropped");
    }
}

#[test]
fn warning_bullets_drops_lead_window_echoes() {
    // The opening sentence is already visible to the reader; echoing it in the
    // banner is noise. A mid-body claim survives.
    let reply = "Ton CTL est passe de 74 a 88 cette semaine.";
    let problems = vec![
        ("Ton CTL est passe de 74 a 88", false),
        ("affirmation hors du lead", false),
    ];
    let bullets = warning_bullets(&problems, reply);
    assert_eq!(bullets, vec!["- affirmation hors du lead".to_owned()]);
}
