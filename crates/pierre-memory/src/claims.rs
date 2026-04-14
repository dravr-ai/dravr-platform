// ABOUTME: Claim verdicts emitted by the Tier 5.5 bullshit detector pipeline
// ABOUTME: Pure types — persistence in pierre-database, pipeline in pierre-evals
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Category a claim belongs to. Each category has its own deterministic
/// bounds, evidence retrieval filters, and Evidence Strength thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimCategory {
    /// Physiology claims — HR, `VO2max`, lactate thresholds, substrate use.
    Physiological,
    /// Training prescription — volume, intensity, periodization.
    TrainingPrescription,
    /// Nutrition — macros, hydration, fuelling timing.
    Nutrition,
    /// Recovery — sleep, HRV, heat/cold, active recovery.
    Recovery,
    /// Supplement — ergogenic aids and dosing.
    Supplement,
    /// Injury rehabilitation — return-to-play timelines and protocols.
    InjuryRehab,
}

impl ClaimCategory {
    /// Stable `snake_case` string used in the database `CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Physiological => "physiological",
            Self::TrainingPrescription => "training_prescription",
            Self::Nutrition => "nutrition",
            Self::Recovery => "recovery",
            Self::Supplement => "supplement",
            Self::InjuryRehab => "injury_rehab",
        }
    }

    /// Parse a category from its stable stringified form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "physiological" => Some(Self::Physiological),
            "training_prescription" => Some(Self::TrainingPrescription),
            "nutrition" => Some(Self::Nutrition),
            "recovery" => Some(Self::Recovery),
            "supplement" => Some(Self::Supplement),
            "injury_rehab" => Some(Self::InjuryRehab),
            _ => None,
        }
    }
}

impl fmt::Display for ClaimCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Verdict status for a claim — what the pipeline concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    /// Claim is backed by evidence at or above the configured strength.
    Supported,
    /// Claim has no evidence meeting the strength threshold.
    Unsupported,
    /// Claim is directly contradicted by evidence or by a deterministic check.
    Contradicted,
    /// Claim is a rhetorical flourish, question, or motivational phrase — not
    /// a factual proposition. Fast path, no retrieval fired.
    Rhetorical,
    /// Pipeline could not reach a confident verdict (e.g. no evidence found,
    /// no matching deterministic rule, judge tied).
    Unverifiable,
}

impl ClaimStatus {
    /// Stable `snake_case` string used in the database `CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::Contradicted => "contradicted",
            Self::Rhetorical => "rhetorical",
            Self::Unverifiable => "unverifiable",
        }
    }

    /// Parse a status from its stable stringified form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "supported" => Some(Self::Supported),
            "unsupported" => Some(Self::Unsupported),
            "contradicted" => Some(Self::Contradicted),
            "rhetorical" => Some(Self::Rhetorical),
            "unverifiable" => Some(Self::Unverifiable),
            _ => None,
        }
    }
}

/// Strength of the evidence backing a supported claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStrength {
    /// No evidence was retrieved — the claim is rhetorical or unverifiable.
    None,
    /// A single observational study or correlational data.
    Weak,
    /// Multiple studies with some disagreement, or a single RCT.
    #[default]
    Mixed,
    /// Peer-reviewed meta-analysis, position stand, or RCT + guideline.
    Strong,
}

impl EvidenceStrength {
    /// Stable `snake_case` string used in the database `CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Weak => "weak",
            Self::Mixed => "mixed",
            Self::Strong => "strong",
        }
    }

    /// Parse an evidence strength from its stable stringified form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "weak" => Some(Self::Weak),
            "mixed" => Some(Self::Mixed),
            "strong" => Some(Self::Strong),
            _ => None,
        }
    }

    /// True when this evidence level meets the given minimum requirement.
    #[must_use]
    pub fn meets(self, minimum: Self) -> bool {
        self >= minimum
    }
}

