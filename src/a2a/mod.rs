// ABOUTME: A2A (Agent-to-Agent) protocol module exports and organization
// ABOUTME: Re-exports protocol types from pierre-a2a; keeps server implementations here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A2A (Agent-to-Agent) Protocol Implementation
//!
//! This module implements the A2A protocol for Pierre, enabling agent-to-agent
//! communication and collaboration with other AI systems.
//!
//! Protocol types (errors, messages, agent card) are defined in the `pierre-a2a`
//! crate. Server implementations that depend on infrastructure (database, auth,
//! tools) remain here.

// Re-export protocol types from pierre-a2a crate
pub use pierre_a2a::agent_card;
pub use pierre_a2a::client_types;
pub use pierre_a2a::protocol_types;
pub use pierre_a2a::{
    map_db_error, map_db_error_str, A2AClientInfo, A2AError, A2AInitializeRequest,
    A2AInitializeResponse, A2AMessage, A2AServerInfo, AgentCard, MessagePart, A2A_VERSION,
};

// Type aliases use the main crate's JsonRpc types for HTTP layer compatibility
use crate::jsonrpc::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};

/// A2A protocol request (JSON-RPC 2.0 request from main crate)
pub type A2ARequest = JsonRpcRequest;
/// A2A protocol response (JSON-RPC 2.0 response from main crate)
pub type A2AResponse = JsonRpcResponse;
/// A2A protocol error response (JSON-RPC 2.0 error from main crate)
pub type A2AErrorResponse = JsonRpcError;

/// A2A authentication and authorization
pub mod auth;
/// A2A client management
pub mod client;
/// A2A protocol types and server implementation
pub mod protocol;
/// System user management for A2A agents
pub mod system_user;

pub use client::A2AClientManager;
pub use protocol::A2AServer;
