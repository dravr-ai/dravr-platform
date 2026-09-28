// ABOUTME: A scrape session flagged needs_reauth is retried by the capture sweep and Athlete Home, once per interval
// ABOUTME: A served retry re-arms it, a refused one waits out the throttle, and a flagged OAuth grant is never retried
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! On 2026-09-25 one `401` from a scraper instance that had not seen a session
//! import flagged an athlete's Strava mirror, and the flag then stood for days:
//! the sweep and Home refreshed `active` connections only, and only a chat
//! turn's live read could clear it. These tests drive the sweep and the Home
//! endpoint against a scraper stand-in that counts the reads it answers, and
//! assert the reads that reach it and the status the row is left in.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
#[path = "helpers/sciotte_mock.rs"]
mod sciotte_mock;

use std::env;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{ConnectionStatus, ConnectionType, TenantId};
use pierre_database::backends::factory::Database;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_tool_runtime::capture_sweep::{refresh_captures, SweepBudget};
use pierre_tool_runtime::reauth_retry::SCRAPE_SESSION_RETRY_INTERVAL_HOURS;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::net::TcpListener;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

use crate::sciotte_mock::seed_sciotte_session;

/// The id of the one ride the scraper stand-in serves.
const RIDE_ID: &str = "15559990077";

/// A scraper stand-in that counts the activity reads it answers.
struct Scraper {
    /// Whether the provider still honours the session; `false` answers the
    /// list `401 session_expired`.
    session_alive: AtomicBool,
    /// Activity-list reads answered, served or refused.
    reads: AtomicUsize,
}

