// ABOUTME: Response date goldens for the chat routes: conversation and message stamps as the web and mobile clients read them
// ABOUTME: Pins created_at and updated_at written in UTC with an offset, with and without a fraction
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `/api/chat/conversations`, one conversation, and
//! its messages (carnet#1620, gate 3).
//!
//! The chat repository stamps every row with the wall clock and takes no
//! instant from its caller, so a conversation is created through the route
//! (stamped with now: compared by shape, fraction kept) and then its row and
//! its messages' rows are moved to instants the suite chooses, with one
//! `UPDATE` each on the test database. Every later read goes through `ChatRoutes::routes` and
//! is compared with a literal table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::http::StatusCode;
use pierre_core::models::AddMessageParams;
use pierre_core::permissions::UserRole;
use pierre_core::transport::TransportPolicy;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use serde_json::{json, Value};

use crate::common;
use crate::helpers::date_golden::{
    dates_in, dates_in_stamped, get, instant, on_the_clock, restamp, seed_account, send, table,
    with_json, Athlete,
};

const MOVE_CONVERSATION: &str =
    "UPDATE chat_conversations SET created_at = $1 WHERE CAST(id AS TEXT) = $2";
const TOUCH_CONVERSATION: &str =
    "UPDATE chat_conversations SET updated_at = $1 WHERE CAST(id AS TEXT) = $2";
const MOVE_MESSAGE: &str = "UPDATE chat_messages SET created_at = $1 WHERE CAST(id AS TEXT) = $2";

async fn athlete(resources: &Arc<ServerContext>, label: &str) -> Athlete {
    seed_account(
        resources,
        label,
        UserRole::User,
        instant("2026-08-01T09:30:00"),
        instant("2026-08-01T09:30:00"),
    )
    .await
}

async fn create_conversation(resources: &Arc<ServerContext>, athlete: &Athlete) -> Value {
    let (status, body) = send(
        ChatRoutes::routes(Arc::clone(resources)),
        with_json(
            "POST",
            "/api/chat/conversations",
            &athlete.token,
            &json!({ "title": "Tempo week", "model": "test-model" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

async fn chat(resources: &Arc<ServerContext>, athlete: &Athlete, uri: &str) -> Value {
    let (status, body) = send(
        ChatRoutes::routes(Arc::clone(resources)),
        get(uri, &athlete.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

#[tokio::test]
async fn a_new_conversation_is_stamped_in_utc_with_an_offset() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = athlete(&resources, "golden-chat-new").await;

        let created = create_conversation(&resources, &athlete).await;

        assert_eq!(
            dates_in_stamped(&created, &["/created_at", "/updated_at"]),
            table(&[
                ("/created_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
                ("/updated_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
            ]),
            "{created}"
        );
        assert_eq!(created["created_at"], created["updated_at"], "{created}");
    })
    .await;
}

#[tokio::test]
async fn a_conversation_and_its_messages_answer_the_instants_their_rows_hold() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, "golden-chat-read").await;
    let database = &resources.agent.database;
    let id = create_conversation(&resources, &athlete).await["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // A question with milliseconds, an answer on a whole second, a follow-up
    // with microseconds.
    let said = [
        ("user", "How was my week?", "2026-08-28T02:59:03.500250"),
        ("assistant", "Steady.", "2026-08-28T02:59:10"),
        ("user", "And the long run?", "2026-08-28T03:01:00.250125"),
    ];
    let user = athlete.user_id.to_string();
    for (role, content, at) in said {
        let message = resources
            .common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: athlete.tenant,
                conversation_id: &id,
                user_id: &user,
                role,
                content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
                about_user_id: None,
                turn_attempted_write: None,
                ai_consent_sources: &[],
            })
            .await
            .unwrap();
        restamp(database, MOVE_MESSAGE, instant(at), &message.id, None).await;
    }
    restamp(
        database,
        MOVE_CONVERSATION,
        instant("2026-08-27T22:00:00"),
        &id,
        Some(athlete.tenant),
    )
    .await;
    restamp(
        database,
        TOUCH_CONVERSATION,
        instant("2026-08-28T03:01:00.250125"),
        &id,
        Some(athlete.tenant),
    )
    .await;

    let one = chat(
        &resources,
        &athlete,
        &format!("/api/chat/conversations/{id}"),
    )
    .await;
    assert_eq!(
        dates_in(&one),
        table(&[
            ("/created_at", "2026-08-27T22:00:00+00:00"),
            ("/updated_at", "2026-08-28T03:01:00.250125+00:00"),
        ]),
        "{one}"
    );

    let listed = chat(&resources, &athlete, "/api/chat/conversations").await;
    let row = listed["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id.as_str())
        .unwrap_or_else(|| panic!("the conversation is listed: {listed}"));
    assert_eq!(
        dates_in(row),
        table(&[
            ("/created_at", "2026-08-27T22:00:00+00:00"),
            (
                "/last_message/created_at",
                "2026-08-28T03:01:00.250125+00:00"
            ),
            ("/updated_at", "2026-08-28T03:01:00.250125+00:00"),
        ]),
        "{row}"
    );

    let messages = chat(
        &resources,
        &athlete,
        &format!("/api/chat/conversations/{id}/messages"),
    )
    .await;
    assert_eq!(
        dates_in(&messages),
        table(&[
            ("/messages/0/created_at", "2026-08-28T02:59:03.500250+00:00"),
            ("/messages/1/created_at", "2026-08-28T02:59:10+00:00"),
            ("/messages/2/created_at", "2026-08-28T03:01:00.250125+00:00"),
        ]),
        "{messages}"
    );
}
