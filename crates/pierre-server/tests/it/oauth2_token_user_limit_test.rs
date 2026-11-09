// ABOUTME: The per-user window on /oauth2/token: over the limit, per user, across replicas, and never spending the grant it refuses
// ABOUTME: Pins that only a live grant counts, and that the log line the abuse alert reads is the one the server writes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `/oauth2/token` is metered per address at 600 a minute, because a connector
//! host refreshes all its athletes from a few egress addresses. Once a request
//! names a live grant its user is known, and that user has a window of their
//! own. A refusal must leave the grant unspent: a refused rotation or code
//! exchange that burned its token would lock out the client it refused.

use crate::common::{self, create_test_server_resources};
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use axum::extract::connect_info::MockConnectInfo;
use axum::Router;
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use jsonwebtoken::dangerous::insecure_decode;
use jsonwebtoken::{encode, Algorithm, Header};
use pierre_auth::auth::Claims;
use pierre_auth::config::RateLimitConfig;
use pierre_auth::oauth2_server::endpoints::OAuth2AuthorizationServer;
use pierre_auth::oauth2_server::models::AuthorizeRequest;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_cache::{Cache, CacheConfig};
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::models::{Tenant, TenantId, User};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use uuid::Uuid;

const REDIRECT: &str = "https://app.example.com/callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

/// A limiter counting in `shared` when given, whose per-user window admits
/// `per_user` redemptions; the per-address window stays out of the way.
fn limiter(per_user: u32, shared: Option<Arc<Cache>>) -> Arc<OAuth2RateLimiter> {
    Arc::new(OAuth2RateLimiter::new(
        shared,
        OAuth2RateLimiter::local_window_store(),
        &RateLimitConfig {
            oauth_token_user_rpm: per_user,
            ..RateLimitConfig::default()
        },
    ))
}

/// The OAuth 2.0 routes metered by `limiter`, as one replica serves them.
fn routes(resources: &Arc<ServerContext>, limiter: Arc<OAuth2RateLimiter>) -> Router {
    let context = OAuth2Context {
        database: resources.agent.database.clone(),
        oauth2_server: resources.common.repos.oauth2_server.clone(),
        tenants: resources.common.repos.tenants.clone(),
        users: resources.common.repos.users.clone(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        config: Arc::new(resources.common.config.oauth2_server.clone()),
        refresh_token_expiry_days: resources.common.config.auth.refresh_token_expiry_days,
        csrf_manager: resources.auth.csrf_manager.clone(),
        accounts: resources.account_service(),
        google_sign_in: None,
        strings: Arc::new(MessagingStringsRegistry::new()),
        first_party_only_sources: Arc::default(),
        rate_limiter: limiter,
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))))
}

