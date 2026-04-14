// ABOUTME: Per-coach configuration for the Tier 5.5 bullshit detector pipeline
// ABOUTME: Loaded from YAML frontmatter in the coach's system prompt; defaults are safe
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Verification Config
//!
//! A compact struct describing how the Tier 5.5 pipeline should behave for
//! a given coach. Coaches can opt out entirely (rhetorical personas), raise
//! the Evidence Strength threshold for specific categories, or choose how
//! the dispatch path reacts to an `Unsupported` or `Contradicted` verdict
//! (warn the user, drop the offending claim silently, or block the whole
//! reply pending retry).
//!
//! Phase A loads it from YAML frontmatter embedded in the coach's system
//! prompt text. The loader is tolerant — any parse failure falls back to
//! the safe default so misconfigured coaches never crash dispatch.

use pierre_memory::{ClaimCategory, EvidenceStrength};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// How the dispatch path reacts when the pipeline emits a non-`Supported`
/// verdict on a claim in a coach's reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationFallback {
    /// Append a warning banner to the reply but let it through.
    #[default]
    Warn,
    /// Drop the offending claim from the reply (Phase D — currently same as Warn).
    Silent,
    /// Reject the reply and ask the LLM to retry.
    Block,
}

/// Per-category toggle + evidence threshold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryConfig {
    /// Whether verification is enabled for this category.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Minimum evidence strength that counts as `Supported`.
    #[serde(default)]
    pub min_strength: EvidenceStrength,
}

impl Default for CategoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_strength: EvidenceStrength::Mixed,
        }
    }
}

/// Full verification config for a single coach.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationConfig {
    /// Master switch. When false the pipeline is skipped entirely.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Per-category toggles and thresholds.
    #[serde(default)]
    pub categories: HashMap<String, CategoryConfig>,
    /// How to react to a non-supported verdict.
    #[serde(default)]
    pub fallback_behavior: VerificationFallback,
}

impl Default for VerificationConfig {
    fn default() -> Self {
        let mut categories = HashMap::new();
        for cat in [
            ClaimCategory::Physiological,
            ClaimCategory::TrainingPrescription,
            ClaimCategory::Nutrition,
            ClaimCategory::Recovery,
            ClaimCategory::Supplement,
            ClaimCategory::InjuryRehab,
        ] {
            categories.insert(cat.as_str().to_owned(), CategoryConfig::default());
        }
        Self {
            enabled: true,
            categories,
            fallback_behavior: VerificationFallback::Warn,
        }
    }
}

const fn default_true() -> bool {
    true
}

impl VerificationConfig {
    /// Resolve the config for a category, returning [`CategoryConfig::default`]
    /// when no specific entry is set.
    #[must_use]
    pub fn for_category(&self, category: ClaimCategory) -> CategoryConfig {
        self.categories
            .get(category.as_str())
            .cloned()
            .unwrap_or_default()
    }

    /// True when the pipeline should run for the given category.
    #[must_use]
    pub fn is_enabled_for(&self, category: ClaimCategory) -> bool {
        self.enabled && self.for_category(category).enabled
    }

    /// Load a verification config from the frontmatter of a coach system
    /// prompt. Looks for the first `---`-delimited YAML block containing a
    /// top-level `verification_config:` key.
    ///
    /// Returns the parsed config when present, or [`VerificationConfig::default`]
    /// on absence / parse failure.
    #[must_use]
    pub fn parse_from_system_prompt(prompt: &str) -> Self {
        let Some(frontmatter) = extract_frontmatter(prompt) else {
            return Self::default();
        };
        parse_verification_config_yaml(frontmatter).unwrap_or_default()
    }
}

fn extract_frontmatter(prompt: &str) -> Option<&str> {
    let trimmed = prompt.trim_start();
    let rest = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))?;
    let end_index = rest.find("\n---\n").or_else(|| rest.find("\r\n---\r\n"))?;
    Some(&rest[..end_index])
}

#[derive(Debug, Deserialize)]
struct FrontmatterShape {
    verification_config: Option<VerificationConfig>,
}

fn parse_verification_config_yaml(frontmatter: &str) -> Option<VerificationConfig> {
    let parsed: FrontmatterShape = serde_yaml::from_str(frontmatter).ok()?;
    parsed.verification_config
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
