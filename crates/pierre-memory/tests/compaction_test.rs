// ABOUTME: Unit tests for CompactionBlock::tokens_saved
// ABOUTME: Pins that a summary longer than its source never reports negative savings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use chrono::Utc;
use pierre_memory::compaction::CompactionBlock;

fn block(original: i32, summary: i32) -> CompactionBlock {
    CompactionBlock {
        id: "b1".into(),
        tenant_id: "t1".into(),
        conversation_id: "c1".into(),
        summary: "earlier turns summarized".into(),
        summary_tokens: summary,
        original_tokens: original,
        first_message_id: "m1".into(),
        last_message_id: "m5".into(),
        created_at: Utc::now(),
    }
}

#[test]
fn tokens_saved_positive() {
    assert_eq!(block(500, 120).tokens_saved(), 380);
}

#[test]
fn tokens_saved_clamped_at_zero() {
    assert_eq!(block(100, 200).tokens_saved(), 0);
}
