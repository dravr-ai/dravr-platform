// ABOUTME: Compile-time guard that every ToolId variant has a UniversalExecutor handler
// ABOUTME: Would have caught the discover_routes regression where the enum was declared but never wired

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(missing_docs)]

use pierre_mcp_server::protocols::universal::tool_registry::ToolId;
use pierre_mcp_server::protocols::universal::{ToolRegistry, UniversalExecutor};
use strum::IntoEnumIterator;

#[test]
fn every_tool_id_variant_has_a_registered_handler() {
    let mut registry = ToolRegistry::new();
    UniversalExecutor::register_all_tools(&mut registry);

    let missing: Vec<ToolId> = ToolId::iter()
        .filter(|id| !registry.has_tool(*id))
        .collect();

    assert!(
        missing.is_empty(),
        "UniversalExecutor::register_all_tools() is missing handlers for {} ToolId \
         variant(s):\n  {:?}\n\n\
         These tools will resolve by name and appear in the catalog sent to the LLM, \
         but calling them returns ProtocolError::InternalError('Tool X not registered') \
         in ~15 ms with no useful logs — exactly the class of bug that left discover_routes \
         silently broken from 2026-04-17 until 2026-04-18.\n\n\
         Fix: wire each missing variant into the matching register_*_tools() helper in \
         crates/pierre-server/src/protocols/universal/executor.rs, then add the \
         re-export to protocols/universal/handlers/mod.rs.",
        missing.len(),
        missing,
    );
}

#[test]
fn tool_id_iter_covers_every_name_map_entry() {
    // Catches the inverse regression: a ToolId variant declared in the enum
    // (so EnumIter picks it up) but never wired into from_name. Such a variant
    // is unreachable from the LLM — the name resolver never returns it — and
    // the handler table silently never fires.
    let orphan: Vec<ToolId> = ToolId::iter()
        .filter(|id| ToolId::from_name(id.name()) != Some(*id))
        .collect();

    assert!(
        orphan.is_empty(),
        "ToolId::from_name() is missing entries for {} variant(s): {:?}\n\
         These tools have an enum entry + a `name()` mapping but no reverse lookup, \
         so `resolve_tool_name(name)` returns None and the LLM can never invoke them. \
         Fix: add `\"<name>\" => Some(Self::<Variant>)` to ToolId::from_name().",
        orphan.len(),
        orphan,
    );
}
