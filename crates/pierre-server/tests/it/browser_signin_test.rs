// ABOUTME: carnet#1617 — a provider sign-in run in the athlete's own browser hands Dravr its cookies
// ABOUTME: Pins the catalogue from sciotte, the domain check, the read that proves the session, and its storage
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The Dravr extension (or the app's `WebView`) signs the athlete in to the
//! provider in their own browser and posts that provider's cookies to
//! `POST /api/providers/browser-signin/{target}`. These tests run the
//! platform's real routes against a loopback stand-in of the scraper service
//! (a test double, per the repo's mock rule), on a thread of its own so it
//! outlives each test's runtime.

use crate::common;
use crate::helpers;

use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use common::{create_test_user_with_email, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use helpers::impersonation::{assert_refusal_audited, assert_support_refused, impersonation_of};
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::SCIOTTE_GARMIN;
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{mint_link_token, MintProviderLinkTokenArgs};
use pierre_routes_auth::AuthRoutes;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use uuid::Uuid;

/// The cookie value the stand-in reads as a session Garmin accepts.
const SIGNED_IN: &str = "signed-in";

/// The prefix of a cookie value whose read the stand-in fails as a fault of
/// its own, not a refusal by the provider.
const FAULT: &str = "fault-";

/// What the stand-in saw: every imported session's cookie names and values,
/// and every session id the platform dropped.
#[derive(Default)]
struct Seen {
    imports: Mutex<Vec<Value>>,
    deletes: Mutex<Vec<String>>,
}

static SCRAPER: OnceLock<(Arc<Seen>, String)> = OnceLock::new();

fn scraper() -> &'static (Arc<Seen>, String) {
    SCRAPER.get_or_init(|| {
        let seen = Arc::new(Seen::default());
        let (sender, receiver) = mpsc::channel();
        let state = Arc::clone(&seen);
        thread::spawn(move || {
            let runtime = Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                sender
                    .send(format!("http://{}", listener.local_addr().unwrap()))
                    .unwrap();
                axum::serve(listener, stand_in(state)).await.unwrap();
            });
        });
        (seen, receiver.recv().unwrap())
    })
}

fn stand_in(seen: Arc<Seen>) -> Router {
    Router::new()
        .route("/providers/browser-signin", get(catalogue))
        .route("/auth/import-session", post(import))
        .route("/api/athlete", get(athlete))
        .route("/auth/sessions/{id}", delete(drop_session))
        .with_state(seen)
}

async fn catalogue() -> Json<Value> {
    Json(json!({
        "providers": [{
            "provider": "garmin",
            "login_url": "https://sso.garmin.com/portal/sso/en-US/sign-in",
            "success_patterns": ["/app", "/modern"],
            "cookie_domain": "garmin.com"
        }]
    }))
}

async fn import(State(seen): State<Arc<Seen>>, Json(body): Json<Value>) -> Json<Value> {
    seen.imports.lock().unwrap().push(body.clone());
    Json(json!({
        "status": "imported",
        "session_id": body["session"]["session_id"],
        "provider": body["provider"]
    }))
}

async fn drop_session(State(seen): State<Arc<Seen>>, Path(id): Path<String>) -> Json<Value> {
    seen.deletes.lock().unwrap().push(id.clone());
    Json(json!({ "status": "removed", "session_id": id }))
}

