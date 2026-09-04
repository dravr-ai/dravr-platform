// ABOUTME: Pins structured_output and visual_blocks onto the hot-reload prompt registry
// ABOUTME: A synced edit to either document must reach the next chat turn without a redeploy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#312.
//!
//! The contremaitre manifest declares 17 system prompts and every one of them
//! is meant to reach a running binary through the sync — webhook or poll — in
//! about a minute. `structured_output.md` governs the JSON plan contract and
//! `visual_blocks.md` the inline-chart contract; both are athlete-visible, and
//! both were read as compiled-in constants at the point the chat pipeline
//! context is built, so an edit to either needed a contremaitre rev bump and a
//! full platform redeploy while the manifest advertised them as syncable.
//!
//! The sync's only effect on a prompt is a `update_system_prompt` write into
//! the registry, so writing into it here is what a landing webhook does.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use common::create_test_server_resources;

const SYNCED_STRUCTURED_OUTPUT: &str =
    "## Structured output\n\nSynced from contremaitre: emit the plan as JSON only.";
const SYNCED_VISUAL_BLOCKS: &str =
    "## Inline visuals\n\nSynced from contremaitre: one chart, and say what it shows.";
const SYNC_SHA: &str = "1111111111111111111111111111111111111111111111111111111111111111";

#[tokio::test]
async fn test_synced_directives_reach_the_next_chat_turn() {
    let resources = create_test_server_resources()
        .await
        .expect("Failed to create test resources");

    let before = resources.chat_pipeline_context();
    assert!(
        !before.structured_output_prompt.is_empty(),
        "the structured-output contract must never reach a turn empty"
    );
    assert!(
        !before.visual_blocks_prompt.is_empty(),
        "the inline-visual contract must never reach a turn empty"
    );

    let registry = &resources.mcp.prompt_registry;
    registry.update_system_prompt(
        "structured_output",
        SYNCED_STRUCTURED_OUTPUT.to_owned(),
        SYNC_SHA.to_owned(),
    );
    registry.update_system_prompt(
        "visual_blocks",
        SYNCED_VISUAL_BLOCKS.to_owned(),
        SYNC_SHA.to_owned(),
    );

    let after = resources.chat_pipeline_context();

    assert_eq!(
        after.structured_output_prompt, SYNCED_STRUCTURED_OUTPUT,
        "a synced structured_output.md must replace the contract the turn carries"
    );
    assert_ne!(
        before.structured_output_prompt, after.structured_output_prompt,
        "the fixture must differ from the pre-sync text or the assertion above proves nothing"
    );

    assert!(
        after.visual_blocks_prompt.starts_with(SYNCED_VISUAL_BLOCKS),
        "a synced visual_blocks.md must replace the prose the turn carries, got: {}",
        after.visual_blocks_prompt
    );
    assert_ne!(
        before.visual_blocks_prompt, after.visual_blocks_prompt,
        "the fixture must differ from the pre-sync text"
    );

    // The bounds half is generated off the schema that validates the blocks,
    // not transcribed from the document, so it survives the prose swap — that
    // is the split the two halves exist for.
    let generated = after
        .visual_blocks_prompt
        .strip_prefix(SYNCED_VISUAL_BLOCKS)
        .expect("the synced prose is the prefix of the assembled directive");
    assert!(
        generated.contains("series[].points"),
        "the schema-generated bounds must still be appended after a prose swap, got: {generated}"
    );
}
