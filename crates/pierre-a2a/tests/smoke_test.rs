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

use pierre_a2a::agent_card::{AgentCard, SecurityScheme, OAUTH2_TOKEN_PATH};
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

#[test]
fn agent_card_advertises_the_client_credentials_token_endpoint() {
    // `/oauth/token` is the password-grant ROPC bridge; the
    // client_credentials grant is served by the OAuth 2.0 authorization
    // server at `OAUTH2_TOKEN_PATH`. Advertising the wrong one hands every
    // discovering agent a 400 unsupported_grant_type.
    let base_url = "https://api.dravr.ai";
    let card = AgentCard::with_base_url(base_url);
    let schemes = card
        .security_schemes
        .as_ref()
        .expect("card must declare security schemes");
    let Some(SecurityScheme::OAuth2(oauth2)) = schemes.get("oauth2ClientCredentials") else {
        panic!("oauth2ClientCredentials must be an oauth2SecurityScheme");
    };
    let flow = oauth2
        .flows
        .client_credentials
        .as_ref()
        .expect("clientCredentials flow");

    assert_eq!(flow.token_url, format!("{base_url}{OAUTH2_TOKEN_PATH}"));
    assert!(
        !flow.token_url.ends_with("/oauth/token"),
        "the ROPC bridge does not serve client_credentials"
    );
}

#[test]
fn agent_card_skill_examples_use_the_accepted_data_part_shape() {
    // SendMessage acts on a `data` part carrying {tool_name, parameters}
    // and refuses anything else, so a natural-language example would send
    // callers down a path the surface rejects.
    let card = AgentCard::new();

    for skill in &card.skills {
        assert!(
            !skill.examples.is_empty(),
            "skill {} must show how to invoke it",
            skill.id
        );
        for example in &skill.examples {
            let parsed: serde_json::Value = serde_json::from_str(example)
                .unwrap_or_else(|e| panic!("skill {} example is not JSON: {e}", skill.id));
            assert_eq!(
                parsed["data"]["tool_name"], skill.id,
                "skill {} example must invoke its own tool",
                skill.id
            );
            assert!(
                parsed["data"]["parameters"].is_object(),
                "skill {} example must carry a parameters object",
                skill.id
            );
        }
    }
}
