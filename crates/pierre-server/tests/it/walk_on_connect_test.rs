// ABOUTME: A connection landing is reported to the first-sync hook, which walks the history: a usr/pwd sign-in and an OAuth grant
// ABOUTME: Pins that the routes report the backend just connected in its token's tenant, after the connect's own reads and post

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Walks start when a connection lands (carnet#1482).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A usr/pwd sign-in that stores a session, and an OAuth grant landing on the
// callback, each report the connection to the first-sync hook
// (`FirstSyncHook::connected`), whose production implementation starts the
// provider's history walk (pinned in `first_sync_post_test`). These tests
// stand a recording hook in its slot and drive the real routes.
//
// The scraper and Garmin's OAuth are the loopback stand-ins of the Garmin
// sign-in suites.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_sign_in_test::{
    athlete, connect_garmin_oauth, server, server_with_garmin_oauth, sign_in,
};
use crate::history_walk_test::{spawn_switchable_scraper, ScraperSwitch};

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::FirstSyncHook;
use tokio::time::sleep;
use uuid::Uuid;

/// How long a report gets to arrive before the test gives up.
const REPORT_DEADLINE: Duration = Duration::from_secs(10);

/// How long the stand-in hook's first read of an on-demand provider takes,
/// standing in for the read and the first post it leads to.
const FIRST_READ_TAKES: Duration = Duration::from_millis(400);

/// Every connection the routes reported, and when an on-demand first read
/// ended.
#[derive(Default)]
struct Reports {
    connected: Mutex<Vec<(Uuid, TenantId, String)>>,
    connected_at: Mutex<Vec<Instant>>,
    first_read_ended: Mutex<Option<Instant>>,
}

#[async_trait]
impl FirstSyncHook for Reports {
    // What a landing does is `first_sync_post_test`'s; only the connection
    // report is read here.
    async fn data_landed(&self, _: Uuid, _: TenantId, _: &str) {}

    async fn connected_on_demand(&self, _: Uuid, _: &str) {
        sleep(FIRST_READ_TAKES).await;
        *self.first_read_ended.lock().unwrap() = Some(Instant::now());
    }

    async fn connected(&self, user_id: Uuid, tenant_id: TenantId, backend: &str) {
        self.connected_at.lock().unwrap().push(Instant::now());
        self.connected
            .lock()
            .unwrap()
            .push((user_id, tenant_id, backend.to_owned()));
    }
}

/// Stand `reports` in the hook slot the auth routes report to.
fn listen(resources: &Arc<ServerContext>) -> Arc<Reports> {
    let reports = Arc::new(Reports::default());
    let hook: Arc<dyn FirstSyncHook> = reports.clone();
    assert!(
        resources.mcp.first_sync.set(hook).is_ok(),
        "the test server installs no hook of its own"
    );
    reports
}

/// The reports once at least one arrived.
async fn reported(reports: &Reports) -> Vec<(Uuid, TenantId, String)> {
    let deadline = Instant::now() + REPORT_DEADLINE;
    loop {
        let connected = reports.connected.lock().unwrap().clone();
        if !connected.is_empty() {
            // A second report would arrive about as fast.
            sleep(Duration::from_millis(200)).await;
            return reports.connected.lock().unwrap().clone();
        }
        assert!(
            Instant::now() < deadline,
            "no connection was reported within {REPORT_DEADLINE:?}"
        );
        sleep(Duration::from_millis(50)).await;
    }
}

/// A usr/pwd sign-in that stores a session reports the session's backend,
/// in the tenant its token lives in, once: its history walk starts there.
#[tokio::test]
async fn a_sign_in_reports_the_session_it_stored() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    let reports = listen(&resources);

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");

    assert_eq!(
        reported(&reports).await,
        vec![(user_id, tenant, oauth_providers::SCIOTTE_GARMIN.to_owned())],
        "bug: a stored session reports its connection once, for its walk"
    );
}

