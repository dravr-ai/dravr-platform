// ABOUTME: Tool I/O value types crate — ToolResult, ToolNotification, NotificationType + tool errors
// ABOUTME: Pure leaf crate consumed by tool implementations across the Pierre platform
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Tools Core
//!
//! Shared value types and error types for tool execution across the Pierre
//! platform. This crate exists so tool implementations can depend on the I/O
//! surface without pulling in the rest of `pierre-server`.
//!
//! ## What's here
//!
//! - [`ToolResult`] — main value returned by every tool execution
//! - [`ToolNotification`] / [`NotificationType`] — side-effect notifications
//!   (OAuth refresh, progress updates, cache invalidation, …)
//! - [`ToolError`] re-exported from `pierre-core::errors::tool`
//!
//! ## What's *not* here
//!
//! Tool *execution* infrastructure (`McpTool` trait, `ToolExecutionContext`,
//! `ToolRuntime`, the registry) still lives in `pierre-server::tools` because
//! it transitively depends on the full server composition root. Extracting
//! that surface is a separate, larger campaign.

#![deny(unsafe_code)]

pub mod result;

pub use result::{NotificationType, ToolNotification, ToolResult};

/// Tool-specific error types re-exported from `pierre-core`.
pub mod errors {
    pub use pierre_core::errors::tool::*;
}

pub use errors::ToolError;