/// The athlete's profile for a session holding the signed-in cookie; a 401
/// as sciotte answers a session the provider refused, for any other.
async fn athlete(State(seen): State<Arc<Seen>>, headers: HeaderMap) -> Response {
    let session_id = headers
        .get("x-session-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let accepted = seen.imports.lock().unwrap().iter().any(|import| {
        import["session"]["session_id"] == session_id
            && import["session"]["cookies"]
                .as_array()
                .is_some_and(|cookies| cookies.iter().any(|c| c["value"] == SIGNED_IN))
    });
    let faulted = seen.imports.lock().unwrap().iter().any(|import| {
        import["session"]["session_id"] == session_id
            && import["session"]["cookies"]
                .as_array()
                .is_some_and(|cookies| {
                    cookies
                        .iter()
                        .any(|c| c["value"].as_str().is_some_and(|v| v.starts_with(FAULT)))
                })
    });
    if faulted {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "internal", "message": "the read service failed" })),
        )
            .into_response();
    }
    if accepted {
        Json(json!({ "id": "900101", "display_name": "Alex Athlete" })).into_response()
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "session_expired", "message": "the provider refused the session" })),
        )
            .into_response()
    }
}

/// One athlete, signed in to Dravr, who accepted Garmin's exposure notice.
struct Athlete {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    bearer: String,
}

impl Athlete {
    async fn new() -> Self {
        let resources = common::create_test_server_resources_with_config(
            common::test_server_config_with_sciotte_usr_pwd_garmin(&scraper().1),
        )
        .await
        .unwrap();
        let (user_id, user) = create_test_user_with_email(
            &resources.agent.database,
            &format!("athlete-{}@example.com", Uuid::new_v4()),
        )
        .await
        .unwrap();
        if let Some(version) = provider_terms_version(SCIOTTE_GARMIN) {
            resources
                .common
                .repos
                .users
                .record_provider_terms(user_id, SCIOTTE_GARMIN, version)
                .await
                .unwrap();
        }
        let tenant_id = resources
            .common
            .repos
            .tenants
            .list_for_user(user_id)
            .await
            .unwrap()
            .first()
            .expect("user has a tenant")
            .id;
        let bearer = format!("Bearer {}", generate_test_token(&resources, &user).await);
        Self {
            resources,
            user_id,
            tenant_id,
            bearer,
        }
    }

    async fn catalogue(&self) -> (u16, Value) {
        let resp = AxumTestRequest::get("/api/providers/browser-signin")
            .header("authorization", &self.bearer)
            .send(AuthRoutes::routes(self.resources.auth_routes_context()))
            .await;
        (resp.status(), resp.json())
    }

    async fn hand_over(&self, target: &str, cookies: &Value) -> (u16, Value) {
        let resp = AxumTestRequest::post(&format!("/api/providers/browser-signin/{target}"))
            .header("authorization", &self.bearer)
            .json(&json!({ "cookies": cookies }))
            .send(AuthRoutes::routes(self.resources.auth_routes_context()))
            .await;
        (resp.status(), resp.json())
    }

    async fn holds_garmin_session(&self) -> bool {
        self.resources
            .common
            .repos
            .oauth_tokens
            .get_token(self.user_id, self.tenant_id, SCIOTTE_GARMIN)
            .await
            .unwrap()
            .is_some()
    }
}

/// The cookies a browser holds once signed in to Garmin, the session cookie's
/// value being `session`.
fn garmin_cookies(session: &str) -> Value {
    json!([
        { "name": "CASTGC", "value": "tgc", "domain": "sso.garmin.com", "path": "/", "secure": true, "http_only": true },
        { "name": "GARMIN-SSO", "value": "1", "domain": ".garmin.com", "path": "/", "secure": true, "http_only": false },
        { "name": "SESSIONID", "value": session, "domain": "connect.garmin.com", "path": "/", "secure": true, "http_only": true, "expires": 1_800_000_000.0 }
    ])
}

/// The session ids of the imports the stand-in saw for a session holding
/// `value`.
fn imports_holding(value: &str) -> Vec<String> {
    scraper()
        .0
        .imports
        .lock()
        .unwrap()
        .iter()
        .filter(|import| {
            import["session"]["cookies"]
                .as_array()
                .is_some_and(|cookies| cookies.iter().any(|c| c["value"] == value))
        })
        .filter_map(|import| import["session"]["session_id"].as_str().map(str::to_owned))
        .collect()
}

