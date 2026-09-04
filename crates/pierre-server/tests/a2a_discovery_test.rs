// ABOUTME: Integration test for A2A 1.0 agent card discovery
// ABOUTME: Pins supportedInterfaces, capabilities, skills, and securitySchemes wire shapes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use pierre_mcp_server::a2a::agent_card::{
    AgentCard, SecurityScheme, BINDING_HTTP_JSON, BINDING_JSONRPC, OAUTH2_TOKEN_PATH,
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

/// `SendMessage` acts on a `data` part carrying `{tool_name, parameters}` and
/// refuses anything else, so every advertised example must be written in
/// that shape and name its own skill. A natural-language example would send
/// a caller down a path the surface rejects.
#[test]
fn test_agent_card_skill_examples_are_executable_data_parts() {
    let card = AgentCard::new();

    for skill in &card.skills {
        assert!(
            !skill.examples.is_empty(),
            "skill {} must show how to invoke it",
            skill.id
        );
        for example in &skill.examples {
            let parsed: serde_json::Value = serde_json::from_str(example).unwrap_or_else(|e| {
                panic!("skill {} example is not a JSON data part: {e}", skill.id)
            });
            assert_eq!(
                parsed["data"]["tool_name"], skill.id,
                "skill {} example must invoke its own tool, got: {example}",
                skill.id
            );
            assert!(
                parsed["data"]["parameters"].is_object(),
                "skill {} example must carry a parameters object, got: {example}",
                skill.id
            );
        }
    }

    // The card declares only the media type the data part travels in.
    assert_eq!(
        card.default_input_modes,
        vec!["application/json".to_owned()]
    );
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
    // The advertised token_url must be the route that actually serves the
    // client_credentials grant. `/oauth/token` is the ROPC bridge and
    // rejects every grant type but `password`, so a card pointing there
    // hands agents a 400 unsupported_grant_type.
    assert!(
        flow.token_url.ends_with(OAUTH2_TOKEN_PATH),
        "clientCredentials token_url must be the OAuth2 authorization server's token endpoint, got {}",
        flow.token_url
    );
    assert!(
        !flow.token_url.ends_with("/oauth/token"),
        "the ROPC bridge at /oauth/token does not serve client_credentials"
    );
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

/// The agent card is the discovery contract for machine callers, so its
/// advertised `clientCredentials` `token_url` must be the route that really
/// dispatches the `client_credentials` grant. This pins the card against
/// the three source facts that make that true: the authorization server
/// mounts `/oauth2/token`, that server dispatches `client_credentials`, and
/// the separate `/oauth/token` ROPC bridge rejects every grant but
/// `password`.
#[test]
fn test_advertised_token_url_is_the_client_credentials_route() {
    const IDENTITY_ROUTES: &str = include_str!("../../pierre-routes-identity/src/oauth2.rs");
    const AUTHZ_SERVER: &str = include_str!("../../pierre-auth/src/oauth2_server/endpoints.rs");
    const ROPC_ROUTES: &str = include_str!("../../pierre-routes-auth/src/lib.rs");
    const ROPC_HANDLER: &str = include_str!("../../pierre-routes-auth/src/login.rs");

    let base_url = "https://api.dravr.ai";
    let card = AgentCard::with_base_url(base_url);
    let schemes = card.security_schemes.as_ref().expect("securitySchemes");
    let Some(SecurityScheme::OAuth2(oauth2)) = schemes.get("oauth2ClientCredentials") else {
        panic!("oauth2ClientCredentials must be an oauth2SecurityScheme");
    };
    let token_url = &oauth2
        .flows
        .client_credentials
        .as_ref()
        .expect("clientCredentials flow")
        .token_url;

    let path = token_url
        .strip_prefix(base_url)
        .expect("token_url must be built from the card's base URL");
    assert_eq!(path, OAUTH2_TOKEN_PATH);

    // The advertised path is a mounted route of the OAuth 2.0 server...
    assert!(
        IDENTITY_ROUTES.contains(format!(r#".route("{path}", post(Self::handle_token))"#).as_str()),
        "no OAuth2 route mounts the advertised token path {path}"
    );
    // ...and that server dispatches the client_credentials grant.
    assert!(
        AUTHZ_SERVER.contains(r#""client_credentials" => self.handle_client_credentials_grant"#),
        "the OAuth2 authorization server no longer dispatches client_credentials"
    );

    // The ROPC bridge is a different route and accepts only the password
    // grant, so advertising it would be a protocol-level dead end.
    assert!(
        ROPC_ROUTES.contains(r#".route("/oauth/token", post(login::handle_oauth2_token))"#),
        "the ROPC bridge route moved; re-verify which route serves client_credentials"
    );
    assert!(
        ROPC_HANDLER.contains(r#"if request.grant_type != "password""#),
        "the ROPC bridge no longer restricts itself to the password grant"
    );
    assert_ne!(
        path, "/oauth/token",
        "the card must not advertise the password-only ROPC bridge for client_credentials"
    );
}
