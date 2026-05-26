// ABOUTME: ServerContext → Arc<dyn ToolRuntime> upcast helper kept in pierre-server.
// ABOUTME: Lives next to ServerContext because pierre-tool-runtime cannot see ServerContext.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Convenience upcast `Arc<ServerContext>` → `Arc<dyn ToolRuntime>`.
//!
//! Callers that hold an [`Arc<ServerContext>`] but need to pass an
//! [`Arc<dyn ToolRuntime>`] (e.g. Axum handlers calling helpers migrated
//! to the trait surface) reach for this helper.
//!
//! The [`ToolRuntime`] trait lives in `pierre-tool-runtime` but its production
//! impl is on `ServerContext`, which stays in pierre-server. That makes this
//! the one place where the upcast needs to happen.

use std::sync::Arc;

use crate::mcp::resources::ServerContext;
use pierre_tool_runtime::runtime::ToolRuntime;

/// Upcast an `Arc<ServerContext>` to `Arc<dyn ToolRuntime>`.
#[must_use]
pub fn into_runtime(s: &Arc<ServerContext>) -> Arc<dyn ToolRuntime> {
    let cloned: Arc<ServerContext> = Arc::clone(s);
    cloned
}