/// Whether the platform dropped the stand-in's copy of `session_id`.
fn dropped(session_id: &str) -> bool {
    scraper()
        .0
        .deletes
        .lock()
        .unwrap()
        .iter()
        .any(|id| id == session_id)
}

#[tokio::test]
async fn the_catalogue_is_sciottes_own() {
    let athlete = Athlete::new().await;
    let (status, body) = athlete.catalogue().await;
    assert_eq!(
        status, 200,
        "bug: the catalogue was answered {status}: {body}"
    );
    assert_eq!(
        body["providers"][0]["provider"], "garmin",
        "bug: the catalogue is not sciotte's: {body}"
    );
    assert_eq!(
        body["providers"][0]["cookie_domain"], "garmin.com",
        "bug: the catalogue is not sciotte's: {body}"
    );
}

#[tokio::test]
async fn a_browser_signed_in_to_garmin_connects_it() {
    let athlete = Athlete::new().await;
    let (status, body) = athlete
        .hand_over("garmin", &garmin_cookies(SIGNED_IN))
        .await;

    assert_eq!(
        status, 200,
        "bug: a signed-in hand-over was answered {status}: {body}"
    );
    assert_eq!(
        body["status"], "connected",
        "bug: a signed-in hand-over did not connect: {body}"
    );
    assert_eq!(
        body["provider"], SCIOTTE_GARMIN,
        "bug: the hand-over connected the wrong provider: {body}"
    );
    assert!(
        athlete.holds_garmin_session().await,
        "bug: a session the provider accepted was not stored"
    );
    let imported = imports_holding(SIGNED_IN);
    assert!(
        !imported.is_empty(),
        "bug: the accepted session never reached sciotte"
    );
    assert!(
        imported.iter().all(|id| !dropped(id)),
        "bug: the session the stored connection reads through was dropped"
    );
}

#[tokio::test]
async fn a_session_the_provider_refuses_is_never_stored() {
    let athlete = Athlete::new().await;
    let refused = format!("expired-{}", Uuid::new_v4());
    let (status, body) = athlete.hand_over("garmin", &garmin_cookies(&refused)).await;

    // A 400, never a 401: a 401 tells the app the athlete's own Dravr
    // session failed, and it re-authenticates or signs them out.
    assert_eq!(
        status, 400,
        "bug: a refused provider session was answered {status}: {body}"
    );
    let imported = imports_holding(&refused);
    assert_eq!(
        imported.len(),
        1,
        "bug: the session was not tried exactly once"
    );
    assert!(
        dropped(&imported[0]),
        "bug: the refused session was left on the read service"
    );
    assert!(
        !athlete.holds_garmin_session().await,
        "bug: a session the provider refused was stored"
    );
}

#[tokio::test]
async fn a_read_service_fault_is_not_called_a_refusal() {
    let athlete = Athlete::new().await;
    let faulted = format!("{FAULT}{}", Uuid::new_v4());
    let (status, body) = athlete.hand_over("garmin", &garmin_cookies(&faulted)).await;

    assert!(
        status >= 500,
        "bug: a read-service fault was answered {status} as if the provider refused: {body}"
    );
    let imported = imports_holding(&faulted);
    assert_eq!(
        imported.len(),
        1,
        "bug: the session was not tried exactly once"
    );
    assert!(
        dropped(&imported[0]),
        "bug: the session whose read failed was left on the read service"
    );
    assert!(
        !athlete.holds_garmin_session().await,
        "bug: a session whose read failed was stored"
    );
}

