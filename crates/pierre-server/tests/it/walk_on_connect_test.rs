// ABOUTME: A connection landing is reported to the first-sync hook, which walks the history: a usr/pwd sign-in and an OAuth grant
// ABOUTME: Pins that the routes report the backend just connected in its token's tenant, after the connect's own reads

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

/// Every connection the routes reported.
#[derive(Default)]
struct Reports {
    connected: Mutex<Vec<(Uuid, TenantId, String)>>,
}

#[async_trait]
impl FirstSyncHook for Reports {
    // What a landing does is `first_sync_post_test`'s; only the connection
    // report is read here.
    async fn data_landed(&self, _: Uuid, _: TenantId, _: &str) {}

    async fn connected_on_demand(&self, _: Uuid, _: &str) {}

    async fn connected(&self, user_id: Uuid, tenant_id: TenantId, backend: &str) {
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
