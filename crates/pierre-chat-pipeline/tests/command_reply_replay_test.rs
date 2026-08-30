// ABOUTME: A persisted slash-command turn stays in the transcript but never replays into a prompt
// ABOUTME: Pins the stamp-keyed drop for both rows of the turn and the actions block it carries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A command reply is history the athlete sees on reload — Telegram keeps a
//! bot's answer in the thread, and so does the in-app transcript now — but it
//! is the platform talking, not the coach. Replayed as an assistant turn, a
//! `/status` listing or a `/coach` picker teaches the model to answer in the
//! platform's voice. Both rows of the turn carry [`COMMAND_FINISH_REASON`], and
//! the replay drops them by that stamp, whatever the text says.

use pierre_chat_pipeline::stages::command_persistence::{
    actions_content_blocks, is_room_visible, CommandPersistence, ROOM_VISIBLE_COMMANDS,
};
use pierre_chat_pipeline::stages::prompt_builder::build_llm_messages;
use pierre_chat_pipeline::{ActionKind, TurnAction};
use pierre_core::models::{
    MessageRecord, PersistedAction, PersistedReplyBlock, ACTIONS_BLOCK_TYPE, COMMAND_FINISH_REASON,
};

fn row(id: &str, role: &str, content: &str, finish_reason: Option<&str>) -> MessageRecord {
    MessageRecord {
        id: id.to_owned(),
        conversation_id: "conv-1".to_owned(),
        role: role.to_owned(),
        content: content.to_owned(),
        token_count: None,
        prompt_tokens: None,
        model: None,
        finish_reason: finish_reason.map(str::to_owned),
        content_blocks: None,
        created_at: "2026-08-26T21:00:00Z".to_owned(),
    }
}

const STATUS_REPLY: &str = "Ton statut Dravr\nFournisseurs : strava\nCanal : web";

#[test]
fn both_rows_of_a_command_turn_are_dropped_from_the_replayed_prompt() {
    let history = vec![
        row("m1", "user", "c'est quoi mon plan?", None),
        row(
            "m2",
            "assistant",
            "Trois sorties cette semaine.",
            Some("stop"),
        ),
        row("m3", "user", "/status", Some(COMMAND_FINISH_REASON)),
        row("m4", "assistant", STATUS_REPLY, Some(COMMAND_FINISH_REASON)),
        row("m5", "user", "et pour samedi?", None),
    ];

    let (messages, source_ids) = build_llm_messages(Some("sys"), &history);

    assert!(
        messages.iter().all(|m| !m.content.contains("/status")),
        "the athlete's command line must not re-enter the prompt"
    );
    assert!(
        messages
            .iter()
            .all(|m| !m.content.contains("Ton statut Dravr")),
        "the platform's answer must not re-enter the prompt"
    );
    for dropped in ["m3", "m4"] {
        assert!(
            !source_ids.iter().any(|s| s.as_deref() == Some(dropped)),
            "{dropped} must not appear in source_ids either"
        );
    }
    // The coaching turns around it are untouched, in order.
    let kept: Vec<&str> = source_ids.iter().flatten().map(String::as_str).collect();
    assert_eq!(kept, ["m1", "m2", "m5"]);
    assert_eq!(messages.len(), 4, "system prompt + three coaching rows");
}

#[test]
fn an_unstamped_row_with_the_same_text_is_kept() {
    // The drop is keyed on the stamp, not on the words: a coach that quotes a
    // status line back is still replayed. Prose matching is exactly the
    // mechanism the withheld-reply fix retired.
    let history = vec![
        row("m1", "user", "que dit mon statut?", None),
        row("m2", "assistant", STATUS_REPLY, Some("stop")),
    ];

    let (messages, _) = build_llm_messages(Some("sys"), &history);

    assert!(
        messages
            .iter()
            .any(|m| m.content.contains("Ton statut Dravr")),
        "an unstamped assistant row must still replay"
    );
}

#[test]
fn is_command_turn_reads_the_stamp_on_either_role() {
    assert!(row("u", "user", "/coach", Some(COMMAND_FINISH_REASON)).is_command_turn());
    assert!(row(
        "a",
        "assistant",
        "Choisis un coach",
        Some(COMMAND_FINISH_REASON)
    )
    .is_command_turn());
    assert!(!row("a", "assistant", "Choisis un coach", Some("stop")).is_command_turn());
    assert!(!row("u", "user", "/coach", None).is_command_turn());
}

#[test]
fn the_actions_block_round_trips_through_content_blocks() {
    let actions = vec![
        TurnAction {
            label: "Recovery Coach".to_owned(),
            kind: ActionKind::Postback,
            value: "/coach add @recovery-coach".to_owned(),
        },
        TurnAction {
            label: "Open Discover".to_owned(),
            kind: ActionKind::OpenUrl,
            value: "https://app.dravr.ai/#discover".to_owned(),
        },
    ];

    let stored = actions_content_blocks(Some("Choose a coach"), &actions)
        .unwrap()
        .expect("controls produce a block");

    // One entry, tagged so the read path can partition it away from visuals.
    let entries: Vec<serde_json::Value> = serde_json::from_str(&stored).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["type"], ACTIONS_BLOCK_TYPE);

    let decoded: Vec<PersistedReplyBlock> = serde_json::from_str(&stored).unwrap();
    assert_eq!(
        decoded,
        vec![PersistedReplyBlock::Actions {
            title: Some("Choose a coach".to_owned()),
            actions: vec![
                PersistedAction {
                    label: "Recovery Coach".to_owned(),
                    action_type: "postback".to_owned(),
                    value: "/coach add @recovery-coach".to_owned(),
                },
                PersistedAction {
                    label: "Open Discover".to_owned(),
                    action_type: "url".to_owned(),
                    value: "https://app.dravr.ai/#discover".to_owned(),
                },
            ],
        }]
    );

    // No controls, no block: a plain text reply keeps a NULL column.
    assert_eq!(actions_content_blocks(Some("title"), &[]).unwrap(), None);
}

#[test]
fn a_shared_room_persists_only_what_it_saw() {
    assert!(CommandPersistence::Always.persists(Some("status")));
    assert!(CommandPersistence::Always.persists(None));

    for name in ROOM_VISIBLE_COMMANDS {
        assert!(
            is_room_visible(Some(name)),
            "{name} is announced in the room"
        );
        assert!(CommandPersistence::RoomVisibleOnly.persists(Some(name)));
    }
    assert!(ROOM_VISIBLE_COMMANDS.contains(&"coach-add"));
    // `/plan share` is the consent to post the caller's plan; bare `/plan` is
    // not, and listing it here would publish every member's plan.
    assert!(ROOM_VISIBLE_COMMANDS.contains(&"plan-share"));
    assert!(!ROOM_VISIBLE_COMMANDS.contains(&"plan"));
    for private in [
        "status",
        "group-invite",
        "group-consent",
        "coach-list",
        "plan",
    ] {
        assert!(!is_room_visible(Some(private)));
        assert!(!CommandPersistence::RoomVisibleOnly.persists(Some(private)));
    }
    assert!(
        !CommandPersistence::RoomVisibleOnly.persists(None),
        "the unknown-command reply is private, so a room never keeps it"
    );
}
