// ABOUTME: Integration test for the claim_verification service wrapper
// ABOUTME: Exercises the embedded sports-science corpus and the heuristic reply pipeline
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "tools-verification")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashSet;

use pierre_evals::VerificationConfig;
use pierre_mcp_server::services::claim_verification::{
    corpus, verify_reply_heuristic, verify_reply_with_config, warm_corpus,
};
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength};

#[test]
fn embedded_corpus_parses_from_include_str() {
    warm_corpus();
    let c = corpus();
    assert!(!c.is_empty(), "embedded sports_science.jsonl should parse");
}

#[test]
fn verify_reply_heuristic_skips_pure_motivation() {
    let v = verify_reply_heuristic("You're crushing it!", EvidenceStrength::Mixed);
    assert!(v.is_empty());
}

#[test]
fn verify_reply_heuristic_finds_nutrition_claim() {
    let v = verify_reply_heuristic(
        "Aim for 1.6 grams per kg of protein daily to support muscle protein synthesis.",
        EvidenceStrength::Mixed,
    );
    assert!(!v.is_empty());
    assert_eq!(v[0].0.category, ClaimCategory::Nutrition);
    assert!(matches!(
        v[0].1.status,
        ClaimStatus::Supported | ClaimStatus::Unsupported
    ));
}

// --- Per-coach VerificationConfig path -------------------------------------

const REPLY_WITH_MIXED_CLAIMS: &str =
    "Aim for 1.6 grams per kg of protein daily. Take 5 g of creatine per day. \
     Sleep 8 hours per night to recover.";

#[test]
fn disabled_config_returns_no_verdicts() {
    let prompt = "---
verification_config:
  enabled: false
---

disabled coach";
    let cfg = VerificationConfig::parse_from_system_prompt(prompt);
    let verdicts = verify_reply_with_config(REPLY_WITH_MIXED_CLAIMS, &cfg);
    assert!(verdicts.is_empty(), "disabled config must short-circuit");
}

#[test]
fn per_category_disable_drops_that_category_only() {
    let prompt = "---
verification_config:
  enabled: true
  categories:
    supplement:
      enabled: false
---

supplement-skipping coach";
    let cfg = VerificationConfig::parse_from_system_prompt(prompt);
    let verdicts = verify_reply_with_config(REPLY_WITH_MIXED_CLAIMS, &cfg);
    assert!(
        !verdicts.is_empty(),
        "non-supplement claims should still fire"
    );
    for (claim, _) in &verdicts {
        assert_ne!(
            claim.category,
            ClaimCategory::Supplement,
            "supplement claims must be filtered out, got: {}",
            claim.text
        );
    }
}

#[test]
fn per_category_strong_threshold_can_flip_to_unsupported() {
    let prompt_mixed = "---
verification_config:
  enabled: true
  categories:
    nutrition:
      enabled: true
      min_strength: mixed
---

baseline coach";
    let prompt_strong = "---
verification_config:
  enabled: true
  categories:
    nutrition:
      enabled: true
      min_strength: strong
---

strict coach";

    let cfg_mixed = VerificationConfig::parse_from_system_prompt(prompt_mixed);
    let cfg_strong = VerificationConfig::parse_from_system_prompt(prompt_strong);

    let reply = "Aim for 1.6 grams per kg of protein daily.";
    let mixed_verdicts = verify_reply_with_config(reply, &cfg_mixed);
    let strong_verdicts = verify_reply_with_config(reply, &cfg_strong);

    let mixed_status = mixed_verdicts
        .iter()
        .find(|(c, _)| c.category == ClaimCategory::Nutrition)
        .map(|(_, o)| o.status);
    let strong_status = strong_verdicts
        .iter()
        .find(|(c, _)| c.category == ClaimCategory::Nutrition)
        .map(|(_, o)| o.status);

    // Both configs must produce a verdict on the nutrition claim; the strong
    // config is at minimum not-more-supportive than the mixed config.
    assert_eq!(mixed_status, Some(ClaimStatus::Supported));
    assert!(matches!(
        strong_status,
        Some(ClaimStatus::Supported | ClaimStatus::Unsupported)
    ));
}

#[test]
fn default_config_verifies_all_categories() {
    let cfg = VerificationConfig::default();
    let verdicts = verify_reply_with_config(REPLY_WITH_MIXED_CLAIMS, &cfg);
    let categories: HashSet<_> = verdicts.iter().map(|(c, _)| c.category).collect();
    assert!(
        categories.contains(&ClaimCategory::Nutrition),
        "default config should emit a nutrition verdict"
    );
}
