// ABOUTME: Integration test for A2A 1.0 agent card discovery
// ABOUTME: Pins supportedInterfaces, capabilities, skills, and securitySchemes wire shapes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use pierre_mcp_server::a2a::agent_card::{
    AgentCard, SecurityScheme, BINDING_HTTP_JSON, BINDING_JSONRPC,
};

#[test]
fn test_agent_card_structure() {
    let card = AgentCard::new();

    // Required A2A 1.0 fields
    assert_eq!(card.name, "Dravr AI");
    assert!(!card.description.is_empty());
    assert!(!card.version.is_empty());
    assert!(
        !card.supported_interfaces.is_empty(),
        "Agent card must declare at least one interface"
    );
    assert!(!card.default_input_modes.is_empty());
    assert!(!card.default_output_modes.is_empty());
    assert!(!card.skills.is_empty());

    // Interfaces are preference-ordered; JSONRPC is preferred, HTTP+JSON is
    // the functional-equivalent alternative. Every entry declares the
    // Major.Minor protocol version.
    assert_eq!(
        card.supported_interfaces[0].protocol_binding,
        BINDING_JSONRPC
    );
    assert!(card.supported_interfaces[0].url.contains("/a2a/jsonrpc"));
    assert!(card
        .supported_interfaces
        .iter()
        .any(|i| i.protocol_binding == BINDING_HTTP_JSON));
    for interface in &card.supported_interfaces {
        assert_eq!(interface.protocol_version, "1.0");
        assert!(
            interface.url.starts_with("http://") || interface.url.starts_with("https://"),
            "Interface URL must be absolute: {}",
            interface.url
        );
    }

    // Capabilities: both streaming and push notifications are implemented.
    assert!(card.capabilities.streaming);
    assert!(card.capabilities.push_notifications);
    assert!(card.capabilities.extended_agent_card);
}

#[test]
fn test_agent_card_skills() {
    let card = AgentCard::new();

    let skill_ids: Vec<&str> = card.skills.iter().map(|s| s.id.as_str()).collect();
    assert!(skill_ids.contains(&"get_activities"));
    assert!(skill_ids.contains(&"analyze_activity"));
    assert!(skill_ids.contains(&"get_athlete"));
    assert!(skill_ids.contains(&"set_goal"));

    // AgentSkill required fields: id, name, description, tags.
    for skill in &card.skills {
        assert!(!skill.id.is_empty());
        assert!(!skill.name.is_empty());
        assert!(!skill.description.is_empty());
        assert!(!skill.tags.is_empty(), "skill {} needs tags", skill.id);
    }
}

#[test]
fn test_agent_card_serialization() {
    let card = AgentCard::new();

    let json = card.to_json().expect("Agent card should serialize to JSON");

    // ProtoJSON member names of the 1.0 card.
    assert!(json.contains("\"supportedInterfaces\""));
    assert!(json.contains("\"protocolBinding\""));
    assert!(json.contains("\"protocolVersion\""));
    assert!(json.contains("\"defaultInputModes\""));
    assert!(json.contains("\"defaultOutputModes\""));
    assert!(json.contains("\"skills\""));
    assert!(json.contains("\"securitySchemes\""));
    assert!(json.contains("\"pushNotifications\""));

    // Pre-1.0 members must be gone.
    assert!(!json.contains("\"transports\""));
    assert!(!json.contains("\"preferredTransport\""));
    assert!(!json.contains("\"additionalInterfaces\""));
    assert!(!json.contains("\"tools\""));

    let deserialized =
        AgentCard::from_json(&json).expect("Agent card should deserialize from JSON");
    assert_eq!(deserialized.name, card.name);
    assert_eq!(
        deserialized.supported_interfaces.len(),
        card.supported_interfaces.len()
    );
    assert_eq!(deserialized.skills.len(), card.skills.len());
}

#[test]
fn test_security_schemes() {
    let card = AgentCard::new();

    let schemes = card.security_schemes.as_ref().expect("securitySchemes");

    // Bearer JWT via the proto oneof wrapper form.
    let Some(SecurityScheme::HttpAuth(bearer)) = schemes.get("bearerAuth") else {
        panic!("bearerAuth must be an httpAuthSecurityScheme");
    };
    assert_eq!(bearer.scheme, "bearer");
    assert_eq!(bearer.bearer_format.as_deref(), Some("JWT"));

    // OAuth2 client-credentials flow with a token URL and scopes.
    let Some(SecurityScheme::OAuth2(oauth2)) = schemes.get("oauth2ClientCredentials") else {
        panic!("oauth2ClientCredentials must be an oauth2SecurityScheme");
    };
    let flow = oauth2
        .flows
        .client_credentials
        .as_ref()
        .expect("clientCredentials flow");
    assert!(flow.token_url.contains("/oauth/token"));
    assert!(!flow.scopes.is_empty());

    // The card requires at least one satisfiable security requirement.
    assert!(!card.security_requirements.is_empty());
    for requirement in &card.security_requirements {
        for name in requirement.schemes.keys() {
            assert!(
                schemes.contains_key(name),
                "securityRequirements references undeclared scheme {name}"
            );
        }
    }
}

#[test]
fn test_agent_card_with_custom_base_url() {
    let base_url = "https://api.pierre.ai";
    let card = AgentCard::with_base_url(base_url);

    for interface in &card.supported_interfaces {
        assert!(
            interface.url.starts_with(base_url),
            "Interface URL should use custom base URL: {}",
            interface.url
        );
    }

    let schemes = card.security_schemes.as_ref().unwrap();
    let Some(SecurityScheme::OAuth2(oauth2)) = schemes.get("oauth2ClientCredentials") else {
        panic!("oauth2ClientCredentials must be an oauth2SecurityScheme");
    };
    assert!(oauth2
        .flows
        .client_credentials
        .as_ref()
        .unwrap()
        .token_url
        .starts_with(base_url));
}
