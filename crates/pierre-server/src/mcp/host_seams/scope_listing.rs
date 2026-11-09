// ABOUTME: Narrows a tools/list answer to the tools the caller's grant covers
// ABOUTME: Reads the same capability-to-scope derivation the call path refuses with, so list and call cannot disagree
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_schema::ToolSchema;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::scopes::missing_scope;

/// Drop from `schemas` every tool whose scope `granted` (the scope strings a
/// credential carried) does not cover.
///
/// A client shown a tool it holds no scope for would offer it, call it and be
/// refused: the listing is the first place the grant is honoured. A token
/// holding only `fitness:read` lists no write tool; a tool that requires no
/// scope stays listed. A name the registry does not resolve is kept, as the
/// call path answers it.
pub(super) fn retain_granted(
    registry: &ToolRegistry,
    schemas: &mut Vec<ToolSchema>,
    granted: &[String],
) {
    let granted = OAuthScope::parse_granted(&granted.join(" "));
    schemas.retain(|schema| {
        registry
            .get(&schema.name)
            .is_none_or(|tool| missing_scope(&granted, tool.capabilities()).is_none())
    });
}
