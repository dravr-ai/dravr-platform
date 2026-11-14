// ABOUTME: A finished backfill's rollup warm never starts a capture on the athlete's other connections
// ABOUTME: Pins the carnet#1478 fix: one provider's capture landing on a shallow cache asks no provider again

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1478. A backfill that persisted rows warms the athlete's default
//! training-history window. While the stored history is too shallow for that
//! window, the compute used to ask every connection for the missing depth.
//! The finished job's in-flight claim covered only its own provider, so the
//! ask started a backfill on each of the athlete's other connections, whose
//! own warms asked again: on dev, one athlete with three providers connected
//! was paged about once a second per provider for eleven hours.
//!
//! The scraper here is a loopback stand-in that serves each seeded session one
//! recent ride and records every activity-list read by session, so the reads
//! each provider received are counted, not inferred.
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the feature.
#![cfg(all(feature = "tools-data", feature = "provider-sciotte"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionType, TenantId};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::BackfillJobKind;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::activity_backfill::{
    provider_tenant_id_str, spawn_activity_backfill, ActivityBackfillJob,
};
use pierre_tool_runtime::implementations::data_helpers::historical_backfill_fetch_limit;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::DEFAULT_BACKFILL_DAYS;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::{sleep, Instant};
use uuid::Uuid;

use crate::common;
use crate::helpers::sciotte_mock::seed_scrape_session;

/// Provider A: the connection whose backfill the test starts.
const PROVIDER_A: &str = oauth_providers::SCIOTTE;
const SESSION_A: &str = "warm-loop-session-a";
/// Provider B: the athlete's other connection, which nothing asked for.
const PROVIDER_B: &str = oauth_providers::SCIOTTE_TRAININGPEAKS;
const SESSION_B: &str = "warm-loop-session-b";

/// How long A's detached backfill gets to finish its job row.
const RUN_BUDGET: Duration = Duration::from_secs(30);
/// How long, once A's job is finished, a stray capture on B gets to show up.
const STRAY_WINDOW: Duration = Duration::from_secs(2);

/// Every activity-list read the stand-in served, as the session it read.
type Reads = Arc<Mutex<Vec<String>>>;

/// A stand-in scraper whose account, for every session, holds one ride five
/// days old: far inside the 90-day default window plus its warm-up, so a warm
/// over it stays partial and the provider holds nothing older.
async fn spawn_scraper() -> (String, Reads) {
    let reads: Reads = Arc::new(Mutex::new(Vec::new()));
    let ride_start = (Utc::now() - ChronoDuration::days(5)).to_rfc3339();
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|Json(body): Json<Value>| async move {
                Json(json!({ "session_id": body["session"]["session_id"] }))
            }),
        )
        .route(
            "/api/athlete",
            get(|| async { Json(json!({ "display_name": "Warm Loop" })) }),
        )
        .route(
            "/api/activities",
            get(move |State(reads): State<Reads>, headers: HeaderMap| {
                let ride_start = ride_start.clone();
                async move {
                    let session = headers
                        .get("x-session-id")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    reads.lock().unwrap().push(session.clone());
                    Json(json!({
                        "count": 1,
                        "activities": [{
                            "id": format!("ride-{session}"),
                            "name": "Sortie",
                            "sport_type": "ride",
                            "start_date": ride_start,
                            "duration_seconds": 3600,
                            "provider": "strava",
                            "distance_meters": 30000.0
                        }],
                        "head_complete": true
                    }))
                }
            }),
        )
        .with_state(Arc::clone(&reads));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), reads)
}

fn reads_of(reads: &Reads, session: &str) -> usize {
    reads
        .lock()
        .unwrap()
        .iter()
        .filter(|read| read.as_str() == session)
        .count()
}

