// ABOUTME: Protocol-related constants module — re-exports compile-time bits from pierre-core
// ABOUTME: Local `constants` submodule exposes runtime-configurable wrappers (server name, version)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use pierre_core::constants::protocol::*;

/// Server-side runtime-configurable protocol values (server name, MCP version).
///
/// Compile-time protocol constants come from `pierre_core::constants::protocol`
/// via the `pub use` above; this submodule layers in the env-driven helpers
/// that depend on the local `ServerConfig` static.
pub mod constants;

// Flatten the runtime helpers so existing call sites
// (`crate::constants::protocol::mcp_protocol_version`) keep resolving.
pub use constants::*;
