// ABOUTME: MCP transport primitives — progress tracking, sampling peer, OAuth callback HTML rendering
// ABOUTME: Leaf crate consumed by pierre-server; no ServerContext, no runtime config globals
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre MCP Transport
//!
//! Transport-layer primitives shared by every MCP runtime path in `pierre-server`:
//!
//! - [`progress`] — `ProgressTracker` for emitting `notifications/progress` updates
//!   during long-running tool invocations.
//! - [`sampling_peer`] — `SamplingPeer` for server-initiated `sampling/createMessage`
//!   requests to MCP clients, with request-id correlation and response routing.
//! - [`oauth_flow_manager`] — `OAuthTemplateRenderer` for OAuth callback success/error
//!   HTML pages served by the `/oauth/callback` endpoints.
//! - [`OAuthCallbackResponse`] — the shared payload type returned by the OAuth
//!   completion flow and rendered into the success template.
//! - [`tenant_isolation`] — JWT validation + tenant-context extraction shared by
//!   the MCP dispatch path, the A2A dispatcher, and the SSE handshake. Consumes the
//!   `McpDispatchCtx` trait from `pierre-runtime-context` so it never touches
//!   pierre-server's composition root directly.
//!
//! These primitives are config-free: timeouts and other tunables are injected by the
//! caller (pierre-server), keeping this crate a leaf in the workspace graph.

#![warn(missing_docs)]

/// OAuth callback HTML rendering (success + error pages served at `/oauth/callback`).
pub mod oauth_flow_manager;
/// Progress notification tracker for long-running MCP tool invocations.
pub mod progress;
/// Bidirectional sampling peer for server-initiated `sampling/createMessage` requests.
pub mod sampling_peer;
/// Tenant isolation + JWT validation helpers (consumes `McpDispatchCtx`).
pub mod tenant_isolation;
/// Shared payload types returned by the OAuth completion flow.
pub mod types;

pub use types::OAuthCallbackResponse;
