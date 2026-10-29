// ABOUTME: Byte budget on the chat tool catalogue, the largest share of a cold prompt (carnet#1196)
// ABOUTME: Fails over budget, and well under it, so a saving lowers the budget in the same commit
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every chat turn sends the whole chat-callable catalogue (registre#406). It
//! is re-sent in every call's prefix and written to cache once per model on
//! every cold turn, and on 2026-10-26 it was an estimated two thirds of a
//! 62,199-token cold prompt. This budget keeps it from growing unnoticed.
//!
//! The unit is bytes of the compact Anthropic wire shape: CI computes it
//! without a provider key, and it does not move when a tokenizer does. The
//! boot log (`tool_catalogue_size`) and every turn's dispatch line report the
//! same measurement from production. This test sees the compiled-in
//! descriptions only; a contremaitre overlay can make the wire larger or
//! smaller than what is counted here.
//!
//! The budget is a ratchet. Going over it fails, and so does falling more than
//! [`RATCHET_SLACK_BYTES`] under it: a change that saves bytes lowers the
//! budget in the same commit, so the saving cannot be spent again unseen.

use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::chat_catalogue::{build_mcp_tools, measure_catalogue, CatalogueSize};
use pierre_tool_runtime::registry::ToolRegistry;

use crate::common;

/// Bytes the chat catalogue may take on the wire. It measured 112,728 bytes
/// (88 tools, ~44k estimated tokens) on 2026-10-27; carnet#1196 rewrote every
/// long description to its purpose, routing and rules, against a target of a
/// fifth under that, about 90,000. The test prints the current size.
///
/// 2026-10-28: 94,750 → 95,100 for `get_group_member_plan_status` (carnet#1244),
/// the coach seat's one plan read, kept to a single argument and a two-line
/// description (~330 bytes).
const CATALOGUE_BUDGET_BYTES: usize = 95_100;

/// How far under budget the catalogue may fall before the budget follows it down.
const RATCHET_SLACK_BYTES: usize = 4 * 1024;

/// How many of the largest tools a report lists
const REPORTED_TOOLS: usize = 20;

fn builtin_registry() -> ToolRegistry {
    common::init_server_config();
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    registry
}

fn report(size: &CatalogueSize) -> String {
    let mut lines = vec![format!(
        "chat catalogue: {} tools, {} bytes, ~{} tokens, digest {}",
        size.tool_count, size.bytes, size.estimated_tokens, size.digest
    )];
    lines.extend(size.per_tool.iter().take(REPORTED_TOOLS).map(|tool| {
        format!(
            "  {:<36} {:>6} bytes  ~{:>5} tokens",
            tool.name, tool.bytes, tool.estimated_tokens
        )
    }));
    lines.join("\n")
}

#[test]
fn chat_catalogue_stays_within_its_byte_budget() {
    let size = measure_catalogue(&build_mcp_tools(&builtin_registry()));
    let report = report(&size);
    println!("{report}");

    assert!(
        size.bytes <= CATALOGUE_BUDGET_BYTES,
        "The chat tool catalogue is {} bytes, over its {CATALOGUE_BUDGET_BYTES}-byte budget. \
         Every turn sends it and every cold turn writes it to cache once per model \
         (carnet#1196): tighten a description before raising the budget.\n{report}",
        size.bytes
    );
    assert!(
        size.bytes + RATCHET_SLACK_BYTES >= CATALOGUE_BUDGET_BYTES,
        "The chat tool catalogue is {} bytes, more than {RATCHET_SLACK_BYTES} under its \
         {CATALOGUE_BUDGET_BYTES}-byte budget. Lower CATALOGUE_BUDGET_BYTES to {} in this \
         commit so the saving stays saved.\n{report}",
        size.bytes,
        size.bytes
    );
}
