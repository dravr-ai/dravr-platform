// ABOUTME: External tests for per-coach verification config parsing (verification_config.rs)
// ABOUTME: Covers defaults, frontmatter parsing, malformed YAML tolerance, and disabling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::verification_config::{VerificationConfig, VerificationFallback};
use pierre_memory::{ClaimCategory, EvidenceStrength};

#[test]
fn default_enables_all_categories_at_mixed_strength() {
    let cfg = VerificationConfig::default();
    assert!(cfg.enabled);
    for cat in [
        ClaimCategory::Physiological,
        ClaimCategory::Nutrition,
        ClaimCategory::Supplement,
    ] {
        let per = cfg.for_category(cat);
        assert!(per.enabled);
        assert_eq!(per.min_strength, EvidenceStrength::Mixed);
    }
}

#[test]
fn parse_missing_frontmatter_returns_default() {
    let prompt = "You are a running coach. Help the user.";
    let cfg = VerificationConfig::parse_from_system_prompt(prompt);
    assert!(cfg.enabled);
    assert!(cfg.is_enabled_for(ClaimCategory::Nutrition));
}

#[test]
fn parse_frontmatter_with_verification_config() {
    let prompt = "---
verification_config:
  enabled: true
  fallback_behavior: block
  categories:
    nutrition:
      enabled: true
      min_strength: strong
    supplement:
      enabled: false
      min_strength: mixed
---

You are a running coach.";
    let cfg = VerificationConfig::parse_from_system_prompt(prompt);
    assert_eq!(cfg.fallback_behavior, VerificationFallback::Block);
    assert_eq!(
        cfg.for_category(ClaimCategory::Nutrition).min_strength,
        EvidenceStrength::Strong
    );
    assert!(!cfg.is_enabled_for(ClaimCategory::Supplement));
    // Unspecified categories fall back to defaults.
    assert!(cfg.is_enabled_for(ClaimCategory::Physiological));
}

#[test]
fn parse_malformed_frontmatter_returns_default() {
    let prompt = "---
verification_config: this is not valid yaml :: :
---

coach prompt";
    let cfg = VerificationConfig::parse_from_system_prompt(prompt);
    assert!(cfg.enabled);
}

#[test]
fn disabled_config_skips_all_categories() {
    let prompt = "---
verification_config:
  enabled: false
---
coach prompt";
    let cfg = VerificationConfig::parse_from_system_prompt(prompt);
    assert!(!cfg.enabled);
    assert!(!cfg.is_enabled_for(ClaimCategory::Nutrition));
}
