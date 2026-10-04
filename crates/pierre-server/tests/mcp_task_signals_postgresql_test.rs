// ABOUTME: Tests for the Postgres LISTEN/NOTIFY carrier of MCP task cancels and input across replicas
// ABOUTME: Two TaskManagers over one database stand in for two pierre replicas behind a load balancer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `PostgreSQL` task signal carrier tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the default-feature SQLite lane trips `missing_docs`.
//
// Why this exists
// ===============
// tronc proves its TaskSignalBus seam with an in-process loopback. Nothing
// there proves that pierre installs a carrier on PostgreSQL, that pg_notify
// really reaches another connection's LISTEN, or that the payload survives
// the trip. Before this carrier, a tasks/cancel answered by another replica
// reached the worker only through a 2s store poll, and a tasks/update for
// another replica's task was refused outright. The SQLite lane has no second
// process to signal, so these tests run on the PostgreSQL lane only.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "postgresql")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use dravr_tronc::mcp::tasks::{
    TaskError, TaskManager, TaskOptions, TaskOwner, TaskSignal, TaskSignalBus, TaskStatus,
    TaskStore,
};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_mcp_server::mcp::task_signals::{PgTaskSignalBus, TaskSignals};
use pierre_mcp_server::mcp::task_store::PierreTaskStore;
use serde_json::{json, Map};
use tokio::task::yield_now;
use tokio::time::timeout;

/// Long enough for a NOTIFY to cross a loaded CI database; a lost signal
/// fails the test rather than hanging it.
const SIGNAL_DEADLINE: Duration = Duration::from_secs(10);

fn owner() -> TaskOwner {
    TaskOwner {
        user_id: Some("11111111-1111-4111-8111-111111111111".to_owned()),
        tenant_id: Some("22222222-2222-4222-8222-222222222222".to_owned()),
    }
}

/// One replica: its own manager and carrier over the shared database,
/// listening as it would after minting its first task.
async fn replica(database: &Database, store: &Arc<dyn TaskStore>) -> Arc<TaskManager> {
    let signals = TaskSignals::for_database(database);
    let manager = Arc::new(signals.install(TaskManager::with_options(
        Arc::clone(store),
        TaskOptions::host_swept(),
    )));
    signals.ensure_listening(&manager).await;
    manager
}

/// The test database's pool. A carrier between replicas needs a database two
/// processes share, so a SQLite test database fails the test rather than
/// letting it pass having checked nothing.
fn postgres_pool(database: &Database) -> sqlx::PgPool {
    match database.backend() {
        DatabaseBackend::PostgreSQL(db) => db.pool().clone(),
        DatabaseBackend::SQLite(_) => {
            panic!("task signal tests need a PostgreSQL DATABASE_URL, got SQLite")
        }
    }
}

#[tokio::test]
async fn a_cancel_answered_by_one_replica_reaches_the_run_on_another() -> Result<()> {
    common::init_server_config();
    let resources = common::create_test_server_resources().await?;
    let database = &resources.agent.database;
    postgres_pool(database);
    let store: Arc<dyn TaskStore> = Arc::new(PierreTaskStore::new(
        resources.common.repos.mcp_tasks.clone(),
    ));
    let a = replica(database, &store).await;
    let b = replica(database, &store).await;

    let run = a.create(&owner()).await?;
    let token = run.cancellation();
    let cancelled = b.cancel(&owner(), run.id()).await?;

    assert_eq!(cancelled.status(), TaskStatus::Cancelled);
    timeout(SIGNAL_DEADLINE, token.cancelled())
        .await
        .expect("replica A's run must see the cancel replica B answered");
    Ok(())
}

#[tokio::test]
async fn input_answered_by_one_replica_reaches_the_operation_on_another() -> Result<()> {
    common::init_server_config();
    let resources = common::create_test_server_resources().await?;
    let database = &resources.agent.database;
    postgres_pool(database);
    let store: Arc<dyn TaskStore> = Arc::new(PierreTaskStore::new(
        resources.common.repos.mcp_tasks.clone(),
    ));
    let a = replica(database, &store).await;
    let b = replica(database, &store).await;

    let mut run = a.create(&owner()).await?;
    let id = run.id().clone();
    let mut requests = Map::new();
    requests.insert(
        "confirm".to_owned(),
        json!({ "method": "elicitation/create" }),
    );
    let operation = tokio::spawn(async move { run.request_input(requests).await });
    timeout(SIGNAL_DEADLINE, async {
        while b.get(&owner(), &id).await.unwrap().status() != TaskStatus::InputRequired {
            yield_now().await;
        }
    })
    .await
    .expect("the operation records its input request");

    let mut responses = Map::new();
    responses.insert("confirm".to_owned(), json!({ "action": "accept" }));
    let working = b.apply_input(&owner(), &id, responses.clone()).await?;
    assert_eq!(working.status(), TaskStatus::Working);

    let answers = timeout(SIGNAL_DEADLINE, operation)
        .await
        .expect("replica A's operation must receive the input replica B took")?;
    assert_eq!(answers?, responses);
    Ok(())
}

#[tokio::test]
async fn a_signal_too_large_for_notify_is_refused_before_it_is_sent() -> Result<()> {
    common::init_server_config();
    let resources = common::create_test_server_resources().await?;
    let pool = postgres_pool(&resources.agent.database);
    let bus = PgTaskSignalBus::new(pool);
    let mut responses = Map::new();
    responses.insert("blob".to_owned(), json!("x".repeat(8_000)));

    let refused = bus
        .publish(TaskSignal::Input {
            task_id: serde_json::from_value(json!("task-1"))?,
            responses,
        })
        .await;

    assert!(
        matches!(refused, Err(TaskError::Store(ref message)) if message.contains("NOTIFY carries at most")),
        "an oversized signal must be refused, not truncated: {refused:?}"
    );
    Ok(())
}
