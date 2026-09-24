// ABOUTME: With the provider_exposure_notice flag armed, a TrainingPeaks or COROS login is refused until the notice is accepted
// ABOUTME: Pins consent per provider and per notice version, and that an account the flag leaves off is never asked

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! TrainingPeaks and COROS are read through the user's own signed-in session,
//! which their terms prohibit for third parties. Every connect surface states that
//! before the credentials field, and the login handler makes it a
//! precondition: without the account's acceptance of the current notice, the
//! credentials must never leave the server. The acceptance belongs to the
//! account — it survives a disconnect, so a reconnect is not asked again —
//! and to the notice's version: a changed notice is asked again.
//!
//! The notice is asked only of an account the `provider_exposure_notice`
//! feature flag arms (off by default, so a demo account connects without it).
//! This account is armed by a per-user override; a second account, left at the
//! default, is asked for nothing.
//!
//! One test, because `DRAVR_SCIOTTE_REMOTE_URL` is process-wide: separate
//! tests in this binary would race on the scraper they point at.

mod common;
mod helpers;

use std::env;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use common::{
    create_test_server_resources, create_test_user, create_test_user_with_email,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::sciotte_remote::{ENV_AUDIENCE, ENV_REMOTE_URL};
use pierre_routes_auth::AuthRoutes;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

/// The backend TrainingPeaks' exposure notice guards.
const TP_NOTICE_BACKEND: &str = "sciotte_trainingpeaks";

/// TrainingPeaks' current exposure-notice version.
fn tp_terms_version() -> &'static str {
    provider_terms_version(TP_NOTICE_BACKEND).expect("TrainingPeaks carries a notice")
}

/// The notice version before it said a coach's account also reads the
/// athletes who confirm a link.
const PREVIOUS_NOTICE_VERSION: &str = "2026-09-22";

/// Every login body the stand-in scraper received, in order.
type Logins = Arc<Mutex<Vec<Value>>>;

