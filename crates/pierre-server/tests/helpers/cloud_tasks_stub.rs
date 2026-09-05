// ABOUTME: A local stand-in for the Cloud Tasks API — records every task create call and answers a chosen status
// ABOUTME: What a test reads back to prove which task a turn was enqueued as, and what a delivery test replays
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as Base64Standard;
use base64::Engine as _;
use pierre_core::errors::AppResult;
use pierre_core::gcp_token::TokenProvider;
use pierre_mcp_server::services::turn_runner::{CloudTasksConfig, CloudTasksRunner, TurnRunner};
use serde_json::Value;
use tokio::net::TcpListener;

/// The queue, target and service account the stub runners are configured with.
pub const QUEUE: &str =
    "projects/dravr-dev/locations/northamerica-northeast1/queues/dravr-mcp-server-turns";
pub const TARGET: &str = "https://dravr-mcp-server-api-123456.northamerica-northeast1.run.app";
pub const SERVICE_ACCOUNT: &str = "dravr-app@dravr-dev.iam.gserviceaccount.com";

/// A token provider that never talks to a metadata server.
pub struct StaticToken;

#[async_trait]
impl TokenProvider for StaticToken {
    async fn access_token(&self) -> AppResult<String> {
        Ok("ya29.enqueuer".to_owned())
    }
}

/// One create call as the stand-in received it.
#[derive(Debug, Clone)]
pub struct ReceivedTask {
    /// `projects/…/queues/…` the task was created on.
    pub queue_path: String,
    /// The `Authorization` header the create call carried.
    pub authorization: Option<String>,
    /// The request body, verbatim.
    pub body: Value,
}

impl ReceivedTask {
    /// The task's name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.body["task"]["name"].as_str().unwrap_or_default()
    }

    /// The URL the task would be delivered to.
    #[must_use]
    pub fn url(&self) -> &str {
        self.body["task"]["httpRequest"]["url"]
            .as_str()
            .unwrap_or_default()
    }

    /// The row id in the delivery URL.
    #[must_use]
    pub fn turn_id(&self) -> String {
        self.url()
            .trim_start_matches(&format!("{TARGET}/internal/turns/"))
            .trim_end_matches("/run")
            .to_owned()
    }

    /// The JSON body the delivery would carry.
    #[must_use]
    pub fn delivery_body(&self) -> Value {
        let raw = self.body["task"]["httpRequest"]["body"]
            .as_str()
            .unwrap_or_default();
        let bytes = Base64Standard.decode(raw).expect("base64 body");
        serde_json::from_slice(&bytes).expect("json body")
    }
}

/// The stand-in queue: answers every create with `status` and keeps the calls.
pub struct QueueStub {
    status: StatusCode,
    received: Mutex<Vec<ReceivedTask>>,
}

impl QueueStub {
    /// A queue that accepts everything.
    #[must_use]
    pub fn accepting() -> Arc<Self> {
        Self::answering(StatusCode::OK)
    }

    /// A queue that answers every create with `status`.
    #[must_use]
    pub fn answering(status: StatusCode) -> Arc<Self> {
        Arc::new(Self {
            status,
            received: Mutex::new(Vec::new()),
        })
    }

    /// Every create call so far, in order.
    #[must_use]
    pub fn received(&self) -> Vec<ReceivedTask> {
        self.received.lock().expect("mutex").clone()
    }

    /// Serve this stub from a local listener and return its base URL.
    pub async fn serve(self: &Arc<Self>) -> String {
        async fn handler(
            State(stub): State<Arc<QueueStub>>,
            Path((project, location, queue)): Path<(String, String, String)>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> impl IntoResponse {
            stub.received.lock().expect("mutex").push(ReceivedTask {
                queue_path: format!("projects/{project}/locations/{location}/queues/{queue}"),
                authorization: headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned),
                body,
            });
            (stub.status, Json(serde_json::json!({ "name": "accepted" })))
        }
        let app = Router::new()
            .route(
                "/projects/{project}/locations/{location}/queues/{queue}/tasks",
                post(handler),
            )
            .with_state(Arc::clone(self));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        format!("http://{addr}")
    }
}

/// A Cloud Tasks runner over the stub at `api_base`, verifying tokens against
/// the certificate map at `certs_url`, waiting `claim_wait` for a blocked
/// claim, with the production turn watchdog.
#[must_use]
pub fn cloud_tasks_runner(
    api_base: &str,
    certs_url: &str,
    claim_wait: Duration,
) -> CloudTasksRunner {
    CloudTasksRunner::new(
        CloudTasksConfig {
            queue: QUEUE.to_owned(),
            target_url: TARGET.to_owned(),
            service_account: SERVICE_ACCOUNT.to_owned(),
            claim_wait,
            api_base: api_base.to_owned(),
            certs_url: certs_url.to_owned(),
        },
        Arc::new(StaticToken),
        Duration::from_mins(16),
    )
    .expect("a watchdog of 960s fits the Cloud Tasks deadline")
}

/// The same runner, as the `TurnRunner` a server context takes.
#[must_use]
pub fn cloud_tasks_turn_runner(
    api_base: &str,
    certs_url: &str,
    claim_wait: Duration,
) -> Arc<TurnRunner> {
    Arc::new(TurnRunner::CloudTasks(Box::new(cloud_tasks_runner(
        api_base, certs_url, claim_wait,
    ))))
}
