// ABOUTME: Integration test for the messaging AG-UI → channel StatusAdapter bridge
// ABOUTME: Drives a Slack mock API through spawn_status_consumer + finalize and asserts the edit trace
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_server::agui::{AgUiEvent, RunOwner, RunRegistry};
use pierre_mcp_server::models::TenantId;
use pierre_mcp_server::services::messaging_status_bridge::spawn_status_consumer;
use pierre_messaging::agui_status::StatusAdapter;
use pierre_messaging::channels::slack::agui_status::SlackStatusAdapter;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use uuid::Uuid;

/// Mock Slack Web API that records every `chat.postMessage` and
/// `chat.update` call the adapter issues.
#[derive(Clone)]
struct MockSlackState {
    calls: Arc<Mutex<Vec<Value>>>,
}

async fn spawn_mock_slack() -> (String, Arc<Mutex<Vec<Value>>>, JoinHandle<()>) {
    use axum::extract::State;
    use axum::routing::post;
    use axum::{Json, Router};

    let calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let state = MockSlackState {
        calls: Arc::clone(&calls),
    };

    let app = Router::new()
        .route(
            "/chat.postMessage",
            post(
                |State(state): State<MockSlackState>, Json(body): Json<Value>| async move {
                    state
                        .calls
                        .lock()
                        .expect("mutex")
                        .push(json!({ "method": "chat.postMessage", "body": body }));
                    Json(json!({
                        "ok": true,
                        "ts": "1700000000.000100",
                        "channel": "C1",
                    }))
                },
            ),
        )
        .route(
            "/chat.update",
            post(
                |State(state): State<MockSlackState>, Json(body): Json<Value>| async move {
                    state
                        .calls
                        .lock()
                        .expect("mutex")
                        .push(json!({ "method": "chat.update", "body": body }));
                    Json(json!({
                        "ok": true,
                        "ts": "1700000000.000100",
                        "channel": "C1",
                    }))
                },
            ),
        )
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), calls, handle)
}

/// Full bridge round-trip against a real (mock) Slack Web API:
///
/// 1. Open a `SlackStatusAdapter` pointed at the mock server. The
///    adapter sends a placeholder `chat.postMessage` and stores the
///    returned `ts`.
/// 2. Register an AG-UI run in a fresh `RunRegistry`.
/// 3. Publish a sequence of AG-UI events (some BEFORE the consumer
///    subscribes, some AFTER — exercising the backlog + live-receiver
///    paths inside `spawn_status_consumer`).
/// 4. Call `adapter.finalize(reply)` with the final assistant reply.
/// 5. Assert the mock API's captured trace: one `chat.postMessage`,
///    one `chat.update` per rendered status, and a terminal
///    `chat.update` carrying the reply.
#[tokio::test(flavor = "multi_thread")]
async fn bridge_drives_slack_edits_through_registry() {
    let (slack_base, slack_calls, slack_handle) = spawn_mock_slack().await;

    // Open the Slack adapter against the mock base URL; the
    // production `open_status_adapter` helper hard-codes
    // `slack.com/api`, which is fine for real runs but useless from
    // an integration test. Using the raw `open_with_base` constructor
    // is the supported test entry point.
    let adapter =
        SlackStatusAdapter::open_with_base("xoxb-fake-token", "C1", None, "thinking…", slack_base)
            .await
            .expect("open placeholder")
            .with_edit_throttle(Duration::ZERO);
    let adapter: Arc<dyn StatusAdapter + Send + Sync> = Arc::new(adapter);

    let run_id = Uuid::new_v4().to_string();
    let registry = Arc::new(RunRegistry::default());
    let owner = RunOwner::new(Uuid::new_v4(), TenantId::new());
    let scope = registry.register_scoped(&run_id, owner);

    // Publish two events BEFORE the consumer subscribes — they must
    // land via the replay backlog so the first status update the user
    // sees still says "thinking…" (not whatever the pipeline is doing
    // when the consumer races the register call).
    publish(
        &registry,
        &run_id,
        &AgUiEvent::run_started(&run_id, Some("thread_1")),
    );
    publish(
        &registry,
        &run_id,
        &AgUiEvent::step_started(&run_id, "prompt_assembly"),
    );

    // Spawn the consumer. It must drain the backlog before waiting on
    // the live receiver.
    let consumer_handle = spawn_status_consumer(&registry, run_id.clone(), Arc::clone(&adapter))
        .expect("consumer spawned");

    // Publish the remaining events AFTER subscribe — these exercise
    // the live-receiver path.
    publish(
        &registry,
        &run_id,
        &AgUiEvent::tool_call_start(&run_id, "call_1", "get_activities"),
    );
    publish(
        &registry,
        &run_id,
        &AgUiEvent::step_started(&run_id, "dispatch"),
    );
    publish(&registry, &run_id, &AgUiEvent::run_finished(&run_id));

    // Give the consumer a beat to drain; `RunFinished` terminates the
    // loop so the handle resolves on its own.
    let _ = timeout(Duration::from_secs(2), consumer_handle).await;

    adapter
        .finalize("Your last run was 5 km at 4:30/km.")
        .await
        .expect("finalize");

    // Dropping the scope unregisters the run and closes the
    // broadcast (anything still attached observes `Closed`).
    drop(scope);
    slack_handle.abort();

    assert_slack_trace(&slack_calls);
}

