// ABOUTME: POST /api/me/plan/thread — the athlete's own plan thread, their Plan tab's Dravr tab, found or opened
// ABOUTME: Pins idempotence, tenant isolation, the cap and archive slot it never holds, and that it is never the personal thread

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1841. A self-coached athlete talks about their own plan with their
//! agent in a thread of its own, flagged `plan_thread`, so Home's personal
//! thread stays free of plan traffic. These tests drive the route and read
//! the flag back through the repository and the conversation list.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::TenantId;
use pierre_database::repositories::Reactivation;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use serde_json::{json, Value};
use uuid::Uuid;

struct Athlete {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

async fn athlete(res: &Arc<ServerContext>, label: &str) -> Athlete {
    let email = format!("{label}-{}@plan-thread.test", Uuid::new_v4());
    let (id, user, tenant) =
        create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    let token = generate_test_token(res, &user).await;
    Athlete {
        id,
        tenant,
        auth: format!("Bearer {token}"),
    }
}

async fn request(
    res: &Arc<ServerContext>,
    who: &Athlete,
    request: AxumTestRequest,
) -> (StatusCode, Value) {
    let resp = request
        .header("authorization", &who.auth)
        .send(ChatRoutes::routes(Arc::clone(res)))
        .await;
    let status = resp.status_code();
    (status, resp.json())
}

/// The athlete's plan thread, as the route answers it.
async fn plan_thread(res: &Arc<ServerContext>, who: &Athlete) -> Value {
    let (status, body) = request(res, who, AxumTestRequest::post("/api/me/plan/thread")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// The athlete's personal thread's id, adopting their newest in-app thread
/// as `GET /api/chat/personal` does; `None` when they have none.
async fn personal_id(res: &Arc<ServerContext>, who: &Athlete) -> Option<String> {
    let (status, body) = request(res, who, AxumTestRequest::get("/api/chat/personal")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["conversation"]["id"].as_str().map(str::to_owned)
}

/// An in-app thread of the athlete's own, opened as a new chat.
async fn new_chat(res: &Arc<ServerContext>, who: &Athlete) -> String {
    let (status, body) = request(
        res,
        who,
        AxumTestRequest::post("/api/chat/conversations").json(&json!({ "title": "a chat" })),
    )
    .await;
    assert!(status.is_success(), "{status}: {body}");
    body["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn the_plan_thread_is_opened_once_and_every_call_answers_it() {
    let res = create_test_server_resources().await.unwrap();
    let alex = athlete(&res, "alex").await;
    let first = plan_thread(&res, &alex).await;
    assert_eq!(first["opened"], true, "{first}");
    let id = first["conversation"]["id"].as_str().unwrap().to_owned();
    let again = plan_thread(&res, &alex).await;
    assert_eq!(again["opened"], false, "{again}");
    assert_eq!(
        again["conversation"]["id"], id,
        "bug: a second call opened a second plan thread: {again}"
    );
    let stored = res
        .common
        .repos
        .chat
        .plan_thread_conversation(&alex.id.to_string(), alex.tenant)
        .await
        .unwrap()
        .expect("the plan thread is stored");
    assert_eq!(
        stored.id, id,
        "bug: the designated plan thread is not the one the route answered"
    );
    assert!(
        !stored.personal,
        "bug: the plan thread is the personal thread"
    );

    let (status, list) =
        request(&res, &alex, AxumTestRequest::get("/api/chat/conversations")).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let row = list["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("the plan thread is listed: {list}"));
    assert!(
        row["plan_thread"] == true && row["personal"] == false,
        "bug: the list cannot tell the plan thread from the personal one: {row}"
    );
}

#[tokio::test]
async fn the_plan_thread_never_becomes_the_personal_thread_nor_takes_its_place() {
    let res = create_test_server_resources().await.unwrap();
    // With no thread at all, the plan thread opened first is never adopted
    // as the personal thread.
    let alex = athlete(&res, "alex").await;
    let alex_plan = plan_thread(&res, &alex).await["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        personal_id(&res, &alex).await,
        None,
        "bug: Home adopted the plan thread as the personal thread"
    );

    // With a personal thread, the plan thread is another thread, bound to the
    // same agent, and Home keeps its own.
    let sam = athlete(&res, "sam").await;
    let chat = new_chat(&res, &sam).await;
    assert_eq!(
        personal_id(&res, &sam).await.as_deref(),
        Some(chat.as_str()),
        "bug: Home did not adopt the athlete's only chat as the personal thread"
    );
    let sam_plan = plan_thread(&res, &sam).await;
    let sam_plan_id = sam_plan["conversation"]["id"].as_str().unwrap().to_owned();
    assert_ne!(
        sam_plan_id, chat,
        "bug: the plan thread is the personal thread"
    );
    assert_eq!(
        personal_id(&res, &sam).await.as_deref(),
        Some(chat.as_str()),
        "bug: the plan thread took the personal thread's place"
    );
    let repos = &res.common.repos;
    let personal = repos
        .chat
        .personal_conversation(&sam.id.to_string(), sam.tenant)
        .await
        .unwrap()
        .unwrap();
    let planned = repos
        .chat
        .plan_thread_conversation(&sam.id.to_string(), sam.tenant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        planned.agent_id, personal.agent_id,
        "bug: the plan thread is bound to another agent than Home's"
    );
    assert!(
        !planned.personal && personal.id == chat,
        "bug: the plan thread and the personal thread are one"
    );
    assert_ne!(
        alex_plan, sam_plan_id,
        "bug: two athletes were answered the same plan thread"
    );
}

#[tokio::test]
async fn each_athlete_reads_only_their_own_plan_thread_in_their_own_tenant() {
    let res = create_test_server_resources().await.unwrap();
    let alex = athlete(&res, "alex").await;
    let sam = athlete(&res, "sam").await;
    let alex_plan = plan_thread(&res, &alex).await["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sam_plan = plan_thread(&res, &sam).await["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(alex_plan, sam_plan, "bug: two athletes share a plan thread");
    let chat = &res.common.repos.chat;
    assert!(
        chat.plan_thread_conversation(&alex.id.to_string(), sam.tenant)
            .await
            .unwrap()
            .is_none(),
        "bug: alex's plan thread read under sam's tenant"
    );
    assert_eq!(
        chat.plan_thread_conversation(&sam.id.to_string(), sam.tenant)
            .await
            .unwrap()
            .map(|thread| thread.id),
        Some(sam_plan),
        "bug: sam reads another plan thread than their own"
    );
}

#[tokio::test]
async fn the_plan_thread_holds_no_slot_of_the_cap() {
    let res = create_test_server_resources().await.unwrap();
    let alex = athlete(&res, "alex").await;
    let chat = &res.common.repos.chat;
    let user = alex.id.to_string();
    let before = chat.count_conversations(&user, alex.tenant).await.unwrap();
    let id = plan_thread(&res, &alex).await["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        chat.count_conversations(&user, alex.tenant).await.unwrap(),
        before,
        "bug: the plan thread counted toward the cap"
    );
    // Archived, it comes back whatever the cap: it never held a slot.
    assert!(chat
        .archive_conversation(&id, &user, alex.tenant)
        .await
        .unwrap());
    assert_eq!(
        chat.reactivate_conversation(&id, &user, alex.tenant, Some(0))
            .await
            .unwrap(),
        Reactivation::Reactivated,
        "bug: the plan thread was refused at the cap"
    );
    // An ordinary thread does hold one.
    let other = new_chat(&res, &alex).await;
    assert!(chat
        .archive_conversation(&other, &user, alex.tenant)
        .await
        .unwrap());
    assert!(
        matches!(
            chat.reactivate_conversation(&other, &user, alex.tenant, Some(0))
                .await
                .unwrap(),
            Reactivation::AtCap { .. }
        ),
        "fixture: an ordinary thread holds a slot"
    );
}
