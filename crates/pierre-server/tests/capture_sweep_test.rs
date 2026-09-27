// ABOUTME: The nightly capture refresh must flag a dead connection, not quietly fetch nothing
// ABOUTME: Pins the flag reaching the database, the snapshot coupling, the honest budget report, and the re-arm
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/admin/diagnostics/capture-staleness` gave a frozen capture a reader. This
//! is the actor: it re-fetches every live connection and, when one's credential
//! turns out to be gone, flips it to `needs_reauth` so the athlete's next turn
//! offers a reconnect link instead of silence.
//!
//! The assertions are deliberately about content and about persisted state. A
//! sweep that reported `flagged` while leaving the connection row `active` would
//! satisfy an `is_ok` test and leave the next turn exactly as silent as the
//! incident that prompted all of this.
//!
//! A flag is one attempt's verdict, and the sweep never walks a flagged
//! connection again, so the verdict is revisited where the next read happens:
//! a fetch the stored credential serves re-arms the connection. The second half
//! of this file pins that against a scraper stand-in whose answer the test
//! switches between a refused session and a served one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{Activity, ConnectionStatus, ConnectionType, TenantId};
use pierre_database::backends::factory::Database;
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::activity_fetch::{fetch_provider_activities, fetch_provider_head};
use pierre_tool_runtime::capture_sweep::{
    refresh_captures, RefreshOutcome, SweepBudget, DEFAULT_CONNECTION_LIMIT,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};
use crate::helpers::sciotte_mock::seed_sciotte_session;

/// One athlete with one connection, and the runtime the sweep runs through.
struct Fixture {
    runtime: Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant: TenantId,
}

/// Register `provider` for a fresh athlete and hand back the sweep's inputs.
async fn fixture_with_connection(provider: &str) -> Fixture {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenants = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, provider, &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    Fixture {
        runtime,
        user_id,
        tenant,
    }
}

/// The sweep's whole point: a connection whose credential is gone gets flagged,
/// and the flag lands in the database.
///
/// The athlete here has a registered connection and no token behind it, which is
/// what a lapsed session looks like to the authenticate path — it reports
/// auth-required rather than a transport error. Reporting the flag is not
/// enough: the reconnect link the athlete's next turn offers is rendered off the
/// persisted `status`, so a report-only sweep would change nothing at all.
#[tokio::test]
async fn the_sweep_flags_a_connection_whose_credential_is_gone() {
    let f = fixture_with_connection("strava").await;

    let report = refresh_captures(&f.runtime, SweepBudget::default())
        .await
        .expect("refresh report");

    let line = report
        .connections
        .iter()
        .find(|c| c.user_id == f.user_id.to_string() && c.provider == "strava")
        .expect("the connection was walked");
    assert!(
        matches!(&line.outcome, RefreshOutcome::Flagged { reason } if reason == "session_expired"),
        "expected an auth-shaped failure to flag, got {:?}",
        line.outcome
    );
    assert_eq!(report.attempted, 1);
    assert_eq!(report.flagged, 1);
    assert_eq!(report.failed, 0);
    assert!(report.completed, "one connection fits the budget");

    let connections = f
        .runtime
        .repos()
        .provider_connections
        .get_for_user(f.user_id, Some(f.tenant))
        .await
        .unwrap();
    assert_eq!(
        connections[0].status,
        ConnectionStatus::NeedsReauth,
        "the flag must be persisted, not merely reported"
    );
}

/// Flagging a connection drops it out of the snapshot the sweep walks, so the
/// next night does not spend another headless-browser scrape on a connection
/// that cannot succeed until the athlete acts.
///
/// This is the coupling to the staleness reader: both halves read one snapshot,
/// so a connection the reader has stopped counting is also one the refresher has
/// stopped retrying.
#[tokio::test]
async fn a_flagged_connection_is_not_swept_again() {
    let f = fixture_with_connection("sciotte").await;

    let first = refresh_captures(&f.runtime, SweepBudget::default())
        .await
        .expect("first sweep");
    assert_eq!(first.attempted, 1, "the live connection was attempted once");
    assert_eq!(first.flagged, 1);

    let second = refresh_captures(&f.runtime, SweepBudget::default())
        .await
        .expect("second sweep");
    assert_eq!(
        second.attempted, 0,
        "a flagged connection must not be re-attempted"
    );
    assert!(
        second.connections.is_empty(),
        "it has left the snapshot entirely, got {:?}",
        second.connections
    );
}

/// Stamp the connection's `connected_at`, as a reconnect does, at `at`.
async fn stamp_connected_at(f: &Fixture, provider: &str, at: DateTime<Utc>) {
    let sql = "UPDATE provider_connections SET connected_at = $1 \
               WHERE user_id = $2 AND tenant_id = $3 AND provider = $4";
    let (user, tenant) = (f.user_id.to_string(), f.tenant.to_string());
    let affected = match &**f.runtime.database() {
        Database::SQLite(db) => sqlx::query(sql)
            .bind(at)
            .bind(&user)
            .bind(&tenant)
            .bind(provider)
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => sqlx::query(sql)
            .bind(at)
            .bind(&user)
            .bind(&tenant)
            .bind(provider)
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
    };
    assert_eq!(affected, 1, "the connection row is stamped");
}

/// A reconnect that lands while the sweep's fetch is in flight is newer than
/// the failure that fetch reports, so the connection stays active. The sweep
/// must then not report it flagged: the operator reading the report would count
/// a disconnect the athlete never had.
///
/// The reconnect is stamped ahead of the sweep's start, which is exactly what
/// one that lands after the fetch began looks like to the guard.
#[tokio::test]
async fn a_connection_reconnected_while_its_fetch_ran_is_not_reported_flagged() {
    let f = fixture_with_connection("strava").await;
    stamp_connected_at(&f, "strava", Utc::now() + Duration::minutes(5)).await;

    let report = refresh_captures(&f.runtime, SweepBudget::default())
        .await
        .expect("refresh report");

    let line = report
        .connections
        .iter()
        .find(|c| c.user_id == f.user_id.to_string() && c.provider == "strava")
        .expect("the connection was walked");
    assert!(
        matches!(&line.outcome, RefreshOutcome::Failed { error } if error.contains("reconnect")),
        "a connection the guard left active is not a flag: {:?}",
        line.outcome
    );
    assert_eq!(report.flagged, 0);
    assert_eq!(report.failed, 1);

    let connections = f
        .runtime
        .repos()
        .provider_connections
        .get_for_user(f.user_id, Some(f.tenant))
        .await
        .unwrap();
    assert_eq!(connections[0].status, ConnectionStatus::Active);
}

/// A sweep that runs out of time says so, per connection and in the summary.
///
/// Silence about the connections it never reached is the exact failure this
/// whole subsystem exists to end: a report that looked complete while a capture
/// went untouched would put the blind spot back one level up.
#[tokio::test]
async fn an_exhausted_budget_is_reported_not_hidden() {
    let f = fixture_with_connection("whoop").await;

    let report = refresh_captures(
        &f.runtime,
        SweepBudget {
            per_connection: StdDuration::from_secs(1),
            total: StdDuration::ZERO,
            connection_limit: DEFAULT_CONNECTION_LIMIT,
        },
    )
    .await
    .expect("refresh report");

    assert_eq!(report.attempted, 0, "no fetch may start past the deadline");
    assert_eq!(report.skipped, 1);
    assert!(!report.completed, "the sweep must admit it did not finish");
    let line = report
        .connections
        .first()
        .expect("the connection is listed");
    assert!(
        matches!(line.outcome, RefreshOutcome::SkippedBudgetExhausted),
        "expected an explicit budget skip, got {:?}",
        line.outcome
    );

    let connections = f
        .runtime
        .repos()
        .provider_connections
        .get_for_user(f.user_id, Some(f.tenant))
        .await
        .unwrap();
    assert_eq!(
        connections[0].status,
        ConnectionStatus::Active,
        "a connection the sweep never reached must not be flagged"
    );
}

// ── Re-arming: a flag the stored session outlived ───────────────────────────

/// `DRAVR_SCIOTTE_REMOTE_URL` is process-wide and each test below points it at
/// its own scraper stand-in, so those tests run one at a time. The tests above
/// never reach a scraper: their athletes hold no session to send.
static SCRAPER_ENV: Mutex<()> = Mutex::const_new(());

/// The id of the one ride the scraper stand-in serves.
const RIDE_ID: &str = "15559990001";

/// What the scraper stand-in answers the activity list with.
struct Scraper {
    /// Whether the provider still honours the session. `false` answers the
    /// list `401 session_expired`, the scraper's word for cookies the provider
    /// refused.
    session_alive: AtomicBool,
    /// Whether the capture it serves reached the list head.
    head_complete: AtomicBool,
    /// When the ride it serves started.
    ride_start: String,
}

/// Spawn a local stand-in for the `dravr-sciotte` scraper service and point
/// the platform at it. Every import succeeds; the list is served or refused by
/// the flags the test holds.
async fn spawn_scraper() -> Arc<Scraper> {
    let scraper = Arc::new(Scraper {
        session_alive: AtomicBool::new(true),
        head_complete: AtomicBool::new(true),
        // Two days old: inside the sweep's head window and the cache's
        // retention.
        ride_start: (Utc::now() - Duration::days(2)).to_rfc3339(),
    });
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": "cap-verified-session" })) }),
        )
        .route(
            "/api/activities",
            get(|State(scraper): State<Arc<Scraper>>| async move {
                if !scraper.session_alive.load(Ordering::SeqCst) {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({ "error": "session_expired" })),
                    );
                }
                (
                    StatusCode::OK,
                    Json(json!({
                        "count": 1,
                        "activities": [{
                            "id": RIDE_ID,
                            "name": "Sortie du matin",
                            "sport_type": "ride",
                            "start_date": scraper.ride_start,
                            "duration_seconds": 3600,
                            "provider": "strava",
                            "distance_meters": 30000.0
                        }],
                        "head_complete": scraper.head_complete.load(Ordering::SeqCst)
                    })),
                )
            }),
        )
        .with_state(Arc::clone(&scraper));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    env::set_var("DRAVR_SCIOTTE_REMOTE_URL", format!("http://{addr}"));
    // The remote client is both-or-neither off loopback; on loopback the
    // audience is unused, and set so the client's construction never depends
    // on which test ran before.
    env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-test");
    scraper
}

/// A fresh athlete whose Strava is served by the sciotte mirror: a `sciotte`
/// connection, a live stored session behind it, and the scraper stand-in the
/// platform reads it through.
async fn athlete_on_the_mirror() -> (Fixture, Arc<Scraper>) {
    let scraper = spawn_scraper().await;
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenants = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::Manual, None)
        .await
        .unwrap();
    seed_sciotte_session(&resources, user_id, tenant).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    (
        Fixture {
            runtime,
            user_id,
            tenant,
        },
        scraper,
    )
}

/// Flag the athlete's mirror connection as the sweep does.
async fn flag_as_the_sweep_does(f: &Fixture) {
    f.runtime
        .repos()
        .provider_connections
        .mark_needs_reauth(
            f.user_id,
            f.tenant,
            "sciotte",
            Some("session_expired"),
            Utc::now(),
        )
        .await
        .unwrap();
}

/// Write `revoked` onto the athlete's mirror connection. No repository method
/// writes that status, so the test sets the column itself.
async fn revoke(f: &Fixture) {
    let sql = "UPDATE provider_connections SET status = 'revoked' \
               WHERE user_id = $1 AND tenant_id = $2 AND provider = 'sciotte'";
    let (user, tenant) = (f.user_id.to_string(), f.tenant.to_string());
    let affected = match &**f.runtime.database() {
        Database::SQLite(db) => sqlx::query(sql)
            .bind(&user)
            .bind(&tenant)
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => sqlx::query(sql)
            .bind(&user)
            .bind(&tenant)
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
    };
    assert_eq!(affected, 1, "the connection row is revoked");
}

/// What the mirror connection's row holds beyond the status the model
/// carries.
#[derive(Debug, PartialEq, Eq)]
struct RowState {
    status: String,
    last_error: Option<String>,
    /// Whether a status transition was ever stamped on the row.
    transition_stamped: bool,
}

impl RowState {
    fn new(status: &str, last_error: Option<&str>, transition_stamped: bool) -> Self {
        Self {
            status: status.to_owned(),
            last_error: last_error.map(str::to_owned),
            transition_stamped,
        }
    }
}

/// Read the mirror connection's row.
async fn row_state(f: &Fixture) -> RowState {
    let sql = "SELECT status, last_error, \
                      CASE WHEN status_changed_at IS NULL THEN 'never' ELSE 'stamped' END \
               FROM provider_connections \
               WHERE user_id = $1 AND tenant_id = $2 AND provider = 'sciotte'";
    let (user, tenant) = (f.user_id.to_string(), f.tenant.to_string());
    let (status, last_error, stamped): (String, Option<String>, String) =
        match &**f.runtime.database() {
            Database::SQLite(db) => sqlx::query_as(sql)
                .bind(&user)
                .bind(&tenant)
                .fetch_one(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_as(sql)
                .bind(&user)
                .bind(&tenant)
                .fetch_one(db.pool())
                .await
                .unwrap(),
        };
    RowState {
        status,
        last_error,
        transition_stamped: stamped == "stamped",
    }
}

/// The recent window every fetch below asks for.
fn head_window() -> ActivityQueryParams {
    ActivityQueryParams {
        limit: Some(50),
        offset: None,
        before: None,
        after: Some((Utc::now() - Duration::days(7)).timestamp()),
    }
}

fn ids(activities: &[Activity]) -> Vec<&str> {
    activities.iter().map(Activity::id).collect()
}

/// The incident end to end. The sweep meets one auth-shaped answer and flags
/// the connection; the athlete's next read, asked for by the name they know
/// the provider by, is served through the very same stored session. That read
/// must put the connection back among the live ones, or the sweep and the Home
/// page, which act on `active` connections only, never refresh it again.
#[tokio::test]
async fn a_flag_the_session_outlived_is_cleared_by_the_next_fetch_it_serves() {
    let _serial = SCRAPER_ENV.lock().await;
    let (f, scraper) = athlete_on_the_mirror().await;

    scraper.session_alive.store(false, Ordering::SeqCst);
    let flagging = refresh_captures(&f.runtime, SweepBudget::default())
        .await
        .expect("refresh report");
    assert_eq!(flagging.attempted, 1);
    assert_eq!(flagging.flagged, 1);
    assert_eq!(
        row_state(&f).await,
        RowState::new("needs_reauth", Some("session_expired"), true),
        "the sweep flagged the connection over the refused read"
    );

    scraper.session_alive.store(true, Ordering::SeqCst);
    let served = fetch_provider_activities(
        &f.runtime,
        "strava",
        f.user_id,
        &f.tenant.to_string(),
        &head_window(),
    )
    .await
    .expect("the stored session serves the read");
    assert_eq!(ids(&served), vec![RIDE_ID]);

    let row = row_state(&f).await;
    assert_eq!(
        (row.status.as_str(), row.last_error.as_deref()),
        ("active", None),
        "the read was asked of `strava` and served by the mirror, so the row stored under \
         `sciotte` is the one re-armed, with the refusal it recorded cleared"
    );

    let next = refresh_captures(&f.runtime, SweepBudget::default())
        .await
        .expect("refresh report");
    assert_eq!(
        (next.attempted, next.refreshed, next.flagged),
        (1, 1, 0),
        "a re-armed connection is back in the snapshot the sweep walks"
    );
}

/// A fetch the provider refuses proves nothing about the credential, so the
/// flag and the reason recorded with it stand.
#[tokio::test]
async fn a_fetch_the_provider_refuses_leaves_the_flag_standing() {
    let _serial = SCRAPER_ENV.lock().await;
    let (f, scraper) = athlete_on_the_mirror().await;
    flag_as_the_sweep_does(&f).await;
    let flagged = row_state(&f).await;
    assert_eq!(
        flagged,
        RowState::new("needs_reauth", Some("session_expired"), true)
    );

    scraper.session_alive.store(false, Ordering::SeqCst);
    let error = fetch_provider_head(
        &f.runtime,
        "sciotte",
        f.user_id,
        &f.tenant.to_string(),
        &head_window(),
    )
    .await
    .expect_err("the provider refused the session");
    assert_eq!(
        error.provider_auth_required_provider().as_deref(),
        Some("sciotte")
    );

    assert_eq!(
        row_state(&f).await,
        flagged,
        "a failed read leaves the row exactly as the flag wrote it"
    );
}

/// `revoked` is access withdrawn, which a reconnect restores and a session
/// that still answers does not.
#[tokio::test]
async fn a_revoked_connection_is_never_rearmed() {
    let _serial = SCRAPER_ENV.lock().await;
    let (f, _scraper) = athlete_on_the_mirror().await;
    revoke(&f).await;

    let served = fetch_provider_head(
        &f.runtime,
        "sciotte",
        f.user_id,
        &f.tenant.to_string(),
        &head_window(),
    )
    .await
    .expect("the stored session serves the read");
    assert_eq!(ids(&served), vec![RIDE_ID]);

    assert_eq!(
        row_state(&f).await,
        RowState::new("revoked", None, false),
        "a served read must not restore access that was withdrawn"
    );
}

/// A healthy connection is not rewritten by the reads it serves: no transition
/// is stamped on a row that never left `active`.
#[tokio::test]
async fn an_active_connection_is_left_untouched_by_a_served_fetch() {
    let _serial = SCRAPER_ENV.lock().await;
    let (f, _scraper) = athlete_on_the_mirror().await;
    assert_eq!(row_state(&f).await, RowState::new("active", None, false));

    let served = fetch_provider_head(
        &f.runtime,
        "sciotte",
        f.user_id,
        &f.tenant.to_string(),
        &head_window(),
    )
    .await
    .expect("the stored session serves the read");
    assert_eq!(ids(&served), vec![RIDE_ID]);

    assert_eq!(
        row_state(&f).await,
        RowState::new("active", None, false),
        "the row is as it was registered"
    );
}

/// A capture whose head the scraper never saw is incomplete, not
/// unauthenticated: the provider accepted the session before it read anything.
/// It re-arms the connection, and is still kept out of the cache.
#[tokio::test]
async fn a_capture_missing_its_head_still_rearms_the_connection() {
    let _serial = SCRAPER_ENV.lock().await;
    let (f, scraper) = athlete_on_the_mirror().await;
    flag_as_the_sweep_does(&f).await;

    scraper.head_complete.store(false, Ordering::SeqCst);
    let served = fetch_provider_head(
        &f.runtime,
        "sciotte",
        f.user_id,
        &f.tenant.to_string(),
        &head_window(),
    )
    .await
    .expect("the stored session serves the read");
    assert_eq!(ids(&served), vec![RIDE_ID]);

    let row = row_state(&f).await;
    assert_eq!(
        (row.status.as_str(), row.last_error.as_deref()),
        ("active", None)
    );
    let cached = f
        .runtime
        .repos()
        .activity_cache
        .get_cached_activities(
            f.user_id,
            &f.tenant,
            None,
            Utc::now() - Duration::days(30),
            Utc::now() + Duration::days(1),
            100,
        )
        .await
        .expect("read activity cache");
    assert!(
        cached.is_empty(),
        "the headless capture is served and re-arms, and is still not written through: {:?}",
        ids(&cached)
    );
}
