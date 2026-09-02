// ABOUTME: Integration tests for the MCP Tasks extension (io.modelcontextprotocol/tasks)
// ABOUTME: Capability advertisement, -32021 gate, durable store isolation, and the tools/call handle path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// Why this exists
// ===============
// The tronc engine's own suite proves the extension mechanics against an
// in-memory store; nothing there proves that PIERRE advertises the extension,
// that its durable `mcp_tasks` store enforces owner isolation, or that a
// declared `tools/call` on `get_activities` actually comes back as a handle
// that later resolves to the real tool result. A regression in the
// `build_mcp_server` wiring (no TaskManager installed → the whole extension
// silently inert, which was the shipped state before this file) passes every
// engine test — only these assertions catch it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use dravr_tronc::mcp::tasks::{
    DetailedTask, Task, TaskId, TaskManager, TaskOwner, TaskPayload, TaskStatus, TaskStore,
};
use dravr_tronc::mcp::tool::ToolContext;
use pierre_mcp_schema::McpRequest;
use pierre_mcp_server::mcp::host_seams::build_mcp_server;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::mcp::task_store::PierreTaskStore;
use serde_json::{json, Value};
use tokio::time::sleep;

/// The reserved extension identifier, as the wire carries it.
const TASKS_EXTENSION: &str = "io.modelcontextprotocol/tasks";

/// `MissingRequiredClientCapabilityError` — the spec-reserved code returned
/// when a client invokes `tasks/*` without declaring the extension.
const MISSING_CLIENT_CAPABILITY: i64 = -32021;

/// Modern-era `_meta`, optionally declaring the tasks extension.
fn modern_meta(declare_tasks: bool) -> Value {
    let capabilities = if declare_tasks {
        json!({ "extensions": { TASKS_EXTENSION: {} } })
    } else {
        json!({})
    };
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": capabilities,
    })
}

/// Drive one request through the engine under `ctx` and return the response
/// as a JSON value.
async fn drive(resources: &Arc<ServerContext>, ctx: &ToolContext, request: Value) -> Result<Value> {
    let server = build_mcp_server(resources.clone());
    let request: McpRequest = serde_json::from_value(request)?;
    let response = server
        .handle_request_with_context(request, ctx)
        .await
        .expect("request with an id must produce a response");
    Ok(serde_json::to_value(response)?)
}

/// A `ToolContext` carrying a freshly seeded tenant member's identity.
async fn seeded_context(resources: &Arc<ServerContext>, email: &str) -> Result<ToolContext> {
    let (user, token) = common::create_test_tenant(resources, email).await?;
    drop(token);
    let repos = resources.coach.database.repositories();
    let tenants = repos.tenants.list_for_user(user.id).await?;
    let tenant_id = tenants.first().expect("seeded user owns a tenant").id;
    Ok(ToolContext::new()
        .with_user(user.id.to_string())
        .with_tenant(tenant_id.to_string()))
}

#[tokio::test]
async fn test_server_discover_advertises_tasks_extension() -> Result<()> {
    common::init_server_config();
    let resources = common::create_test_server_resources().await?;
    // server/discover is the extension's advertisement surface — legacy
    // initialize deliberately omits it, since the extension exists only in
    // the modern era.
    let body = drive(
        &resources,
        &ToolContext::default(),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": {}
        }),
    )
    .await?;

    let extensions = &body["result"]["capabilities"]["extensions"];
    assert!(
        extensions.get(TASKS_EXTENSION).is_some(),
        "server/discover must advertise the tasks extension once a TaskManager is installed; got: {extensions}"
    );
    Ok(())
}

