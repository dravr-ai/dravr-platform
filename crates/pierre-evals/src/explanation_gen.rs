// ABOUTME: Layer 6 of the bullshit detector — builds the user-facing explanation for a verdict
// ABOUTME: Pure Rust templater for Phase A; LLM rewriting deferred to Phase D
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Explanation Generator
//!
//! Given a [`VerdictOutcome`] and the originating claim, produces a short
//! human-readable explanation suitable for the "Ask me about this claim"
//! UX drawer. Phase A uses a deterministic template so the explanation is
//! always available even when no LLM is configured; Phase D will add an
//! optional LLM-rewriting pass for better prose.

use crate::claim_extractor::ExtractedClaim;
use crate::verdict_engine::VerdictOutcome;
use pierre_memory::ClaimStatus;

/// Render the user-facing explanation for a verdict.
#[must_use]
pub fn render(claim: &ExtractedClaim, outcome: &VerdictOutcome) -> String {
    let header = match outcome.status {
        ClaimStatus::Supported => "Supported",
        ClaimStatus::Unsupported => "Unsupported",
        ClaimStatus::Contradicted => "Contradicted",
        ClaimStatus::Rhetorical => "Rhetorical",
        ClaimStatus::Unverifiable => "Unverifiable",
    };

    let mut out = format!(
        "**{header}** ({}, {})\n\n> {}\n\n{}",
        claim.category.as_str(),
        outcome.evidence_strength.as_str(),
        claim.text,
        outcome.explanation,
    );

    if let Some(refs) = &outcome.evidence_refs {
        out.push_str("\n\nReferences: ");
        out.push_str(refs);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict_engine::VerdictOutcome;
    use pierre_memory::{ClaimCategory, EvidenceStrength, VerdictLayer};

    #[test]
    fn renders_supported_verdict_with_refs() {
        let claim = ExtractedClaim {
            text: "Aim for 1.6 g/kg protein daily".into(),
            category: ClaimCategory::Nutrition,
        };
        let outcome = VerdictOutcome {
            status: ClaimStatus::Supported,
            evidence_strength: EvidenceStrength::Strong,
            confidence: 0.8,
            layer_fired: VerdictLayer::Evidence,
            explanation: "Supported by Morton 2018 meta-analysis".into(),
            evidence_refs: Some("doi:10.1/a".into()),
        };
        let rendered = render(&claim, &outcome);
        assert!(rendered.contains("Supported"));
        assert!(rendered.contains("nutrition"));
        assert!(rendered.contains("Morton"));
        assert!(rendered.contains("doi:10.1/a"));
    }

    #[test]
    fn renders_rhetorical_without_refs() {
        let claim = ExtractedClaim {
            text: "You're crushing it!".into(),
            category: ClaimCategory::TrainingPrescription,
        };
        let outcome = VerdictOutcome {
            status: ClaimStatus::Rhetorical,
            evidence_strength: EvidenceStrength::None,
            confidence: 1.0,
            layer_fired: VerdictLayer::Rhetoric,
            explanation: "Treated as rhetorical".into(),
            evidence_refs: None,
        };
        let rendered = render(&claim, &outcome);
        assert!(rendered.contains("Rhetorical"));
        assert!(!rendered.contains("References"));
    }
}
