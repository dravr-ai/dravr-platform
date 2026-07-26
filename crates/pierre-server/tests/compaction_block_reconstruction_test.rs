// ABOUTME: Read-side reconstruction tests for build_llm_messages_with_blocks splicing
// ABOUTME: Covers single splice, overlap precedence, straddling/out-of-window and foreign-conversation skips
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use chrono::{TimeZone, Utc};
use pierre_chat_pipeline::stages::prompt_builder::build_llm_messages_with_blocks;
use pierre_database::database::MessageRecord;
use pierre_llm::MessageRole;
use pierre_memory::CompactionBlock;
use pierre_services::conversation_compaction::REPLAYED_SUMMARY_PREFIX;

/// Build a `MessageRecord` with an explicit id and role.
fn record(id: &str, role: &str, content: &str) -> MessageRecord {
    MessageRecord {
        id: id.to_owned(),
        conversation_id: "conv-1".to_owned(),
        role: role.to_owned(),
        content: content.to_owned(),
        token_count: None,
        prompt_tokens: None,
        model: None,
        finish_reason: None,
        structured_content: None,
        created_at: "2026-05-15T15:10:00Z".to_owned(),
    }
}

/// Build a `CompactionBlock` anchored on two history-row ids, ordered by the
/// `seconds` offset (smaller = created earlier, so it appears first in the ASC
/// vector the repository returns).
fn block(id: &str, first_id: &str, last_id: &str, summary: &str, seconds: u32) -> CompactionBlock {
    CompactionBlock {
        id: id.to_owned(),
        tenant_id: "t1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        summary: summary.to_owned(),
        summary_tokens: 10,
        original_tokens: 100,
        first_message_id: first_id.to_owned(),
        last_message_id: last_id.to_owned(),
        created_at: Utc.with_ymd_and_hms(2026, 5, 15, 12, 0, seconds).unwrap(),
    }
}

/// A six-row history: m1..m6 alternating user/assistant.
fn six_row_history() -> Vec<MessageRecord> {
    vec![
        record("m1", "user", "q one"),
        record("m2", "assistant", "a one"),
        record("m3", "user", "q two"),
        record("m4", "assistant", "a two"),
        record("m5", "user", "q three"),
        record("m6", "assistant", "a three"),
    ]
}

#[test]
fn single_block_mid_history_splices_one_summary_preserving_order() {
    let history = six_row_history();
    // Block covers [m2..m4] (indices 1..=3).
    let blocks = vec![block("b1", "m2", "m4", "middle summary", 0)];

    let (messages, source_ids) = build_llm_messages_with_blocks(Some("sys"), &history, &blocks);

    // system + m1 + summary + m5 + m6 = 5.
    assert_eq!(messages.len(), 5);
    assert_eq!(source_ids.len(), messages.len());

    assert_eq!(messages[0].role, MessageRole::System);
    assert_eq!(source_ids[0], None);

    assert_eq!(messages[1].content, "q one");
    assert_eq!(source_ids[1].as_deref(), Some("m1"));

    // The single spliced summary. It rides as User text under the shared
    // framing prefix: the live provider keeps only the first system message and
    // drops the rest, so a System summary never reached the model.
    assert_eq!(messages[2].role, MessageRole::User);
    assert!(messages[2].content.ends_with("middle summary"));
    assert!(messages[2].content.starts_with(REPLAYED_SUMMARY_PREFIX));
    assert_eq!(source_ids[2], None);

    // Order preserved: m5 then m6 after the summary, source_ids aligned.
    assert_eq!(messages[3].content, "q three");
    assert_eq!(source_ids[3].as_deref(), Some("m5"));
    assert_eq!(messages[4].content, "a three");
    assert_eq!(source_ids[4].as_deref(), Some("m6"));

    // Covered rows absent.
    for covered in ["a one", "q two", "a two"] {
        assert!(messages.iter().all(|m| m.content != covered));
    }
}

#[test]
fn overlapping_blocks_apply_only_the_earlier_created() {
    let history = six_row_history();
    // b_early covers [m2..m4]; b_late covers [m3..m5] — ranges intersect at
    // m3/m4. b_early is created first (seconds=0 < seconds=5) so it wins; the
    // later overlapping block is skipped, no double summary.
    let blocks = vec![
        block("b_early", "m2", "m4", "earlier summary", 0),
        block("b_late", "m3", "m5", "later summary", 5),
    ];

    let (messages, source_ids) = build_llm_messages_with_blocks(Some("sys"), &history, &blocks);

    // Exactly one summary message, carrying the earlier block's text.
    let summaries: Vec<&str> = messages
        .iter()
        .filter(|m| m.content.starts_with(REPLAYED_SUMMARY_PREFIX))
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(summaries.len(), 1, "no double summary on overlap");
    assert!(summaries[0].ends_with("earlier summary"));
    // ends_with, not `!=`: every summary now carries the framing prefix, so an
    // equality check against the bare text would pass even if the overlapping
    // block WERE spliced, silently retiring the overlap-precedence guarantee.
    assert!(
        messages
            .iter()
            .all(|m| !m.content.ends_with("later summary")),
        "later overlapping block must be skipped"
    );

    // system + m1 + earlier-summary([m2..m4]) + m5 + m6 = 5.
    assert_eq!(messages.len(), 5);
    assert_eq!(source_ids.len(), messages.len());
    assert_eq!(messages[4].content, "a three");
    assert_eq!(source_ids[4].as_deref(), Some("m6"));
}

#[test]
fn block_with_boundary_id_outside_window_is_skipped() {
    let history = six_row_history();
    // first_message_id "m0" is not in the loaded window (the cap dropped it) —
    // the block straddles outside, so it is skipped entirely and m1..m6 render
    // raw. Also covers a last_id absence via the symmetric case below.
    let blocks = vec![
        block("b_straddle", "m0", "m3", "straddling summary", 0),
        block("b_tail_missing", "m4", "m99", "tail-missing summary", 1),
    ];

    let (messages, source_ids) = build_llm_messages_with_blocks(Some("sys"), &history, &blocks);

    // No summary spliced: system + 6 raw rows = 7.
    assert_eq!(messages.len(), 7);
    assert_eq!(source_ids.len(), messages.len());
    assert!(
        !messages.iter().any(|m| m.content.contains("summary")),
        "blocks with out-of-window boundary ids must be skipped"
    );
    // Every history row renders raw, in order.
    for (i, row) in history.iter().enumerate() {
        assert_eq!(source_ids[i + 1].as_deref(), Some(row.id.as_str()));
        assert_eq!(messages[i + 1].content, row.content);
    }
}

#[test]
fn block_for_a_different_conversation_is_skipped() {
    let history = six_row_history();
    // A block whose ids belong to a DIFFERENT conversation: neither id resolves
    // in this history, so it is skipped and all rows render raw.
    let blocks = vec![block(
        "b_other",
        "other-conv-msg-1",
        "other-conv-msg-9",
        "foreign summary",
        0,
    )];

    let (messages, source_ids) = build_llm_messages_with_blocks(Some("sys"), &history, &blocks);

    assert_eq!(messages.len(), 7, "system + 6 raw rows, no splice");
    assert_eq!(source_ids.len(), messages.len());
    assert!(messages.iter().all(|m| m.content != "foreign summary"));
    assert_eq!(messages[1].content, "q one");
    assert_eq!(messages[6].content, "a three");
}
