// ABOUTME: A2A protocol types, agent card, and client data structures
// ABOUTME: Standalone types extractable from the main crate for parallel compilation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre A2A Protocol Types
//!
//! Standalone protocol types, message definitions, agent card, and client
//! data structures for the A2A (Agent-to-Agent) protocol. Server-side
//! implementations (handlers, auth, database operations) remain in the
//! main crate due to tight coupling with infrastructure.
//!
//! ## Modules
//!
//! * [`agent_card`] — Agent capability discovery and advertisement
//! * [`client_types`] — Client registration, credentials, usage types
//! * [`jsonrpc`] — Minimal JSON-RPC 2.0 types for A2A type aliases
//! * [`protocol_types`] — Protocol message types and error enum

#![deny(unsafe_code)]

// Re-export pierre-core modules for `crate::errors`, `crate::constants`, etc.
pub use pierre_core::constants;
pub use pierre_core::errors;
pub use pierre_core::models;

/// Agent card metadata and capabilities
pub mod agent_card;
/// Client registration and credential types
pub mod client_types;
/// Minimal JSON-RPC 2.0 types for A2A protocol
pub mod jsonrpc;
/// A2A protocol message types and error definitions
pub mod protocol_types;

pub use agent_card::AgentCard;
pub use client_types::{
    A2AClientTier, A2ARateLimitStatus, A2AToken, A2AUsageParams, ClientCredentials,
    ClientRegistrationRequest, ClientUsageStats,
};
pub use jsonrpc::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, JSONRPC_VERSION};
pub use protocol_types::{
    A2AClientInfo, A2AError, A2AInitializeRequest, A2AInitializeResponse, A2AMessage,
    A2AServerInfo, MessagePart, A2A_VERSION,
};

/// A2A protocol request (JSON-RPC 2.0 request)
pub type A2ARequest = JsonRpcRequest;
/// A2A protocol response (JSON-RPC 2.0 response)
pub type A2AResponse = JsonRpcResponse;
/// A2A protocol error response (JSON-RPC 2.0 error)
pub type A2AErrorResponse = JsonRpcError;

/// Helper function for mapping database errors to A2A errors
pub fn map_db_error(context: &str) -> impl Fn(errors::AppError) -> A2AError + '_ {
    move |e| A2AError::InternalError(format!("{context}: {e}"))
}

/// Helper function for mapping database errors to A2A errors with string context
pub fn map_db_error_str(context: String) -> impl Fn(errors::AppError) -> A2AError {
    move |e| A2AError::InternalError(format!("{context}: {e}"))
}
