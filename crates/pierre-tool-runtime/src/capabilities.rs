// ABOUTME: Named capability sets shared by the tools that declare the same runtime requirements
// ABOUTME: Built on dravr-tronc's ToolCapabilities, the one flag set the registry and transports read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Capability sets several tools declare verbatim.
//!
//! Every tool declares [`ToolCapabilities`] from dravr-tronc directly: it is
//! the set the registry gates on and the transports read. Fitness domain
//! grouping (goals, recipes, sleep) is the registry's string category, given
//! at `register_with_category`, never a flag.

use dravr_tronc::mcp::tool::ToolCapabilities;

/// The capability set of a read-only tool that cannot answer without a
/// connected fitness provider.
///
/// Named because many tools across `analytics`, `data`, `goals` and `sleep`
/// declare exactly this triple, and the dispatch chokepoint refuses every one
/// of them for a providerless athlete. Spelling it once keeps that set from
/// drifting tool by tool — a tool that silently loses `REQUIRES_PROVIDER` stops
/// being gated and goes back to serving the empty shapes a model narrates as
/// fact.
pub const PROVIDER_READ: ToolCapabilities = ToolCapabilities::REQUIRES_AUTH
    .union(ToolCapabilities::READS_DATA)
    .union(ToolCapabilities::REQUIRES_PROVIDER);