/// An OAuth grant landing on the callback reports the grant's backend, in
/// the tenant its token lives in, once.
#[tokio::test]
async fn an_oauth_callback_reports_the_grant_it_stored() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    let reports = listen(&resources);

    connect_garmin_oauth(&resources, user_id, tenant).await;

    assert_eq!(
        reported(&reports).await,
        vec![(user_id, tenant, oauth_providers::GARMIN.to_owned())],
        "bug: a grant landing reports its connection once, for its walk"
    );
}

#[cfg(feature = "provider-strava")]
mod strava {
    //! Strava over OAuth is read on demand: its callback starts the first
    //! read, and the connection's walk waits for it and its post.

    use super::*;

    use axum::routing::{get, post};
    use axum::{Json, Router};
    use chrono::Utc;
    use pierre_config::environment::ProviderEndpointOverride;
    use pierre_routes_auth::AuthRoutes;
    use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
    use serde_json::json;
    use tokio::net::TcpListener;
    use url::Url;

    use crate::helpers::axum_test::AxumTestRequest;

    /// A Strava stand-in answering the code exchange and the profile.
    async fn strava_oauth() -> String {
        let app = Router::new()
            .route(
                "/oauth/token",
                post(|| async {
                    let expires_at = Utc::now().timestamp() + 6 * 3600;
                    Json(json!({
                        "token_type": "Bearer",
                        // >= 40 chars and not "at_"-prefixed: the provider's token validation.
                        "access_token": format!("strava_access_token_{:0>30}", 9),
                        "refresh_token": "strava_refresh",
                        "expires_at": expires_at,
                        "expires_in": 21_600,
                        "athlete": { "id": 4242, "firstname": "Wal" }
                    }))
                }),
            )
            .route(
                "/athlete",
                get(|| async { Json(json!({ "id": 4242, "firstname": "Wal" })) }),
            )
            .route("/athlete/activities", get(|| async { Json(json!([])) }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    /// A Strava grant landing on the callback is read on demand at once; its
    /// connection is reported, for its walk, only once that read and the
    /// first post it leads to are done, so the walk never reads the session
    /// beside the grant while the post's turn does.
    #[tokio::test]
    async fn a_strava_callback_reports_its_grant_after_the_first_read() {
        let base = strava_oauth().await;
        let mut config = common::test_server_config();
        common::set_server_app(&mut config, "strava", "walk_client", "walk_secret");
        common::set_provider_endpoints(
            &mut config,
            "strava",
            ProviderEndpointOverride {
                token_url: Some(format!("{base}/oauth/token")),
                api_base_url: Some(base.clone()),
                ..ProviderEndpointOverride::default()
            },
        );
        let resources = server(config).await;
        let (user_id, _user, tenant, _token) = athlete(&resources).await;
        let reports = listen(&resources);

        let service = OAuthService::new(resources.data(), resources.common.config.clone());
        let authorization = service
            .get_auth_url(user_id, tenant, "strava", AuthUrlOptions::default())
            .await
            .expect("a Strava authorization URL");
        let state = Url::parse(&authorization.authorization_url)
            .unwrap()
            .query_pairs()
            .find(|(name, _)| name == "state")
            .map(|(_, value)| value.into_owned())
            .expect("the authorization URL carries its state");
        let resp = AxumTestRequest::get(&format!(
            "/api/oauth/callback/strava?code=walk-code&state={state}&scope=read,activity:read_all"
        ))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
        assert!(
            resp.status() < 400,
            "the callback lands: {} {}",
            resp.status(),
            resp.text()
        );

        assert_eq!(
            reported(&reports).await,
            vec![(user_id, tenant, oauth_providers::STRAVA.to_owned())],
            "bug: a Strava grant landing reports its connection once, for its walk"
        );
        let first_read_ended = reports
            .first_read_ended
            .lock()
            .unwrap()
            .expect("the callback starts the on-demand first read");
        let connected_at = reports.connected_at.lock().unwrap()[0];
        assert!(
            connected_at >= first_read_ended,
            "bug: the walk was asked while the first read and its post still ran"
        );
    }
}
