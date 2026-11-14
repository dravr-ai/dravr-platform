// ABOUTME: A plan card stored with the bool outside-the-evidence mark reads back from history as the label, in the reader's language
// ABOUTME: Stores the snapshot through the repository, reads it through GET …/messages, and checks the row itself is untouched
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Replies saved from 079ffba87 (carnet#1495) until carnet#1552 store a plan
//! card whose days mark the warning as a bool. Since carnet#1552 the clients
//! print the label the server sends, so history words those snapshots on
//! the way out, as a live card words its own, and leaves the row as stored.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use axum::http::StatusCode;
use common::{create_test_server_resources, create_test_tenant};
use helpers::axum_test::AxumTestRequest;
use pierre_contremaitre::evidence_strings::KEY_EVIDENCE_OUTSIDE;
use pierre_core::models::AddMessageParams;
use pierre_core::transport::TransportPolicy;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_services::plan_card::{PLAN_CARD_SOURCE_TOOL, WORKOUT_PLAN_BLOCK_TYPE};
use serde_json::{json, Value};
use std::sync::Arc;

/// The reader's language, chosen so the label cannot be the bundled English.
const READER_LOCALE: &str = "de";

#[tokio::test]
async fn a_stored_bool_mark_reads_back_as_the_label_in_the_readers_language() {
    let resources = create_test_server_resources().await.unwrap();
    let (user, token) = create_test_tenant(&resources, "plan-card-history@example.com")
        .await
        .unwrap();
    let repos = resources.agent.database.repositories();
    repos
        .users
        .update_locale(user.id, READER_LOCALE)
        .await
        .unwrap();
    let tenant_id = repos.tenants.list_for_user(user.id).await.unwrap()[0].id;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let auth = format!("Bearer {token}");

    let created = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &auth)
        .json(&json!({ "title": "Plan", "model": "gemini-1.5-flash" }))
        .send(router.clone())
        .await;
    assert_eq!(created.status_code(), StatusCode::CREATED);
    let conversation_id = created.json::<Value>()["id"]
        .as_str()
        .expect("the conversation's id")
        .to_owned();

    // The snapshot as 079ffba87 wrote it: one day marked, one day not.
    let stored = json!([{
        "type": WORKOUT_PLAN_BLOCK_TYPE,
        "source_tool": PLAN_CARD_SOURCE_TOOL,
        "plan": { "weeks": [{
            "week_start": "2026-11-16",
            "focus": "Base",
            "current": true,
            "days": [
                { "date": "2026-11-17", "sport": "run", "workout": "30/15", "intensity": "hard",
                  "rest": false, "outside_evidence": true },
                { "date": "2026-11-18", "sport": "run", "workout": "Easy", "intensity": "easy",
                  "rest": false, "outside_evidence": false }
            ]
        }] }
    }])
    .to_string();
    let user_id = user.id.to_string();
    repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conversation_id,
            user_id: &user_id,
            role: "assistant",
            content: "Voilà ton plan.",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: Some(&stored),
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        })
        .await
        .unwrap();

    let page = AxumTestRequest::get(&format!(
        "/api/chat/conversations/{conversation_id}/messages"
    ))
    .header("authorization", &auth)
    .send(router)
    .await;
    assert_eq!(page.status_code(), StatusCode::OK);
    let page: Value = page.json();
    let card = page["messages"]
        .as_array()
        .expect("the thread's messages")
        .iter()
        .filter_map(|message| message["scene_blocks"].as_str())
        .flat_map(|scenes| serde_json::from_str::<Vec<Value>>(scenes).expect("scenes are an array"))
        .find(|block| block["kind"] == WORKOUT_PLAN_BLOCK_TYPE)
        .unwrap_or_else(|| panic!("the plan card in the history read: {page}"));
    let days = &card["plan"]["weeks"][0]["days"];

    assert_eq!(
        days[0]["outside_evidence"],
        resources
            .mcp
            .messaging_strings_registry
            .get(KEY_EVIDENCE_OUTSIDE, READER_LOCALE),
        "bug: a stored mark reads back as the warning the live card carries"
    );
    assert_eq!(
        days[0]["outside_evidence"], "Außerhalb der Evidenz",
        "bug: the warning reads in the reader's language"
    );
    assert!(
        days[1].get("outside_evidence").is_none(),
        "bug: an unmarked day carries no key, as on a live card: {}",
        days[1]
    );

    let row = repos
        .chat
        .get_messages(&conversation_id, &user_id, tenant_id)
        .await
        .unwrap()
        .into_iter()
        .find(|message| message.role == "assistant")
        .expect("the stored reply");
    let row_blocks: Value =
        serde_json::from_str(row.content_blocks.as_deref().expect("the stored blocks")).unwrap();
    assert_eq!(
        row_blocks[0]["plan"]["weeks"][0]["days"][0]["outside_evidence"], true,
        "bug: the read rewrote the stored row; it must stay as written so a rollback still reads it"
    );
}
