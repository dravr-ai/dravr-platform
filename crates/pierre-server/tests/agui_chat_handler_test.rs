// ABOUTME: End-to-end test asserting the chat send_message handler actually emits AG-UI events
// ABOUTME: Drives the real handler with `agui_run_id` and verifies a parallel SSE subscriber sees them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use futures_util::StreamExt;
use pierre_mcp_server::agui::events::AgUiEventKind;
use pierre_mcp_server::agui::{AgUiEvent, AgUiRoutes, RunOwner};
use pierre_mcp_server::mcp::resources::ServerResources;
use pierre_mcp_server::routes::chat::ChatRoutes;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::env;
use std::net::SocketAddr;
use std::str;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// RAII guard that overrides a set of environment variables.
///
/// For the duration of a scope, restores their prior values on drop
/// — both re-setting the original value and unsetting the variable
/// if it was previously absent.
///
/// The `#[serial_test::serial]` attribute on the test is a
/// correctness requirement (concurrent tests must not observe the
/// override); this guard is the other half of the contract, so a
/// panicking or early-returning test still leaves the environment in
/// the shape the rest of the suite expects.
struct EnvGuard {
    restore: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    /// Override each `(key, value)` pair, saving the prior value so
    /// it can be restored on drop.
    fn apply(overrides: &[(&'static str, Option<&str>)]) -> Self {
        let mut restore = Vec::with_capacity(overrides.len());
        for (key, new_value) in overrides {
            restore.push((*key, env::var(key).ok()));
            match new_value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
        Self { restore }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prior) in &self.restore {
            match prior {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
    }
}

/// Force the chat pipeline to fail at the dispatch stage.
///
/// Selects the Gemini provider with no API key so the pipeline still
/// emits `RUN_STARTED` + `STEP_STARTED("prompt_assembly")` +
/// `STEP_FINISHED(...)` + `STEP_STARTED("dispatch")` +
/// `STEP_FINISHED(...)` before the dispatch error propagates and
/// the outer wrapper emits `RUN_ERROR` — so this is a complete
/// production-flow proof that does not require a live LLM endpoint.
///
/// Returned guard restores the prior environment on drop so a
/// future parallel test that forgets `#[serial]` does not inherit
/// a Gemini-no-key configuration and break in a confusing way.
fn force_dispatch_failure_via_env() -> EnvGuard {
    EnvGuard::apply(&[
        ("PIERRE_LLM_PROVIDER", Some("gemini")),
        ("GEMINI_API_KEY", None),
    ])
}

async fn spawn_server() -> (Arc<ServerResources>, String) {
    let resources = common::create_test_server_resources()
        .await
        .expect("server resources");

    let chat_router = ChatRoutes::routes(Arc::clone(&resources));
    let agui_router =
        AgUiRoutes::routes((*resources.agui_registry).clone(), Arc::clone(&resources));
    let app = chat_router.merge(agui_router);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (resources, format!("http://{addr}"))
}

async fn create_conversation(client: &Client, base_url: &str, token: &str) -> String {
    let resp = client
        .post(format!("{base_url}/api/chat/conversations"))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "title": "agui-wiring-test",
            "model": "gemini-1.5-flash",
        }))
        .send()
        .await
        .expect("create conv");
    assert!(
        resp.status().is_success(),
        "create conversation: {}",
        resp.status()
    );
    let body: Value = resp.json().await.expect("conv json");
    body["id"].as_str().expect("conv id").to_owned()
}

/// Read SSE `event: agui` JSON payloads from a response body until
/// the deadline, or until `n` frames have arrived.
async fn read_agui_frames_until(
    response: reqwest::Response,
    deadline: Duration,
    max: usize,
) -> Vec<String> {
    let mut buffer = String::new();
    let mut out = Vec::new();
    let mut stream = response.bytes_stream();

    let _ = timeout(deadline, async {
        while let Some(chunk) = stream.next().await {
            let Ok(bytes) = chunk else {
                break;
            };
            let Ok(decoded) = str::from_utf8(&bytes) else {
                continue;
            };
            buffer.push_str(decoded);

            while let Some(end) = buffer.find("\n\n") {
                let raw = buffer[..end].to_owned();
                buffer.drain(..=end + 1);

                let mut event_name = "";
                let mut data_lines: Vec<&str> = Vec::new();
                for line in raw.split('\n') {
                    if let Some(rest) = line.strip_prefix("event:") {
                        event_name = rest.trim();
                    } else if let Some(rest) = line.strip_prefix("data:") {
                        data_lines.push(rest.trim());
                    }
                }
                if event_name == "agui" && !data_lines.is_empty() {
                    out.push(data_lines.join("\n"));
                    if out.len() >= max {
                        return;
                    }
                }
            }
        }
    })
    .await;

    out
}

/// Subscribe with retry: the chat handler registers the run only
/// after it accepts the POST, so the SSE client races to subscribe
/// while the POST is in flight. 404 responses trigger a retry.
async fn subscribe_with_retry(
    client: Client,
    url: String,
    token: String,
    deadline: Duration,
) -> reqwest::Response {
    let start = Instant::now();
    loop {
        let response = client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(ACCEPT, "text/event-stream")
            .send()
            .await
            .expect("SSE GET");
        if response.status().is_success() {
            return response;
        }
        assert!(
            start.elapsed() <= deadline,
            "SSE subscribe never succeeded; last status = {}",
            response.status()
        );
        sleep(Duration::from_millis(20)).await;
    }
}

/// Drives the real `send_message` handler with an `agui_run_id`.
///
/// Verifies a parallel SSE subscriber receives the events the
/// pipeline emits during the turn. The pipeline is forced to fail
/// at dispatch (no LLM key) so the test does not need a live LLM —
/// failure still produces a full run lifecycle through `RUN_ERROR`.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn chat_handler_emits_agui_events_for_real_request() {
    let _env = force_dispatch_failure_via_env();

