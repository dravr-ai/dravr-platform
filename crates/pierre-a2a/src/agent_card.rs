// ABOUTME: A2A 1.0 AgentCard — spec-shaped discovery document served at /.well-known/agent-card.json
// ABOUTME: Declares supportedInterfaces, capabilities, skills, and securitySchemes per the normative a2a.proto
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Agent Card Implementation
//!
//! Implements the A2A 1.0 Agent Card for Pierre, enabling agent discovery
//! and interface/security negotiation. Field names follow the `ProtoJSON`
//! serialization of the normative `a2a.proto` (`camelCase` members, per-
//! interface `protocolVersion`, oneof-wrapped security schemes).

use std::collections::BTreeMap;
use std::env;

use serde::{Deserialize, Serialize};

use crate::protocol_types::A2A_VERSION;

/// Protocol binding identifier for the JSON-RPC 2.0 interface (spec §5.2)
pub const BINDING_JSONRPC: &str = "JSONRPC";
/// Protocol binding identifier for the HTTP+JSON/REST interface (spec §5.2)
pub const BINDING_HTTP_JSON: &str = "HTTP+JSON";

/// Media type of the structured `data` part that carries a tool invocation.
/// The message surface acts on that part alone, so it is the card's only
/// declared input mode.
pub const DATA_PART_MEDIA_TYPE: &str = "application/json";

/// Path of the OAuth 2.0 authorization server's token endpoint.
///
/// This is the route that serves the `client_credentials` grant advertised by
/// the card's `oauth2ClientCredentials` scheme. Mounted by
/// `pierre-routes-identity` and published as `token_endpoint` in the RFC 8414
/// discovery document.
pub const OAUTH2_TOKEN_PATH: &str = "/oauth2/token";

/// A2A 1.0 Agent Card for Pierre
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    /// Agent name ("Dravr AI")
    pub name: String,
    /// Human-readable description of the agent's purpose
    pub description: String,
    /// Agent implementation version (independent of the protocol version)
    pub version: String,
    /// Supported transport interfaces, ordered by preference (first = preferred)
    #[serde(rename = "supportedInterfaces")]
    pub supported_interfaces: Vec<AgentInterface>,
    /// Optional protocol feature capabilities
    pub capabilities: AgentCapabilities,
    /// Default accepted input media types (skills may override)
    #[serde(rename = "defaultInputModes")]
    pub default_input_modes: Vec<String>,
    /// Default produced output media types (skills may override)
    #[serde(rename = "defaultOutputModes")]
    pub default_output_modes: Vec<String>,
    /// Skills this agent offers
    pub skills: Vec<AgentSkill>,
    /// The organization behind the agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,
    /// Human-readable documentation URL
    #[serde(rename = "documentationUrl", skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
    /// Named security schemes accepted by this agent
    #[serde(rename = "securitySchemes", skip_serializing_if = "Option::is_none")]
    pub security_schemes: Option<BTreeMap<String, SecurityScheme>>,
    /// Security requirements — a request must satisfy at least one entry
    #[serde(
        rename = "securityRequirements",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub security_requirements: Vec<SecurityRequirement>,
}

/// A transport interface exposed by the agent (A2A 1.0 `AgentInterface`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInterface {
    /// Base URL of the interface (required)
    pub url: String,
    /// Protocol binding — official values `JSONRPC`, `GRPC`, `HTTP+JSON` (required)
    #[serde(rename = "protocolBinding")]
    pub protocol_binding: String,
    /// A2A protocol version served at this interface, `Major.Minor` (required)
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Tenant identifier for multi-tenant deployments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

/// Optional protocol capabilities (A2A 1.0 `AgentCapabilities`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCapabilities {
    /// Whether `SendStreamingMessage`/`SubscribeToTask` (SSE) are supported
    #[serde(default)]
    pub streaming: bool,
    /// Whether push notification webhooks are supported
    #[serde(rename = "pushNotifications", default)]
    pub push_notifications: bool,
    /// Whether an authenticated extended agent card is available
    #[serde(rename = "extendedAgentCard", default)]
    pub extended_agent_card: bool,
    /// Protocol extensions supported by this agent
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<AgentExtension>,
}