/// A stand-in for the scraper service: records each credential login, answers
/// it as authenticated for the provider it named, and exports a session.
async fn spawn_recording_scraper(logins: Logins) -> String {
    let app = Router::new()
        .route(
            "/auth/login-with-credentials",
            post(
                |State(logins): State<Logins>, Json(body): Json<Value>| async move {
                    let provider = body["provider"].clone();
                    logins.lock().unwrap().push(body);
                    Json(json!({
                        "status": "authenticated",
                        "session_id": "recorded-session",
                        "provider": provider,
                    }))
                },
            ),
        )
        .route(
            "/auth/sessions/{session_id}/export",
            get(|| async {
                Json(json!({
                    "session": {
                        "session_id": "recorded-session",
                        "cookies": [],
                        "created_at": "2026-09-22T12:00:00Z",
                        "expires_at": null
                    }
                }))
            }),
        )
        .with_state(logins);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn login(
    resources: &Arc<ServerContext>,
    token: &str,
    target: &str,
    tos_consent: bool,
) -> (u16, Value) {
    let resp = AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({
            "email": "coach@example.test",
            "password": "not-a-real-password",
            "target": target,
            "tos_consent": tos_consent,
        }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    (
        resp.status(),
        serde_json::from_str(&resp.text()).unwrap_or(Value::Null),
    )
}

async fn card_consent_required(resources: &Arc<ServerContext>, token: &str, card: &str) -> bool {
    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", &format!("Bearer {token}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let body: Value = serde_json::from_str(&resp.text()).unwrap();
    body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider"] == card)
        .unwrap_or_else(|| panic!("no {card} card: {body}"))["consent_required"]
        .as_bool()
        .unwrap_or_else(|| panic!("{card} card carries consent_required: {body}"))
}

fn providers_seen(logins: &Logins) -> Vec<String> {
    logins
        .lock()
        .unwrap()
        .iter()
        .map(|body| body["provider"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test]
async fn a_trainingpeaks_login_needs_the_accounts_consent_and_keeps_it() {
    let logins: Logins = Arc::new(Mutex::new(Vec::new()));
    env::set_var(
        ENV_REMOTE_URL,
        spawn_recording_scraper(logins.clone()).await,
    );
    env::remove_var(ENV_AUDIENCE);

    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let token = generate_test_token(&resources, &user).await;
    let users = &resources.common.repos.users;

    // Left at the default, the flag asks this account for no notice.
    assert!(
        !card_consent_required(&resources, &token, oauth_providers::SCIOTTE_TRAININGPEAKS).await
    );
    assert!(!card_consent_required(&resources, &token, "sciotte_coros").await);
    resources
        .common
        .repos
        .feature_flags
        .set_user_override(user_id, FeatureKey::ProviderExposureNotice, true, None)
        .await
        .unwrap();

    // Armed, and before anything is accepted, the TrainingPeaks card asks for the notice
    // and no other card does.
    assert!(
        card_consent_required(&resources, &token, oauth_providers::SCIOTTE_TRAININGPEAKS).await
    );
    assert!(!card_consent_required(&resources, &token, oauth_providers::SCIOTTE_GARMIN).await);

    // 1. No consent: refused, and the credentials never reached the scraper.
    let (status, body) = login(&resources, &token, "trainingpeaks", false).await;
    assert_eq!(
        status, 400,
        "a login without the notice accepted is refused: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("notice"),
        "the refusal names what is missing: {body}"
    );
    assert!(
        logins.lock().unwrap().is_empty(),
        "no credential may leave the server before the notice is accepted"
    );
    assert_eq!(
        users
            .provider_terms_version(user_id, TP_NOTICE_BACKEND)
            .await
            .unwrap(),
        None
    );

    // 2. Consent on this attempt: recorded, then the login runs, on the
    //    TrainingPeaks scraper, and the session lands on the TrainingPeaks mirror.
    let (status, body) = login(&resources, &token, "trainingpeaks", true).await;
    assert_eq!(
        status, 200,
        "an accepted notice lets the login through: {body}"
    );
    assert_eq!(providers_seen(&logins), vec!["trainingpeaks"]);
    assert_eq!(
        users
            .provider_terms_version(user_id, TP_NOTICE_BACKEND)
            .await
            .unwrap()
            .as_deref(),
        Some(tp_terms_version()),
        "the acceptance is recorded against the notice version shown"
    );
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    assert!(
        resources
            .common
            .repos
            .oauth_tokens
            .get_token(user_id, tenant_id, oauth_providers::SCIOTTE_TRAININGPEAKS)
            .await
            .unwrap()
            .is_some(),
        "the session is stored under the TrainingPeaks mirror, not Strava's"
    );
    assert!(
        !card_consent_required(&resources, &token, oauth_providers::SCIOTTE_TRAININGPEAKS).await
    );

    // 3. The acceptance belongs to the account: a disconnect leaves it.
    disconnect_trainingpeaks(&resources, user_id, tenant_id).await;
    assert_eq!(
        users
            .provider_terms_version(user_id, TP_NOTICE_BACKEND)
            .await
            .unwrap()
            .as_deref(),
        Some(tp_terms_version()),
        "a disconnect must not erase the account's answer to the notice"
    );

    // 4. So a reconnect is not asked again.
    let (status, body) = login(&resources, &token, "trainingpeaks", false).await;
    assert_eq!(
        status, 200,
        "an account that accepted the notice reconnects without it: {body}"
    );
    assert_eq!(
        providers_seen(&logins),
        vec!["trainingpeaks", "trainingpeaks"]
    );

    // 5. Garmin has no notice and never asks.
    let (status, body) = login(&resources, &token, "garmin", false).await;
    assert_eq!(status, 200, "a Garmin login needs no notice: {body}");
    assert_eq!(
        providers_seen(&logins),
        vec!["trainingpeaks", "trainingpeaks", "garmin"]
    );

    // 6. An answer to an earlier notice is no answer to this one: the notice
    //    that did not yet say a coach's account reads linked athletes was
    //    "2026-09-22", and an account that accepted only it is asked again.
    assert_ne!(tp_terms_version(), PREVIOUS_NOTICE_VERSION);
    users
        .record_provider_terms(user_id, TP_NOTICE_BACKEND, PREVIOUS_NOTICE_VERSION)
        .await
        .unwrap();
    assert!(
        card_consent_required(&resources, &token, oauth_providers::SCIOTTE_TRAININGPEAKS).await,
        "a changed notice is shown again"
    );
    let (status, body) = login(&resources, &token, "trainingpeaks", false).await;
    assert_eq!(
        status, 400,
        "a login on an earlier notice's answer is refused: {body}"
    );
    assert_eq!(
        providers_seen(&logins),
        vec!["trainingpeaks", "trainingpeaks", "garmin"],
        "the refused login reached no scraper"
    );
    let (status, body) = login(&resources, &token, "trainingpeaks", true).await;
    assert_eq!(
        status, 200,
        "accepting the new notice lets it through: {body}"
    );
    assert_eq!(
        users
            .provider_terms_version(user_id, TP_NOTICE_BACKEND)
            .await
            .unwrap()
            .as_deref(),
        Some(tp_terms_version())
    );

    // 7. COROS asks for its own notice: the TrainingPeaks answer is no answer
    //    to it, a login without it never reaches the scraper, and accepting it
    //    records it under COROS' backend and lets the login through to the
    //    COROS scraper.
    assert!(card_consent_required(&resources, &token, "sciotte_coros").await);
    let seen_before_coros = providers_seen(&logins);
    let (status, body) = login(&resources, &token, "coros", false).await;
    assert_eq!(
        status, 400,
        "a COROS login without its notice is refused: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("COROS"),
        "the refusal names the provider: {body}"
    );
    assert_eq!(
        providers_seen(&logins),
        seen_before_coros,
        "no credential leaves the server before COROS' notice is accepted"
    );
    let (status, body) = login(&resources, &token, "coros", true).await;
    assert_eq!(
        status, 200,
        "COROS' accepted notice lets the login through: {body}"
    );
    assert_eq!(
        providers_seen(&logins).last().map(String::as_str),
        Some("coros")
    );
    assert_eq!(
        users
            .provider_terms_version(user_id, "sciotte_coros")
            .await
            .unwrap()
            .as_deref(),
        provider_terms_version("sciotte_coros")
    );
    assert!(
        resources
            .common
            .repos
            .oauth_tokens
            .get_token(user_id, tenant_id, "sciotte_coros")
            .await
            .unwrap()
            .is_some(),
        "the session is stored under the COROS mirror"
    );
    assert!(!card_consent_required(&resources, &token, "sciotte_coros").await);

    // 8. An account the flag leaves off is never asked: its cards carry no
    //    notice, and a login without one reaches the scraper and records no
    //    acceptance, since none was asked for.
    let (other_id, other) =
        create_test_user_with_email(&resources.agent.database, "no-notice@example.test")
            .await
            .unwrap();
    let other_token = generate_test_token(&resources, &other).await;
    for card in [oauth_providers::SCIOTTE_TRAININGPEAKS, "sciotte_coros"] {
        assert!(
            !card_consent_required(&resources, &other_token, card).await,
            "{card} asks an unarmed account for nothing"
        );
    }
    for target in ["trainingpeaks", "coros"] {
        let seen = providers_seen(&logins).len();
        let (status, body) = login(&resources, &other_token, target, false).await;
        assert_eq!(
            status, 200,
            "an unarmed {target} login needs no notice: {body}"
        );
        assert_eq!(
            providers_seen(&logins)[seen..],
            [target.to_owned()],
            "the unarmed {target} login reaches its scraper"
        );
    }
    for backend in [TP_NOTICE_BACKEND, "sciotte_coros"] {
        assert_eq!(
            users
                .provider_terms_version(other_id, backend)
                .await
                .unwrap(),
            None,
            "no acceptance is recorded where none was asked"
        );
    }

    env::remove_var(ENV_REMOTE_URL);
}

async fn disconnect_trainingpeaks(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    OAuthService::new(resources.data(), resources.common.config.clone())
        .disconnect_provider(
            user_id,
            oauth_providers::TRAININGPEAKS,
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("a TrainingPeaks disconnect by its user-facing name succeeds");
}
