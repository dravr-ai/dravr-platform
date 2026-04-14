// ABOUTME: Integration tests for the Tier 5.5 bullshit detector pipeline
// ABOUTME: Covers rhetoric filter, deterministic bounds, evidence retrieval, and config loading
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_evals::{
    check_claim,
    claim_extractor::{extract_heuristic, ExtractedClaim},
    classify_rhetoric,
    evidence_retriever::EvidenceCorpus,
    verification_config::VerificationConfig,
    RhetoricVerdict,
};
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};

const TEST_CORPUS: &str = r#"
{"id":"doi:10.1/a","category":"nutrition","proposition":"Protein intake of 1.6 to 2.2 g per kg body weight per day maximizes muscle protein synthesis","strength":"strong","citation":"Morton 2018"}
{"id":"doi:10.1/b","category":"supplement","proposition":"Creatine monohydrate at 3 to 5 g per day increases high-intensity performance","strength":"strong","citation":"ISSN 2017"}
{"id":"doi:10.1/c","category":"physiological","proposition":"Maximum heart rate is approximated by 208 minus 0.7 times age","strength":"mixed","citation":"Tanaka 2001"}
{"id":"doi:10.1/d","category":"recovery","proposition":"Adults require 7 to 9 hours of sleep per night","strength":"strong","citation":"AASM 2015"}
"#;

fn corpus() -> EvidenceCorpus {
    EvidenceCorpus::from_jsonl(TEST_CORPUS).expect("parse test corpus")
}

#[test]
fn rhetoric_classifier_rejects_motivation() {
    assert_eq!(
        classify_rhetoric("Great job on that workout!"),
        RhetoricVerdict::Rhetorical
    );
    assert_eq!(
        classify_rhetoric("You're absolutely crushing it."),
        RhetoricVerdict::Rhetorical
    );
}

#[test]
fn rhetoric_classifier_lets_factual_claims_through() {
    assert_eq!(
        classify_rhetoric("Creatine at 5 g per day increases power output."),
        RhetoricVerdict::Propositional
    );
}

#[test]
fn evidence_retrieval_finds_supporting_citation() {
    let claim = ExtractedClaim {
        text: "Aim for 1.6 grams of protein per kg of body weight daily.".into(),
        category: ClaimCategory::Nutrition,
    };
    let outcome = check_claim(&claim, &corpus(), EvidenceStrength::Mixed);
    assert_eq!(outcome.status, ClaimStatus::Supported);
    assert_eq!(outcome.evidence_strength, EvidenceStrength::Strong);
    assert_eq!(outcome.layer_fired, VerdictLayer::Evidence);
    assert!(outcome.evidence_refs.is_some());
}

#[test]
fn evidence_layer_falls_through_to_unsupported() {
    let claim = ExtractedClaim {
        text: "Drinking kale smoothies cures tendinitis overnight.".into(),
        category: ClaimCategory::Nutrition,
    };
    let outcome = check_claim(&claim, &corpus(), EvidenceStrength::Mixed);
    assert_eq!(outcome.status, ClaimStatus::Unsupported);
}

#[test]
fn deterministic_contradicts_implausible_heart_rate() {
    let claim = ExtractedClaim {
        text: "Your max heart rate is 400 bpm.".into(),
        category: ClaimCategory::Physiological,
    };
    let outcome = check_claim(&claim, &corpus(), EvidenceStrength::Mixed);
    assert_eq!(outcome.status, ClaimStatus::Contradicted);
    assert_eq!(outcome.layer_fired, VerdictLayer::Deterministic);
}

#[test]
fn deterministic_contradicts_absurd_protein_intake() {
    let claim = ExtractedClaim {
        text: "Eat 50 grams per kg of protein daily.".into(),
        category: ClaimCategory::Nutrition,
    };
    let outcome = check_claim(&claim, &corpus(), EvidenceStrength::Mixed);
    assert_eq!(outcome.status, ClaimStatus::Contradicted);
}

#[test]
fn rhetoric_short_circuits_pipeline() {
    let claim = ExtractedClaim {
        text: "You're crushing it, champ!".into(),
        category: ClaimCategory::TrainingPrescription,
    };
    let outcome = check_claim(&claim, &corpus(), EvidenceStrength::Mixed);
    assert_eq!(outcome.status, ClaimStatus::Rhetorical);
    assert_eq!(outcome.layer_fired, VerdictLayer::Rhetoric);
}