#[tokio::test]
async fn test_tasks_get_without_declaration_hits_capability_gate() -> Result<()> {
    common::init_server_config();
    let resources = common::create_test_server_resources().await?;
    let body = drive(
        &resources,
        &ToolContext::default(),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tasks/get",
            "params": {
                "_meta": modern_meta(false),
                "taskId": "irrelevant"
            }
        }),
    )
    .await?;

    assert_eq!(
        body["error"]["code"].as_i64(),
        Some(MISSING_CLIENT_CAPABILITY),
        "tasks/get without the declared extension must return -32021; got: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn test_unknown_task_reads_as_absent() -> Result<()> {
    common::init_server_config();
    let resources = common::create_test_server_resources().await?;
    let ctx = seeded_context(&resources, "tasks-absent@example.com").await?;
    let body = drive(
        &resources,
        &ctx,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tasks/get",
            "params": {
                "_meta": modern_meta(true),
                "taskId": "no-such-task"
            }
        }),
    )
    .await?;

    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("not found"),
        "an unknown task id must read as absent; got: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn test_durable_store_round_trip_owner_isolation_and_expiry() -> Result<()> {
    common::init_server_config();
    let resources = common::create_test_server_resources().await?;
    let store: Arc<dyn TaskStore> = Arc::new(PierreTaskStore::new(
        resources.common.repos.mcp_tasks.clone(),
    ));
    let manager = TaskManager::new(store.clone());

    let owner = TaskOwner {
        user_id: Some("11111111-1111-4111-8111-111111111111".to_owned()),
        tenant_id: Some("22222222-2222-4222-8222-222222222222".to_owned()),
    };
    let stranger = TaskOwner {
        user_id: Some("33333333-3333-4333-8333-333333333333".to_owned()),
        tenant_id: Some("44444444-4444-4444-8444-444444444444".to_owned()),
    };

    // Round trip: create working, read back identical timestamps and pacing.
    let task_id = TaskId::new("round-trip-task");
    let created = manager.create(&owner, task_id.clone()).await?;
    let fetched = manager.get(&owner, &task_id).await?;
    assert_eq!(fetched.status(), TaskStatus::Working);
    assert_eq!(fetched.task.created_at, created.created_at);
    assert_eq!(fetched.task.poll_interval_ms, created.poll_interval_ms);

    // Owner isolation: the same id under a different owner reads as absent —
    // both through the store and through the manager.
    assert!(store.get(&stranger, &task_id).await?.is_none());
    assert!(manager.get(&stranger, &task_id).await.is_err());

    // Completion carries the result payload back out of the durable store.
    let mut result = serde_json::Map::new();
    result.insert(
        "content".to_owned(),
        json!([{ "type": "text", "text": "42" }]),
    );
    manager.complete(&owner, &task_id, result).await?;
    let done = manager.get(&owner, &task_id).await?;
    assert_eq!(done.status(), TaskStatus::Completed);
    match &done.payload {
        TaskPayload::Completed { result } => {
            assert_eq!(result["content"][0]["text"], "42");
        }
        other => panic!("expected completed payload, got {other:?}"),
    }

    // A terminal task refuses further transitions.
    assert!(manager.cancel(&owner, &task_id).await.is_err());

    // Expiry: a 1ms-TTL task disappears from reads and is swept from the table.
    let expiring = DetailedTask::new(
        Task::new(TaskId::new("expiring-task"), Some(1), Some(1_000)),
        TaskPayload::Working,
    );
    store.create(&owner, expiring).await?;
    sleep(Duration::from_millis(50)).await;
    assert!(store
        .get(&owner, &TaskId::new("expiring-task"))
        .await?
        .is_none());
    let swept = store.sweep_expired().await?;
    assert!(
        swept >= 1,
        "the expired task must be swept; removed {swept}"
    );
    Ok(())
}

/// Drive one declared `tools/call` that must answer with a handle, poll it to
/// completion, and return the settled `tasks/get` body.
async fn handle_round_trip(
    resources: &Arc<ServerContext>,
    ctx: &ToolContext,
    tool: &str,
    arguments: Value,
) -> Result<Value> {
    let body = drive(
        resources,
        ctx,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "_meta": modern_meta(true),
                "name": tool,
                "arguments": arguments
            }
        }),
    )
    .await?;
    assert_eq!(
        body["result"]["resultType"].as_str(),
        Some("task"),
        "a declared {tool} call over budget must answer with a task handle; got: {body}"
    );
    assert_eq!(body["result"]["status"].as_str(), Some("working"));
    let task_id = body["result"]["taskId"]
        .as_str()
        .expect("handle carries a taskId")
        .to_owned();

    // Poll tasks/get until the detached call settles the task.
    for _ in 0..100 {
        let poll = drive(
            resources,
            ctx,
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tasks/get",
                "params": {
                    "_meta": modern_meta(true),
                    "taskId": task_id
                }
            }),
        )
        .await?;
        if poll["result"]["status"].as_str() == Some("completed") {
            return Ok(poll);
        }
        sleep(Duration::from_millis(100)).await;
    }
    panic!("{tool} task must complete within the polling window");
}

/// Assert a settled `tasks/get` body carries the real tools/call result shape
/// — a content array a returns-nothing stub could not produce.
fn assert_settled_content(settled: &Value) {
    assert_eq!(settled["result"]["resultType"].as_str(), Some("complete"));
    let content = settled["result"]["result"]["content"]
        .as_array()
        .expect("completed task carries the tool result's content array");
    assert!(
        !content.is_empty(),
        "tool result content must be non-empty; got: {settled}"
    );
}