/// A protocol extension declaration (A2A 1.0 `AgentExtension`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExtension {
    /// Extension URI (required)
    pub uri: String,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether clients must support this extension to interact
    #[serde(default)]
    pub required: bool,
}

/// A skill offered by the agent (A2A 1.0 `AgentSkill`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    /// Unique skill identifier (required)
    pub id: String,
    /// Human-readable name (required)
    pub name: String,
    /// What the skill does (required)
    pub description: String,
    /// Descriptive tags (required)
    pub tags: Vec<String>,
    /// Example prompts the skill handles
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    /// Input media types, when different from the card defaults
    #[serde(rename = "inputModes", default, skip_serializing_if = "Vec::is_empty")]
    pub input_modes: Vec<String>,
    /// Output media types, when different from the card defaults
    #[serde(rename = "outputModes", default, skip_serializing_if = "Vec::is_empty")]
    pub output_modes: Vec<String>,
}

/// The organization providing the agent (A2A 1.0 `AgentProvider`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProvider {
    /// Provider homepage URL (required)
    pub url: String,
    /// Provider organization name (required)
    pub organization: String,
}

/// A named security scheme (A2A 1.0 `SecurityScheme` oneof).
///
/// `ProtoJSON` oneof wrapper form: the JSON member name identifies the
/// variant, e.g. `{"httpAuthSecurityScheme": {"scheme": "bearer"}}` —
/// unlike the pre-1.0 `OpenAPI` `type` discriminator.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SecurityScheme {
    /// HTTP authentication (RFC 9110), e.g. bearer tokens
    #[serde(rename = "httpAuthSecurityScheme")]
    HttpAuth(HttpAuthSecurityScheme),
    /// API key in a header, query parameter, or cookie
    #[serde(rename = "apiKeySecurityScheme")]
    ApiKey(ApiKeySecurityScheme),
    /// OAuth 2.0 flows
    #[serde(rename = "oauth2SecurityScheme")]
    OAuth2(OAuth2SecurityScheme),
}

/// HTTP authentication scheme details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpAuthSecurityScheme {
    /// IANA HTTP authentication scheme name, e.g. "bearer" (required)
    pub scheme: String,
    /// Bearer token format hint, e.g. "JWT"
    #[serde(rename = "bearerFormat", skip_serializing_if = "Option::is_none")]
    pub bearer_format: Option<String>,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// API key scheme details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeySecurityScheme {
    /// Where the key is sent: "query", "header", or "cookie" (required)
    pub location: String,
    /// Parameter/header name carrying the key (required)
    pub name: String,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// OAuth 2.0 scheme details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2SecurityScheme {
    /// Supported OAuth 2.0 flows (required)
    pub flows: OAuthFlows,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// OAuth 2.0 flow configurations (A2A 1.0 `OAuthFlows`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OAuthFlows {
    /// Client-credentials flow
    #[serde(rename = "clientCredentials", skip_serializing_if = "Option::is_none")]
    pub client_credentials: Option<ClientCredentialsOAuthFlow>,
    /// Authorization-code flow
    #[serde(rename = "authorizationCode", skip_serializing_if = "Option::is_none")]
    pub authorization_code: Option<AuthorizationCodeOAuthFlow>,
}

/// OAuth 2.0 client-credentials flow configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCredentialsOAuthFlow {
    /// Token endpoint URL (required)
    #[serde(rename = "tokenUrl")]
    pub token_url: String,
    /// Available scopes: name → description (required)
    pub scopes: BTreeMap<String, String>,
}

