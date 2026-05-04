// ABOUTME: Focused dependency-injection contexts extracted from the canonical ServerContext
// ABOUTME: Lets narrow services depend only on the slice they need
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Focused dependency-injection contexts.
//!
//! `ServerContext` (in `crate::mcp::resources`) is the single canonical
//! container that lives in the server's `State` and is passed to every
//! handler. The contexts in this module are *narrow extracts* materialized
//! on demand by `ServerContext::auth()`, `ServerContext::data()`, etc.
//!
//! Wide handlers take `Arc<ServerContext>` and call methods on it.
//! Narrow services (e.g. `AuthService`, `OAuthFlowService`) take only the
//! subset of contexts they touch, so their signatures express real
//! coupling rather than borrowing from the whole container.

/// Authentication context with auth manager, middleware, and Firebase auth
pub mod auth;
/// Configuration context with OAuth, tenant settings, and admin config management
pub mod config;
/// Data context with database, cache, and provider access
pub mod data;
/// Extension context for plugins, sampling peer, and progress notifications
pub mod extension;
/// Notification context for WebSocket and SSE
pub mod notification;
/// Security context for CSRF, redaction, and rate limiting
pub mod security;
/// Focused-context extractors layered on `ServerContext` (the canonical container)
mod server;

/// Re-export the canonical `ServerContext` so consumers can `use crate::context::ServerContext`.
pub use crate::mcp::resources::ServerContext;
/// Authentication context
pub use auth::AuthContext;
/// Configuration context
pub use config::ConfigContext;
/// Data access context
pub use data::DataContext;
/// Extension context
pub use extension::ExtensionContext;
/// Notification context
pub use notification::NotificationContext;
/// Security context
pub use security::SecurityContext;
