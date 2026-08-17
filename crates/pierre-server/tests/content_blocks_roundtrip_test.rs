// ABOUTME: content_blocks must survive a write/read round trip on BOTH backends
// ABOUTME: Runs against PostgreSQL when the postgresql feature is on, SQLite otherwise
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

//! A column added to one `ChatRepository` implementation and not the other is a
//! failure mode this repo has actually shipped: `SQLite` goes green while
//! `PostgreSQL` silently drops the data. `create_test_db()` picks its backend
//! from the `postgresql` feature, so this one test covers both — it is the
//! `PostgreSQL` run in `ci-postgres.yml` that makes it meaningful.

#[path = "helpers/messaging_fixtures.rs"]
mod messaging_fixtures;

use messaging_fixtures::{create_test_db, seed_conversation, seed_user};
use pierre_core::models::AddMessageParams;

/// Two blocks, as the extractor would encode them: a chart and a table.
const BLOCKS: &str = r#"[{"type":"chart","kind":"line","source_tool":"analyze_training_load","x":{"label":"Date","type":"time"},"series":[{"label":"CTL","points":[["2026-07-01",42.0],["2026-07-02",43.1]]}]},{"type":"table","source_tool":"get_activities","columns":["Day","Distance"],"rows":[["Tue","12 km"],["Sun","24 km"]]}]"#;

#[tokio::test]
async fn content_blocks_survive_a_round_trip() {
    let db = create_test_db().await;
    // A real user row: chat_conversations has a FK to it.
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_id).await;

    db.repositories()
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conversation_id,
            user_id: &user_id,
            role: "assistant",
            content: "Ta charge grimpe. ⟦viz:0⟧ Et voici le détail. ⟦viz:1⟧",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: Some("claude-sonnet-5"),
            content_blocks: Some(BLOCKS),
        })
        .await
        .unwrap();

    let messages = db
        .repositories()
        .chat
        .get_messages(&conversation_id, &user_id, tenant_id)
        .await
        .unwrap();

    let stored = messages
        .iter()
        .find(|m| m.role == "assistant")
        .expect("the assistant message must be readable back");

    let blocks = stored
        .content_blocks
        .as_deref()
        .expect("content_blocks must survive the write — a NULL here is the dropped-column bug");

    // Assert on parsed content, not string equality: a backend is free to
    // normalise whitespace, and comparing raw text would make this brittle
    // without making it stricter.
    let parsed: serde_json::Value = serde_json::from_str(blocks).expect("stored blocks are JSON");
    let array = parsed.as_array().expect("blocks are an array");
    assert_eq!(array.len(), 2, "both blocks must come back");
    assert_eq!(array[0]["type"], "chart");
    assert_eq!(array[0]["source_tool"], "analyze_training_load");
    assert_eq!(array[1]["type"], "table");
    assert_eq!(
        array[1]["rows"][1][1], "24 km",
        "nested row data must survive intact"
    );

    // The markers in the prose are what let a client interleave text and
    // blocks; losing them would leave the blocks unplaceable.
    assert!(stored.content.contains("⟦viz:0⟧"));
    assert!(stored.content.contains("⟦viz:1⟧"));
}

#[tokio::test]
async fn an_ordinary_reply_stores_null_blocks() {
    let db = create_test_db().await;
    // A real user row: chat_conversations has a FK to it.
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_id).await;

    db.repositories()
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conversation_id,
            user_id: &user_id,
            role: "assistant",
            content: "Repose-toi aujourd'hui.",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    let messages = db
        .repositories()
        .chat
        .get_messages(&conversation_id, &user_id, tenant_id)
        .await
        .unwrap();

    let stored = messages.iter().find(|m| m.role == "assistant").unwrap();
    assert!(
        stored.content_blocks.is_none(),
        "a reply with no blocks must store NULL, not an empty array"
    );
}
