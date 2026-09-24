// ABOUTME: Unit tests for claim-verification enums — category, status, layer and evidence strength
// ABOUTME: Pins the string round-trips and the evidence ordering the verifier thresholds rely on
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};

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
fn evidence_strength_deserializes_moderate_as_mixed() {
    // dravr-contremaitre evidence frontmatter uses `moderate`; the serde
    // path must accept it as a synonym for `mixed`, matching `parse`.
    assert_eq!(
        serde_json::from_str::<EvidenceStrength>("\"moderate\"").ok(),
        Some(EvidenceStrength::Mixed)
    );
    assert_eq!(
        EvidenceStrength::parse("moderate"),
        Some(EvidenceStrength::Mixed)
    );
    // Canonical form serializes back to `mixed`, not `moderate`.
    assert_eq!(
        serde_json::to_string(&EvidenceStrength::Mixed)
            .ok()
            .as_deref(),
        Some("\"mixed\"")
    );
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
        VerdictLayer::Personalized,
        VerdictLayer::Evidence,
        VerdictLayer::Consistency,
        VerdictLayer::Judge,
    ] {
        assert_eq!(VerdictLayer::parse(l.as_str()), Some(l));
    }
}
