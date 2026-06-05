// ABOUTME: External tests for the verdict explanation renderer (explanation_gen.rs)
// ABOUTME: Covers supported-with-refs and rhetorical-without-refs rendering paths
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::claim_extractor::ExtractedClaim;
use pierre_evals::explanation_gen::render;
use pierre_evals::verdict_engine::VerdictOutcome;
use pierre_memory::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};

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
