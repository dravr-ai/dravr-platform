// ABOUTME: Per-coach configuration for the bullshit detector pipeline
// ABOUTME: Loaded from YAML frontmatter in the coach's system prompt; defaults are safe
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Verification Config
//!
//! A compact struct describing how the verification pipeline should behave for
//! a given coach. Coaches can opt out entirely (rhetorical personas), raise
//! the Evidence Strength threshold for specific categories, or choose how
//! the dispatch path reacts to an `Unsupported` or `Contradicted` verdict
//! (warn the user, record the verdict silently without altering the reply,
//! or block the whole reply pending retry).
//!
//! Phase A loads it from YAML frontmatter embedded in the coach's system
//! prompt text. The loader is tolerant — any parse failure falls back to
//! the safe default so misconfigured coaches never crash dispatch.

use crate::personalized::{
    CoachConfiguredStrategy, ConservativeStrategy, TightStrategy, ToleranceStrategy,
    DEFAULT_MARGIN_FRAC,
};
use crate::verdict_engine::VerdictOutcome;
use pierre_memory::{ClaimCategory, ClaimStatus, EvidenceStrength};
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
    /// Currently a transparent pass-through: the reply is returned unchanged
    /// (no banner, no claim removed), so a non-`Supported` verdict is recorded
    /// for the admin UI but never surfaced to the user. Distinct from `Warn`,
    /// which appends a banner. Claim-span removal is a future implementation.
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
    /// Strategy + tuning for the personalized-physiology layer.
    #[serde(default)]
    pub personalized: PersonalizedConfig,
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
            ClaimCategory::AthleteData,
        ] {
            categories.insert(cat.as_str().to_owned(), CategoryConfig::default());
        }
        Self {
            enabled: true,
            categories,
            fallback_behavior: VerificationFallback::Warn,
            personalized: PersonalizedConfig::default(),
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

/// Which [`ToleranceStrategy`] the personalized-physiology layer uses to decide
/// when a claim contradicts the athlete's range. Selected per-coach via YAML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToleranceMode {
    /// Buffer margin read from the coach YAML (`margin_frac`). The default.
    #[default]
    CoachConfigured,
    /// Fixed safety buffer the coach cannot loosen.
    Conservative,
    /// Zero buffer: any value outside the range is contradicted.
    Tight,
}

/// What a personalized verdict does to the reply. Selected
/// per-coach via YAML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionMode {
    /// Reuse the coach's existing [`VerificationFallback`]. The default.
    #[default]
    Inherit,
    /// Record the verdict for admin / the human coach, never alter the reply.
    AuditOnly,
    /// Append a warning banner to the athlete's reply.
    UserWarn,
}

/// Per-coach configuration for the personalized-physiology layer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PersonalizedConfig {
    /// Master switch for the personalized-physiology layer.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Which tolerance strategy decides contradictions.
    #[serde(default)]
    pub tolerance: ToleranceMode,
    /// Buffer margin (fraction of the metric value) for the
    /// [`ToleranceMode::CoachConfigured`] strategy.
    #[serde(default = "default_margin_frac")]
    pub margin_frac: f64,
    /// What a personalized verdict does to the reply.
    #[serde(default)]
    pub action: ActionMode,
}

impl Default for PersonalizedConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            tolerance: ToleranceMode::default(),
            margin_frac: DEFAULT_MARGIN_FRAC,
            action: ActionMode::default(),
        }
    }
}

fn default_margin_frac() -> f64 {
    DEFAULT_MARGIN_FRAC
}

impl PersonalizedConfig {
    /// Build the [`ToleranceStrategy`] selected by `tolerance`. The default
    /// ([`ToleranceMode::CoachConfigured`]) carries the YAML `margin_frac`.
    #[must_use]
    pub fn tolerance_strategy(&self) -> Box<dyn ToleranceStrategy> {
        match self.tolerance {
            ToleranceMode::CoachConfigured => Box::new(CoachConfiguredStrategy {
                margin_frac: self.margin_frac,
            }),
            ToleranceMode::Conservative => Box::new(ConservativeStrategy::default()),
            ToleranceMode::Tight => Box::new(TightStrategy),
        }
    }

    /// Build the [`ContradictionPolicy`] selected by `action`. The default
    /// ([`ActionMode::Inherit`]) defers to the coach's [`VerificationFallback`].
    #[must_use]
    pub fn contradiction_policy(&self) -> Box<dyn ContradictionPolicy> {
        match self.action {
            ActionMode::Inherit => Box::new(InheritConfigPolicy),
            ActionMode::AuditOnly => Box::new(AuditOnlyPolicy),
            ActionMode::UserWarn => Box::new(UserWarnPolicy),
        }
    }
}

/// The effective action the dispatch path takes for a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedAction {
    /// Leave the reply untouched (the verdict was supportive or rhetorical).
    Pass,
    /// Append a warning banner to the reply but let it through.
    WarnBanner,
    /// Record the verdict for the admin trail without altering the reply.
    RecordOnly,
    /// Reject the reply and ask the LLM to retry.
    BlockRetry,
}

/// Pluggable policy mapping a verdict + coach config to a [`ResolvedAction`].
///
/// Selected per-coach via [`ActionMode`]. The default ([`InheritConfigPolicy`])
/// reproduces the pipeline's pre-Layer-2.5 behavior exactly.
pub trait ContradictionPolicy: Send + Sync {
    /// Resolve what the dispatch path should do with `verdict`.
    fn resolve(&self, verdict: &VerdictOutcome, config: &VerificationConfig) -> ResolvedAction;
}

/// Default policy: defer to the coach's existing [`VerificationFallback`].
pub struct InheritConfigPolicy;

impl ContradictionPolicy for InheritConfigPolicy {
    fn resolve(&self, verdict: &VerdictOutcome, config: &VerificationConfig) -> ResolvedAction {
        if is_passing(verdict) {
            return ResolvedAction::Pass;
        }
        match config.fallback_behavior {
            VerificationFallback::Warn => ResolvedAction::WarnBanner,
            VerificationFallback::Silent => ResolvedAction::RecordOnly,
            VerificationFallback::Block => ResolvedAction::BlockRetry,
        }
    }
}

/// Record-only policy: the verdict reaches the admin trail and the human coach,
/// never the athlete.
pub struct AuditOnlyPolicy;

impl ContradictionPolicy for AuditOnlyPolicy {
    fn resolve(&self, verdict: &VerdictOutcome, _config: &VerificationConfig) -> ResolvedAction {
        if is_passing(verdict) {
            ResolvedAction::Pass
        } else {
            ResolvedAction::RecordOnly
        }
    }
}

/// User-facing policy: always append a warning banner for a non-supported verdict.
pub struct UserWarnPolicy;

impl ContradictionPolicy for UserWarnPolicy {
    fn resolve(&self, verdict: &VerdictOutcome, _config: &VerificationConfig) -> ResolvedAction {
        if is_passing(verdict) {
            ResolvedAction::Pass
        } else {
            ResolvedAction::WarnBanner
        }
    }
}

/// A verdict whose status needs no dispatch action (supportive or
/// non-propositional).
fn is_passing(verdict: &VerdictOutcome) -> bool {
    matches!(
        verdict.status,
        ClaimStatus::Supported | ClaimStatus::Rhetorical
    )
}