/// Which layer of the detector pipeline produced the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictLayer {
    /// Layer 1 — rhetoric / question / greeting filter, no LLM.
    Rhetoric,
    /// Layer 2 — deterministic per-category bounds (pure Rust).
    Deterministic,
    /// Layer 3 — evidence RAG against the curated corpus.
    Evidence,
    /// Layer 4 — cross-check against other claims in the same reply.
    Consistency,
    /// Layer 5 — LLM-as-judge fallback when earlier layers were inconclusive.
    Judge,
}

impl VerdictLayer {
    /// Stable `snake_case` string used in the database `CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rhetoric => "rhetoric",
            Self::Deterministic => "deterministic",
            Self::Evidence => "evidence",
            Self::Consistency => "consistency",
            Self::Judge => "judge",
        }
    }

    /// Parse a layer from its stable stringified form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rhetoric" => Some(Self::Rhetoric),
            "deterministic" => Some(Self::Deterministic),
            "evidence" => Some(Self::Evidence),
            "consistency" => Some(Self::Consistency),
            "judge" => Some(Self::Judge),
            _ => None,
        }
    }
}

/// A single verdict emitted by the Tier 5.5 claim verification pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimVerdict {
    /// Stable identifier.
    pub id: String,
    /// Tenant that owns the verdict.
    pub tenant_id: String,
    /// User the claim was said to.
    pub user_id: String,
    /// Coach persona that authored the claim, if known.
    pub coach_id: Option<String>,
    /// Conversation the claim came from.
    pub conversation_id: Option<String>,
    /// Message the claim was extracted from.
    pub message_id: Option<String>,
    /// The raw claim text as extracted.
    pub claim_text: String,
    /// Category assigned by the extractor.
    pub category: ClaimCategory,
    /// Pipeline verdict.
    pub status: ClaimStatus,
    /// Evidence strength backing the verdict (None for rhetorical/unverifiable).
    pub evidence_strength: EvidenceStrength,
    /// Pipeline confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Which layer produced this verdict.
    pub layer_fired: VerdictLayer,
    /// User-facing explanation of the verdict, generated by the explanation layer.
    pub explanation: Option<String>,
    /// Comma-separated DOIs / PMIDs that backed a `Supported` verdict.
    pub evidence_refs: Option<String>,
    /// When the verdict was emitted.
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_round_trips_through_str() {
        for c in [
            ClaimCategory::Physiological,
            ClaimCategory::TrainingPrescription,
            ClaimCategory::Nutrition,
            ClaimCategory::Recovery,
            ClaimCategory::Supplement,
            ClaimCategory::InjuryRehab,
        ] {
            assert_eq!(ClaimCategory::parse(c.as_str()), Some(c));
        }
    }

    #[test]
    fn evidence_strength_ordering() {
        assert!(EvidenceStrength::Strong > EvidenceStrength::Mixed);
        assert!(EvidenceStrength::Mixed > EvidenceStrength::Weak);
        assert!(EvidenceStrength::Weak > EvidenceStrength::None);
    }

    #[test]
    fn meets_threshold_is_ge() {
        assert!(EvidenceStrength::Strong.meets(EvidenceStrength::Mixed));
        assert!(EvidenceStrength::Mixed.meets(EvidenceStrength::Mixed));
        assert!(!EvidenceStrength::Weak.meets(EvidenceStrength::Mixed));
    }

    #[test]
    fn status_round_trip() {
        for s in [
            ClaimStatus::Supported,
            ClaimStatus::Unsupported,
            ClaimStatus::Contradicted,
            ClaimStatus::Rhetorical,
            ClaimStatus::Unverifiable,
        ] {
            assert_eq!(ClaimStatus::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn layer_round_trip() {
        for l in [
            VerdictLayer::Rhetoric,
            VerdictLayer::Deterministic,
            VerdictLayer::Evidence,
            VerdictLayer::Consistency,
            VerdictLayer::Judge,
        ] {
            assert_eq!(VerdictLayer::parse(l.as_str()), Some(l));
        }
    }
}
