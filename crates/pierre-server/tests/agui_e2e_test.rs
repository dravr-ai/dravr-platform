// ABOUTME: End-to-end HTTP SSE test for the AG-UI route in pierre-server
// ABOUTME: Boots axum with real auth, asserts fan-out + owner checks + 401/403 rejections
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use futures_util::StreamExt;
use pierre_mcp_server::agui::emitter::AgUiSink;
use pierre_mcp_server::agui::events::AgUiEventKind;
use pierre_mcp_server::agui::registry::RunOwner;
use pierre_mcp_server::agui::{AgUiEvent, AgUiEventFilter, AgUiRoutes, BroadcastSink};
use pierre_mcp_server::mcp::resources::ServerContext;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use reqwest::{Client, StatusCode};
use std::net::SocketAddr;
use std::str;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

/// Bootstraps a full-auth test fixture: real `ServerContext`, a
/// test user, their JWT, and the AG-UI router mounted on an ephemeral
/// port.
struct Fixture {
    resources: Arc<ServerContext>,
    base_url: String,
    owner: RunOwner,
    token: String,
    _server: JoinHandle<()>,
}

impl Fixture {
    async fn spawn() -> Self {
        let resources = common::create_test_server_resources()
            .await
            .expect("create test resources");
        let (user, token) = common::create_test_tenant(&resources, "agui-e2e@example.com")
            .await
            .expect("create user + token");

        let tenant_id = {
            let repos = resources.database.repositories();
            let tenants = repos.tenants.list_for_user(user.id).await.expect("tenants");
            tenants.first().expect("tenant exists").id
        };
        let owner = RunOwner::new(user.id, tenant_id);

        let app = AgUiRoutes::routes((*resources.agui_registry).clone(), Arc::clone(&resources));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });

        Self {
            resources,
            base_url: format!("http://{addr}"),
            owner,
            token,
            _server: handle,
        }
    }

    fn url(&self, run_id: &str) -> String {
        format!("{}/api/agui/runs/{run_id}/stream", self.base_url)
    }

    async fn second_user_token(&self, email: &str) -> String {
        let (_user, token) = common::create_test_tenant(&self.resources, email)
            .await
            .expect("second user");
        token
    }
}

/// Read at most `n` SSE data frames matching `event: agui` off a raw
/// HTTP response body. Returns the decoded JSON payloads in order.
async fn read_n_agui_frames(
    response: reqwest::Response,
    n: usize,
    deadline: Duration,
) -> Vec<String> {
    let mut buffer = String::new();
    let mut out = Vec::new();
    let mut stream = response.bytes_stream();

    timeout(deadline, async {
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.expect("stream chunk");
            buffer.push_str(str::from_utf8(&bytes).expect("utf-8 frames"));

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
                    if out.len() >= n {
                        return;
                    }
                }
            }
        }
    })
    .await
    .expect("deadline elapsed while reading SSE frames");

    out
}

/// Full round-trip over HTTP: an authenticated owner subscribes and
/// receives every event the broadcast sink publishes, in order, with
/// the AG-UI wire format (`event: agui`, JSON data tagged by `type`).
#[tokio::test(flavor = "multi_thread")]
async fn broadcast_sink_events_reach_authenticated_owner() {
    let fx = Fixture::spawn().await;
    let run_id = "run_e2e_fanout";

    let _producer = fx.resources.agui_registry.register(run_id, fx.owner);

    let client = Client::builder().no_gzip().build().expect("client");
    let response = client
        .get(fx.url(run_id))
        .header(AUTHORIZATION, format!("Bearer {}", fx.token))
        .header(ACCEPT, "text/event-stream")
        .send()
        .await
        .expect("SSE subscribe");
    assert!(
        response.status().is_success(),
        "status = {}",
        response.status()
    );

    sleep(Duration::from_millis(50)).await;

    let sink = BroadcastSink::new(
        (*fx.resources.agui_registry).clone(),
        AgUiEventFilter::default(),
    );
    sink.emit(&AgUiEvent::run_started(run_id, Some("thread_1")))
        .await;
    sink.emit(&AgUiEvent::step_started(run_id, "dispatch"))
        .await;
    sink.emit(&AgUiEvent::step_finished(run_id, "dispatch"))
        .await;
    sink.emit(&AgUiEvent::run_finished(run_id)).await;

    let frames = read_n_agui_frames(response, 4, Duration::from_secs(3)).await;
    assert_eq!(frames.len(), 4, "frames = {frames:?}");

    let kinds: Vec<AgUiEventKind> = frames
        .iter()
        .map(|raw| {
            serde_json::from_str::<AgUiEvent>(raw)
                .expect("decode")
                .kind()
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            AgUiEventKind::RunStarted,
            AgUiEventKind::StepStarted,
            AgUiEventKind::StepFinished,
            AgUiEventKind::RunFinished,
        ]
    );
}

