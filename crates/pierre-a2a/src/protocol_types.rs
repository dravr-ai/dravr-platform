// ABOUTME: A2A protocol error enum and message type definitions
// ABOUTME: Standalone types for protocol messages, initialization, and error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::jsonrpc::JsonRpcError;
use crate::models::OAuthAppCredentials;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A2A Protocol Version
pub const A2A_VERSION: &str = "1.0.0";

/// A2A Protocol Error types
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum A2AError {
    /// Invalid request parameters or format
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    /// Client not registered
    #[error("Client not registered: {0}")]
    ClientNotRegistered(String),
    /// Database operation failed
    #[error("Database error: {0}")]
    DatabaseError(String),
    /// Internal server error
    #[error("Internal error: {0}")]
    InternalError(String),
    /// Client has been deactivated
    #[error("Client deactivated: {0}")]
    ClientDeactivated(String),
    /// Rate limit exceeded
    #[error("Rate limit exceeded: {0}")]
    RateLimitExceeded(String),
    /// Session expired or invalid
    #[error("Session expired: {0}")]
    SessionExpired(String),
    /// Invalid session token
    #[error("Invalid session token: {0}")]
    InvalidSessionToken(String),
    /// Insufficient permissions
    #[error("Insufficient permissions: {0}")]
    InsufficientPermissions(String),
    /// Resource not found
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),
    /// Service temporarily unavailable
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),
}

impl From<A2AError> for JsonRpcError {
    fn from(error: A2AError) -> Self {
        let (code, message) = match error {
            A2AError::InvalidRequest(msg) => (-32602, format!("Invalid params: {msg}")),
            A2AError::AuthenticationFailed(msg) => {
                (-32001, format!("Authentication failed: {msg}"))
            }
            A2AError::ClientNotRegistered(msg) => (-32003, format!("Client not registered: {msg}")),
            A2AError::DatabaseError(msg) => (-32000, format!("Database error: {msg}")),
            A2AError::InternalError(msg) => (-32603, format!("Internal error: {msg}")),
            A2AError::ClientDeactivated(msg) => (-32004, format!("Client deactivated: {msg}")),
            A2AError::RateLimitExceeded(msg) => (-32005, format!("Rate limit exceeded: {msg}")),
            A2AError::SessionExpired(msg) => (-32006, format!("Session expired: {msg}")),
            A2AError::InvalidSessionToken(msg) => (-32007, format!("Invalid session token: {msg}")),
            A2AError::InsufficientPermissions(msg) => {
                (-32008, format!("Insufficient permissions: {msg}"))
            }
            A2AError::ResourceNotFound(msg) => (-32009, format!("Resource not found: {msg}")),
            A2AError::ServiceUnavailable(msg) => (-32010, format!("Service unavailable: {msg}")),
        };

        Self {
            code,
            message,
            data: None,
        }
    }
}

/// A2A Initialize Request structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AInitializeRequest {
    /// A2A protocol version
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Client information
    #[serde(rename = "clientInfo")]
    pub client_info: A2AClientInfo,
    /// Client capabilities
    pub capabilities: Vec<String>,
    /// Optional OAuth application credentials provided by the client
    #[serde(
        rename = "oauthCredentials",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub oauth_credentials: Option<HashMap<String, OAuthAppCredentials>>,
}

/// A2A Client Information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AClientInfo {
    /// Client application name
    pub name: String,
    /// Client application version
    pub version: String,
    /// Optional client description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A2A Initialize Response structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AInitializeResponse {
    /// Negotiated protocol version
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Server information
    #[serde(rename = "serverInfo")]
    pub server_info: A2AServerInfo,
    /// Server capabilities
    pub capabilities: Vec<String>,
}

/// A2A Server Information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AServerInfo {
    /// Server application name
    pub name: String,
    /// Server application version
    pub version: String,
    /// Optional server description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl A2AInitializeResponse {
    /// Create a new A2A initialize response with server information
    #[must_use]
    pub fn new(protocol_version: String, server_name: String, server_version: String) -> Self {
        Self {
            protocol_version,
            server_info: A2AServerInfo {
                name: server_name,
                version: server_version,
                description: Some(
                    "AI-powered fitness data analysis and insights platform".to_owned(),
                ),
            },
            capabilities: vec![
                "message/send".to_owned(),
                "message/stream".to_owned(),
                "tasks/create".to_owned(),
                "tasks/get".to_owned(),
                "tasks/cancel".to_owned(),
                "tasks/pushNotificationConfig/set".to_owned(),
                "tools/list".to_owned(),
                "tools/call".to_owned(),
            ],
        }
    }
}

/// A2A Message structure for agent communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AMessage {
    /// Unique message identifier
    pub id: String,
    /// Message content parts (text, data, or files)
    pub parts: Vec<MessagePart>,
    /// Optional metadata key-value pairs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, Value>>,
}

/// A2A Message Part types
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessagePart {
    /// Plain text message content
    #[serde(rename = "text")]
    Text {
        /// Text content
        content: String,
    },
    /// Structured data content (JSON)
    #[serde(rename = "data")]
    Data {
        /// Data content as JSON value
        content: Value,
    },
    /// File attachment content
    #[serde(rename = "file")]
    File {
        /// File name
        name: String,
        /// MIME type of the file
        mime_type: String,
        /// File content (base64 encoded)
        content: String,
    },
}