fn publish(registry: &RunRegistry, run_id: &str, event: &AgUiEvent) {
    let payload = serde_json::to_string(event).expect("serialize AG-UI event");
    // `RunRegistry::publish` stores in the replay buffer *and*
    // broadcasts live — the same path the chat pipeline's
    // `BroadcastSink` uses in production. Going through `scope.send`
    // directly would skip the replay buffer and defeat the
    // backlog-vs-live assertion this test is making.
    let _ = registry.publish(run_id, payload);
}

fn assert_slack_trace(calls: &Mutex<Vec<Value>>) {
    let captured = calls.lock().expect("mutex").clone();

    eprintln!("\n── Slack bridge trace ({} calls) ──", captured.len());
    for (i, c) in captured.iter().enumerate() {
        let method = c["method"].as_str().unwrap_or("?");
        let text = c["body"]["text"].as_str().unwrap_or("");
        eprintln!("  {:>2}. {:<18} text={text:?}", i + 1, method);
    }
    eprintln!("── end trace ──\n");

    let post_count = captured
        .iter()
        .filter(|c| c["method"] == "chat.postMessage")
        .count();
    assert_eq!(post_count, 1, "one chat.postMessage: {captured:?}");

    let update_texts: Vec<String> = captured
        .iter()
        .filter(|c| c["method"] == "chat.update")
        .map(|c| c["body"]["text"].as_str().unwrap_or_default().to_owned())
        .collect();

    // Expected updates: RunStarted + StepStarted(prompt_assembly) +
    // ToolCallStart(get_activities) + StepStarted(dispatch) +
    // finalize(reply) = 5.
    assert!(
        update_texts.len() >= 5,
        "expected at least 5 updates, got {}: {update_texts:?}",
        update_texts.len()
    );

    // Replay-backlog events must land — otherwise the RunStarted
    // from before subscribe gets dropped on the floor.
    assert!(
        update_texts.iter().any(|t| t == "thinking…"),
        "RunStarted (backlog) must render as thinking…: {update_texts:?}"
    );
    assert!(
        update_texts.iter().any(|t| t == "reading your question…"),
        "StepStarted(prompt_assembly) (backlog) must render: {update_texts:?}"
    );
    // Live-receiver events.
    assert!(
        update_texts.iter().any(|t| t == "calling get_activities…"),
        "ToolCallStart (live) must render: {update_texts:?}"
    );
    assert!(
        update_texts.iter().any(|t| t == "generating response…"),
        "StepStarted(dispatch) (live) must render: {update_texts:?}"
    );
    // Final edit is the assistant reply.
    assert_eq!(
        update_texts.last().map(String::as_str),
        Some("Your last run was 5 km at 4:30/km."),
    );
}
