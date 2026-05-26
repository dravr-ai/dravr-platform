// ABOUTME: Server-Sent Events (SSE) implementation for real-time notifications and MCP protocol streaming
// ABOUTME: Provides unified SSE infrastructure for both OAuth notifications and MCP bidirectional communication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre SSE
//!
//! Server-Sent Events infrastructure for the Pierre platform. Originally
//! lived inside `pierre-server::sse`; extracted as a workspace crate so
//! the broadcast manager and HTTP route handlers are decoupled from the
//! composition root.
//!
//! ## Components
//!
//! - [`a2a_task_stream`]: A2A task progress streaming
//! - [`manager`]: Central SSE connection manager (broadcast hub)
//! - [`notifications`]: OAuth notification stream
//! - [`routes`]: HTTP route handlers for SSE endpoints
//!
//! The MCP protocol stream itself is held behind the [`manager::ProtocolStream`]
//! trait. The concrete implementation (`McpProtocolStream`) stays in
//! `pierre-server` because it depends on the `ToolHandlers` tool-call
//! dispatcher that has not yet been extracted; pierre-server constructs
//! streams via the [`manager::ProtocolStreamFactory`] handle injected into
//! the manager at startup.

/// A2A task streaming for progress updates
pub mod a2a_task_stream;

/// Central SSE manager for connection lifecycle and message routing
pub mod manager;

/// OAuth notification streaming for user-specific events
pub mod notifications;

/// HTTP route handlers for SSE endpoints
pub mod routes;

// Re-exports
pub use a2a_task_stream::A2ATaskStream;

pub use manager::{
    ConnectionMetadata, ConnectionType, ProtocolStream, ProtocolStreamFactory, SseManager,
};

pub use notifications::NotificationStream;

pub use routes::SseRoutes;
