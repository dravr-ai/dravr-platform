// ABOUTME: ToolCapabilities bitflags — declares the runtime requirements every MCP tool advertises
// ABOUTME: Used by ToolRegistry for role-based filtering, provider gating, and cache invalidation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Bitflag set describing what an [`McpTool`](trait) needs from the runtime
//! and which surface area it operates on.
//!
//! [`McpTool`]: <https://docs.rs/pierre-mcp-server> — see `pierre_mcp_server::tools::traits::McpTool`

use bitflags::bitflags;

bitflags! {
    /// Capabilities that tools can declare for filtering and discovery.
    ///
    /// These flags enable:
    /// - Role-based access control (admin vs user tools)
    /// - Provider dependency checking (tools that need connected providers)
    /// - Feature categorization for plan-based filtering
    /// - Caching decisions based on read/write behavior
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ToolCapabilities: u16 {
        /// Tool requires an authenticated user
        const REQUIRES_AUTH = 0b0000_0000_0001;
        /// Tool requires tenant context
        const REQUIRES_TENANT = 0b0000_0000_0010;
        /// Tool requires a connected fitness provider
        const REQUIRES_PROVIDER = 0b0000_0000_0100;
        /// Tool reads data (activities, stats, etc.)
        const READS_DATA = 0b0000_0000_1000;
        /// Tool writes/modifies data
        const WRITES_DATA = 0b0000_0001_0000;
        /// Tool performs analytics/calculations
        const ANALYTICS = 0b0000_0010_0000;
        /// Tool manages goals
        const GOALS = 0b0000_0100_0000;
        /// Tool manages configuration
        const CONFIGURATION = 0b0000_1000_0000;
        /// Tool manages recipes
        const RECIPES = 0b0001_0000_0000;
        /// Tool manages coaches
        const COACHES = 0b0010_0000_0000;
        /// Tool requires admin privileges
        const ADMIN_ONLY = 0b0100_0000_0000;
        /// Tool handles sleep/recovery data
        const SLEEP_RECOVERY = 0b1000_0000_0000;
        /// Tool reads or writes who the athlete IS — profile fields,
        /// configuration, which providers are linked — rather than the fitness
        /// data they have accumulated.
        ///
        /// Sits beside `READS_DATA`/`WRITES_DATA` rather than replacing them: a
        /// profile tool still reads or writes, and this says *what*. Together
        /// the pair picks the OAuth scope the caller must hold, which is the
        /// split a consent screen has to be able to state — an integration may
        /// legitimately need an athlete's training history without needing
        /// their identity, or the reverse. Folded together, a grant for one is
        /// a grant for both.
        const PROFILE = 0b0001_0000_0000_0000;
    }
}

/// The capability set of a read-only tool that cannot answer without a
/// connected fitness provider.
///
/// Named because eighteen tools across `data`, `goals` and `sleep` declare
/// exactly this triple, and the dispatch chokepoint refuses every one of them
/// for a providerless athlete. Spelling it once keeps that set from drifting
/// tool by tool — a tool that silently loses `REQUIRES_PROVIDER` stops being
/// gated and goes back to serving the empty shapes a model narrates as fact.
pub const PROVIDER_READ: ToolCapabilities = ToolCapabilities::REQUIRES_AUTH
    .union(ToolCapabilities::READS_DATA)
    .union(ToolCapabilities::REQUIRES_PROVIDER);

/// [`PROVIDER_READ`] plus the analytics marker, for the computed-insight tools.
pub const PROVIDER_ANALYTICS: ToolCapabilities = PROVIDER_READ.union(ToolCapabilities::ANALYTICS);

impl ToolCapabilities {
    /// Check if tool requires any form of authentication
    #[must_use]
    pub const fn requires_auth(self) -> bool {
        self.contains(Self::REQUIRES_AUTH)
    }

    /// Check if tool requires tenant context
    #[must_use]
    pub const fn requires_tenant(self) -> bool {
        self.contains(Self::REQUIRES_TENANT)
    }

    /// Check if tool requires a connected provider
    #[must_use]
    pub const fn requires_provider(self) -> bool {
        self.contains(Self::REQUIRES_PROVIDER)
    }

    /// Check if tool is admin-only
    #[must_use]
    pub const fn is_admin_only(self) -> bool {
        self.contains(Self::ADMIN_ONLY)
    }

    /// Check if tool reads data (useful for caching decisions)
    #[must_use]
    pub const fn reads_data(self) -> bool {
        self.contains(Self::READS_DATA)
    }

    /// Check if tool writes data (useful for cache invalidation)
    #[must_use]
    pub const fn writes_data(self) -> bool {
        self.contains(Self::WRITES_DATA)
    }

    /// Check if tool performs analytics
    #[must_use]
    pub const fn is_analytics(self) -> bool {
        self.contains(Self::ANALYTICS)
    }

    /// Get a description of all enabled capabilities for logging
    #[must_use]
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();

        if self.contains(Self::REQUIRES_AUTH) {
            parts.push("requires_auth");
        }
        if self.contains(Self::REQUIRES_TENANT) {
            parts.push("requires_tenant");
        }
        if self.contains(Self::REQUIRES_PROVIDER) {
            parts.push("requires_provider");
        }
        if self.contains(Self::READS_DATA) {
            parts.push("reads_data");
        }
        if self.contains(Self::WRITES_DATA) {
            parts.push("writes_data");
        }
        if self.contains(Self::ANALYTICS) {
            parts.push("analytics");
        }
        if self.contains(Self::GOALS) {
            parts.push("goals");
        }
        if self.contains(Self::CONFIGURATION) {
            parts.push("configuration");
        }
        if self.contains(Self::RECIPES) {
            parts.push("recipes");
        }
        if self.contains(Self::COACHES) {
            parts.push("coaches");
        }
        if self.contains(Self::ADMIN_ONLY) {
            parts.push("admin_only");
        }
        if self.contains(Self::SLEEP_RECOVERY) {
            parts.push("sleep_recovery");
        }

        if parts.is_empty() {
            "none".to_owned()
        } else {
            parts.join(", ")
        }
    }
}
