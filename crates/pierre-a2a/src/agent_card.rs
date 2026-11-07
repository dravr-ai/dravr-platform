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

use pierre_auth::config::OAuth2ServerConfig;
use pierre_core::permissions::scopes::OAuthScope;
use serde::{Deserialize, Serialize};

use crate::protocol_types::A2A_VERSION;

/// Protocol binding identifier for the JSON-RPC 2.0 interface (spec §5.2)
pub const BINDING_JSONRPC: &str = "JSONRPC";
/// Protocol binding identifier for the HTTP+JSON/REST interface (spec §5.2)
pub const BINDING_HTTP_JSON: &str = "HTTP+JSON";

/// Media type of the structured `data` part that carries a tool invocation:
/// what every tool skill reads.
pub const DATA_PART_MEDIA_TYPE: &str = "application/json";
/// The skill a `text` part reaches: a coaching turn, not a registered tool.
pub const COACHING_SKILL_ID: &str = "coaching";
/// The grant the coaching skill needs: a turn's prompt carries the athlete's
/// training and who they are, whatever tool it goes on to call. The card's
/// authorization-code requirement names both, so a client that asks for what
/// the card requires can use every skill an athlete's grant reaches.
pub(crate) const COACHING_SKILL_SCOPES: [OAuthScope; 2] =
    [OAuthScope::FitnessRead, OAuthScope::ProfileRead];
/// Media type of a `text` part: what the coaching skill reads and writes.
pub const TEXT_PART_MEDIA_TYPE: &str = "text/plain";

/// Path of the OAuth 2.0 authorization server's token endpoint.
///
/// This is the route that serves the `authorization_code` and
/// `client_credentials` grants the card's OAuth schemes advertise. Mounted by
/// `pierre-routes-identity` and published as `token_endpoint` in the RFC 8414
/// discovery document.
pub const OAUTH2_TOKEN_PATH: &str = "/oauth2/token";

/// Path of the OAuth 2.0 authorization server's authorization endpoint,
/// published as `authorization_endpoint` in the RFC 8414 discovery document.
pub const OAUTH2_AUTHORIZE_PATH: &str = "/oauth2/authorize";

/// Path of the RFC 8414 authorization server metadata document on the issuer.
pub const OAUTH2_METADATA_PATH: &str = "/.well-known/oauth-authorization-server";

/// Where a person reads about the agent.
const DOCUMENTATION_URL: &str = "https://dravr.ai/docs";

/// Path of the agent's icon on the protocol door, a static file of the web app.
pub const ICON_PATH: &str = "/pwa-512.png";

/// Name of the card's authorization-code scheme: an athlete delegates to an
/// agent, the marketplace path.
pub const SCHEME_AUTHORIZATION_CODE: &str = "oauth2AuthorizationCode";

/// Name of the card's client-credentials scheme: a registered B2B client, which
/// acts as the user who registered it.
pub const SCHEME_CLIENT_CREDENTIALS: &str = "oauth2ClientCredentials";

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
    /// URL of an icon for the agent
    #[serde(rename = "iconUrl", skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
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
    /// The OAuth 2.0 flow this scheme offers (required)
    pub flows: OAuthFlows,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// URL of the authorization server's RFC 8414 metadata document
    #[serde(rename = "oauth2MetadataUrl", skip_serializing_if = "Option::is_none")]
    pub oauth2_metadata_url: Option<String>,
}

/// OAuth 2.0 flow configuration (A2A 1.0 `OAuthFlows`).
///
/// A `oneof` in the normative proto: one scheme offers exactly one flow, so an
/// agent offering two publishes two schemes.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OAuthFlows {
    /// Authorization-code flow
    #[serde(rename = "authorizationCode")]
    AuthorizationCode(AuthorizationCodeOAuthFlow),
    /// Client-credentials flow
    #[serde(rename = "clientCredentials")]
    ClientCredentials(ClientCredentialsOAuthFlow),
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
    pub schemes: BTreeMap<String, StringList>,
}

/// `ProtoJSON` form of the proto `StringList` message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringList {
    /// The individual values
    #[serde(default)]
    pub list: Vec<String>,
}