/// Regression for the P0: a request with no `Authorization` header
/// MUST return 401. The run exists and the caller knows the id — the
/// only thing between them and the PII stream is the JWT gate.
#[tokio::test(flavor = "multi_thread")]
async fn missing_authorization_header_is_401() {
    let fx = Fixture::spawn().await;
    let run_id = "run_e2e_no_auth";
    let _producer = fx.resources.agui_registry.register(run_id, fx.owner);

    let response = Client::new()
        .get(fx.url(run_id))
        .send()
        .await
        .expect("HTTP send");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Regression for the P0: a request carrying a malformed bearer token
/// MUST return 401.
#[tokio::test(flavor = "multi_thread")]
async fn malformed_bearer_token_is_401() {
    let fx = Fixture::spawn().await;
    let run_id = "run_e2e_bad_token";
    let _producer = fx.resources.agui_registry.register(run_id, fx.owner);

    let response = Client::new()
        .get(fx.url(run_id))
        .header(AUTHORIZATION, "Bearer not-a-real-jwt")
        .send()
        .await
        .expect("HTTP send");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Regression for the P0: even with a valid JWT, a user who does NOT
/// own the run MUST receive 403. Defends against cross-user and
/// cross-tenant read by `run_id` guess/leak.
#[tokio::test(flavor = "multi_thread")]
async fn authenticated_but_wrong_user_is_403() {
    let fx = Fixture::spawn().await;
    let run_id = "run_e2e_wrong_user";
    let _producer = fx.resources.agui_registry.register(run_id, fx.owner);

    let other_token = fx.second_user_token("agui-e2e-other@example.com").await;

    let response = Client::new()
        .get(fx.url(run_id))
        .header(AUTHORIZATION, format!("Bearer {other_token}"))
        .send()
        .await
        .expect("HTTP send");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// Subscribing to a run that was never registered returns 404 so the
/// channel adapter can surface a user-facing error rather than hang.
#[tokio::test(flavor = "multi_thread")]
async fn unknown_run_returns_not_found() {
    let fx = Fixture::spawn().await;

    let response = Client::new()
        .get(fx.url("run_that_does_not_exist"))
        .header(AUTHORIZATION, format!("Bearer {}", fx.token))
        .send()
        .await
        .expect("HTTP send");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// A client that subscribes AFTER the pipeline has already emitted events receives the replay backlog over HTTP.
///
/// Events land in order, before the client starts seeing live
/// events. This is the fix for the P1 concern about reconnecting
/// clients losing early events.
#[tokio::test(flavor = "multi_thread")]
async fn late_subscribers_receive_replay_over_http() {
    let fx = Fixture::spawn().await;
    let run_id = "run_e2e_replay";
    let _producer = fx.resources.agui_registry.register(run_id, fx.owner);

    // Emit events BEFORE the client subscribes — the replay buffer
    // captures them and the handler flushes them on connect.
    let sink = BroadcastSink::new(
        (*fx.resources.agui_registry).clone(),
        AgUiEventFilter::default(),
    );
    sink.emit(&AgUiEvent::run_started(run_id, Some("thread_1")))
        .await;
    sink.emit(&AgUiEvent::step_started(run_id, "dispatch"))
        .await;
    sink.emit(&AgUiEvent::step_finished(run_id, "dispatch"))
        .await;

    let client = Client::builder().no_gzip().build().expect("client");
    let response = client
        .get(fx.url(run_id))
        .header(AUTHORIZATION, format!("Bearer {}", fx.token))
        .header(ACCEPT, "text/event-stream")
        .send()
        .await
        .expect("SSE subscribe");
    assert!(response.status().is_success());

    // Subscribe triggers a backlog flush. Then emit one live event
    // so we can confirm the live stream still works after replay.
    sleep(Duration::from_millis(50)).await;
    sink.emit(&AgUiEvent::run_finished(run_id)).await;

    let frames = read_n_agui_frames(response, 4, Duration::from_secs(3)).await;
    assert_eq!(frames.len(), 4, "frames = {frames:?}");
    let kinds: Vec<AgUiEventKind> = frames
        .iter()
        .map(|raw| {
            serde_json::from_str::<AgUiEvent>(raw)
                .expect("decode")
                .kind()
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            AgUiEventKind::RunStarted,
            AgUiEventKind::StepStarted,
            AgUiEventKind::StepFinished,
            AgUiEventKind::RunFinished,
        ]
    );
}

/// Events the filter denies are dropped inside the sink and never
/// surface on the HTTP stream — even for the owner.
#[tokio::test(flavor = "multi_thread")]
async fn filtered_events_do_not_cross_http_boundary() {
    let fx = Fixture::spawn().await;
    let run_id = "run_e2e_filter";
    let _producer = fx.resources.agui_registry.register(run_id, fx.owner);

    let client = Client::builder().no_gzip().build().expect("client");
    let response = client
        .get(fx.url(run_id))
        .header(AUTHORIZATION, format!("Bearer {}", fx.token))
        .send()
        .await
        .expect("SSE subscribe");
    assert!(response.status().is_success());
    sleep(Duration::from_millis(50)).await;

    let filter = AgUiEventFilter::deny_all()
        .with(AgUiEventKind::RunStarted)
        .with(AgUiEventKind::RunFinished);
    let sink = BroadcastSink::new((*fx.resources.agui_registry).clone(), filter);

    sink.emit(&AgUiEvent::run_started(run_id, None)).await;
    sink.emit(&AgUiEvent::step_started(run_id, "prompt_assembly"))
        .await;
    sink.emit(&AgUiEvent::step_finished(run_id, "prompt_assembly"))
        .await;
    sink.emit(&AgUiEvent::run_finished(run_id)).await;

    let frames = read_n_agui_frames(response, 2, Duration::from_secs(3)).await;
    assert_eq!(frames.len(), 2);
    assert!(frames[0].contains("RUN_STARTED"));
    assert!(frames[1].contains("RUN_FINISHED"));
    assert!(frames.iter().all(|f| !f.contains("STEP_STARTED")));
}