/// A public client registered over HTTP; returns its `client_id`.
async fn public_client(resources: &Arc<ServerContext>) -> String {
    let response = AxumTestRequest::post("/oauth2/register")
        .json(&json!({
            "redirect_uris": [REDIRECT],
            "client_name": "Connector",
            "token_endpoint_auth_method": "none",
        }))
        .send(routes(resources, limiter(1_000, None)))
        .await;
    assert_eq!(response.status(), 201, "{}", response.body_text());
    response.json::<Value>()["client_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn athlete(resources: &Arc<ServerContext>) -> User {
    let repos = &resources.common.repos;
    let user = User::new(
        format!("token-user-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some("Athlete".to_owned()),
    );
    repos.users.create(&user).await.unwrap();
    repos
        .tenants
        .create(&Tenant {
            id: TenantId::generate(),
            name: "Athlete Tenant".to_owned(),
            slug: format!("tenant-{}", user.id),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    user
}

/// An authorization code `user` granted `client_id`.
async fn code_for(resources: &Arc<ServerContext>, client_id: &str, user: &User) -> String {
    let repos = &resources.common.repos;
    OAuth2AuthorizationServer::from_config(
        &resources.common.config.oauth2_server,
        repos.oauth2_server.clone(),
        repos.tenants.clone(),
        repos.users.clone(),
        resources.auth.auth_manager.clone(),
        resources.auth.jwks_manager.clone(),
        resources.common.config.auth.refresh_token_expiry_days,
    )
    .authorize(
        AuthorizeRequest {
            response_type: "code".to_owned(),
            client_id: client_id.to_owned(),
            redirect_uri: REDIRECT.to_owned(),
            scope: None,
            state: None,
            code_challenge: Some(
                general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes())),
            ),
            code_challenge_method: Some("S256".to_owned()),
            resource: None,
        },
        Some(user.id),
        None,
    )
    .await
    .unwrap()
    .code
}

async fn exchange(router: Router, client_id: &str, code: &str) -> AxumTestResponse {
    AxumTestRequest::post("/oauth2/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ])
        .send(router)
        .await
}

async fn refresh(router: Router, client_id: &str, token: &str) -> AxumTestResponse {
    AxumTestRequest::post("/oauth2/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", token),
        ])
        .send(router)
        .await
}

/// The refresh token of a connection `user` made, redeemed through a limiter
/// of its own so it spends nothing of the window a test is about.
async fn connected(resources: &Arc<ServerContext>, client_id: &str, user: &User) -> String {
    let code = code_for(resources, client_id, user).await;
    let answer = exchange(routes(resources, limiter(1_000, None)), client_id, &code).await;
    assert_eq!(answer.status(), 200, "{}", answer.body_text());
    answer.json::<Value>()["refresh_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn assert_refused(answer: &AxumTestResponse) {
    assert_eq!(answer.status(), 429, "{}", answer.body_text());
    let wait: u64 = answer
        .header("retry-after")
        .expect("a refusal says how long to wait")
        .parse()
        .unwrap();
    assert!((1..=60).contains(&wait), "Retry-After {wait}");
    let body: Value = serde_json::from_str(&answer.body_text()).unwrap();
    assert_eq!(body["error"], "too_many_requests");
    assert!(body.get("retry_after").is_none(), "{body}");
}

/// Past the window a refresh is refused, and the refused token is still the
/// client's: it redeems as soon as the window has room. A refusal that rotated
/// it would answer the client's retry as a replay and end the connection.
#[tokio::test]
async fn a_refresh_past_the_window_is_refused_and_keeps_its_token() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = public_client(&resources).await;
    let user = athlete(&resources).await;
    let token = connected(&resources, &client_id, &user).await;

    let metered = limiter(1, None);
    let first = refresh(routes(&resources, metered.clone()), &client_id, &token).await;
    assert_eq!(first.status(), 200, "{}", first.body_text());
    let successor = first.json::<Value>()["refresh_token"]
        .as_str()
        .unwrap()
        .to_owned();

    let refused = refresh(routes(&resources, metered), &client_id, &successor).await;
    assert_refused(&refused);

    let retried = refresh(
        routes(&resources, limiter(1_000, None)),
        &client_id,
        &successor,
    )
    .await;
    assert_eq!(retried.status(), 200, "{}", retried.body_text());
}

/// A code exchange past the window leaves the code redeemable, so the athlete
/// does not have to authorize again.
#[tokio::test]
async fn a_code_exchange_past_the_window_is_refused_and_keeps_its_code() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = public_client(&resources).await;
    let user = athlete(&resources).await;
    let first = code_for(&resources, &client_id, &user).await;
    let second = code_for(&resources, &client_id, &user).await;

    let metered = limiter(1, None);
    let admitted = exchange(routes(&resources, metered.clone()), &client_id, &first).await;
    assert_eq!(admitted.status(), 200, "{}", admitted.body_text());

    let refused = exchange(routes(&resources, metered), &client_id, &second).await;
    assert_refused(&refused);

    let retried = exchange(
        routes(&resources, limiter(1_000, None)),
        &client_id,
        &second,
    )
    .await;
    assert_eq!(retried.status(), 200, "{}", retried.body_text());
}