impl AgentCard {
    /// The Agent Card of this deployment.
    ///
    /// The protocol URLs name the published protocol door
    /// ([`OAuth2ServerConfig::protocol_origin`], from `MCP_RESOURCE_URL`), so
    /// the card is the same whichever host serves it. The OAuth URLs name the
    /// issuer, as its RFC 8414 document does, which may be another host.
    #[must_use]
    pub fn new(config: &OAuth2ServerConfig) -> Self {
        let base_url = config.protocol_origin();
        Self {
            name: "Dravr AI".into(),
            description: "AI-powered fitness data analysis and insights platform providing comprehensive activity analysis, performance tracking, and intelligent recommendations for athletes and fitness enthusiasts. Tool skills are invoked as structured tool calls: a message carries a data part shaped {\"tool_name\": \"<skill id>\", \"parameters\": {...}}. The coaching skill reads a plain text part.".into(),
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
            default_input_modes: vec![DATA_PART_MEDIA_TYPE.into(), TEXT_PART_MEDIA_TYPE.into()],
            default_output_modes: vec!["application/json".into(), "text/plain".into()],
            skills: Self::create_skills(),
            provider: Some(AgentProvider {
                url: "https://dravr.ai".into(),
                organization: "dravr.ai".into(),
            }),
            documentation_url: Some(DOCUMENTATION_URL.into()),
            icon_url: Some(format!("{base_url}{ICON_PATH}")),
            security_schemes: Some(Self::create_security_schemes(&config.issuer_url)),
            security_requirements: vec![
                Self::requirement(
                    SCHEME_AUTHORIZATION_CODE,
                    COACHING_SKILL_SCOPES
                        .iter()
                        .map(|scope| scope.as_str().to_owned())
                        .collect(),
                ),
                // A client-credentials token acts for no athlete, so it never
                // reaches the coaching skill and is asked for the read grant only.
                Self::requirement(SCHEME_CLIENT_CREDENTIALS, vec![read_scope()]),
                Self::requirement("bearerAuth", Vec::new()),
            ],
        }
    }

    /// A security requirement of one scheme.
    fn requirement(scheme: &str, scopes: Vec<String>) -> SecurityRequirement {
        SecurityRequirement {
            schemes: BTreeMap::from([(scheme.to_owned(), StringList { list: scopes })]),
        }
    }

    /// Named security schemes advertised by the card
    fn create_security_schemes(issuer: &str) -> BTreeMap<String, SecurityScheme> {
        // The same vocabulary `scopes_supported` publishes: every scope the
        // authorization server can grant a client, and nothing it would refuse.
        let scopes: BTreeMap<String, String> = OAuthScope::ALL
            .into_iter()
            .filter(|scope| scope.is_delegable())
            .map(|scope| (scope.as_str().to_owned(), scope.description().to_owned()))
            .collect();
        let token_url = format!("{issuer}{OAUTH2_TOKEN_PATH}");
        let metadata_url = format!("{issuer}{OAUTH2_METADATA_PATH}");

        BTreeMap::from([
            (
                "bearerAuth".to_owned(),
                SecurityScheme::HttpAuth(HttpAuthSecurityScheme {
                    scheme: "bearer".into(),
                    bearer_format: Some("JWT".into()),
                    description: Some(
                        "Dravr-issued OAuth access token in the Authorization header".into(),
                    ),
                }),
            ),
            (
                SCHEME_AUTHORIZATION_CODE.to_owned(),
                SecurityScheme::OAuth2(OAuth2SecurityScheme {
                    flows: OAuthFlows::AuthorizationCode(AuthorizationCodeOAuthFlow {
                        authorization_url: format!("{issuer}{OAUTH2_AUTHORIZE_PATH}"),
                        token_url: token_url.clone(), // Safe: String ownership for both flows
                        scopes: scopes.clone(),       // Safe: BTreeMap ownership for both flows
                        pkce_required: true,
                    }),
                    description: Some(
                        "An athlete authorizes the agent to act for them (authorization code with PKCE)"
                            .into(),
                    ),
                    oauth2_metadata_url: Some(metadata_url.clone()), // Safe: String ownership for both flows
                }),
            ),
            (
                SCHEME_CLIENT_CREDENTIALS.to_owned(),
                SecurityScheme::OAuth2(OAuth2SecurityScheme {
                    flows: OAuthFlows::ClientCredentials(ClientCredentialsOAuthFlow {
                        token_url,
                        scopes,
                    }),
                    description: Some(
                        "OAuth2 client-credentials flow for registered A2A clients".into(),
                    ),
                    oauth2_metadata_url: Some(metadata_url),
                }),
            ),
        ])
    }

    /// Skills advertised by the card.
    ///
    /// A tool skill is invoked by sending it as a `SendMessage` `data` part, so
    /// its examples are written in that literal wire shape. The coaching skill
    /// is the one a `text` part reaches, and its examples are plain sentences.
    fn create_skills() -> Vec<AgentSkill> {
        vec![
            AgentSkill {
                id: COACHING_SKILL_ID.into(),
                name: "Coaching".into(),
                description: "Ask the athlete's coach a question in plain language and get a \
                              written answer grounded in their training. Needs a grant the \
                              athlete authorized, holding fitness:read and profile:read."
                    .into(),
                tags: vec!["fitness".into(), "coaching".into(), "conversation".into()],
                examples: vec![
                    "How did my training go this week?".into(),
                    "Should I run long tomorrow or take the day off?".into(),
                ],
                input_modes: vec![TEXT_PART_MEDIA_TYPE.into()],
                output_modes: vec![TEXT_PART_MEDIA_TYPE.into()],
            },
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

/// The scope a client-credentials connection asks for.
fn read_scope() -> String {
    OAuthScope::FitnessRead.as_str().to_owned()
}
