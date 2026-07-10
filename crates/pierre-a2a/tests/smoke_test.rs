// ABOUTME: Smoke test for pierre-a2a's public API (A2A 1.0 AgentCard shape)
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
use pierre_a2a::A2A_VERSION;

#[test]
fn agent_card_default_constructor_populates_required_fields() {
    let card = AgentCard::new();
    assert!(!card.name.is_empty(), "agent card must advertise a name");
    assert!(
        !card.version.is_empty(),
        "agent card must advertise a version"
    );
    assert!(
        !card.supported_interfaces.is_empty(),
        "agent card must advertise at least one interface"
    );
    assert!(
        !card.skills.is_empty(),
        "agent card must advertise at least one skill"
    );
    assert!(
        !card.default_input_modes.is_empty() && !card.default_output_modes.is_empty(),
        "agent card must declare default input/output modes"
    );
    assert!(
        card.supported_interfaces
            .iter()
            .all(|i| i.protocol_version == A2A_VERSION),
        "every interface must declare the served protocol version"
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
    assert_eq!(
        parsed.supported_interfaces.len(),
        card.supported_interfaces.len()
    );
    assert_eq!(parsed.skills.len(), card.skills.len());
    assert_eq!(parsed.capabilities.streaming, card.capabilities.streaming);
    assert_eq!(
        parsed.capabilities.push_notifications,
        card.capabilities.push_notifications
    );
}