#[tokio::test]
async fn a_cookie_of_another_site_refuses_the_hand_over_before_any_import() {
    let athlete = Athlete::new().await;
    let marker = format!("foreign-{}", Uuid::new_v4());
    let mut cookies = garmin_cookies(&marker);
    cookies.as_array_mut().unwrap().push(json!({
        "name": "SID", "value": "google", "domain": ".google.com", "path": "/", "secure": true, "http_only": true
    }));
    let (status, body) = athlete.hand_over("garmin", &cookies).await;

    assert_eq!(
        status, 400,
        "bug: a foreign cookie was answered {status}: {body}"
    );
    assert!(
        imports_holding(&marker).is_empty(),
        "bug: a foreign cookie reached sciotte"
    );
    assert!(
        !athlete.holds_garmin_session().await,
        "bug: a hand-over carrying a foreign cookie was stored"
    );
}

#[tokio::test]
async fn a_provider_with_no_browser_sign_in_is_refused() {
    let athlete = Athlete::new().await;
    let (status, body) = athlete.hand_over("whoop", &garmin_cookies(SIGNED_IN)).await;
    assert_eq!(
        status, 400,
        "bug: a provider with no browser sign-in was answered {status}: {body}"
    );
}

#[tokio::test]
async fn a_channels_link_token_reaches_neither_route() {
    let athlete = Athlete::new().await;
    let token = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id: athlete.user_id,
            tenant_id: athlete.tenant_id.as_uuid(),
            provider: "sciotte",
            target: "garmin",
            channel: "telegram",
            channel_thread: None,
        },
        &athlete.resources.auth.admin_jwt_secret,
    )
    .unwrap();
    let hosted = Athlete {
        bearer: format!("Bearer {token}"),
        ..athlete
    };
    let marker = format!("hosted-{}", Uuid::new_v4());

    let (status, body) = hosted.catalogue().await;
    assert_eq!(
        status, 403,
        "bug: a link-token read the catalogue, answered {status}: {body}"
    );
    let (status, body) = hosted.hand_over("garmin", &garmin_cookies(&marker)).await;
    assert_eq!(
        status, 403,
        "bug: a link-token handed over a session, answered {status}: {body}"
    );
    assert!(
        imports_holding(&marker).is_empty(),
        "bug: a link-token's hand-over reached sciotte"
    );
    assert!(
        !hosted.holds_garmin_session().await,
        "bug: a link-token's hand-over was stored"
    );
}

#[tokio::test]
async fn an_unauthenticated_caller_reaches_neither_route() {
    let athlete = Athlete::new().await;
    let anonymous = Athlete {
        bearer: String::new(),
        ..athlete
    };
    let (status, body) = anonymous.catalogue().await;
    assert_eq!(
        status, 401,
        "bug: an anonymous caller read the catalogue, answered {status}: {body}"
    );
    let (status, body) = anonymous
        .hand_over("garmin", &garmin_cookies(SIGNED_IN))
        .await;
    assert_eq!(
        status, 401,
        "bug: an anonymous hand-over was answered {status}: {body}"
    );
}

/// An admin acting as the athlete stores no provider session: it would keep
/// reading the athlete's account after the support session ends
/// (carnet#1634). The refusal is audited; the athlete's own hand-over is
/// served.
#[tokio::test]
async fn a_support_session_hands_over_no_sign_in() {
    let athlete = Athlete::new().await;
    let support = impersonation_of(&athlete.resources, &athlete.bearer).await;

    let resp = AxumTestRequest::post("/api/providers/browser-signin/garmin")
        .header("authorization", &support)
        .json(&json!({ "cookies": garmin_cookies(SIGNED_IN) }))
        .send(AuthRoutes::routes(athlete.resources.auth_routes_context()))
        .await;
    assert_support_refused(
        resp.status(),
        &resp.body_text(),
        "hand over a browser sign-in",
    );
    assert!(
        !athlete.holds_garmin_session().await,
        "no session was stored"
    );
    assert_refusal_audited(
        &athlete.resources,
        athlete.user_id,
        "hand over a provider sign-in from the browser",
    )
    .await;

    let (status, body) = athlete
        .hand_over("garmin", &garmin_cookies(SIGNED_IN))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        athlete.holds_garmin_session().await,
        "the athlete's own hand-over stores the Garmin session"
    );
}
