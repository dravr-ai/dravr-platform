// ABOUTME: Tests for Pillar
// ABOUTME: String round-trip, unknown values and the snake_case wire form

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::pillar::Pillar;

#[test]
fn roundtrip_str() {
    for pillar in Pillar::ALL {
        assert_eq!(Pillar::parse(pillar.as_str()), Some(pillar));
    }
}

#[test]
fn unknown_returns_none() {
    assert!(Pillar::parse("activity").is_none());
    assert!(Pillar::parse("nutrition").is_none());
    assert!(Pillar::parse("recovery").is_none());
    assert!(Pillar::parse("mobility").is_none());
}

#[test]
fn serde_wire_form_is_snake_case() {
    assert_eq!(
        serde_json::to_string(&Pillar::TrainingAndMovement).unwrap_or_default(),
        "\"training_and_movement\""
    );
    assert_eq!(
        serde_json::to_string(&Pillar::RecoveryOptimisation).unwrap_or_default(),
        "\"recovery_optimisation\""
    );
}
