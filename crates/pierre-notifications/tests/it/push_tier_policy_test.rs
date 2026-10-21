// ABOUTME: Truth-table tests for the PushTier ladder and PushPolicy floor semantics
// ABOUTME: Pins floor Pn ⇒ deliver tiers ≤ Pn only, and unknown labels ⇒ permissive no-gate

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The persona notification policy's arithmetic, pinned.
//!
//! The floor semantics are the part of registre#7 that is easiest to invert
//! silently — "floor P0" reads like "P0 is gated" when it means the exact
//! opposite (only P0 delivers) — so the full floor × tier truth table is
//! asserted here, not sampled.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::str::FromStr;

use pierre_notifications::{DigestCadence, PushPolicy, PushTier};

fn policy(floor: Option<PushTier>) -> PushPolicy {
    PushPolicy {
        persona: "casual".to_owned(),
        floor,
        digest: Some(DigestCadence::Weekly),
        armed: true,
    }
}

const ALL_TIERS: [PushTier; 4] = [PushTier::P0, PushTier::P1, PushTier::P2, PushTier::P3];

#[test]
fn tiers_order_by_urgency_p0_lowest() {
    assert!(PushTier::P0 < PushTier::P1);
    assert!(PushTier::P1 < PushTier::P2);
    assert!(PushTier::P2 < PushTier::P3);
    assert_eq!(ALL_TIERS.iter().max(), Some(&PushTier::P3));
    assert_eq!(ALL_TIERS.iter().min(), Some(&PushTier::P0));
}

/// The full floor × tier table: floor `Pn` delivers tiers ≤ `Pn` only.
#[test]
fn floor_gates_exactly_the_tiers_above_it() {
    let expectations: [(PushTier, [bool; 4]); 4] = [
        // floor      gated? for [P0, P1, P2, P3]
        (PushTier::P0, [false, true, true, true]),
        (PushTier::P1, [false, false, true, true]),
        (PushTier::P2, [false, false, false, true]),
        (PushTier::P3, [false, false, false, false]),
    ];
    for (floor, gated) in expectations {
        let p = policy(Some(floor));
        for (tier, expect_gated) in ALL_TIERS.into_iter().zip(gated) {
            assert_eq!(
                p.gates(tier),
                expect_gated,
                "floor {floor} × tier {tier}: expected gated={expect_gated}"
            );
        }
    }
}

/// Casual's promise verbatim: floor P0 gates P1 (a P1 event does NOT pass a
/// P0 floor), and P0 always delivers.
#[test]
fn casual_p0_floor_gates_p1_through_p3_and_passes_p0() {
    let p = policy(Some(PushTier::P0));
    assert!(!p.gates(PushTier::P0), "P0 must always deliver");
    assert!(p.gates(PushTier::P1), "floor P0 gates P1");
    assert!(p.gates(PushTier::P2), "floor P0 gates P2");
    assert!(p.gates(PushTier::P3), "floor P0 gates P3");
}

#[test]
fn no_floor_passes_every_tier() {
    let p = policy(None);
    for tier in ALL_TIERS {
        assert!(!p.gates(tier), "no floor must never gate {tier}");
    }
}

/// An unknown tier-floor label parses to an error, which policy resolution
/// treats as `floor: None` — permissive, never someone else's floor. A typo
/// in contremaitre must widen delivery, not near-mute the user.
#[test]
fn unknown_tier_label_is_permissive_not_casuals_floor() {
    for label in ["P4", "p9", "critical", ""] {
        let parsed = PushTier::from_str(label);
        assert!(parsed.is_err(), "'{label}' must not parse to a tier");
    }
    let unknown_floor_policy = policy("P4".parse().ok());
    assert_eq!(unknown_floor_policy.floor, None);
    for tier in ALL_TIERS {
        assert!(!unknown_floor_policy.gates(tier));
    }
}

#[test]
fn tier_labels_round_trip_and_accept_lowercase() {
    for tier in ALL_TIERS {
        assert_eq!(PushTier::from_str(tier.as_str()).unwrap(), tier);
        assert_eq!(
            PushTier::from_str(&tier.as_str().to_lowercase()).unwrap(),
            tier
        );
    }
    assert_eq!(PushTier::P2.to_string(), "P2");
}

#[test]
fn digest_cadence_parses_the_yaml_labels() {
    assert_eq!(
        DigestCadence::from_str("weekly").unwrap(),
        DigestCadence::Weekly
    );
    assert_eq!(
        DigestCadence::from_str("per_session").unwrap(),
        DigestCadence::PerSession
    );
    assert_eq!(
        DigestCadence::from_str("per_athlete").unwrap(),
        DigestCadence::PerAthlete
    );
    assert_eq!(
        DigestCadence::from_str("daily").unwrap(),
        DigestCadence::Daily
    );
    assert!(DigestCadence::from_str("fortnightly").is_err());
    assert_eq!(DigestCadence::Weekly.as_str(), "weekly");
}