/// The handle path and the inline fast path live in ONE test because the
/// budget knob is a process-global env var — two parallel test threads
/// flipping it would race. Sequential in one function, there is no window
/// where another declared call can observe the zero budget.
#[tokio::test]
async fn test_declared_tools_call_handle_resolves_then_fast_path_answers_inline() -> Result<()> {
    common::init_server_config();
    let resources = common::create_test_server_resources().await?;
    let ctx = seeded_context(&resources, "tasks-handle@example.com").await?;

    // A zero fast-path budget converts every declared task-capable call into
    // a handle, making the asynchronous path deterministic in a test.
    env::set_var("PIERRE_MCP_TASK_FAST_PATH_MS", "0");
    let settled =
        handle_round_trip(&resources, &ctx, "get_activities", json!({ "limit": 1 })).await?;
    // The write-path conversion: compute_training_history's persistence is an
    // upsert keyed by date, so work settling behind a handle after the client
    // stops polling leaves harmless, re-computable state.
    let history =
        handle_round_trip(&resources, &ctx, "compute_training_history", json!({})).await?;
    env::remove_var("PIERRE_MCP_TASK_FAST_PATH_MS");

    assert_settled_content(&settled);
    assert_settled_content(&history);

    // Fast path, now under the default 10s budget: the same declared call
    // must answer inline — no task handle, no polling round trip.
    let body = drive(
        &resources,
        &ctx,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "_meta": modern_meta(true),
                "name": "get_activities",
                "arguments": { "limit": 1 }
            }
        }),
    )
    .await?;

    assert_eq!(
        body["result"]["resultType"].as_str(),
        Some("complete"),
        "an under-budget declared call must answer inline; got: {body}"
    );
    let content = body["result"]["content"]
        .as_array()
        .expect("inline tool result carries its content array");
    assert!(!content.is_empty());
    Ok(())
}

/// `active_tasks` is what makes `subscriptions/listen` able to emit at all: the
/// trait defaults it to an empty list, so a store that forgets to override it
/// opens a subscription that stays silent forever — a success no `is_ok()`
/// assertion can catch. This asserts the contract the watcher relies on:
/// non-terminal tasks are listed, terminal ones are not, another owner's work
/// never appears, and an expired task drops out.
#[tokio::test]
async fn test_active_tasks_lists_only_this_owners_live_work() -> Result<()> {
    common::init_server_config();
    let resources = common::create_test_server_resources().await?;
    let store: Arc<dyn TaskStore> = Arc::new(PierreTaskStore::new(
        resources.common.repos.mcp_tasks.clone(),
    ));

    let owner = TaskOwner {
        user_id: Some("55555555-5555-4555-8555-555555555555".to_owned()),
        tenant_id: Some("66666666-6666-4666-8666-666666666666".to_owned()),
    };
    let stranger = TaskOwner {
        user_id: Some("77777777-7777-4777-8777-777777777777".to_owned()),
        tenant_id: Some("88888888-8888-4888-8888-888888888888".to_owned()),
    };

    // Two live tasks for the owner, one for a stranger.
    for id in ["live-a", "live-b"] {
        store
            .create(
                &owner,
                DetailedTask::new(
                    Task::new(TaskId::new(id), Some(600_000), Some(1_000)),
                    TaskPayload::Working,
                ),
            )
            .await?;
    }
    store
        .create(
            &stranger,
            DetailedTask::new(
                Task::new(TaskId::new("someone-elses"), Some(600_000), Some(1_000)),
                TaskPayload::Working,
            ),
        )
        .await?;

    let active = store.active_tasks(&owner).await?;
    let ids: Vec<&str> = active.iter().map(|t| t.task.task_id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["live-a", "live-b"],
        "both live tasks must be listed, ordered stably, and the stranger's must not appear"
    );

    // A completed task leaves the active set — this is how the watcher learns a
    // task settled, so listing it would stall the transition it exists to emit.
    let manager = TaskManager::new(store.clone());
    let mut result = serde_json::Map::new();
    result.insert(
        "content".to_owned(),
        json!([{ "type": "text", "text": "ok" }]),
    );
    manager
        .complete(&owner, &TaskId::new("live-a"), result)
        .await?;
    let after = store.active_tasks(&owner).await?;
    let ids: Vec<&str> = after.iter().map(|t| t.task.task_id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["live-b"],
        "a completed task must drop out of the active set"
    );

    // An expired task drops out too, even though it never reached a terminal
    // status — otherwise a dead handle would be watched forever.
    store
        .create(
            &owner,
            DetailedTask::new(
                Task::new(TaskId::new("expiring-live"), Some(1), Some(1_000)),
                TaskPayload::Working,
            ),
        )
        .await?;
    sleep(Duration::from_millis(50)).await;
    let ids: Vec<String> = store
        .active_tasks(&owner)
        .await?
        .iter()
        .map(|t| t.task.task_id.as_str().to_owned())
        .collect();
    assert!(
        !ids.iter().any(|id| id == "expiring-live"),
        "an expired task must not be watched; got {ids:?}"
    );

    // An unauthenticated owner owns nothing and so watches nothing.
    let anonymous = TaskOwner {
        user_id: None,
        tenant_id: None,
    };
    assert!(
        store.active_tasks(&anonymous).await?.is_empty(),
        "an unauthenticated owner must see no tasks"
    );
    Ok(())
}
