// ABOUTME: A2A (Agent-to-Agent) protocol — types, agent card, server, client manager, auth
// ABOUTME: Decoupled from pierre-server via the A2ACtx trait in pierre-runtime-context
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre A2A Protocol
//!
//! Standalone implementation of the A2A (Agent-to-Agent) protocol for the
//! Pierre platform. Exposes the message types, agent card, JSON-RPC server,
//! client-management facade, system-user provisioning, and authenticator
//! used by `pierre-server`'s `/api/a2a/*` routes.
//!
//! Server-side modules (`auth`, `client`, `protocol`, `system_user`) accept
//! the narrow `A2ACtx` trait from `pierre-runtime-context` instead of the
//! full `ServerContext` — that's how this crate stays leaf-shaped while still
//! reaching the auth manager, JWKS, repositories, and configured base URL.
//!
//! ## Modules
//!
//! * [`agent_card`] — Agent capability discovery and advertisement
//! * [`auth`] — A2A authenticator (API key + `OAuth2`)
//! * [`client`] — A2A client manager (registration, sessions, usage, rate limits)
//! * [`client_types`] — Client registration, credentials, usage types
//! * [`jsonrpc`] — Minimal JSON-RPC 2.0 types for A2A type aliases
//! * [`protocol`] — A2A JSON-RPC server (handles `a2a/*` methods)
//! * [`protocol_types`] — Protocol message types and error enum
//! * [`system_user`] — System-user provisioning for A2A clients

#![deny(unsafe_code)]

// Re-export pierre-core modules for `crate::errors`, `crate::constants`, etc.
pub use pierre_core::constants;
pub use pierre_core::errors;
pub use pierre_core::models;

/// Agent card metadata and capabilities
pub mod agent_card;
/// A2A authentication and authorization
pub mod auth;
/// A2A client management
pub mod client;
/// Client registration and credential types
pub mod client_types;
/// Minimal JSON-RPC 2.0 types for A2A protocol
pub mod jsonrpc;
/// A2A protocol JSON-RPC server implementation
pub mod protocol;
/// A2A protocol message types and error definitions
pub mod protocol_types;
/// System user management for A2A agents
pub mod system_user;

pub use agent_card::AgentCard;
pub use client::A2AClientManager;
pub use client_types::{
    A2AClientTier, A2ARateLimitStatus, A2AToken, A2AUsageParams, ClientCredentials,
    ClientRegistrationRequest, ClientUsageStats,
};
pub use jsonrpc::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, JSONRPC_VERSION};
pub use protocol::A2AServer;
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
