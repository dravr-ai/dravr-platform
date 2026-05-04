// ABOUTME: Smoke test for pierre-database's shared sync helpers
// ABOUTME: Covers enum string mapping + email validation without spinning up a DB
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Smoke integration tests for the crate public API.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use pierre_core::models::{UserStatus, UserTier};
use pierre_database::backends::shared::enums::{
    str_to_user_role, str_to_user_status, str_to_user_tier, user_status_to_str, user_tier_to_str,
};
use pierre_database::backends::shared::validation::validate_email;

#[test]
fn enum_string_mapping_round_trips_user_tier() {
    let tiers = [
        UserTier::Starter,
        UserTier::Professional,
        UserTier::Enterprise,
    ];
    for tier in tiers {
        let parsed = str_to_user_tier(user_tier_to_str(&tier));
        assert_eq!(parsed, tier, "user tier round-trip failed for {tier:?}");
    }
}

#[test]
fn enum_string_mapping_round_trips_user_status() {
    let statuses = [
        UserStatus::Active,
        UserStatus::Pending,
        UserStatus::Suspended,
    ];
    for status in statuses {
        let parsed = str_to_user_status(user_status_to_str(&status));
        assert_eq!(parsed, status);
    }
}

#[test]
fn enum_string_mapping_falls_back_to_default_for_unknown_role() {
    // Should not panic on unknown DB strings — safe default behavior.
    let role = str_to_user_role("never_seen_this_role_before");
    // Whatever the default is, we just want graceful degradation.
    let _ = role; // discard, but ensure no panic
}

#[test]
fn validate_email_accepts_well_formed_addresses() {
    validate_email("user@example.com").expect("standard email must validate");
    validate_email("first.last+tag@subdomain.example.co").expect("plus addressing must validate");
}

#[test]
fn validate_email_rejects_strings_without_at_or_too_short() {
    // The validator's contract is intentionally narrow: must contain '@'
    // and be at least 3 chars. It is not a full RFC 5322 parser.
    assert!(validate_email("not-an-email").is_err(), "no '@' must fail");
    assert!(validate_email("").is_err(), "empty must fail");
    assert!(validate_email("a@").is_err(), "len < 3 must fail");
}
