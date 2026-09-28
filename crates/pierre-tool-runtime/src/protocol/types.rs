// ABOUTME: Core types for universal protocol system
// ABOUTME: Request, response, and executor types used across the universal protocol
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::protocol::executor::UniversalExecutor;
use crate::protocols::ProtocolError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Debug;

/// Universal request structure for protocol-agnostic tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalRequest {
    /// Name of the tool to execute
    pub tool_name: String,
    /// Tool-specific parameters as JSON
    pub parameters: Value,
    /// User ID making the request
    pub user_id: String,
    /// Protocol identifier (e.g., "mcp", "a2a")
    pub protocol: String,
    /// Optional tenant ID for multi-tenant isolation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

/// Universal response structure for tool execution results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalResponse {
    /// Whether the tool execution succeeded
    pub success: bool,
    /// Tool execution result as JSON
    pub result: Option<Value>,
    /// Error message if execution failed
    pub error: Option<String>,
    /// Additional metadata about the execution
    pub metadata: Option<HashMap<String, Value>>,
}

/// Metadata key set on a tool's `UniversalResponse` when the underlying
/// `AppError` was `ProviderAuthRequired`. The tool loop scans this key to
/// know it should exit early without continuing iteration.
///
/// Lives here (rather than in the `client-chat`-gated `tool_execution`
/// module) so it can be shared with always-compiled callers such as
/// [`provider_helpers`](super::provider_helpers).
pub const META_AUTH_REQUIRED_PROVIDER: &str = "auth_required_provider";

/// The provider slug a failed response says the athlete must reconnect, if any.
///
/// Every caller that re-raises the typed auth error, drops a cached page, or
/// decides a recovery turn reads [`META_AUTH_REQUIRED_PROVIDER`] the same way:
/// the key must be present *and* hold a string. Reading it by hand once
/// admitted a non-string value as a reconnect signal, so the extraction lives
/// here beside the key it reads.
#[must_use]
pub fn auth_required_provider(response: &UniversalResponse) -> Option<String> {
    response
        .metadata
        .as_ref()?
        .get(META_AUTH_REQUIRED_PROVIDER)?
        .as_str()
        .map(ToOwned::to_owned)
}

/// Universal tool definition with handler function
#[derive(Debug, Clone)]
pub struct UniversalTool {
    /// Tool name identifier
    pub name: String,
    /// Human-readable tool description
    pub description: String,
    /// Handler function for tool execution
    pub handler:
        fn(&UniversalToolExecutor, UniversalRequest) -> Result<UniversalResponse, ProtocolError>,
}

/// Type alias for `UniversalExecutor` used in tool handler signatures
pub type UniversalToolExecutor = UniversalExecutor;
