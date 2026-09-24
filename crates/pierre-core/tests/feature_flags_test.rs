// ABOUTME: Tests for FeatureKey
// ABOUTME: String and serde round-trips, the ALL constant, defaults and unknown keys

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::expect_used, clippy::unwrap_used)]

use std::str::FromStr;

use pierre_core::feature_flags::*;

#[test]
fn all_variants_round_trip_through_str() {
    for &key in FeatureKey::ALL {
        let s = key.as_str();
        assert_eq!(FeatureKey::from_str(s).unwrap(), key);
    }
}

#[test]
fn all_constant_matches_variants() {
    // If a new variant is added without updating ALL, this test catches it.
    let mut from_all: Vec<&str> = FeatureKey::ALL.iter().map(|k| k.as_str()).collect();
    from_all.sort_unstable();
    let mut expected = vec![
        "api_tokens",
        "billing_header",
        "persona_notification_policy",
        "provider_exposure_notice",
    ];
    expected.sort_unstable();
    assert_eq!(from_all, expected);
}

#[test]
fn defaults_are_all_disabled() {
    for &key in FeatureKey::ALL {
        assert!(
            !key.default_enabled(),
            "{key} should default to disabled; new flags must opt in by admin"
        );
    }
}

#[test]
fn unknown_key_errors() {
    let err = FeatureKey::from_str("does_not_exist").unwrap_err();
    assert_eq!(err.0, "does_not_exist");
}

#[test]
fn display_matches_as_str() {
    assert_eq!(FeatureKey::ApiTokens.to_string(), "api_tokens");
    assert_eq!(FeatureKey::BillingHeader.to_string(), "billing_header");
    assert_eq!(
        FeatureKey::PersonaNotificationPolicy.to_string(),
        "persona_notification_policy"
    );
}

#[test]
fn serde_uses_snake_case() {
    let json = serde_json::to_string(&FeatureKey::ApiTokens).unwrap();
    assert_eq!(json, "\"api_tokens\"");
    let back: FeatureKey = serde_json::from_str("\"billing_header\"").unwrap();
    assert_eq!(back, FeatureKey::BillingHeader);
}
