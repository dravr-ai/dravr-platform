// ABOUTME: MCP protocol schema definitions and message structures
// ABOUTME: Defines JSON-RPC protocol schemas for Model Context Protocol communication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! MCP Protocol Schema Definitions
//!
//! This module contains type-safe definitions for all MCP protocol messages,
//! capabilities, and tool schemas. This ensures protocol compliance and makes
//! it easy to modify the schema without hardcoding JSON.

// Schema sub-modules are retained for reference during migration but no longer
// used by get_tools(). The ToolRegistry is the single source of truth for tool schemas.

use crate::constants::get_server_config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// JSON-RPC and notification method constants
const JSONRPC_VERSION: &str = "2.0";
const METHOD_PROGRESS: &str = "notifications/progress";
const METHOD_CANCELLED: &str = "notifications/cancelled";
const METHOD_OAUTH_COMPLETED: &str = "notifications/oauth_completed";

// Note: Schema type strings ("string", "object", etc.) and property descriptions
// must be converted to String via .into() when inserted into HashMap/Vec because
// serde requires owned data for serialization. These allocations are necessary
// and cannot be eliminated without changing the serde data model to use Cow or &'static str.

/// MCP Protocol Information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolInfo {
    /// MCP protocol version (e.g., "2025-06-18")
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
}

/// Server Information per MCP spec
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Server name identifier (machine-readable)
    pub name: String,
    /// Server version string
    pub version: String,
    /// Human-readable display title (MCP 2025-11-25)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Human-readable server description (MCP 2025-11-25)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Server website URL (MCP 2025-11-25)
    #[serde(rename = "websiteUrl", skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
}

/// MCP Tool Schema Definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    /// Tool name identifier
    pub name: String,
    /// Human-readable tool description
    pub description: String,
    /// JSON Schema for tool input parameters
    #[serde(rename = "inputSchema")]
    pub input_schema: JsonSchema,
    /// Behavioral annotations for the tool (MCP 2025-11-25)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
}

impl ToolSchema {
    /// Create a tool schema without annotations
    #[must_use]
    pub fn without_annotations(
        name: String,
        description: String,
        input_schema: JsonSchema,
    ) -> Self {
        Self {
            name,
            description,
            input_schema,
            annotations: None,
        }
    }

    /// Create a tool schema with behavioral annotations (MCP 2025-11-25)
    #[must_use]
    pub fn with_annotations(
        name: String,
        description: String,
        input_schema: JsonSchema,
        annotations: ToolAnnotations,
    ) -> Self {
        Self {
            name,
            description,
            input_schema,
            annotations: Some(annotations),
        }
    }
}

/// Behavioral annotations for MCP tools (MCP 2025-11-25)
///
/// Provides hints to clients about tool behavior, enabling better UX decisions
/// such as confirmation prompts for destructive operations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolAnnotations {
    /// Human-readable display title
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Whether the tool only reads data without side effects
    #[serde(rename = "readOnlyHint", skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    /// Whether the tool may perform destructive operations (delete, overwrite)
    #[serde(rename = "destructiveHint", skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    /// Whether calling the tool repeatedly with the same args has no additional effect
    #[serde(rename = "idempotentHint", skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    /// Whether the tool interacts with external entities beyond the server
    #[serde(rename = "openWorldHint", skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
}

/// JSON Schema Definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSchema {
    /// Schema type (e.g., "object", "string")
    #[serde(rename = "type")]
    pub schema_type: String,
    /// Property definitions for object schemas
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, PropertySchema>>,
    /// List of required property names
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
}

/// Tool Call for executing a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Name of the tool to execute
    pub name: String,
    /// Tool arguments as JSON
    pub arguments: Option<serde_json::Value>,
}

/// Tool Response after execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponse {
    /// Response content items
    pub content: Vec<Content>,
    /// Whether the tool execution resulted in an error
    #[serde(rename = "isError")]
    pub is_error: bool,
    /// Structured response data
    #[serde(rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<serde_json::Value>,
}