/// One athlete filling the window leaves another's untouched, through the same
/// client from the same address.
#[tokio::test]
async fn the_window_is_per_user() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = public_client(&resources).await;
    let (alice, bob) = (athlete(&resources).await, athlete(&resources).await);
    let alice_token = connected(&resources, &client_id, &alice).await;
    let bob_token = connected(&resources, &client_id, &bob).await;

    let metered = limiter(1, None);
    let first = refresh(
        routes(&resources, metered.clone()),
        &client_id,
        &alice_token,
    )
    .await;
    assert_eq!(first.status(), 200, "{}", first.body_text());
    let successor = first.json::<Value>()["refresh_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_refused(&refresh(routes(&resources, metered.clone()), &client_id, &successor).await);

    let other = refresh(routes(&resources, metered), &client_id, &bob_token).await;
    assert_eq!(other.status(), 200, "{}", other.body_text());
}

/// With the shared store two replicas count into one window, so a client gains
/// nothing by reaching another instance.
#[tokio::test]
async fn replicas_sharing_the_store_share_the_window() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = public_client(&resources).await;
    let user = athlete(&resources).await;
    let token = connected(&resources, &client_id, &user).await;
    let shared = Arc::new(
        Cache::new(CacheConfig {
            redis_url: None,
            enable_background_cleanup: false,
            ..CacheConfig::default()
        })
        .await
        .unwrap(),
    );
    let (replica_a, replica_b) = (limiter(1, Some(shared.clone())), limiter(1, Some(shared)));

    let first = refresh(routes(&resources, replica_a), &client_id, &token).await;
    assert_eq!(first.status(), 200, "{}", first.body_text());
    let successor = first.json::<Value>()["refresh_token"]
        .as_str()
        .unwrap()
        .to_owned();

    assert_refused(&refresh(routes(&resources, replica_b), &client_id, &successor).await);
}

/// Only a live grant presented by its own client counts: nobody fills an
/// athlete's window with made-up tokens, a token already rotated out, or the
/// athlete's token presented from another client. Those last two end the
/// family they name, so the athlete's other connection is what must survive.
#[tokio::test]
async fn only_a_live_grant_of_its_own_client_counts() {
    let resources = create_test_server_resources().await.unwrap();
    let client_id = public_client(&resources).await;
    let stranger_client = public_client(&resources).await;
    let user = athlete(&resources).await;
    let rotated_out = connected(&resources, &client_id, &user).await;
    let free = limiter(1_000, None);
    let first = refresh(routes(&resources, free), &client_id, &rotated_out).await;
    assert_eq!(first.status(), 200, "{}", first.body_text());
    let stolen = connected(&resources, &client_id, &user).await;
    let untouched = connected(&resources, &client_id, &user).await;

    let metered = limiter(1, None);
    for _ in 0..3 {
        let unknown = refresh(routes(&resources, metered.clone()), &client_id, "made-up").await;
        assert_eq!(unknown.status(), 400, "{}", unknown.body_text());
        let code = exchange(routes(&resources, metered.clone()), &client_id, "made-up").await;
        assert_eq!(code.status(), 400, "{}", code.body_text());
        let replay = refresh(
            routes(&resources, metered.clone()),
            &client_id,
            &rotated_out,
        )
        .await;
        assert_eq!(replay.status(), 400, "{}", replay.body_text());
    }
    let foreign = refresh(
        routes(&resources, metered.clone()),
        &stranger_client,
        &stolen,
    )
    .await;
    assert_eq!(foreign.status(), 400, "{}", foreign.body_text());

    let own = refresh(routes(&resources, metered), &client_id, &untouched).await;
    assert_eq!(own.status(), 200, "{}", own.body_text());
}

#[derive(Default)]
struct MessageCapture {
    messages: Arc<Mutex<Vec<String>>>,
}

struct MessageVisitor<'a>(&'a mut String);

impl Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            *self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for MessageCapture {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        self.messages.lock().unwrap().push(message);
    }
}

