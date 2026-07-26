// ABOUTME: Unit tests for build_llm_messages replay of tool_call and tool_result rows
// ABOUTME: Confirms persisted tool rounds rebuild the same Vec<ChatMessage> shape the loop pushed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use chrono::{TimeZone, Utc};
use pierre_chat_pipeline::stages::prompt_builder::{
    build_llm_messages, build_llm_messages_with_blocks,
};
use pierre_database::database::MessageRecord;
use pierre_llm::MessageRole;
use pierre_memory::CompactionBlock;
use pierre_services::conversation_compaction::{COMPACTION_MARKER, REPLAYED_SUMMARY_PREFIX};

/// Build a deterministic `MessageRecord` for a given role + content. The
/// non-content fields don't influence `build_llm_messages` so they get
/// stable test values; the assertions look only at role + content.
fn record(role: &str, content: &str) -> MessageRecord {
    MessageRecord {
        id: format!("msg-{role}-{}", content.len()),
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

#[test]
fn tool_call_and_tool_result_roles_replay_in_order() {
    let history = vec![
        record("user", "Give me my last 7 activities"),
        record("tool_call", "Pulling your last 7 activities."),
        record(
            "tool_result",
            "[Tool Result for get_activities]: {\"activity_list\":\"3 trails, 3 MTB, 1 hike\"}",
        ),
        record("assistant", "Here is your week summary…"),
        record("user", "And now my last 7"),
    ];

    let (messages, source_ids) = build_llm_messages(Some("system instructions"), &history);

    assert_eq!(messages.len(), 6, "system + 5 history rows");
    assert_eq!(messages[0].role, MessageRole::System);
    assert_eq!(messages[1].role, MessageRole::User);
    assert_eq!(messages[1].content, "Give me my last 7 activities");

    // tool_call replays as assistant text so the model treats it as its
    // own prior turn — same shape `run_api_tool_loop` pushes mid-loop.
    assert_eq!(messages[2].role, MessageRole::Assistant);
    assert_eq!(messages[2].content, "Pulling your last 7 activities.");

    // tool_result replays as a user message so the formatted tool output
    // ("[Tool Result for X]: …") lands where the in-memory loop puts it.
    assert_eq!(messages[3].role, MessageRole::User);
    assert!(messages[3].content.contains("activity_list"));

    assert_eq!(messages[4].role, MessageRole::Assistant);
    assert_eq!(messages[5].role, MessageRole::User);
    assert_eq!(messages[5].content, "And now my last 7");

    // `source_ids` is index-aligned with `messages`: same length, `None` for
    // the system prompt at index 0, and each non-system message carries its
    // originating history-row id. No rows were stripped here so all 5 history
    // ids survive.
    assert_eq!(source_ids.len(), messages.len());
    assert_eq!(source_ids[0], None, "system prompt has no source id");
    for (i, h) in history.iter().enumerate() {
        assert_eq!(
            source_ids[i + 1].as_deref(),
            Some(h.id.as_str()),
            "message {} maps to history row {}",
            i + 1,
            i
        );
    }
}

#[test]
fn tool_round_without_assistant_preamble_replays_correctly() {
    // Production case: the model emits a tool call with no preamble text.
    // Only the tool_result row is persisted (chat pipeline skips empty
    // assistant_text via the same is_empty guard the in-memory loop uses).
    let history = vec![
        record("user", "What did I run yesterday?"),
        record(
            "tool_result",
            "[Tool Result for get_activities]: {\"activity_list\":\"Trail run, 8km\"}",
        ),
        record("assistant", "Yesterday you ran an 8 km trail."),
    ];

    let (messages, source_ids) = build_llm_messages(None, &history);

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[1].role, MessageRole::User);
    assert!(messages[1].content.contains("Trail run"));
    assert_eq!(messages[2].role, MessageRole::Assistant);

    // No system prompt was passed, so every entry maps to a history row in
    // order and the first `source_id` is `Some`, not `None`.
    assert_eq!(source_ids.len(), messages.len());
    for (i, h) in history.iter().enumerate() {
        assert_eq!(source_ids[i].as_deref(), Some(h.id.as_str()));
    }
}

#[test]
fn tool_result_scaffolding_and_parroted_echo_are_stripped_from_replay() {
    // Real persisted `tool_result` rows hold the `<tool_result>` XML that
    // `format_tool_results_as_text` emits (preamble + blocks). A prior parroted
    // assistant turn holds the same blocks. Replaying either verbatim teaches the
    // model to imitate the format and emit a tool-result echo instead of an
    // answer — a long thread then degrades to empty/parroted replies. The replay
    // must strip the scaffolding (which reduces to empty) and skip it, while a
    // real synthesized answer survives untouched.
    let scaffold = "Here are the results from the tools you requested:\n\n\
                    <tool_result name=\"get_activities\">\n\
                    {\"activity_list\":\"3 trails\"}\n\
                    </tool_result>";
    let history = vec![
        record("user", "Show me my 2022 races"),
        record("tool_result", scaffold),
        record("assistant", "You raced 7 times in 2022."),
        // A prior parroted assistant echo — pure scaffolding, must also drop.
        record("assistant", scaffold),
        record("user", "And 2023?"),
    ];

    let (messages, source_ids) = build_llm_messages(Some("system instructions"), &history);

    // system + the two user turns + the one real assistant answer. The
    // scaffolding `tool_result` row and the parroted assistant echo both strip
    // to empty and are skipped.
    assert_eq!(messages.len(), 4, "scaffolding + parroted echo dropped");
    assert!(
        messages.iter().all(|m| !m.content.contains("<tool_result")),
        "no <tool_result> scaffolding may survive into the prompt"
    );
    assert_eq!(messages[0].role, MessageRole::System);
    assert_eq!(messages[1].content, "Show me my 2022 races");
    assert_eq!(messages[2].content, "You raced 7 times in 2022.");
    assert_eq!(messages[3].content, "And 2023?");

    // Dropped rows take NO `source_ids` slot — the two stripped rows produce no
    // entry, so `source_ids` is index-aligned with `messages` (length 4) yet
    // shorter than `history + 1` (would be 6 with no drops). This is exactly
    // the misalignment id-anchoring fixes: a positional remap would be off by
    // the two dropped rows.
    assert_eq!(source_ids.len(), messages.len());
    assert!(source_ids.len() < history.len() + 1);
    assert_eq!(source_ids[0], None, "system prompt has no source id");
    // The surviving messages map to the three non-stripped history rows in
    // order: the first user turn, the real assistant answer, the last user turn.
    assert_eq!(source_ids[1].as_deref(), Some(history[0].id.as_str()));
    assert_eq!(source_ids[2].as_deref(), Some(history[2].id.as_str()));
    assert_eq!(source_ids[3].as_deref(), Some(history[4].id.as_str()));
}

#[test]
fn unknown_roles_are_dropped_defensively() {
    let history = vec![
        record("user", "hi"),
        record("noise_role", "should be skipped"),
        record("assistant", "hello"),
    ];

    let (messages, source_ids) = build_llm_messages(None, &history);

    assert_eq!(messages.len(), 2, "unknown role dropped");
    assert_eq!(messages[0].content, "hi");
    assert_eq!(messages[1].content, "hello");

    // The unknown-role row contributes no message and no `source_ids` entry, so
    // the two remain index-aligned and map to the two known-role history rows.
    assert_eq!(source_ids.len(), messages.len());
    assert_eq!(source_ids[0].as_deref(), Some(history[0].id.as_str()));
    assert_eq!(source_ids[1].as_deref(), Some(history[2].id.as_str()));
}

/// Build a `MessageRecord` with an explicit id so block boundary ids are
/// unambiguous (the `record` helper derives id from role+len, which collides
/// across same-shape rows).
fn record_id(id: &str, role: &str, content: &str) -> MessageRecord {
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

/// Build a `CompactionBlock` anchored on two history-row ids.
fn block(id: &str, first_id: &str, last_id: &str, summary: &str) -> CompactionBlock {
    CompactionBlock {
        id: id.to_owned(),
        tenant_id: "t1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        summary: summary.to_owned(),
        summary_tokens: 10,
        original_tokens: 100,
        first_message_id: first_id.to_owned(),
        last_message_id: last_id.to_owned(),
        created_at: Utc.with_ymd_and_hms(2026, 5, 15, 12, 0, 0).unwrap(),
    }
}

#[test]
fn block_covering_rows_is_replaced_by_one_summary_message() {
    let history = vec![
        record_id("m1", "user", "first question"),
        record_id("m2", "assistant", "first answer"),
        record_id("m3", "user", "second question"),
        record_id("m4", "assistant", "second answer"),
        record_id("m5", "user", "latest question"),
    ];
    // Block covers rows [m2..m4] (indices 1..=3).
    let blocks = vec![block("b1", "m2", "m4", "Summary of the middle exchange")];

    let (messages, source_ids) =
        build_llm_messages_with_blocks(Some("system instructions"), &history, &blocks);

    // system + m1 + one summary + m5 = 4 messages.
    assert_eq!(messages.len(), 4, "covered rows collapse into one summary");
    assert_eq!(source_ids.len(), messages.len());

    assert_eq!(messages[0].role, MessageRole::System);
    assert_eq!(source_ids[0], None);

    // m1 flows through raw.
    assert_eq!(messages[1].role, MessageRole::User);
    assert_eq!(messages[1].content, "first question");
    assert_eq!(source_ids[1].as_deref(), Some("m1"));

    // The summary replaces m2..m4: a User message carrying the block summary
    // under the shared framing prefix (a System message here would be dropped by
    // the live provider), no UI marker, source_id None.
    assert_eq!(messages[2].role, MessageRole::User);
    assert!(messages[2].content.starts_with(REPLAYED_SUMMARY_PREFIX));
    assert!(messages[2]
        .content
        .ends_with("Summary of the middle exchange"));
    assert!(
        !messages[2].content.contains(COMPACTION_MARKER),
        "injected summary must not carry the UI render marker"
    );
    assert!(
        !messages[2].content.contains("[pierre:compaction]"),
        "injected summary must read as authoritative history"
    );
    assert_eq!(source_ids[2], None, "injected summary occupies a None slot");

    // The covered raw rows are absent.
    for covered in ["first answer", "second question", "second answer"] {
        assert!(
            messages.iter().all(|m| m.content != covered),
            "covered row {covered:?} must not survive raw"
        );
    }

    // m5 (outside the block) flows through raw.
    assert_eq!(messages[3].content, "latest question");
    assert_eq!(source_ids[3].as_deref(), Some("m5"));
}

#[test]
fn empty_blocks_path_is_byte_identical_to_build_llm_messages() {
    let history = vec![
        record("user", "Give me my last 7 activities"),
        record("tool_call", "Pulling your last 7 activities."),
        record(
            "tool_result",
            "[Tool Result for get_activities]: {\"activity_list\":\"3 trails\"}",
        ),
        record("assistant", "Here is your week summary…"),
        record("user", "And now my last 7"),
    ];

    let (base_messages, base_ids) = build_llm_messages(Some("system instructions"), &history);
    let (blocks_messages, blocks_ids) =
        build_llm_messages_with_blocks(Some("system instructions"), &history, &[]);

    // The no-blocks path must produce byte-identical messages and source_ids —
    // the existing replay tests then transitively cover the wrapper.
    assert_eq!(base_ids, blocks_ids);
    assert_eq!(base_messages.len(), blocks_messages.len());
    for (a, b) in base_messages.iter().zip(blocks_messages.iter()) {
        assert_eq!(a.role, b.role);
        assert_eq!(a.content, b.content);
    }
}