/// Content types for MCP messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Content {
    /// Plain text content
    #[serde(rename = "text")]
    Text {
        /// Text content string
        text: String,
    },
    /// Image content with base64 data
    #[serde(rename = "image")]
    Image {
        /// Base64-encoded image data
        data: String,
        /// MIME type of the image (e.g., "image/png")
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    /// Resource reference with URI
    #[serde(rename = "resource")]
    Resource {
        /// URI of the resource
        uri: String,
        /// Optional text description of the resource
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// MIME type of the resource
        #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    /// Progress update for long-running operations
    #[serde(rename = "progress")]
    Progress {
        /// Token identifying the operation
        #[serde(rename = "progressToken")]
        progress_token: String,
        /// Current progress value
        progress: f64,
        /// Optional total value for calculating percentage
        total: Option<f64>,
    },
}

/// Tool definition structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Tool name identifier
    pub name: String,
    /// Human-readable tool description
    pub description: String,
    /// JSON Schema for tool input as raw JSON value
    pub input_schema: serde_json::Value,
}

/// JSON Schema Property Definition
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PropertySchema {
    /// Property type (e.g., "string", "number", "boolean")
    #[serde(rename = "type")]
    pub property_type: String,
    /// Human-readable property description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Item schema for array-type properties (JSON Schema `items` keyword)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<Self>>,
    /// Nested property definitions for object-type properties
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, Self>>,
    /// Required fields for object-type properties
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
}

/// MCP Server Capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// Experimental capabilities not in MCP spec
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<HashMap<String, serde_json::Value>>,
    /// Server logging capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<LoggingCapability>,
    /// Server prompts capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
    /// Server resources capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    /// Server tools capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    /// Server authentication capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthCapability>,
    /// Server OAuth 2.0 capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth2: Option<OAuth2Capability>,
    /// Server completion (auto-complete) capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion: Option<CompletionCapability>,
    /// Server sampling (LLM calls) capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<SamplingCapability>,
}

/// Tools capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsCapability {
    /// Whether the server supports list changed notifications
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

/// Logging capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingCapability {}

/// Prompts capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptsCapability {
    /// Whether the server supports list changed notifications
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

/// Resources capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcesCapability {
    /// Whether the server supports resource subscriptions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<bool>,
    /// Whether the server supports list changed notifications
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

/// Authentication capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthCapability {
    /// OAuth 2.0 authentication details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth2: Option<OAuth2Capability>,
}

/// OAuth 2.0 capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Capability {
    /// OAuth 2.0 discovery URL
    #[serde(rename = "discoveryUrl")]
    pub discovery_url: String,
    /// OAuth 2.0 authorization endpoint
    #[serde(rename = "authorizationEndpoint")]
    pub authorization_endpoint: String,
    /// OAuth 2.0 token endpoint
    #[serde(rename = "tokenEndpoint")]
    pub token_endpoint: String,
    /// OAuth 2.0 client registration endpoint (RFC 7591)
    #[serde(rename = "registrationEndpoint")]
    pub registration_endpoint: String,
}

/// Completion (auto-complete) capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionCapability {}

/// Client capabilities (for processing client initialize requests)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCapabilities {
    /// Experimental client capabilities
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<HashMap<String, serde_json::Value>>,
    /// Client sampling capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<SamplingCapability>,
    /// Client roots capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<RootsCapability>,
}

/// Sampling capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingCapability {}

/// Roots capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootsCapability {
    /// Whether the client supports list changed notifications
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

/// Complete MCP Initialize Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResponse {
    /// Negotiated protocol version
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Server information
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
    /// Server capabilities
    pub capabilities: ServerCapabilities,
    /// Optional server instructions for the client
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Initialize Request from client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeRequest {
    /// Client's requested protocol version
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Client information
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
    /// Client capabilities
    pub capabilities: ClientCapabilities,
    /// Optional OAuth application credentials provided by the client
    #[serde(
        rename = "oauthCredentials",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub oauth_credentials: Option<HashMap<String, OAuthAppCredentials>>,
}

pub use pierre_core::models::OAuthAppCredentials;

/// Client Information per MCP spec
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    /// Client name identifier (machine-readable)
    pub name: String,
    /// Client version string
    pub version: String,
    /// Human-readable display title (MCP 2025-11-25)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Human-readable client description (MCP 2025-11-25)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Client website URL (MCP 2025-11-25)
    #[serde(
        default,
        rename = "websiteUrl",
        skip_serializing_if = "Option::is_none"
    )]
    pub website_url: Option<String>,
}

impl InitializeResponse {
    /// Create a new initialize response with current server configuration
    #[must_use]
    pub fn new(protocol_version: String, server_name: String, server_version: String) -> Self {
        Self::new_with_ports(protocol_version, server_name, server_version, 8081)
    }

