// ABOUTME: Verifies coach grading excludes Unsupported training-prescription verdicts
// ABOUTME: from the score so advice-heavy coaches aren't penalized in store rank.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_memory::{ClaimCategory, ClaimStatus, ClaimVerdict, EvidenceStrength, VerdictLayer};
use pierre_services::coach_grading::grade_from_verdicts;

fn verdict(category: ClaimCategory, status: ClaimStatus) -> ClaimVerdict {
    ClaimVerdict {
        id: "v".to_owned(),
        tenant_id: "t".to_owned(),
        user_id: "u".to_owned(),
        coach_id: Some("coach".to_owned()),
        conversation_id: None,
        message_id: None,
        claim_text: "claim".to_owned(),
        category,
        status,
        evidence_strength: EvidenceStrength::None,
        confidence: 1.0,
        layer_fired: VerdictLayer::Evidence,
        explanation: None,
        evidence_refs: None,
        created_at: Utc::now(),
    }
}

#[test]
fn unsupported_training_prescription_is_excluded_from_score() {
    let verdicts: Vec<ClaimVerdict> = (0..4)
        .map(|_| {
            verdict(
                ClaimCategory::TrainingPrescription,
                ClaimStatus::Unsupported,
            )
        })
        .collect();
    let grade = grade_from_verdicts("coach".to_owned(), &verdicts);

    // All four are excused prescriptions: none penalize the score, which falls
    // back to the neutral 0.5 (scored denominator is empty).
    assert_eq!(grade.unsupported_prescription, 4);
    assert_eq!(grade.unsupported, 0);
    assert!(
        (grade.score - 0.5).abs() < f32::EPSILON,
        "advice-only coach should not be penalized, got {}",
        grade.score
    );
}

#[test]
fn unsupported_non_prescription_still_penalizes() {
    let verdicts: Vec<ClaimVerdict> = (0..4)
        .map(|_| verdict(ClaimCategory::Physiological, ClaimStatus::Unsupported))
        .collect();
    let grade = grade_from_verdicts("coach".to_owned(), &verdicts);

    // Physiology claims DO have a corpus, so Unsupported there is a real
    // quality signal: 0.25 weight each, denominator 4 -> score 0.25.
    assert_eq!(grade.unsupported, 4);
    assert_eq!(grade.unsupported_prescription, 0);
    assert!(
        grade.score < 0.5,
        "uncited physiology claims should be penalized, got {}",
        grade.score
    );
}

#[test]
fn prescription_heavy_coach_outscores_uncited_physiology_coach() {
    let advice: Vec<ClaimVerdict> = (0..4)
        .map(|_| {
            verdict(
                ClaimCategory::TrainingPrescription,
                ClaimStatus::Unsupported,
            )
        })
        .collect();
    let physiology: Vec<ClaimVerdict> = (0..4)
        .map(|_| verdict(ClaimCategory::Physiological, ClaimStatus::Unsupported))
        .collect();

    let advice_grade = grade_from_verdicts("advice".to_owned(), &advice);
    let physiology_grade = grade_from_verdicts("physiology".to_owned(), &physiology);

    assert!(
        advice_grade.score > physiology_grade.score,
        "advice-heavy coach ({}) must not rank below an uncited-physiology coach ({})",
        advice_grade.score,
        physiology_grade.score
    );
}
