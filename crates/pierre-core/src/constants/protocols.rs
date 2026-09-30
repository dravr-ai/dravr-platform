// ABOUTME: Protocol configuration constants for various communication protocols
// ABOUTME: Defines timeouts, limits, and configuration for HTTP and other protocols
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Protocol configuration constants

/// HTTP protocol constants
pub mod http {
    /// Default HTTP timeout in seconds
    pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
}

/// MCP protocol constants
pub mod mcp {
    /// Default MCP timeout in seconds
    pub const DEFAULT_TIMEOUT_SECS: u64 = 60;
}
