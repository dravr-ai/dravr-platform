// ABOUTME: Unit tests for build_llm_messages replay of tool_call and tool_result rows
// ABOUTME: Confirms persisted tool rounds rebuild the same Vec<ChatMessage> shape the loop pushed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_chat_pipeline::stages::prompt_builder::build_llm_messages;
use pierre_database::database::MessageRecord;
use pierre_llm::MessageRole;

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

    let messages = build_llm_messages(Some("system instructions"), &history);

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

    let messages = build_llm_messages(None, &history);

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[1].role, MessageRole::User);
    assert!(messages[1].content.contains("Trail run"));
    assert_eq!(messages[2].role, MessageRole::Assistant);
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

    let messages = build_llm_messages(Some("system instructions"), &history);

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
}

#[test]
fn unknown_roles_are_dropped_defensively() {
    let history = vec![
        record("user", "hi"),
        record("noise_role", "should be skipped"),
        record("assistant", "hello"),
    ];

    let messages = build_llm_messages(None, &history);

    assert_eq!(messages.len(), 2, "unknown role dropped");
    assert_eq!(messages[0].content, "hi");
    assert_eq!(messages[1].content, "hello");
}