/// OAuth 2.0 authorization-code flow configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationCodeOAuthFlow {
    /// Authorization endpoint URL (required)
    #[serde(rename = "authorizationUrl")]
    pub authorization_url: String,
    /// Token endpoint URL (required)
    #[serde(rename = "tokenUrl")]
    pub token_url: String,
    /// Available scopes: name → description (required)
    pub scopes: BTreeMap<String, String>,
    /// Whether PKCE (RFC 7636) is required (new in 1.0)
    #[serde(rename = "pkceRequired", default)]
    pub pkce_required: bool,
}

/// A security requirement (A2A 1.0 `SecurityRequirement`):
/// scheme name → required scope values. All schemes in one requirement must
/// be satisfied together; requirements in a list are alternatives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityRequirement {
    /// Scheme name → required scopes (proto `map<string, StringList>`)
    pub schemes: BTreeMap<String, StringListValue>,
}

/// `ProtoJSON` form of the proto `StringList` message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringListValue {
    /// The list values
    #[serde(default)]
    pub values: Vec<String>,
}

impl AgentCard {
    /// Fallback base URL when `BASE_URL` env var is not set
    const FALLBACK_BASE_URL: &'static str = "http://localhost:8081";

    /// Resolve the base URL from `BASE_URL` env var, falling back to localhost
    #[must_use]
    fn resolve_base_url() -> String {
        env::var("BASE_URL").unwrap_or_else(|_| Self::FALLBACK_BASE_URL.to_owned())
    }

    /// Create a new Agent Card for Pierre with specified base URL
    ///
    /// Prefer this constructor when you have access to server configuration.
    #[must_use]
    pub fn with_base_url(base_url: &str) -> Self {
        Self {
            name: "Dravr AI".into(),
            description: "AI-powered fitness data analysis and insights platform providing comprehensive activity analysis, performance tracking, and intelligent recommendations for athletes and fitness enthusiasts. Skills are invoked as structured tool calls: a message carries a data part shaped {\"tool_name\": \"<skill id>\", \"parameters\": {...}}.".into(),
            version: "1.0.0".into(),
            supported_interfaces: vec![
                AgentInterface {
                    url: format!("{base_url}/a2a/jsonrpc"),
                    protocol_binding: BINDING_JSONRPC.into(),
                    protocol_version: A2A_VERSION.into(),
                    tenant: None,
                },
                AgentInterface {
                    url: format!("{base_url}/a2a"),
                    protocol_binding: BINDING_HTTP_JSON.into(),
                    protocol_version: A2A_VERSION.into(),
                    tenant: None,
                },
            ],
            capabilities: AgentCapabilities {
                streaming: true,
                push_notifications: true,
                extended_agent_card: true,
                extensions: Vec::new(),
            },
            default_input_modes: vec![DATA_PART_MEDIA_TYPE.into()],
            default_output_modes: vec!["application/json".into(), "text/plain".into()],
            skills: Self::create_skills(),
            provider: Some(AgentProvider {
                url: "https://dravr.ai".into(),
                organization: "dravr.ai".into(),
            }),
            documentation_url: Some("https://docs.pierre.ai/a2a".into()),
            security_schemes: Some(Self::create_security_schemes(base_url)),
            security_requirements: vec![
                SecurityRequirement {
                    schemes: BTreeMap::from([(
                        "bearerAuth".to_owned(),
                        StringListValue { values: Vec::new() },
                    )]),
                },
                SecurityRequirement {
                    schemes: BTreeMap::from([(
                        "oauth2ClientCredentials".to_owned(),
                        StringListValue {
                            values: vec!["fitness:read".to_owned()],
                        },
                    )]),
                },
            ],
        }
    }

    /// Create a new Agent Card for Pierre using `BASE_URL` env var
    ///
    /// Falls back to `http://localhost:8081` if `BASE_URL` is not set.
    /// For explicit URL control, use `with_base_url()`.
    #[must_use]
    pub fn new() -> Self {
        Self::with_base_url(&Self::resolve_base_url())
    }