/// `public_mcp_monitoring.tf` alerts on the words of this refusal. Reworded on
/// one side only, the alert would never fire and nothing else would say so.
#[tokio::test]
async fn the_refusal_the_alert_reads_is_the_one_the_server_logs() {
    let terraform = include_str!("../../../../infra/environments/dev/public_mcp_monitoring.tf");
    let filter = Regex::new(
        r#"(?s)resource "google_logging_metric" "oauth_token_user_limited" \{.*?jsonPayload\.message=~"\^([^"]+)""#,
    )
    .unwrap();
    let wanted = &filter
        .captures(terraform)
        .expect("public_mcp_monitoring.tf counts the per-user token refusal")[1];

    let resources = create_test_server_resources().await.unwrap();
    common::init_server_config();
    let client_id = public_client(&resources).await;
    let user = athlete(&resources).await;
    let token = connected(&resources, &client_id, &user).await;
    let metered = limiter(0, None);

    let capture = MessageCapture::default();
    let messages = Arc::clone(&capture.messages);
    let guard = tracing_subscriber::registry().with(capture).set_default();
    let refused = refresh(routes(&resources, metered), &client_id, &token).await;
    drop(guard);

    assert_refused(&refused);
    let logged = messages.lock().unwrap();
    assert!(
        logged.iter().any(|message| message.starts_with(wanted)),
        "the alert reads {wanted:?}; the server logged {logged:#?}"
    );
}

/// `token` as it reads once expired, re-signed with the server's active key.
fn expired(resources: &Arc<ServerContext>, token: &str) -> String {
    let mut claims = insecure_decode::<Claims>(token).unwrap().claims;
    claims.iat -= 7200;
    claims.exp = Utc::now().timestamp() - 3600;
    let key = resources.auth.jwks_manager.get_active_key().unwrap();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    encode(&header, &claims, &key.encoding_key().unwrap()).unwrap()
}

async fn validate_and_refresh(
    router: Router,
    bearer: &str,
    refresh_token: &str,
) -> AxumTestResponse {
    AxumTestRequest::post("/oauth2/validate-and-refresh")
        .header("authorization", &format!("Bearer {bearer}"))
        .json(&json!({ "refresh_token": refresh_token }))
        .send(router)
        .await
}

/// `/oauth2/validate-and-refresh` rotates too, so it counts in the same window.
/// A full window answers `429` with its wait and does not read as a dead token:
/// the token presented is not rotated away, and works once the window has room.
#[tokio::test]
async fn validate_and_refresh_counts_in_the_users_window_and_refuses_with_a_wait() {
    common::init_server_config();
    let resources = create_test_server_resources().await.unwrap();
    let client_id = public_client(&resources).await;
    let user = athlete(&resources).await;
    let metered = limiter(1, None);
    let code = code_for(&resources, &client_id, &user).await;
    let open = routes(&resources, limiter(1_000, None));
    let exchanged: Value = exchange(open.clone(), &client_id, &code).await.json();
    let stale = expired(&resources, exchanged["access_token"].as_str().unwrap());
    let first = exchanged["refresh_token"].as_str().unwrap();

    let admitted = validate_and_refresh(routes(&resources, metered.clone()), &stale, first).await;
    assert_eq!(admitted.status(), 200, "{}", admitted.body_text());
    let admitted: Value = admitted.json();
    assert_eq!(admitted["status"], "refreshed", "{admitted}");
    let rotated = admitted["refresh_token"].as_str().unwrap();

    let refused = validate_and_refresh(routes(&resources, metered), &stale, rotated).await;
    assert_eq!(refused.status(), 429, "{}", refused.body_text());
    assert!(refused.header("retry-after").is_some(), "says how long");
    let body: Value = refused.json();
    assert_eq!(body["status"], "retry_later", "{body}");
    assert_eq!(body["reason"], "too_many_requests", "{body}");
    assert_eq!(
        body["requires_full_reauth"], false,
        "a full window never sends the athlete back to consent: {body}"
    );

    let unspent = validate_and_refresh(open, &stale, rotated).await;
    let unspent: Value = unspent.json();
    assert_eq!(
        unspent["status"], "refreshed",
        "the refused refresh rotated nothing: {unspent}"
    );
}
