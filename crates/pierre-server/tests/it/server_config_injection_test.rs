// ABOUTME: Pins that settings read once at startup reach the server through ServerConfig, not the environment
// ABOUTME: A context built over an explicit config uses exactly the values that config carries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Production reads each of these settings from the environment once, into
//! `ServerConfig`, and the server is built from that config. A test (or a
//! second server in the same process) hands its own config instead, and the
//! context it gets must honour it: these tests build contexts over explicit
//! configs and read back what each one wired.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::delete;
use axum::Router;
use dravr_sciotte::client::SessionRemoval;
use pierre_auth::strava_pool::strava_seat_summary;
use pierre_config::environment::{ProviderEndpointOverride, SciotteServiceConfig};
use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
use tokio::net::TcpListener;
use url::Url;

use crate::common::{
    create_test_server_resources_with_config, create_test_user_with_plan, set_provider_endpoints,
    test_server_config, test_server_config_with_app, test_server_config_with_sciotte,
};

/// Every session id a `DELETE /auth/sessions/{id}` named.
type Deletes = Arc<Mutex<Vec<String>>>;

/// A loopback stand-in for the scraper service that records the sessions it
/// is asked to drop, and the URL it serves at.
async fn recording_scraper() -> (Deletes, String) {
    let deletes: Deletes = Arc::default();
    let app = Router::new()
        .route(
            "/auth/sessions/{id}",
            delete(
                |State(seen): State<Deletes>, Path(id): Path<String>| async move {
                    seen.lock().unwrap().push(id);
                    StatusCode::NO_CONTENT
                },
            ),
        )
        .with_state(Arc::clone(&deletes));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (deletes, format!("http://{addr}"))
}

#[tokio::test]
async fn a_config_with_no_scraper_service_builds_no_sciotte_client() {
    let resources = create_test_server_resources_with_config(test_server_config())
        .await
        .unwrap();

    assert!(
        resources
            .fitness
            .provider_registry
            .sciotte_client()
            .is_none(),
        "no service configured, no client"
    );
}

#[tokio::test]
async fn the_sciotte_client_calls_the_service_its_config_names() {
    let (deletes, url) = recording_scraper().await;
    let resources = create_test_server_resources_with_config(test_server_config_with_sciotte(&url))
        .await
        .unwrap();

    let client = resources
        .fitness
        .provider_registry
        .sciotte_client()
        .expect("a configured service builds the client");
    let removal = client.delete_session("sess-from-config").await.unwrap();

    assert_eq!(removal, SessionRemoval::Removed);
    assert_eq!(
        *deletes.lock().unwrap(),
        vec!["sess-from-config".to_owned()],
        "the call reached the stand-in the config named"
    );
}

#[tokio::test]
async fn a_service_off_loopback_needs_its_audience() {
    let mut without_audience = test_server_config();
    without_audience.external_services.sciotte = SciotteServiceConfig {
        remote_url: Some("https://sciotte.example.run.app".to_owned()),
        audience: None,
    };
    let refused = create_test_server_resources_with_config(without_audience)
        .await
        .unwrap();
    assert!(
        refused.fitness.provider_registry.sciotte_client().is_none(),
        "a remote service with no audience would refuse every unsigned call"
    );

    let mut with_audience = test_server_config();
    with_audience.external_services.sciotte = SciotteServiceConfig {
        remote_url: Some("https://sciotte.example.run.app".to_owned()),
        audience: Some("dravr-sciotte-test".to_owned()),
    };
    let built = create_test_server_resources_with_config(with_audience)
        .await
        .unwrap();
    assert!(
        built.fitness.provider_registry.sciotte_client().is_some(),
        "the audience completes a remote service's configuration"
    );
}

/// The value of query parameter `key` in `url`.
fn query_param(url: &str, key: &str) -> Option<String> {
    Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

#[tokio::test]
async fn an_authorization_runs_under_the_server_app_and_callback_its_config_names() {
    let mut config = test_server_config_with_app("whoop", "config-whoop-client", "config-secret");
    "https://app.config.test".clone_into(&mut config.base_url);
    let resources = create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let (user_id, _user, tenant_id) = create_test_user_with_plan(
        &resources.agent.database,
        "config-app@example.com",
        "starter",
    )
    .await
    .unwrap();

    let authorization = OAuthService::new(resources.data(), resources.common.config.clone())
        .get_auth_url(user_id, tenant_id, "whoop", AuthUrlOptions::default())
        .await
        .expect("the configured server app authorizes");

    let url = &authorization.authorization_url;
    assert_eq!(
        query_param(url, "client_id").as_deref(),
        Some("config-whoop-client")
    );
    assert_eq!(
        query_param(url, "redirect_uri").as_deref(),
        Some("https://app.config.test/api/oauth/callback/whoop"),
        "the callback sits under the configured public URL"
    );
}

#[tokio::test]
async fn a_server_with_no_app_for_a_provider_refuses_its_authorization() {
    let resources = create_test_server_resources_with_config(test_server_config())
        .await
        .unwrap();
    let (user_id, _user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, "no-app@example.com", "starter")
            .await
            .unwrap();

    let refused = OAuthService::new(resources.data(), resources.common.config.clone())
        .get_auth_url(user_id, tenant_id, "whoop", AuthUrlOptions::default())
        .await
        .expect_err("no client to run under");
    assert!(
        refused
            .message
            .contains("No OAuth credentials configured for provider whoop"),
        "{}",
        refused.message
    );
}

#[tokio::test]
async fn endpoint_overrides_reach_the_provider_registry_the_server_builds() {
    let mut config = test_server_config();
    set_provider_endpoints(
        &mut config,
        "whoop",
        ProviderEndpointOverride {
            token_url: Some("http://127.0.0.1:9/oauth/token".to_owned()),
            api_base_url: Some("http://127.0.0.1:9/developer/v2".to_owned()),
            ..ProviderEndpointOverride::default()
        },
    );
    let resources = create_test_server_resources_with_config(config)
        .await
        .unwrap();

    let whoop = resources
        .fitness
        .provider_registry
        .default_config("whoop")
        .expect("whoop is registered");
    assert_eq!(whoop.token_url, "http://127.0.0.1:9/oauth/token");
    assert_eq!(whoop.api_base_url, "http://127.0.0.1:9/developer/v2");
    assert_eq!(
        whoop.auth_url, "https://api.prod.whoop.com/oauth/oauth2/auth",
        "a field the override leaves unset keeps its default"
    );
}

#[tokio::test]
async fn the_strava_seat_cap_counts_from_the_config() {
    let mut config = test_server_config();
    config.oauth.strava_seat_cap = Some(3);
    let resources = create_test_server_resources_with_config(config)
        .await
        .unwrap();

    let summary = strava_seat_summary(
        resources.common.repos.oauth_tokens.as_ref(),
        &resources.common.config.oauth,
    )
    .await
    .unwrap();
    assert_eq!(
        summary.total, 3,
        "no pool app: the server app's cap is the total"
    );
    assert_eq!(summary.used, 0);
}
