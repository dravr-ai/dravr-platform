// ABOUTME: A dead username-and-password session is the connection flagged, not the Garmin grant beside it
// ABOUTME: A sync stops at the first refusal, and a flagged session is not read again until a reconnect
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete of 2026-11-03 held two Garmin connections: a Garmin API grant
//! that was fine, and a username-and-password session (`sciotte_garmin`) that
//! had expired. Every failure was reported, flagged and noticed as `garmin`:
//! the athlete reconnected the grant, which was fine, and the session stayed
//! dead and unflagged, so every sync and every Home load signed in to it again.
//! These tests drive the reads against a scraper stand-in that counts what
//! reaches it and assert which connection is left flagged. Since carnet#1386
//! the grant is Garmin's one ongoing source, so a session beside it is never
//! read at all.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration, Utc};
use pierre_core::constants::oauth_providers;
use pierre_core::models::{ConnectionStatus, ConnectionType, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::activity_fetch::fetch_provider_activities;
use pierre_tool_runtime::reauth_retry::{claimed_retry, flagged_session};
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::SecretString;
use serde_json::json;
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::helpers::sciotte_mock::seed_scrape_session;

/// The session id the athlete's username-and-password row carries.
const SESSION_ID: &str = "garmin-web-session";

/// A scraper stand-in that counts the activity-list reads it answers.
struct Scraper {
    /// Whether Garmin still honours the session; `false` answers `401`.
    session_alive: AtomicBool,
    reads: AtomicUsize,
}

impl Scraper {
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

async fn spawn_scraper() -> (Arc<Scraper>, String) {
    let scraper = Arc::new(Scraper {
        session_alive: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": SESSION_ID })) }),
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
                            "id": "21550000001",
                            "name": "Course du matin",
                            "sport_type": "run",
                            "start_date": (Utc::now() - Duration::days(1)).to_rfc3339(),
                            "duration_seconds": 3000,
                            "provider": "garmin",
                            "distance_meters": 10000.0
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
    (scraper, format!("http://{addr}"))
}

/// One athlete with a username-and-password Garmin session, and a Garmin API
/// grant beside it when `with_grant`.
struct Fixture {
    resources: Arc<ServerContext>,
    scraper: Arc<Scraper>,
    user_id: Uuid,
    tenant: TenantId,
}

async fn fixture(label: &str, with_grant: bool) -> Fixture {
    let (scraper, scraper_url) = spawn_scraper().await;
    let resources = common::create_test_server_resources_with_sciotte(&scraper_url)
        .await
        .unwrap();
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, _user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let f = Fixture {
        resources,
        scraper,
        user_id,
        tenant,
    };
    f.connect_session().await;
    if with_grant {
        f.connect_grant().await;
    }
    f
}

impl Fixture {
    fn runtime(&self) -> Arc<dyn ToolRuntime> {
        self.resources.clone()
    }

    /// The username-and-password session and its connection, as the hosted
    /// login writes them; a reconnect writes the same.
    async fn connect_session(&self) {
        seed_scrape_session(
            &self.resources,
            self.user_id,
            self.tenant,
            oauth_providers::SCIOTTE_GARMIN,
            SESSION_ID,
        )
        .await;
        self.resources
            .common
            .repos
            .provider_connections
            .register_connection(
                self.user_id,
                self.tenant,
                oauth_providers::SCIOTTE_GARMIN,
                &ConnectionType::Manual,
                None,
            )
            .await
            .unwrap();
    }

    /// A healthy Garmin API grant and its connection.
    async fn connect_grant(&self) {
        let now = Utc::now();
        let repos = &self.resources.common.repos;
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: self.user_id,
                tenant_id: self.tenant.to_string(),
                provider: oauth_providers::GARMIN.to_owned(),
                access_token: SecretString::from("garmin-access-do-not-log"),
                refresh_token: Some(SecretString::from("garmin-refresh-do-not-log")),
                token_type: "Bearer".to_owned(),
                expires_at: Some(now + Duration::hours(12)),
                scope: None,
                provider_user_id: Some("garmin-user".to_owned()),
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(
                self.user_id,
                self.tenant,
                oauth_providers::GARMIN,
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
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

    /// The athlete's recent activities, read as a chat turn reads them.
    async fn read(&self, provider: &str) -> Option<usize> {
        let params = ActivityQueryParams {
            after: Some((Utc::now() - Duration::days(14)).timestamp()),
            before: None,
            limit: Some(20),
            offset: None,
        };
        fetch_provider_activities(
            &self.runtime(),
            provider,
            self.user_id,
            &self.tenant.to_string(),
            &params,
        )
        .await
        .map(|rows| rows.len())
    }
}

/// Beside a Garmin grant the session serves nothing (carnet#1386): a read
/// naming either Garmin backend goes to the grant, the dead session is never
/// signed in to, and neither connection is flagged by a read that never
/// reached it.
#[tokio::test]
async fn a_dead_session_beside_a_garmin_grant_is_never_read() {
    let f = fixture("attribution", true).await;

    f.read(oauth_providers::GARMIN).await;
    f.read(oauth_providers::SCIOTTE_GARMIN).await;

    assert_eq!(
        f.scraper.reads(),
        0,
        "bug: the session beside the grant must never be signed in to"
    );
    assert_eq!(
        f.status(oauth_providers::GARMIN).await,
        ConnectionStatus::Active,
        "bug: the Garmin grant was fine and must stay active"
    );
    assert_eq!(
        f.status(oauth_providers::SCIOTTE_GARMIN).await,
        ConnectionStatus::Active,
        "a read that never reached the session says nothing about it"
    );
}

/// With no grant to fall back on, a flagged session is served from the cache:
/// no read reaches the scraper until the athlete reconnects, and the reads
/// resume once they have.
#[tokio::test]
async fn a_flagged_session_is_not_read_again_until_the_athlete_reconnects() {
    let f = fixture("not-recalled", false).await;

    f.read(oauth_providers::GARMIN).await;
    assert_eq!(
        f.scraper.reads(),
        1,
        "bug: the first read must reach the scraper once"
    );
    assert_eq!(
        f.status(oauth_providers::SCIOTTE_GARMIN).await,
        ConnectionStatus::NeedsReauth
    );
    assert_eq!(
        flagged_session(&f.runtime(), f.user_id, f.tenant, oauth_providers::GARMIN)
            .await
            .as_deref(),
        Some("sciotte_garmin"),
        "the read of `garmin` goes to the flagged session"
    );

    for _ in 0..4 {
        f.read(oauth_providers::GARMIN).await;
        f.read(oauth_providers::SCIOTTE_GARMIN).await;
    }
    assert_eq!(
        f.scraper.reads(),
        1,
        "bug: every read after the first refusal must be served without the scraper"
    );

    // A claimed retry (the capture sweep's, Home's) is the one read let through.
    claimed_retry(f.read(oauth_providers::GARMIN)).await;
    assert_eq!(f.scraper.reads(), 2, "a claimed retry reaches the scraper");

    // The athlete signs in again: the connection is re-armed and reads resume.
    f.scraper.session_alive.store(true, Ordering::SeqCst);
    f.connect_session().await;
    assert_eq!(
        f.status(oauth_providers::SCIOTTE_GARMIN).await,
        ConnectionStatus::Active,
        "a reconnect re-arms the connection"
    );
    assert_eq!(
        f.read(oauth_providers::GARMIN).await,
        Some(1),
        "bug: a reconnected session must be read again"
    );
    assert_eq!(
        f.scraper.reads(),
        3,
        "bug: a reconnected session is read again"
    );
}

#[cfg(feature = "health-sync")]
mod health_sync {
    use super::{fixture, Fixture, SESSION_ID};

    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::NaiveDate;
    use dravr_enforme::providers::sciotte_reader::{
        AuthSession, DailySummary, DailySummaryReader, ScraperError, ScraperResult,
    };
    use dravr_enforme::traits::credential_store::CredentialStore;
    use pierre_core::constants::oauth_providers;
    use pierre_core::models::ConnectionStatus;
    use pierre_mcp_server::services::health_sync_refresher::install_health_sync_refresher;
    use std::sync::Arc;

    /// A reader every session read of which Garmin refuses, recording each.
    #[derive(Default)]
    struct DeadSession {
        asked: Mutex<Vec<(String, NaiveDate)>>,
    }

    impl DeadSession {
        fn asked(&self) -> usize {
            self.asked.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl DailySummaryReader for DeadSession {
        async fn daily_summary(
            &self,
            _provider: &str,
            session: &AuthSession,
            date: NaiveDate,
        ) -> ScraperResult<DailySummary> {
            assert_eq!(
                session.session_id, SESSION_ID,
                "bug: the sync must read the athlete's own session"
            );
            self.asked
                .lock()
                .unwrap()
                .push((session.session_id.clone(), date));
            Err(ScraperError::SessionExpired {
                reason: "session_expired".to_owned(),
            })
        }
    }

    async fn sync(f: &Fixture, reader: &Arc<dyn DailySummaryReader>) {
        let storage = f
            .resources
            .fitness
            .sync_storage
            .as_ref()
            .expect("health sync is wired");
        let orchestrator = storage.build_orchestrator_with_reader(reader).unwrap();
        // Failed or refused, the outcome is the flag and the reads below.
        drop(
            orchestrator
                .sync_user(&f.user_id.to_string(), oauth_providers::GARMIN)
                .await,
        );
    }

    /// The scheduled sync of 2026-11-03, for a usr/pwd-only athlete whose
    /// session serves Garmin's wellness (beside a grant it no longer does,
    /// carnet#1386), and Garmin refuses it. The sync reads it once, not once
    /// per data type and again after the refresh; it flags the session; and
    /// the next sync does not read it at all.
    #[tokio::test]
    async fn a_sync_stops_at_the_first_refusal_and_flags_the_session() {
        let f = fixture("health-sync", false).await;
        install_health_sync_refresher(&f.resources);
        let dead = Arc::new(DeadSession::default());
        let reader: Arc<dyn DailySummaryReader> = Arc::<DeadSession>::clone(&dead);

        sync(&f, &reader).await;
        assert_eq!(
            dead.asked(),
            1,
            "bug: after the first refusal the sync must not read the session again"
        );
        assert_eq!(
            f.status(oauth_providers::SCIOTTE_GARMIN).await,
            ConnectionStatus::NeedsReauth,
            "bug: the refused session must be the connection flagged"
        );

        sync(&f, &reader).await;
        assert_eq!(
            dead.asked(),
            1,
            "bug: a flagged session must not be read by the next sync"
        );

        // A reconnect re-arms the connection, and the next sync reads the new
        // session again.
        f.connect_session().await;
        let storage = f.resources.fitness.sync_storage.as_ref().unwrap();
        assert!(
            storage
                .get_credentials(&f.user_id.to_string(), oauth_providers::GARMIN)
                .await
                .unwrap()
                .is_some(),
            "a reconnected session serves the sync again"
        );
        sync(&f, &reader).await;
        assert_eq!(dead.asked(), 2, "bug: a reconnected session is read again");
    }
}