impl Scraper {
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

/// Spawn the stand-in and point the platform at it. Must run before the
/// server context is built.
async fn spawn_scraper() -> Arc<Scraper> {
    let scraper = Arc::new(Scraper {
        session_alive: AtomicBool::new(true),
        reads: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": "cap-verified-session" })) }),
        )
        .route(
            "/api/activities",
            get(|State(scraper): State<Arc<Scraper>>| async move {
                scraper.reads.fetch_add(1, Ordering::SeqCst);
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
                            "start_date": (Utc::now() - Duration::days(1)).to_rfc3339(),
                            "duration_seconds": 3600,
                            "provider": "strava",
                            "distance_meters": 30000.0
                        }],
                        "head_complete": true
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
    env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-test");
    scraper
}

/// One athlete whose Strava is served by the sciotte mirror, flagged as the
/// incident left it, plus a bearer for their Home page.
struct Fixture {
    resources: Arc<ServerContext>,
    scraper: Arc<Scraper>,
    user_id: Uuid,
    tenant: TenantId,
    token: String,
}

async fn fixture(label: &str) -> Fixture {
    let scraper = spawn_scraper().await;
    let resources = common::create_test_server_resources().await.unwrap();
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let token = common::generate_test_token(&resources, &user).await;
    let f = Fixture {
        resources,
        scraper,
        user_id,
        tenant,
        token,
    };
    f.connect("sciotte").await;
    seed_sciotte_session(&f.resources, user_id, tenant).await;
    f.flag("sciotte").await;
    f
}

impl Fixture {
    fn runtime(&self) -> Arc<dyn ToolRuntime> {
        self.resources.clone()
    }

    async fn connect(&self, provider: &str) {
        self.resources
            .common
            .repos
            .provider_connections
            .register_connection(
                self.user_id,
                self.tenant,
                provider,
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
    }

    async fn flag(&self, provider: &str) {
        self.resources
            .common
            .repos
            .provider_connections
            .mark_needs_reauth(
                self.user_id,
                self.tenant,
                provider,
                Some("session_expired"),
                Utc::now(),
            )
            .await
            .unwrap();
    }

    /// Move `provider`'s flag, and any retry already stamped on it, back to
    /// `at` — as if both happened then. No repository writes an old stamp,
    /// so the fixture writes it in SQL.
    async fn age_flag(&self, provider: &str, at: DateTime<Utc>) {
        const SQL: &str = "UPDATE provider_connections \
             SET status_changed_at = $1, \
                 reauth_retry_at = CASE WHEN reauth_retry_at IS NULL THEN NULL ELSE $1 END \
             WHERE user_id = $2 AND tenant_id = $3 AND provider = $4";
        let (user, tenant) = (self.user_id.to_string(), self.tenant.to_string());
        let affected = match self.resources.agent.database.as_ref() {
            Database::SQLite(db) => sqlx::query(SQL)
                .bind(at)
                .bind(&user)
                .bind(&tenant)
                .bind(provider)
                .execute(db.pool())
                .await
                .unwrap()
                .rows_affected(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query(SQL)
                .bind(at)
                .bind(&user)
                .bind(&tenant)
                .bind(provider)
                .execute(db.pool())
                .await
                .unwrap()
                .rows_affected(),
        };
        assert_eq!(affected, 1, "the {provider} connection's flag is aged");
    }

    /// Age `provider`'s flag past one retry interval.
    async fn age_past_interval(&self, provider: &str) {
        self.age_flag(
            provider,
            Utc::now() - Duration::hours(SCRAPE_SESSION_RETRY_INTERVAL_HOURS + 1),
        )
        .await;
    }

    async fn status(&self, provider: &str) -> ConnectionStatus {
        self.resources
            .common
            .repos
            .provider_connections
            .get_for_user(self.user_id, Some(self.tenant))
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.provider == provider)
            .expect("the connection exists")
            .status
    }

    /// `GET /api/me/activities/recent` as the athlete.
    async fn home(&self) -> Value {
        let response = athlete_home_routes()
            .with_state(Arc::clone(&self.resources))
            .oneshot(
                Request::builder()
                    .uri("/api/me/activities/recent")
                    .header("authorization", format!("Bearer {}", self.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Wait for every task the Home route spawned to finish.
    async fn await_background(&self) {
        let deadline = Instant::now() + StdDuration::from_secs(20);
        while !self.resources.common.turns.is_empty() {
            assert!(
                Instant::now() < deadline,
                "the background refresh did not finish within 20s"
            );
            sleep(StdDuration::from_millis(25)).await;
        }
    }
}

/// The sweep retries a flagged scrape session once its interval has passed,
/// and not again within it however often it runs; the retry is the ordinary
/// read, so one the session serves puts the connection back among the live.
#[tokio::test]
#[serial]
async fn the_sweep_retries_a_flagged_scrape_session_once_per_interval() {
    let f = fixture("sweep-retry").await;
    f.scraper.session_alive.store(false, Ordering::SeqCst);

    let within = refresh_captures(&f.runtime(), SweepBudget::default())
        .await
        .unwrap();
    assert_eq!(within.retried, 0, "a flag younger than the interval waits");
    assert_eq!(f.scraper.reads(), 0);

    f.age_past_interval("sciotte").await;
    let refused = refresh_captures(&f.runtime(), SweepBudget::default())
        .await
        .unwrap();
    assert_eq!((refused.retried, refused.attempted), (1, 1));
    assert_eq!(refused.flagged, 1, "the refused retry is reported flagged");
    assert_eq!(f.scraper.reads(), 1);
    assert_eq!(f.status("sciotte").await, ConnectionStatus::NeedsReauth);

    let throttled = refresh_captures(&f.runtime(), SweepBudget::default())
        .await
        .unwrap();
    assert_eq!(throttled.retried, 0, "retried within the interval");
    assert_eq!(f.scraper.reads(), 1, "the scraper is not asked again");

    f.scraper.session_alive.store(true, Ordering::SeqCst);
    f.age_past_interval("sciotte").await;
    let served = refresh_captures(&f.runtime(), SweepBudget::default())
        .await
        .unwrap();
    assert_eq!((served.retried, served.refreshed), (1, 1));
    assert_eq!(f.scraper.reads(), 2);
    assert_eq!(
        f.status("sciotte").await,
        ConnectionStatus::Active,
        "a retry the session served re-arms it"
    );
}

/// A flagged OAuth grant is dead until the athlete reconnects: the sweep never
/// walks it, however long it has been flagged.
#[tokio::test]
#[serial]
async fn the_sweep_never_retries_a_flagged_oauth_grant() {
    let f = fixture("sweep-oauth").await;
    f.connect("strava").await;
    f.flag("strava").await;
    f.age_past_interval("strava").await;

    let report = refresh_captures(&f.runtime(), SweepBudget::default())
        .await
        .unwrap();
    assert!(
        report
            .connections
            .iter()
            .all(|line| line.provider != "strava"),
        "the flagged OAuth connection is not walked: {:?}",
        report.connections
    );
    assert_eq!(f.status("strava").await, ConnectionStatus::NeedsReauth);
    let claimable = f
        .resources
        .common
        .repos
        .provider_connections
        .claim_reauth_retry(f.user_id, f.tenant, "strava", Utc::now())
        .await
        .unwrap();
    assert!(claimable, "the sweep never stamped a retry on it");
}

/// Home starts a refresh for a flagged scrape session whose interval passed,
/// says the page is stale while it runs, and does not start another within
/// the interval: a second load reads no scraper and reports nothing stale.
#[tokio::test]
#[serial]
async fn home_retries_a_flagged_scrape_session_once_within_the_interval() {
    let f = fixture("home-retry").await;
    f.scraper.session_alive.store(false, Ordering::SeqCst);
    f.age_past_interval("sciotte").await;

    let first = f.home().await;
    assert_eq!(first["stale"], true, "{first}");
    f.await_background().await;
    assert_eq!(f.scraper.reads(), 1, "one retry reached the scraper");
    assert_eq!(f.status("sciotte").await, ConnectionStatus::NeedsReauth);

    let second = f.home().await;
    assert_eq!(second["stale"], false, "{second}");
    f.await_background().await;
    assert_eq!(f.scraper.reads(), 1, "no second retry within the interval");
}

/// A Home retry the session serves re-arms the connection and writes the ride
/// through, so the next load shows it.
#[tokio::test]
#[serial]
async fn a_home_retry_the_session_serves_rearms_the_connection() {
    let f = fixture("home-rearm").await;
    f.age_past_interval("sciotte").await;

    assert_eq!(f.home().await["stale"], true);
    f.await_background().await;
    assert_eq!(f.scraper.reads(), 1);
    assert_eq!(f.status("sciotte").await, ConnectionStatus::Active);

    let body = f.home().await;
    let ids: Vec<&str> = body["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [RIDE_ID], "the served retry wrote the ride through");
}

/// Home never retries a flagged OAuth grant: the page is not stale on its
/// account, and no retry is stamped on it.
#[tokio::test]
#[serial]
async fn home_never_retries_a_flagged_oauth_grant() {
    let f = fixture("home-oauth").await;
    // The scrape session is flagged too recently to retry, leaving the OAuth
    // grant the only candidate.
    f.connect("strava").await;
    f.flag("strava").await;
    f.age_past_interval("strava").await;

    let body = f.home().await;
    assert_eq!(body["stale"], false, "{body}");
    assert!(
        f.resources.common.turns.is_empty(),
        "no background refresh was started"
    );
    assert_eq!(f.scraper.reads(), 0);
    let claimable = f
        .resources
        .common
        .repos
        .provider_connections
        .claim_reauth_retry(f.user_id, f.tenant, "strava", Utc::now())
        .await
        .unwrap();
    assert!(claimable, "Home never stamped a retry on it");
}
