// ABOUTME: Unit tests for user facts — kind and source parsing plus the confidence threshold
// ABOUTME: Pins the lenient fallbacks that match the columns' NOT NULL defaults
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use chrono::Utc;
use pierre_memory::facts::{FactKind, FactSource, PredicateCode, UserFact};
use pierre_memory::scope::MemoryScope;

#[test]
fn fact_kind_roundtrip() {
    for kind in [
        FactKind::Preference,
        FactKind::Physiology,
        FactKind::Injury,
        FactKind::Goal,
        FactKind::Schedule,
        FactKind::Equipment,
        FactKind::NorthStar,
        FactKind::Medical,
        FactKind::Other,
    ] {
        assert_eq!(FactKind::parse_lenient(kind.as_str()), kind);
    }
}

#[test]
fn fact_source_roundtrip() {
    for source in [
        FactSource::Onboarding,
        FactSource::Conversation,
        FactSource::Device,
        FactSource::Coach,
    ] {
        assert_eq!(FactSource::parse_lenient(source.as_str()), source);
    }
    // Unknown values default to Conversation (NOT NULL DEFAULT in the DB).
    assert_eq!(
        FactSource::parse_lenient("garbage"),
        FactSource::Conversation
    );
}

#[test]
fn unknown_kind_falls_back_to_other() {
    assert_eq!(FactKind::parse_lenient("hallucinated"), FactKind::Other);
}

#[test]
fn is_confident_threshold() {
    let now = Utc::now();
    let fact = UserFact {
        id: "f1".into(),
        tenant_id: "t1".into(),
        user_id: "u1".into(),
        agent_id: None,
        scope: MemoryScope::User,
        kind: FactKind::Goal,
        pillar: None,
        predicate_code: PredicateCode::WorkingToward,
        object: "sub-3 marathon".into(),
        confidence: 0.72,
        source: FactSource::Conversation,
        valid_until: None,
        source_msg_id: Some("m1".into()),
        created_at: now,
        updated_at: now,
    };
    assert!(fact.is_confident(0.7));
    assert!(!fact.is_confident(0.8));
}
