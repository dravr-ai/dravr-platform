// ABOUTME: Minimal JSON-RPC 2.0 types for A2A protocol communication
// ABOUTME: Structurally compatible with main crate's jsonrpc module for seamless conversion
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Minimal JSON-RPC 2.0 type definitions for `pierre-a2a`.
//!
//! These types are **intentionally duplicated** from the main server crate's
//! `jsonrpc` module to avoid a circular dependency: `pierre-a2a` is a leaf
//! crate that cannot depend on the main server crate. The structs are
//! serde-compatible with the main crate's versions, so data serialised from
//! one can be deserialised into the other.
//!
//! If these structs diverge, update both locations:
//! - `crates/pierre-a2a/src/jsonrpc.rs` (this file)
//! - `crates/pierre-server/src/jsonrpc/mod.rs` (main crate)

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;

/// JSON-RPC 2.0 version string
pub const JSONRPC_VERSION: &str = "2.0";

/// JSON-RPC 2.0 Request
///
/// Structurally compatible with the main server crate's `JsonRpcRequest`.
/// Protocol-specific extensions (like `auth_token`) are included as optional fields.
#[derive(Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (always "2.0")
    pub jsonrpc: String,

    /// Method name to invoke
    pub method: String,

    /// Optional parameters for the method
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,

    /// Request identifier (for correlation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,

    /// Authorization header value (Bearer token) - MCP/A2A extension
    #[serde(rename = "auth", skip_serializing_if = "Option::is_none", default)]
    pub auth_token: Option<String>,

    /// Optional HTTP headers for tenant context and other metadata - MCP extension
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub headers: Option<HashMap<String, Value>>,

    /// Protocol-specific metadata (additional extensions)
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub metadata: HashMap<String, String>,
}

// Custom Debug implementation that redacts sensitive auth tokens
impl fmt::Debug for JsonRpcRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonRpcRequest")
            .field("jsonrpc", &self.jsonrpc)
            .field("method", &self.method)
            .field("params", &self.params)
            .field("id", &self.id)
            .field(
                "auth_token",
                &self.auth_token.as_ref().map(|token| {
                    // Redact token: show first 10 and last 8 characters, or "[REDACTED]" if short
                    if token.len() > 20 {
                        format!("{}...{}", &token[..10], &token[token.len() - 8..])
                    } else {
                        "[REDACTED]".to_owned()
                    }
                }),
            )
            .field("headers", &self.headers)
            .field("metadata", &self.metadata)
            .finish()
    }
}

/// JSON-RPC 2.0 Response
///
/// Exactly one of `result` or `error` must be present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version (always "2.0")
    pub jsonrpc: String,

    /// Result of the method call (mutually exclusive with error)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,

    /// Error information (mutually exclusive with result)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,

    /// Request identifier for correlation
    pub id: Option<Value>,
}

/// JSON-RPC 2.0 Error Object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code (standard codes: -32700 to -32600)
    pub code: i32,

    /// Human-readable error message
    pub message: String,

    /// Additional error information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}
