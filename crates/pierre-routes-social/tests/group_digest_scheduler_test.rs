// ABOUTME: Verifies the weekly-digest scheduler reads the per-tier weekly_digest
// ABOUTME: flag, making the previously-dormant GroupFeatureFlags load-bearing.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_routes_social::group_digest_scheduler::tier_enables_digest;

#[test]
fn weekly_digest_eligibility_follows_the_tier_flag() {
    // Professional and Enterprise tiers enable weekly_digest; Starter does not.
    assert!(tier_enables_digest("professional"));
    assert!(tier_enables_digest("pro"));
    assert!(tier_enables_digest("enterprise"));
    assert!(!tier_enables_digest("starter"));
    // Unknown plans fall back to the Starter tier (no digest).
    assert!(!tier_enables_digest("free"));
}
