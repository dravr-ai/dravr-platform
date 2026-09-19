// ABOUTME: The A2A reaper fails tasks left non-terminal past the stale threshold and leaves live ones alone
// ABOUTME: Pins the failed status, the orphaned message, the terminal stream event and the untouched sibling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A task killed mid-run used to read `working` forever and hang every later
//! subscriber (carnet#462). The reaper is the only writer that ends such a
//! row, so the test drives it directly: one task aged past the threshold,
//! one fresh, and a subscriber on the aged one that must receive the
//! terminal event rather than wait on a closed process.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use pierre_a2a::events::TASK_EVENTS;
use pierre_a2a::reaper::{reap_stale_tasks, ORPHANED_MESSAGE};
use pierre_a2a::{PartContent, StreamResponse, TaskState};
use pierre_core::models::a2a::TaskStatus;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_runtime_context::A2ACtx;
use serde_json::json;
use tokio::time::{sleep, timeout};

use common::create_test_server_resources;

async fn register_client(resources: &Arc<ServerContext>) -> String {
    let (user, _jwt) = common::create_test_tenant(resources, "reaper@example.com")
        .await
        .expect("seed user + tenant");
    let registration = pierre_a2a::ClientRegistrationRequest {
        name: format!("reaper-client-{}", user.id),
        description: "reaper test client".into(),
        capabilities: vec!["fitness-data-analysis".into()],
        redirect_uris: vec![],
        contact_email: "reaper@example.com".into(),
    };
    resources
        .a2a
        .a2a_client_manager
        .register_client(registration, user.id)
        .await
        .expect("register A2A client")
        .client_id
}

#[tokio::test]
async fn a_task_left_working_past_the_threshold_is_failed_and_its_subscriber_released() {
    let resources = create_test_server_resources().await.expect("resources");
    let client_id = register_client(&resources).await;
    let ctx: Arc<dyn A2ACtx> = Arc::clone(&resources) as Arc<dyn A2ACtx>;
    let repos = A2ACtx::repos(resources.as_ref());

    // The orphan: submitted, moved to working, then its runner died.
    let orphan = repos
        .a2a
        .create_task(&client_id, None, "message", &json!({}), Some("ctx-orphan"))
        .await
        .unwrap();
    repos
        .a2a
        .update_task_status(&orphan, &TaskStatus::Working, None, None)
        .await
        .unwrap();
    // A subscriber waiting on the orphan, as a SubscribeToTask on a fresh
    // instance would.
    let mut waiting = TASK_EVENTS.subscribe(&orphan);

    // The threshold is one second and SQLite's datetime has second
    // resolution: the cutoff `now - 1s` is truncated to a whole second, so
    // the orphan's write second must be strictly below it. A wait of two
    // and a half seconds puts at least one whole second between the two
    // whatever sub-second offset the write landed on.
    sleep(Duration::from_millis(2_500)).await;

    // The live sibling: created after the wait, so it is younger than the
    // threshold and must survive the same reap.
    let live = repos
        .a2a
        .create_task(&client_id, None, "message", &json!({}), Some("ctx-live"))
        .await
        .unwrap();
    repos
        .a2a
        .update_task_status(&live, &TaskStatus::Working, None, None)
        .await
        .unwrap();

    let reaped = reap_stale_tasks(&ctx, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(reaped, 1, "exactly the orphan is reaped");

    let orphan_row = repos.a2a.get_task(&orphan).await.unwrap().unwrap();
    assert_eq!(orphan_row.status, TaskStatus::Failed);
    let message = orphan_row
        .status_message
        .expect("the reaped task carries a status message");
    assert!(
        message.to_string().contains("stopped before it finished"),
        "the status message says what happened: {message}"
    );

    let live_row = repos.a2a.get_task(&live).await.unwrap().unwrap();
    assert_eq!(
        live_row.status,
        TaskStatus::Working,
        "a task younger than the threshold is left running"
    );

    // The subscriber gets the terminal event instead of hanging.
    let event = timeout(Duration::from_secs(2), waiting.recv())
        .await
        .expect("the subscriber is released")
        .expect("a terminal event is published, not just a close");
    match event {
        StreamResponse::StatusUpdate(update) => {
            assert_eq!(update.task_id, orphan);
            assert_eq!(update.status.state, TaskState::Failed);
            let text = update
                .status
                .message
                .and_then(|m| m.parts.into_iter().next())
                .map(|p| match p.content {
                    PartContent::Text(text) => text,
                    other => format!("{other:?}"),
                })
                .unwrap_or_default();
            assert_eq!(
                text, ORPHANED_MESSAGE,
                "the event carries the orphaned message"
            );
        }
        other => panic!("expected a status update, got {other:?}"),
    }
}

#[tokio::test]
async fn a_reap_with_nothing_stale_touches_nothing() {
    let resources = create_test_server_resources().await.expect("resources");
    let client_id = register_client(&resources).await;
    let ctx: Arc<dyn A2ACtx> = Arc::clone(&resources) as Arc<dyn A2ACtx>;
    let repos = A2ACtx::repos(resources.as_ref());

    let task = repos
        .a2a
        .create_task(&client_id, None, "message", &json!({}), None)
        .await
        .unwrap();

    let reaped = reap_stale_tasks(&ctx, Duration::from_mins(10))
        .await
        .unwrap();
    assert_eq!(reaped, 0);
    assert_eq!(
        repos.a2a.get_task(&task).await.unwrap().unwrap().status,
        TaskStatus::Submitted
    );
}
