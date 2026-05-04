// ABOUTME: Smoke test for pierre-a2a's public API (AgentCard, capabilities, JSON shape)
// ABOUTME: Guards the discovery payload contract that A2A clients depend on
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

use pierre_a2a::agent_card::AgentCard;

#[test]
fn agent_card_default_constructor_populates_required_fields() {
    let card = AgentCard::new();
    assert!(!card.name.is_empty(), "agent card must advertise a name");
    assert!(
        !card.version.is_empty(),
        "agent card must advertise a version"
    );
    assert!(
        !card.capabilities.is_empty(),
        "agent card must advertise at least one capability"
    );
    assert!(
        !card.transports.is_empty(),
        "agent card must advertise at least one transport"
    );
    assert!(
        !card.tools.is_empty(),
        "agent card must advertise at least one tool"
    );
}

#[test]
fn agent_card_serde_round_trip_preserves_required_fields() {
    let card = AgentCard::new();
    let json = serde_json::to_string(&card).expect("agent card must serialize to JSON");
    let parsed: AgentCard =
        serde_json::from_str(&json).expect("serialized agent card must round-trip back");

    assert_eq!(parsed.name, card.name);
    assert_eq!(parsed.version, card.version);
    assert_eq!(parsed.capabilities, card.capabilities);
    assert_eq!(parsed.transports.len(), card.transports.len());
    assert_eq!(parsed.tools.len(), card.tools.len());
}