    /// Named security schemes advertised by the card
    fn create_security_schemes(base_url: &str) -> BTreeMap<String, SecurityScheme> {
        let scopes = BTreeMap::from([
            (
                "fitness:read".to_owned(),
                "Read fitness activities and athlete data".to_owned(),
            ),
            (
                "analytics:read".to_owned(),
                "Read analytics and intelligence outputs".to_owned(),
            ),
            ("goals:read".to_owned(), "Read fitness goals".to_owned()),
            (
                "goals:write".to_owned(),
                "Create and update fitness goals".to_owned(),
            ),
        ]);

        BTreeMap::from([
            (
                "bearerAuth".to_owned(),
                SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                    scheme: "bearer".into(),
                    bearer_format: Some("JWT".into()),
                    description: Some("Pierre-issued JWT in the Authorization header".into()),
                }),
            ),
            (
                "oauth2ClientCredentials".to_owned(),
                SecurityScheme::OAuth2(OAuth2SecurityScheme {
                    flows: OAuthFlows {
                        client_credentials: Some(ClientCredentialsOAuthFlow {
                            token_url: format!("{base_url}{OAUTH2_TOKEN_PATH}"),
                            scopes,
                        }),
                        authorization_code: None,
                    },
                    description: Some(
                        "OAuth2 client-credentials flow for registered A2A clients".into(),
                    ),
                }),
            ),
        ])
    }

    /// Skills advertised by the card.
    ///
    /// A skill is invoked by sending it as a `SendMessage` `data` part, so
    /// every example is written in that literal wire shape — the only form
    /// the message surface accepts.
    fn create_skills() -> Vec<AgentSkill> {
        vec![
            AgentSkill {
                id: "get_activities".into(),
                name: "Get Activities".into(),
                description: "Retrieve user fitness activities from connected providers".into(),
                tags: vec!["fitness".into(), "activities".into(), "data".into()],
                examples: vec![
                    r#"{"data": {"tool_name": "get_activities", "parameters": {"limit": 5}}}"#
                        .into(),
                    r#"{"data": {"tool_name": "get_activities", "parameters": {"after": 1640995200, "before": 1672531200}}}"#
                        .into(),
                ],
                input_modes: Vec::new(),
                output_modes: Vec::new(),
            },
            AgentSkill {
                id: "analyze_activity".into(),
                name: "Analyze Activity".into(),
                description: "AI-powered analysis of a specific fitness activity".into(),
                tags: vec!["fitness".into(), "analysis".into(), "intelligence".into()],
                examples: vec![
                    r#"{"data": {"tool_name": "analyze_activity", "parameters": {"activity_id": "12345", "provider": "strava"}}}"#
                        .into(),
                ],
                input_modes: Vec::new(),
                output_modes: Vec::new(),
            },
            AgentSkill {
                id: "get_athlete".into(),
                name: "Get Athlete Profile".into(),
                description: "Retrieve athlete profile information".into(),
                tags: vec!["fitness".into(), "profile".into()],
                examples: vec![
                    r#"{"data": {"tool_name": "get_athlete", "parameters": {"provider": "strava"}}}"#
                        .into(),
                ],
                input_modes: Vec::new(),
                output_modes: Vec::new(),
            },
            AgentSkill {
                id: "set_goal".into(),
                name: "Set Goal".into(),
                description: "Set a fitness goal for the user".into(),
                tags: vec!["fitness".into(), "goals".into()],
                examples: vec![
                    r#"{"data": {"tool_name": "set_goal", "parameters": {"goal_type": "distance", "target_value": 40, "timeframe": "week"}}}"#
                        .into(),
                ],
                input_modes: Vec::new(),
                output_modes: Vec::new(),
            },
        ]
    }

    /// Serialize the agent card to JSON
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Create agent card from JSON
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - JSON parsing fails
    /// - JSON structure is invalid
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

impl Default for AgentCard {
    fn default() -> Self {
        Self::new()
    }
}
