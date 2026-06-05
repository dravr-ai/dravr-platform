// ABOUTME: External tests for the heuristic claim extractor (claim_extractor.rs)
// ABOUTME: Covers sentence splitting, category classification (incl. French), and extraction
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::claim_extractor::{classify_heuristic, extract_heuristic, split_sentences};
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
    assert!(extract_heuristic(reply).is_empty());
}
