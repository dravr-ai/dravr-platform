// ABOUTME: What the Cloud Tasks runner puts on the queue — the task's name, target, deadline, token and body
// ABOUTME: Drives CloudTasksRunner::enqueue against a stub queue, so the create call is asserted field by field
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The create call is the whole contract with the queue (registre#126).
//!
//! Everything the delivery depends on is decided here: the URL the task is
//! delivered to, the identity and audience of the token it carries, the
//! deadline Cloud Tasks waits before giving up, the tenant the delivery
//! claims under, and the name that makes a retried create idempotent. None
//! of it is observable in production before it is wrong, so each field is
//! read back off the stub.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod helpers;

use std::time::Duration;

use axum::http::StatusCode;
use pierre_core::errors::ErrorCode;
use pierre_core::models::TenantId;
use pierre_mcp_server::services::turn_runner::CloudTasksRunner;
use serde_json::Value;
use tokio::net::TcpListener;

use crate::helpers::cloud_tasks_stub::{
    cloud_tasks_runner, QueueStub, QUEUE, SERVICE_ACCOUNT, TARGET,
};

const SA: &str = SERVICE_ACCOUNT;

fn runner(api_base: &str) -> CloudTasksRunner {
    cloud_tasks_runner(api_base, "http://127.0.0.1:1/certs", Duration::from_mins(4))
}

#[tokio::test]
async fn a_turn_is_enqueued_as_a_task_the_run_route_can_verify() {
    let stub = QueueStub::accepting();
    let base = stub.serve().await;
    let runner = runner(&base);
    let tenant = TenantId::generate();

    runner.enqueue(tenant, "row-7", 0).await.unwrap();

    let received = stub.received();
    assert_eq!(received.len(), 1, "one create call per enqueue");
    let call = &received[0];
    assert_eq!(
        call.queue_path, QUEUE,
        "the task lands on the configured queue"
    );
    assert_eq!(
        call.authorization.as_deref(),
        Some("Bearer ya29.enqueuer"),
        "the create call carries the token the provider minted"
    );

    let task = &call.body["task"];
    assert_eq!(task["name"], runner.task_name("row-7", 0));
    assert_eq!(task["dispatchDeadline"], "1020s", "watchdog plus a minute");
    let http = &task["httpRequest"];
    assert_eq!(http["url"], format!("{TARGET}/internal/turns/row-7/run"));
    assert_eq!(http["httpMethod"], "POST");
    assert_eq!(http["headers"]["Content-Type"], "application/json");
    assert_eq!(http["oidcToken"]["serviceAccountEmail"], SA);
    assert_eq!(
        http["oidcToken"]["audience"], TARGET,
        "the audience is the bare target the verifier checks"
    );
    let body: Value = call.delivery_body();
    assert_eq!(
        body["tenant_id"],
        tenant.to_string(),
        "the delivery carries the tenant the claim runs under"
    );
}

#[tokio::test]
async fn a_task_that_already_exists_is_success() {
    // The same create call retried after a dropped response: the queue has
    // the task, and telling the caller to try again would enqueue it twice.
    let stub = QueueStub::answering(StatusCode::CONFLICT);
    let base = stub.serve().await;
    runner(&base)
        .enqueue(TenantId::generate(), "row-7", 0)
        .await
        .unwrap();
    assert_eq!(stub.received().len(), 1);
}

#[tokio::test]
async fn a_refused_create_is_an_external_service_error() {
    let stub = QueueStub::answering(StatusCode::INTERNAL_SERVER_ERROR);
    let base = stub.serve().await;
    let err = runner(&base)
        .enqueue(TenantId::generate(), "row-7", 0)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ExternalServiceError);
    assert!(
        err.message.contains("500"),
        "the status reaches the log: {}",
        err.message
    );
}

#[tokio::test]
async fn an_unreachable_queue_is_unavailable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let err = runner(&format!("http://{addr}"))
        .enqueue(TenantId::generate(), "row-7", 0)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ExternalServiceUnavailable);
}