    /// Create a new initialize response with specific HTTP port for OAuth endpoints
    #[must_use]
    pub fn new_with_ports(
        protocol_version: String,
        server_name: String,
        server_version: String,
        http_port: u16,
    ) -> Self {
        Self {
            protocol_version,
            server_info: ServerInfo {
                name: server_name,
                version: server_version,
                title: Some("Dravr".to_owned()),
                description: Some(
                    "MCP server for fitness data analytics, coaching, and nutrition planning"
                        .to_owned(),
                ),
                website_url: None,
            },
            capabilities: ServerCapabilities {
                experimental: None,
                logging: Some(LoggingCapability {}),
                prompts: None,
                resources: Some(ResourcesCapability {
                    subscribe: None,
                    list_changed: Some(false),
                }),
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                auth: Some(AuthCapability {
                    oauth2: Some({
                        let base = get_server_config()
                            .map_or_else(|| format!("http://localhost:{http_port}"), |c| c.base_url.clone());
                        OAuth2Capability {
                            discovery_url: format!("{base}/.well-known/oauth-authorization-server"),
                            authorization_endpoint: format!("{base}/oauth2/authorize"),
                            token_endpoint: format!("{base}/oauth2/token"),
                            registration_endpoint: format!("{base}/oauth2/register"),
                        }
                    }),
                }),
                oauth2: Some({
                    let base = get_server_config()
                        .map_or_else(|| format!("http://localhost:{http_port}"), |c| c.base_url.clone());
                    OAuth2Capability {
                        discovery_url: format!("{base}/.well-known/oauth-authorization-server"),
                        authorization_endpoint: format!("{base}/oauth2/authorize"),
                        token_endpoint: format!("{base}/oauth2/token"),
                        registration_endpoint: format!("{base}/oauth2/register"),
                    }
                }),
                completion: Some(CompletionCapability {}),
                sampling: Some(SamplingCapability {}),
            },
            instructions: Some("This server provides fitness data tools for Strava and Fitbit integration. OAuth must be configured at tenant level via REST API. Use `get_activities`, `get_athlete`, and other analytics tools to access your fitness data.".into()),
        }
    }
}

/// Progress notification for long-running operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressNotification {
    /// JSON-RPC version ("2.0")
    pub jsonrpc: String,
    /// Method name ("notifications/progress")
    pub method: String,
    /// Progress notification parameters
    pub params: ProgressParams,
}

/// Progress notification parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressParams {
    /// Token identifying the operation being tracked
    #[serde(rename = "progressToken")]
    pub progress_token: String,
    /// Current progress value
    pub progress: f64,
    /// Optional total value for percentage calculation
    pub total: Option<f64>,
    /// Optional human-readable progress message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ProgressNotification {
    /// Create a new progress notification
    #[must_use]
    pub fn new(
        progress_token: String,
        progress: f64,
        total: Option<f64>,
        message: Option<String>,
    ) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            method: METHOD_PROGRESS.to_owned(),
            params: ProgressParams {
                progress_token,
                progress,
                total,
                message,
            },
        }
    }

    /// Create a new cancellation notification
    #[must_use]
    pub fn cancelled(progress_token: String, message: Option<String>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            method: METHOD_CANCELLED.to_owned(),
            params: ProgressParams {
                progress_token,
                progress: 0.0,
                total: None,
                message,
            },
        }
    }
}

/// OAuth completion notification for MCP clients
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCompletedNotification {
    /// JSON-RPC version ("2.0")
    pub jsonrpc: String,
    /// Method name ("notifications/oauth/completed")
    pub method: String,
    /// OAuth completion parameters
    pub params: OAuthCompletedParams,
}

/// OAuth completion notification parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCompletedParams {
    /// OAuth provider name (e.g., "strava", "google")
    pub provider: String,
    /// Whether the OAuth flow completed successfully
    pub success: bool,
    /// Human-readable status message
    pub message: String,
    /// User ID if authentication succeeded
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

impl OAuthCompletedNotification {
    /// Create a new OAuth completion notification
    #[must_use]
    pub fn new(provider: String, success: bool, message: String, user_id: Option<String>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            method: METHOD_OAUTH_COMPLETED.to_owned(),
            params: OAuthCompletedParams {
                provider,
                success,
                message,
                user_id,
            },
        }
    }
}

// === MCP SAMPLING (LLM CALL) TYPES ===