#[test]
fn heuristic_extractor_splits_multi_sentence_reply() {
    let reply = "Aim for 1.6g per kg of protein daily. Take 5g of creatine per day. \
                 Sleep 8 hours per night for recovery. Nice work!";
    let claims = extract_heuristic(reply);
    assert!(
        claims.len() >= 3,
        "expected at least 3 claims, got {}",
        claims.len()
    );
}

#[test]
fn heuristic_extractor_drops_pure_motivation() {
    let reply = "Awesome work today! You're crushing it! Keep going!";
    let claims = extract_heuristic(reply);
    assert!(
        claims.is_empty(),
        "motivational-only replies should yield zero extracted claims"
    );
}

#[test]
fn verification_config_default_allows_everything_at_mixed() {
    let cfg = VerificationConfig::default();
    assert!(cfg.enabled);
    for cat in [
        ClaimCategory::Physiological,
        ClaimCategory::TrainingPrescription,
        ClaimCategory::Nutrition,
        ClaimCategory::Recovery,
        ClaimCategory::Supplement,
        ClaimCategory::InjuryRehab,
    ] {
        assert!(cfg.is_enabled_for(cat), "default should enable {cat:?}");
    }
}

#[test]
fn verification_config_parses_coach_frontmatter() {
    let prompt = "---
verification_config:
  enabled: true
  fallback_behavior: warn
  categories:
    supplement:
      enabled: false
      min_strength: strong
---

You are a no-nonsense running coach.";
    let cfg = VerificationConfig::parse_from_system_prompt(prompt);
    assert!(cfg.enabled);
    assert!(!cfg.is_enabled_for(ClaimCategory::Supplement));
    assert!(cfg.is_enabled_for(ClaimCategory::Nutrition));
}

#[test]
fn verification_config_disabled_skips_all_categories() {
    let prompt = "---
verification_config:
  enabled: false
---

Coach body";
    let cfg = VerificationConfig::parse_from_system_prompt(prompt);
    assert!(!cfg.enabled);
    for cat in [
        ClaimCategory::Nutrition,
        ClaimCategory::Physiological,
        ClaimCategory::Supplement,
    ] {
        assert!(!cfg.is_enabled_for(cat));
    }
}

#[test]
fn evidence_retriever_respects_category_filter() {
    let corpus = corpus();
    let matches = corpus.retrieve("protein", ClaimCategory::Supplement, 3);
    assert!(
        matches.is_empty(),
        "protein should not match supplement category"
    );
}

#[test]
fn evidence_retriever_scores_keyword_overlap() {
    let corpus = corpus();
    let matches = corpus.retrieve(
        "How much protein should athletes consume per kg body weight?",
        ClaimCategory::Nutrition,
        5,
    );
    assert!(!matches.is_empty());
    assert_eq!(matches[0].record.id, "doi:10.1/a");
}

#[test]
fn higher_minimum_strength_can_flip_supported_to_unsupported() {
    let claim = ExtractedClaim {
        text: "Your max heart rate is 208 minus 0.7 times your age.".into(),
        category: ClaimCategory::Physiological,
    };
    let cfg_strong = check_claim(&claim, &corpus(), EvidenceStrength::Strong);
    assert_eq!(cfg_strong.status, ClaimStatus::Unsupported);

    let cfg_mixed = check_claim(&claim, &corpus(), EvidenceStrength::Mixed);
    assert_eq!(cfg_mixed.status, ClaimStatus::Supported);
}

#[test]
fn extracted_claim_is_serde_round_trip_safe() {
    let claim = ExtractedClaim {
        text: "Creatine at 5g/day boosts high-intensity performance".into(),
        category: ClaimCategory::Supplement,
    };
    let json = serde_json::to_string(&claim).unwrap();
    let back: ExtractedClaim = serde_json::from_str(&json).unwrap();
    assert_eq!(back.category, ClaimCategory::Supplement);
    assert!(back.text.contains("creatine") || back.text.contains("Creatine"));
}

#[test]
fn recovery_sleep_out_of_range_is_contradicted() {
    let claim = ExtractedClaim {
        text: "You should aim for 20 hours of sleep per night.".into(),
        category: ClaimCategory::Recovery,
    };
    let outcome = check_claim(&claim, &corpus(), EvidenceStrength::Mixed);
    assert_eq!(outcome.status, ClaimStatus::Contradicted);
}

#[test]
fn explanation_text_mentions_layer_source() {
    let claim = ExtractedClaim {
        text: "Your max heart rate is 900 bpm".into(),
        category: ClaimCategory::Physiological,
    };
    let outcome = check_claim(&claim, &corpus(), EvidenceStrength::Mixed);
    assert!(outcome.explanation.to_lowercase().contains("bound"));
}
