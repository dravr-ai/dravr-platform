// ABOUTME: Integration test for A2A 1.0 agent card discovery
// ABOUTME: Pins supportedInterfaces, capabilities, skills, and securitySchemes wire shapes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;
mod helpers;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::Router;
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::a2a::agent_card::{
    AgentCard, SecurityScheme, BINDING_HTTP_JSON, BINDING_JSONRPC, OAUTH2_TOKEN_PATH,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use serde_json::Value;

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

/// The `OAuth2` authorization server's router, as the server mounts it.
fn oauth2_routes(resources: &Arc<ServerContext>) -> Router {
    let context = OAuth2Context {
        database: resources.agent.database.clone(),
        oauth2_server: resources.common.repos.oauth2_server.clone(),
        tenants: resources.common.repos.tenants.clone(),
        users: resources.common.repos.users.clone(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        config: Arc::new(resources.common.config.oauth2_server.clone()),
        refresh_token_expiry_days: resources.common.config.auth.refresh_token_expiry_days,
        csrf_manager: resources.auth.csrf_manager.clone(),
        accounts: resources.oauth2_accounts(),
        google_sign_in: None,
        rate_limiter: Arc::new(OAuth2RateLimiter::new(
            None,
            OAuth2RateLimiter::local_window_store(),
            &resources.common.config.rate_limiting,
        )),
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))))
}

/// `POST` the `client_credentials` grant for a client nobody registered.
async fn request_client_credentials(app: Router, path: &str) -> AxumTestResponse {
    AxumTestRequest::post(path)
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", "unregistered-agent"),
            ("client_secret", "not-a-secret"),
        ])
        .send(app)
        .await
}

/// The agent card is the discovery contract for machine callers, so its
/// advertised `clientCredentials` `token_url` must be a route the authorization
/// server answers, and must not be the first-party `/oauth/token` bridge.
///
/// Asked of the two token routes themselves, with a client nobody registered.
/// The advertised path reaches the authorization server's token endpoint,
/// which refuses the client as `invalid_client`. That shows the path is
/// mounted and nothing more: the endpoint authenticates the client before it
/// reads the grant, so an unregistered client gets the same answer for any
/// grant type. That the route dispatches `client_credentials` is held by
/// `a_registered_client_gets_a_token_at_the_card_token_url_and_calls_a2a_with_it`
/// in `a2a_client_credentials_token_test`, where a registered client is issued
/// a token at this same path.
///
/// The bridge refuses the grant type outright, and that half is a dispatch
/// claim this test can make: a card pointing there would hand every agent a
/// protocol-level dead end.
#[tokio::test]
async fn test_advertised_token_url_is_mounted_and_is_not_the_password_bridge() {
    let resources = common::create_test_server_resources().await.unwrap();

    let base_url = "https://api.dravr.ai";
    let card = AgentCard::with_base_url(base_url);
    let schemes = card.security_schemes.as_ref().expect("securitySchemes");
    let Some(SecurityScheme::OAuth2(oauth2)) = schemes.get("oauth2ClientCredentials") else {
        panic!("oauth2ClientCredentials must be an oauth2SecurityScheme");
    };
    let path = oauth2
        .flows
        .client_credentials
        .as_ref()
        .expect("clientCredentials flow")
        .token_url
        .strip_prefix(base_url)
        .expect("token_url must be built from the card's base URL")
        .to_owned();

    // The advertised path is mounted on the authorization server: its token
    // endpoint answers, in OAuth's own error shape, that it does not know this
    // client.
    let served = request_client_credentials(oauth2_routes(&resources), &path).await;
    assert_ne!(
        served.status(),
        404,
        "no OAuth2 route mounts the advertised token path {path}"
    );
    let refusal: Value = served.json();
    assert_eq!(
        refusal["error"], "invalid_client",
        "the advertised token path {path} must reach the authorization server's token \
         endpoint, which refuses an unregistered client: {refusal}"
    );

    // The first-party token route serves the password and refresh_token grants
    // only, so advertising it would be wrong.
    let bridge = request_client_credentials(
        AuthRoutes::routes(resources.auth_routes_context()),
        "/oauth/token",
    )
    .await;
    let refusal: Value = bridge.json();
    assert_eq!(
        refusal["error"], "unsupported_grant_type",
        "the first-party token route now serves client_credentials; re-verify the card: {refusal}"
    );
    assert_ne!(
        path, "/oauth/token",
        "the card must not advertise the password-only bridge for client_credentials"
    );
}

/// The card's OAuth flow publishes the vocabulary the authorization server
/// grants, and nothing else. It used to list `analytics:read`, `goals:read`
/// and `goals:write` — names the server does not define — and omit
/// `fitness:write`, so a client that followed it could never obtain the
/// scope its advertised `set_goal` skill needs.
#[test]
fn test_card_scopes_are_the_authorization_server_vocabulary() {
    let card = AgentCard::with_base_url("https://api.dravr.ai");
    let schemes = card.security_schemes.as_ref().expect("securitySchemes");
    let Some(SecurityScheme::OAuth2(oauth2)) = schemes.get("oauth2ClientCredentials") else {
        panic!("oauth2ClientCredentials must be an oauth2SecurityScheme");
    };
    let flow = oauth2
        .flows
        .client_credentials
        .as_ref()
        .expect("clientCredentials flow");

    let advertised: Vec<&str> = flow.scopes.keys().map(String::as_str).collect();
    assert_eq!(
        advertised,
        vec![
            "fitness:read",
            "fitness:write",
            "profile:read",
            "profile:write"
        ]
    );
    let mut supported = OAuthScope::delegable_as_str();
    supported.sort_unstable();
    assert_eq!(advertised, supported);
    assert!(flow
        .scopes
        .values()
        .all(|description| !description.is_empty()));
}
