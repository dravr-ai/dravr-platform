// ABOUTME: Smoke test for pierre-memory's public API (FactKind, UserFact, UserFactMetrics)
// ABOUTME: Guards the type contract that pierre-database and pierre-server depend on
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

use pierre_memory::facts::{FactKind, UserFactMetrics};

#[test]
fn fact_kind_str_round_trip_covers_all_variants() {
    let variants = [
        FactKind::Preference,
        FactKind::Physiology,
        FactKind::Injury,
        FactKind::Goal,
        FactKind::Schedule,
        FactKind::Equipment,
        FactKind::Other,
    ];

    for kind in variants {
        let serialized = kind.as_str();
        let parsed = FactKind::parse_lenient(serialized);
        assert_eq!(parsed, kind, "round-trip failed for {serialized}");
    }
}

#[test]
fn fact_kind_parse_lenient_falls_back_to_other_for_unknown() {
    assert_eq!(
        FactKind::parse_lenient("nonsense_kind_we_have_never_seen"),
        FactKind::Other,
        "unknown DB strings must not panic — they degrade to Other"
    );
}

#[test]
fn user_fact_metrics_empty_is_zeroed_out() {
    let metrics = UserFactMetrics::empty();
    assert_eq!(metrics.total_facts, 0);
    assert_eq!(metrics.facts_last_24h, 0);
    assert_eq!(metrics.facts_last_7d, 0);
    assert_eq!(metrics.distinct_users, 0);
    assert!(metrics.facts_by_kind.is_empty());
    assert!(metrics.newest_updated_at.is_none());
}