    let (resources, base_url) = spawn_server().await;
    let (user, token) = common::create_test_tenant(&resources, "agui-handler@example.com")
        .await
        .expect("create user + token");

    let client = Client::builder().no_gzip().build().expect("client");
    let conversation_id = create_conversation(&client, &base_url, &token).await;

    let run_id = Uuid::new_v4().to_string();
    let sse_url = format!("{base_url}/api/agui/runs/{run_id}/stream");

    // Pre-register the run from the test side under the same owner
    // the handler will resolve from the JWT. The handler's
    // `setup_agui` calls `register` with the same owner, which the
    // registry treats idempotently (returns the existing sender),
    // so the test's pre-registration only widens the SSE-subscribe
    // window without changing what the handler does. Without it the
    // SSE poll races against a sub-millisecond pipeline failure path
    // and is flaky on cold caches.
    let tenant_id = {
        let repos = resources.database.repositories();
        let tenants = repos.tenants.list_for_user(user.id).await.expect("tenants");
        tenants.first().expect("tenant exists").id
    };
    let owner = RunOwner::new(user.id, tenant_id);
    let _test_scope = resources.agui_registry.register_scoped(&run_id, owner);

    // Open the SSE subscription up-front. It can connect immediately
    // because the run is already registered.
    let sse_handle = {
        let client = client.clone();
        let token = token.clone();
        tokio::spawn(async move {
            let response =
                subscribe_with_retry(client, sse_url, token, Duration::from_secs(5)).await;
            read_agui_frames_until(response, Duration::from_secs(15), 10).await
        })
    };
    sleep(Duration::from_millis(50)).await;

    // Send the chat message with the matching agui_run_id. The
    // handler registers the run, runs the pipeline (which emits
    // events as it advances), then returns. The dispatch failure is
    // expected — RUN_ERROR is part of the proof.
    let post_url = format!("{base_url}/api/chat/conversations/{conversation_id}/messages");
    let post_response = client
        .post(&post_url)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "content": "Tell me about my last run",
            "agui_run_id": run_id,
        }))
        .send()
        .await
        .expect("POST send_message");
    let post_status = post_response.status();
    let post_body = post_response.text().await.unwrap_or_default();
    eprintln!("POST status={post_status} body={post_body}");

    // Pipeline should fail at dispatch → 5xx. We don't assert the
    // exact code; the key invariant is that the SSE stream observed
    // the run lifecycle.
    assert!(
        post_status.is_server_error() || post_status == StatusCode::BAD_GATEWAY,
        "expected dispatch failure status, got {post_status} body={post_body}"
    );

    let frames = sse_handle.await.expect("sse task");
    assert!(
        !frames.is_empty(),
        "AG-UI subscriber must observe at least RUN_STARTED — wiring is broken"
    );

    // Decode every frame and assert the start + error markers landed
    // on the wire. The exact set of intermediate STEP events depends
    // on how far prompt assembly progresses before dispatch fails;
    // we assert the run-lifecycle bookends are present.
    let kinds: Vec<AgUiEventKind> = frames
        .iter()
        .filter_map(|raw| {
            serde_json::from_str::<AgUiEvent>(raw)
                .ok()
                .map(|e| e.kind())
        })
        .collect();
    assert!(
        kinds.contains(&AgUiEventKind::RunStarted),
        "expected RUN_STARTED in {kinds:?}"
    );
    assert!(
        kinds.contains(&AgUiEventKind::RunError),
        "expected RUN_ERROR in {kinds:?}"
    );

    let _ = user; // keep the user value alive through the test
}

/// A non-UUID `agui_run_id` must reject with HTTP 400 and an error
/// message naming the field. Pins the defensive parse in
/// `ChatRoutes::setup_agui` so a future refactor that silently
/// accepts garbage (e.g. uses `to_owned()` instead of
/// `Uuid::parse_str`) is caught. The failure message also doubles as
/// client-facing docs: if a mobile app sends the wrong format it
/// sees the actionable line "`agui_run_id` must be a UUID string".
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn chat_handler_rejects_non_uuid_agui_run_id() {
    let (resources, base_url) = spawn_server().await;
    let (_user, token) = common::create_test_tenant(&resources, "agui-uuid-reject@example.com")
        .await
        .expect("create user + token");

    let client = Client::builder().no_gzip().build().expect("client");
    let conversation_id = create_conversation(&client, &base_url, &token).await;

    let post_url = format!("{base_url}/api/chat/conversations/{conversation_id}/messages");
    let response = client
        .post(&post_url)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "content": "Hello",
            "agui_run_id": "not-a-uuid",
        }))
        .send()
        .await
        .expect("POST send_message");

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "non-UUID agui_run_id must reject 400"
    );

    let body = response.text().await.unwrap_or_default();
    assert!(
        body.contains("agui_run_id") && body.to_lowercase().contains("uuid"),
        "400 body should name the bad field + expected format, got: {body}"
    );
}