/// Request to create a message using the client's LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMessageRequest {
    /// Messages to send to the LLM
    pub messages: Vec<PromptMessage>,
    /// Optional model preferences
    #[serde(rename = "modelPreferences", skip_serializing_if = "Option::is_none")]
    pub model_preferences: Option<ModelPreferences>,
    /// Optional system prompt
    #[serde(rename = "systemPrompt", skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Include context from MCP servers
    #[serde(rename = "includeContext", skip_serializing_if = "Option::is_none")]
    pub include_context: Option<String>,
    /// Maximum tokens to generate
    #[serde(rename = "maxTokens")]
    pub max_tokens: i32,
    /// Sampling temperature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Stop sequences
    #[serde(rename = "stopSequences", skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    /// Additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Result from create message request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMessageResult {
    /// Role of the message (usually "assistant")
    pub role: String,
    /// Content of the generated message
    pub content: MessageContent,
    /// Model that was used
    pub model: String,
    /// Stop reason for completion
    #[serde(rename = "stopReason", skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

/// Message content wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageContent {
    /// Type of content (usually "text")
    #[serde(rename = "type")]
    pub content_type: String,
    /// Text content
    pub text: String,
}

/// Model preferences for sampling
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelPreferences {
    /// Model hints in preference order
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<Vec<ModelHint>>,
    /// Cost priority (0.0-1.0, where 1.0 prefers cheaper models)
    #[serde(rename = "costPriority", skip_serializing_if = "Option::is_none")]
    pub cost_priority: Option<f64>,
    /// Speed priority (0.0-1.0, where 1.0 prefers faster models)
    #[serde(rename = "speedPriority", skip_serializing_if = "Option::is_none")]
    pub speed_priority: Option<f64>,
    /// Intelligence priority (0.0-1.0, where 1.0 prefers more capable models)
    #[serde(
        rename = "intelligencePriority",
        skip_serializing_if = "Option::is_none"
    )]
    pub intelligence_priority: Option<f64>,
}

/// Hint for model selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelHint {
    /// Model name (e.g., "claude-3-5-sonnet")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Prompt message for LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptMessage {
    /// Role of the message sender
    pub role: String,
    /// Content of the message
    pub content: Content,
}

impl PromptMessage {
    /// Create a user message
    #[must_use]
    pub fn user(content: Content) -> Self {
        Self {
            role: "user".to_owned(),
            content,
        }
    }

    /// Create an assistant message
    #[must_use]
    pub fn assistant(content: Content) -> Self {
        Self {
            role: "assistant".to_owned(),
            content,
        }
    }
}

// === MCP COMPLETION (AUTO-COMPLETE) TYPES ===

/// Request for completion suggestions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteRequest {
    /// Reference to the item being completed
    #[serde(rename = "ref")]
    pub ref_: CompletionReference,
    /// Current argument being completed
    pub argument: ArgumentValue,
}

/// Reference to completion context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionReference {
    /// Type of reference
    #[serde(rename = "type")]
    pub type_: String,
    /// Name of the tool/resource/prompt
    pub name: String,
}

/// Argument value for completion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgumentValue {
    /// Name of the argument
    pub name: String,
    /// Current value being typed
    pub value: String,
}

/// Result from completion request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteResult {
    /// Completion suggestions
    pub completion: Completion,
}

impl Default for CompleteResult {
    fn default() -> Self {
        Self {
            completion: Completion {
                values: vec![],
                total: Some(0),
                has_more: Some(false),
            },
        }
    }
}

/// Completion suggestion list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Completion {
    /// Suggested completion values
    pub values: Vec<String>,
    /// Total number of possible completions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    /// Whether there are more completions available
    #[serde(rename = "hasMore", skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
}

// === MCP ROOTS TYPES ===

/// Root directory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Root {
    /// URI of the root directory
    pub uri: String,
    /// Human-readable name
    pub name: String,
}

// === SHARED HELPERS FOR DOMAIN SCHEMA FILES ===

/// Get all available tools via `ToolRegistry` (single source of truth)
///
/// Creates a fresh `ToolRegistry` with all built-in tools and returns their schemas.
/// This is used by tests and standalone endpoints. Production code should use the
/// shared `ToolRegistry` from `ServerContext` instead.
#[must_use]
pub fn get_tools() -> Vec<ToolSchema> {
    use crate::tools::registry::ToolRegistry;

    let mut registry = ToolRegistry::new();
    registry.register_builtin_tools();
    registry.all_schemas()
}