/// Whether the ledger holds a backfill job for `(user, provider)` in `tenant`,
/// read from the table itself.
async fn job_on_file(database: &Database, user_id: Uuid, tenant: TenantId, provider: &str) -> bool {
    const SQL: &str = "SELECT COUNT(*) FROM activity_backfill_jobs \
                       WHERE user_id = $1 AND tenant_id = $2 AND provider = $3";
    let user = user_id.to_string();
    let rows: i64 = match database.backend() {
        DatabaseBackend::SQLite(db) => sqlx::query_scalar(SQL)
            .bind(&user)
            .bind(tenant)
            .bind(provider)
            .fetch_one(db.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => sqlx::query_scalar(SQL)
            .bind(&user)
            .bind(tenant)
            .bind(provider)
            .fetch_one(db.pool())
            .await
            .unwrap(),
    };
    rows > 0
}

/// Connect `provider` for the athlete under a live scrape session `session`.
async fn connect(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    provider: &str,
    session: &str,
) {
    seed_scrape_session(resources, user_id, tenant, provider, session).await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, provider, &ConnectionType::Manual, None)
        .await
        .unwrap();
}

/// An athlete with two capturable connections and a cache too shallow for the
/// default window. One backfill on A runs to completion: its warm computes a
/// partial window and, on the fixed code, asks no provider for the missing
/// depth — B is never read and no B job is recorded, and A is read once.
///
/// Before the fix, A's warm started a backfill on B (A's own in-flight claim
/// only declined A), and B's warm asked A again: the cross-provider loop of
/// carnet#1478.
#[tokio::test]
async fn a_finished_backfills_warm_starts_no_capture_on_another_provider() {
    common::init_server_config();
    common::init_test_http_clients();
    let (scraper, reads) = spawn_scraper().await;
    let resources = common::create_test_server_resources_with_sciotte(&scraper)
        .await
        .unwrap();
    let (user_id, _user, tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("warm-loop-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    connect(&resources, user_id, tenant, PROVIDER_A, SESSION_A).await;
    connect(&resources, user_id, tenant, PROVIDER_B, SESSION_B).await;
    let database = Arc::clone(&resources.agent.database);
    let runtime: Arc<dyn ToolRuntime> = resources;

    // The depth the training-history capture asks for: the default window and
    // a generous warm-up, which a five-day-old ride never reaches.
    let after = (Utc::now() - ChronoDuration::days(DEFAULT_BACKFILL_DAYS + 90)).timestamp();
    let started = spawn_activity_backfill(ActivityBackfillJob {
        resources: Arc::clone(&runtime),
        user_id,
        tenant_id: tenant,
        tenant_id_str: provider_tenant_id_str(tenant),
        provider_name: PROVIDER_A.to_owned(),
        query_params: ActivityQueryParams {
            after: Some(after),
            limit: Some(historical_backfill_fetch_limit()),
            ..ActivityQueryParams::default()
        },
        pierre_conversation_id: None,
        kind: BackfillJobKind::Backfill,
        turn_id: None,
        asked_message_id: None,
    })
    .await;
    assert!(started, "a fresh pair starts a backfill");

    // A's job row is removed only after its run, warm included, returns.
    let deadline = Instant::now() + RUN_BUDGET;
    while job_on_file(&database, user_id, tenant, PROVIDER_A).await {
        assert!(
            Instant::now() < deadline,
            "A's backfill never finished its job within {RUN_BUDGET:?}"
        );
        sleep(Duration::from_millis(50)).await;
    }
    let rows = runtime
        .repos()
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant,
            Some(PROVIDER_A),
            Utc::now() - ChronoDuration::days(30),
            Utc::now() + ChronoDuration::days(1),
            50,
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "A's capture landed, so its warm ran");

    // A capture the warm asked for records its B row before A's own row goes,
    // and reads B before that row goes in turn, so either shows here at once.
    // The window afterwards catches a capture started later still.
    let quiet_until = Instant::now() + STRAY_WINDOW;
    loop {
        assert!(
            !job_on_file(&database, user_id, tenant, PROVIDER_B).await,
            "bug: A's warm started a backfill on B (carnet#1478)"
        );
        assert_eq!(
            reads_of(&reads, SESSION_B),
            0,
            "bug: A's warm read B's activity list (carnet#1478)"
        );
        if Instant::now() >= quiet_until {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        reads_of(&reads, SESSION_A),
        1,
        "bug: A was asked again after its own backfill finished (carnet#1478)"
    );
}
