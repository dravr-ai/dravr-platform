// ABOUTME: E2E tests for carnet#33 + carnet#50 — disconnecting a provider revokes the grant upstream
// ABOUTME: Asserts each provider's documented revocation wire shape, and that rows + cached activities die

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use base64::prelude::{Engine as _, BASE64_STANDARD};
use chrono::{DateTime, Duration, Utc};
use pierre_auth::oauth2_client::OAuth2Config;
use pierre_config::environment::ServerConfig;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserOAuthToken,
    UserStatus,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::OAuthService;
use pierre_services::provider_revocation::{
    revocation_shape, revoke_for_disconnect, revoke_upstream_grant, RevocationShape,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::spawn_blocking;
use tokio::time::timeout;
use uuid::Uuid;

use crate::common::create_test_server_resources;

/// How long a captured request may take to arrive before the test fails with
/// a reason instead of hanging: revocation is best-effort in production, so a
/// skipped call would otherwise leave the await pending forever.
const CAPTURE_WAIT: StdDuration = StdDuration::from_secs(15);

const OK_204: &str = "HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
const OK_200: &str = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
const SERVER_ERROR_500: &str =
    "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

/// A JSON 200 for a token refresh, minting `access_token` for one hour.
fn refresh_response(access_token: &str) -> String {
    let body = format!(
        r#"{{"access_token":"{access_token}","refresh_token":"rotated-refresh-do-not-log","expires_in":3600,"token_type":"Bearer"}}"#
    );
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// One raw HTTP request as the upstream saw it: the request line, the
/// headers (names lower-cased for lookup) and the body.
struct CapturedRequest {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl CapturedRequest {
    fn parse(raw: &str) -> Self {
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
        let mut lines = head.lines();
        let request_line = lines.next().unwrap_or_default();
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let target = parts.next().unwrap_or_default().to_owned();
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        Self {
            method,
            target,
            headers,
            body: body.to_owned(),
        }
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Read one HTTP/1.1 request off the socket: the head up to the blank line,
/// then exactly `content-length` bytes of body, however hyper split the
/// writes.
async fn read_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            return String::from_utf8_lossy(&buf).into_owned();
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
    let content_length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    while buf.len() < head_end + content_length {
        let n = stream.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// A stub upstream that answers a fixed script of responses, one connection
/// each, hands every request it saw back through `requests`, and closes its
/// port once the script is spent — so the same URL can serve a success, a
/// 500 and an "unreachable" scenario in sequence without touching env vars
/// between them (the registry reads `PIERRE_<PROVIDER>_*_URL` once, at
/// construction, and tests in this binary run in parallel).
struct ScriptedUpstream {
    base_url: String,
    requests: mpsc::Receiver<CapturedRequest>,
}

impl ScriptedUpstream {
    async fn serve(responses: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel(responses.len().max(1));
        tokio::spawn(async move {
            let mut listener = Some(listener);
            let last = responses.len().saturating_sub(1);
            for (index, response) in responses.into_iter().enumerate() {
                let Some(active) = listener.as_ref() else {
                    return;
                };
                let Ok((mut stream, _)) = active.accept().await else {
                    return;
                };
                let raw = read_request(&mut stream).await;
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
                if index == last {
                    // Close the port BEFORE handing the request over, so the
                    // test's next connection is refused rather than queued on
                    // a listener that no longer answers.
                    listener.take();
                }
                let _ = tx.send(CapturedRequest::parse(&raw)).await;
            }
        });
        Self {
            base_url: format!("http://{addr}"),
            requests: rx,
        }
    }

    async fn next_request(&mut self, what: &str) -> CapturedRequest {
        timeout(CAPTURE_WAIT, self.requests.recv())
            .await
            .unwrap_or_else(|_| {
                panic!("no {what} reached the stub within {CAPTURE_WAIT:?} — the best-effort path bailed (see WARN logs)")
            })
            .expect("stub task alive")
    }

    /// The stub saw nothing since the last `next_request`. Every revocation
    /// is awaited inside the disconnect, so by the time the disconnect
    /// returns any request it made has already been captured.
    fn assert_silent(&mut self, what: &str) {
        assert!(
            self.requests.try_recv().is_err(),
            "{what} must not reach the provider"
        );
    }
}

/// Serve one canned 200 and hand back the raw request bytes, so the assertion
/// can look at exactly what the revocation put on the wire.
async fn capture_one_request() -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut buf = vec![0_u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]).into_owned();
        let response = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = tx.send(request);
    });

    (format!("http://{addr}"), rx)
}

async fn seed_user_with_tenant(resources: &ServerContext) -> (Uuid, TenantId) {
    let password_hash =
        spawn_blocking(|| bcrypt::hash("Revoke123!", bcrypt::DEFAULT_COST).unwrap())
            .await
            .unwrap();
    let mut user = User::new(
        format!("revoke-{}@example.com", Uuid::new_v4()),
        password_hash,
        Some("Revoke User".to_owned()),
    );
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: "Revocation Tenant".to_owned(),
        slug: format!("revoke-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    (user_id, tenant_id)
}

fn strava_ride(id: &str) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("ride {id}"),
        SportType::Ride,
        Utc::now() - Duration::days(1),
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(30_000.0)
    .build()
}

/// A stored token row for `backend`, as the OAuth callback would have
/// written it.
fn token_row(
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: DateTime<Utc>,
) -> UserOAuthToken {
    UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: backend.to_owned(),
        access_token: access_token.to_owned(),
        refresh_token: refresh_token.map(str::to_owned),
        token_type: "Bearer".to_owned(),
        expires_at: Some(expires_at),
        scope: Some("read".to_owned()),
        provider_user_id: None,
        oauth_app_client_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// Seed a fresh user connected to `backend`: the connection row, the token
/// row and one provider-derived cached activity — everything a disconnect
/// must delete.
async fn seed_connected(
    resources: &ServerContext,
    backend: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: DateTime<Utc>,
) -> (Uuid, TenantId) {
    let (user_id, tenant_id) = seed_user_with_tenant(resources).await;
    let repos = &resources.common.repos;
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, backend, &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .oauth_tokens
        .upsert_token(&token_row(
            user_id,
            tenant_id,
            backend,
            access_token,
            refresh_token,
            expires_at,
        ))
        .await
        .unwrap();
    let ride = ActivityBuilder::new(
        format!("{backend}-ride-{user_id}"),
        format!("{backend} ride"),
        SportType::Ride,
        Utc::now() - Duration::days(1),
        3_600,
        backend.to_owned(),
    )
    .distance_meters(20_000.0)
    .build();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant_id, backend, &[ride])
        .await
        .unwrap();
    (user_id, tenant_id)
}

/// The local side of a disconnect: token row, connection row and cached
/// activities for `backend` are all gone.
async fn assert_locally_disconnected(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    let repos = &resources.common.repos;
    let token_after = repos
        .oauth_tokens
        .get_token(user_id, tenant_id, backend)
        .await
        .unwrap();
    assert!(token_after.is_none(), "{backend} token row must be deleted");

    let connections = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert!(
        connections.iter().all(|c| c.provider != backend),
        "{backend} connection row must be deleted, got {connections:?}"
    );

    let cached = repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant_id,
            Some(backend),
            Utc::now() - Duration::days(30),
            Utc::now(),
            100,
        )
        .await
        .unwrap();
    assert!(
        cached.is_empty(),
        "cached {backend} activities must be deleted on disconnect, got {} rows",
        cached.len()
    );
}

/// The service under test, with the revocation endpoints the
/// `ServerConfig` carries (Strava, Fitbit, Garmin) pointed wherever the
/// caller mutated them. The harness builds `ServerConfig::default()` and
/// never reads `*_REVOKE_URL`, so this is the same knob the env var turns in
/// a deployment.
fn oauth_service(resources: &Arc<ServerContext>, config: ServerConfig) -> OAuthService {
    OAuthService::new(resources.data(), Arc::new(config))
}

fn client_credentials(client_id: &str, client_secret: &str) -> OAuth2Config {
    OAuth2Config {
        client_id: client_id.to_owned(),
        client_secret: client_secret.to_owned(),
        auth_url: String::new(),
        token_url: String::new(),
        redirect_uri: String::new(),
        scopes: vec![],
        use_pkce: false,
    }
}

fn basic_auth_header(client_id: &str, client_secret: &str) -> String {
    format!(
        "Basic {}",
        BASE64_STANDARD.encode(format!("{client_id}:{client_secret}"))
    )
}

/// Disconnecting Strava must (1) revoke the grant upstream — the June-2026
/// `/oauth/revoke` shape: HTTP Basic auth with the client credentials and the
/// stored refresh token as `token` — and (2) delete the token row, the
/// connection row, AND the provider-derived cached activities (§7.4's
/// deletion obligation). A disconnect that only deleted local rows was the
/// 'disconnect that revoked nothing' stub class (carnet#33).
#[tokio::test]
async fn disconnect_revokes_upstream_and_deletes_provider_data() {
    // Client credentials resolve through the same env fallback the real
    // authorize/exchange path uses; set them before resources exist. (The
    // revocation URL itself is injected via ServerConfig below — the harness
    // never reads STRAVA_REVOKE_URL.)
    let (revoke_url, captured) = capture_one_request().await;
    env::set_var("STRAVA_CLIENT_ID", "revoke-test-client");
    env::set_var("STRAVA_CLIENT_SECRET", "revoke-test-secret");

    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id) = seed_user_with_tenant(&resources).await;

    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant_id.to_string(),
            provider: "strava".to_owned(),
            access_token: "access-material-do-not-log".to_owned(),
            refresh_token: Some("refresh-material-do-not-log".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: Some(Utc::now() + Duration::hours(6)),
            scope: Some("read,activity:read_all".to_owned()),
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            user_id,
            &tenant_id,
            "strava",
            &[strava_ride("ride-revoke-1")],
        )
        .await
        .unwrap();

    // The test harness builds `ServerConfig::default()` (it never reads env),
    // so aim the revocation endpoint at the stub through the config the
    // service actually consults — the same knob STRAVA_REVOKE_URL turns in a
    // real deployment.
    let mut config = (*resources.common.config).clone();
    config.external_services.strava_api.revoke_url = revoke_url.clone();
    let oauth_service = OAuthService::new(resources.data(), Arc::new(config));
    oauth_service
        .disconnect_provider(user_id, "strava", Some(tenant_id.as_uuid()))
        .await
        .expect("disconnect succeeds");

    // 1. The upstream revocation actually happened, in the new endpoint's
    //    shape. Basic auth of "revoke-test-client:revoke-test-secret".
    // Bounded wait: revocation is best-effort in production, so a skipped
    // call would otherwise leave this await pending forever instead of
    // failing with a reason.
    let request = timeout(StdDuration::from_secs(15), captured)
        .await
        .expect("no revocation request reached the stub within 15s — the best-effort path bailed (see WARN logs)")
        .expect("revocation request captured");
    let expected_basic = format!(
        "authorization: basic {}",
        BASE64_STANDARD
            .encode("revoke-test-client:revoke-test-secret")
            .to_ascii_lowercase()
    );
    let lower = request.to_ascii_lowercase();
    assert!(
        lower.contains(&expected_basic),
        "revocation must carry the client credentials as HTTP Basic auth; got:\n{request}"
    );
    assert!(
        request.contains("token=refresh-material-do-not-log"),
        "revocation must spend the stored refresh token; got:\n{request}"
    );
    assert!(
        request.contains("token_type_hint=refresh_token"),
        "revocation should hint the token type; got:\n{request}"
    );

    // 2. Local rows are gone.
    let token_after = resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "strava")
        .await
        .unwrap();
    assert!(token_after.is_none(), "token row must be deleted");

    let connections = resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert!(
        connections.iter().all(|c| c.provider != "strava"),
        "connection row must be deleted, got {connections:?}"
    );

    // 3. §7.4: provider-derived cached activities are gone too.
    let cached = resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant_id,
            Some("strava"),
            Utc::now() - Duration::days(30),
            Utc::now(),
            100,
        )
        .await
        .unwrap();
    assert!(
        cached.is_empty(),
        "cached Strava activities must be deleted on disconnect, got {} rows",
        cached.len()
    );

    env::remove_var("STRAVA_CLIENT_ID");
    env::remove_var("STRAVA_CLIENT_SECRET");
}

/// Disconnecting WHOOP through the chokepoint deregisters the user with
/// WHOOP's documented `DELETE /developer/v2/user/access` — the stored access
/// token as `Bearer`, nothing else — and, whatever WHOOP answers, deletes
/// the local rows. Four users walk the four scenarios against one stub:
/// a live token, an expired token (refreshed at the token endpoint first,
/// then spent), a 500, and an unreachable upstream. The registry reads its
/// `PIERRE_WHOOP_*_URL` overrides once at construction, so the env is set
/// before the resources exist and every scenario shares the one stub.
#[tokio::test]
async fn disconnect_whoop_deregisters_user_and_survives_upstream_failure() {
    let mut upstream = ScriptedUpstream::serve(vec![
        OK_204.to_owned(),
        refresh_response("whoop-access-fresh-do-not-log"),
        OK_204.to_owned(),
        SERVER_ERROR_500.to_owned(),
    ])
    .await;
    env::set_var(
        "PIERRE_WHOOP_REVOKE_URL",
        format!("{}/developer/v2/user/access", upstream.base_url),
    );
    env::set_var(
        "PIERRE_WHOOP_TOKEN_URL",
        format!("{}/oauth/oauth2/token", upstream.base_url),
    );
    env::set_var("WHOOP_CLIENT_ID", "whoop-test-client");
    env::set_var("WHOOP_CLIENT_SECRET", "whoop-test-secret");

    let resources = create_test_server_resources().await.unwrap();
    let service = oauth_service(&resources, (*resources.common.config).clone());

    // 1. A live access token is spent as-is.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "whoop",
        "whoop-access-live-do-not-log",
        Some("whoop-refresh-do-not-log"),
        Utc::now() + Duration::hours(1),
    )
    .await;
    service
        .disconnect_provider(user_id, "whoop", Some(tenant_id.as_uuid()))
        .await
        .expect("disconnect succeeds");
    let request = upstream.next_request("WHOOP deregistration").await;
    assert_eq!(request.method, "DELETE");
    assert_eq!(request.target, "/developer/v2/user/access");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer whoop-access-live-do-not-log"),
        "WHOOP deregistration authenticates with the user's access token"
    );
    assert_eq!(request.body, "", "WHOOP's DELETE carries no body");
    assert_locally_disconnected(&resources, user_id, tenant_id, "whoop").await;

    // 2. An expired access token is refreshed first, then the fresh one is spent.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "whoop",
        "whoop-access-stale-do-not-log",
        Some("whoop-refresh-stale-do-not-log"),
        Utc::now() - Duration::hours(2),
    )
    .await;
    service
        .disconnect_provider(user_id, "whoop", Some(tenant_id.as_uuid()))
        .await
        .expect("disconnect succeeds");
    let refresh = upstream.next_request("WHOOP token refresh").await;
    assert_eq!(refresh.method, "POST");
    assert_eq!(refresh.target, "/oauth/oauth2/token");
    for expected in [
        "grant_type=refresh_token",
        "refresh_token=whoop-refresh-stale-do-not-log",
        "client_id=whoop-test-client",
        "client_secret=whoop-test-secret",
    ] {
        assert!(
            refresh.body.contains(expected),
            "refresh must carry {expected}; got body {}",
            refresh.body
        );
    }
    let request = upstream
        .next_request("WHOOP deregistration after refresh")
        .await;
    assert_eq!(request.method, "DELETE");
    assert_eq!(request.target, "/developer/v2/user/access");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer whoop-access-fresh-do-not-log"),
        "the refreshed access token is the one spent"
    );
    assert_locally_disconnected(&resources, user_id, tenant_id, "whoop").await;

    // 3. WHOOP answers 500: the attempt was made, local deletion still happens.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "whoop",
        "whoop-access-500-do-not-log",
        Some("whoop-refresh-500-do-not-log"),
        Utc::now() + Duration::hours(1),
    )
    .await;
    service
        .disconnect_provider(user_id, "whoop", Some(tenant_id.as_uuid()))
        .await
        .expect("disconnect succeeds despite upstream 500");
    let request = upstream.next_request("WHOOP deregistration (500)").await;
    assert_eq!(
        request.header("authorization"),
        Some("Bearer whoop-access-500-do-not-log")
    );
    assert_locally_disconnected(&resources, user_id, tenant_id, "whoop").await;

    // 4. WHOOP is unreachable (the stub's port is closed): local deletion still happens.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "whoop",
        "whoop-access-down-do-not-log",
        Some("whoop-refresh-down-do-not-log"),
        Utc::now() + Duration::hours(1),
    )
    .await;
    service
        .disconnect_provider(user_id, "whoop", Some(tenant_id.as_uuid()))
        .await
        .expect("disconnect succeeds with the provider unreachable");
    assert_locally_disconnected(&resources, user_id, tenant_id, "whoop").await;

    env::remove_var("WHOOP_CLIENT_ID");
    env::remove_var("WHOOP_CLIENT_SECRET");
    env::remove_var("PIERRE_WHOOP_REVOKE_URL");
    env::remove_var("PIERRE_WHOOP_TOKEN_URL");
}

/// Garmin Health API has no token-revocation endpoint: consent withdrawal is
/// `DELETE /wellness-api/rest/user/registration` with the access token as
/// `Bearer`, refreshed first when expired. The user-facing `garmin` name
/// resolves to the `sciotte_garmin` mirror at the chokepoint (Garmin's API
/// is partner-gated), so the `garmin` OAuth backend is exercised through
/// `revoke_for_disconnect` directly, with the endpoints the `ServerConfig`
/// carries (`GARMIN_REVOKE_URL` / `GARMIN_TOKEN_URL` in a deployment).
#[tokio::test]
async fn garmin_backend_deregisters_user_with_bearer_delete() {
    let mut upstream = ScriptedUpstream::serve(vec![
        OK_204.to_owned(),
        refresh_response("garmin-access-fresh-do-not-log"),
        OK_204.to_owned(),
    ])
    .await;
    env::set_var("GARMIN_CLIENT_ID", "garmin-test-client");
    env::set_var("GARMIN_CLIENT_SECRET", "garmin-test-secret");

    let resources = create_test_server_resources().await.unwrap();
    let mut config = (*resources.common.config).clone();
    config.external_services.garmin_api.revoke_url =
        format!("{}/wellness-api/rest/user/registration", upstream.base_url);
    config.external_services.garmin_api.token_url =
        format!("{}/oauth-service/oauth/access_token", upstream.base_url);
    let service = oauth_service(&resources, config);

    assert_eq!(
        revocation_shape(&service, "garmin"),
        Some(RevocationShape::BearerDeregistration {
            revoke_url: format!("{}/wellness-api/rest/user/registration", upstream.base_url),
            token_url: format!("{}/oauth-service/oauth/access_token", upstream.base_url),
        }),
        "the Garmin arm reads its endpoints from the Garmin API config"
    );

    // 1. Live token: one DELETE, bearer-authenticated, no body.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "garmin",
        "garmin-access-live-do-not-log",
        Some("garmin-refresh-do-not-log"),
        Utc::now() + Duration::hours(12),
    )
    .await;
    revoke_for_disconnect(&service, user_id, tenant_id, "garmin").await;
    let request = upstream.next_request("Garmin deregistration").await;
    assert_eq!(request.method, "DELETE");
    assert_eq!(request.target, "/wellness-api/rest/user/registration");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer garmin-access-live-do-not-log")
    );
    assert_eq!(request.body, "");

    // 2. Expired token: refresh at the token endpoint, then DELETE with the fresh one.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "garmin",
        "garmin-access-stale-do-not-log",
        Some("garmin-refresh-stale-do-not-log"),
        Utc::now() - Duration::days(1),
    )
    .await;
    revoke_for_disconnect(&service, user_id, tenant_id, "garmin").await;
    let refresh = upstream.next_request("Garmin token refresh").await;
    assert_eq!(refresh.method, "POST");
    assert_eq!(refresh.target, "/oauth-service/oauth/access_token");
    assert!(
        refresh
            .body
            .contains("refresh_token=garmin-refresh-stale-do-not-log"),
        "refresh must spend the stored refresh token; got body {}",
        refresh.body
    );
    let request = upstream
        .next_request("Garmin deregistration after refresh")
        .await;
    assert_eq!(request.method, "DELETE");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer garmin-access-fresh-do-not-log")
    );

    env::remove_var("GARMIN_CLIENT_ID");
    env::remove_var("GARMIN_CLIENT_SECRET");
}

/// Fitbit revokes like Strava — `POST /oauth2/revoke`, HTTP Basic client
/// credentials, the refresh token as `token` — except that Fitbit documents
/// `token` alone, so no `token_type_hint` goes on the wire. The default test
/// build does not register the Fitbit provider, so the arm is driven through
/// `revoke_upstream_grant` with the shape the dispatch table produces.
#[tokio::test]
async fn fitbit_backend_revokes_with_basic_auth_and_token_only() {
    let mut upstream = ScriptedUpstream::serve(vec![OK_200.to_owned()]).await;
    let resources = create_test_server_resources().await.unwrap();
    let mut config = (*resources.common.config).clone();
    config.external_services.fitbit_api.revoke_url = format!("{}/oauth2/revoke", upstream.base_url);
    let service = oauth_service(&resources, config);

    let shape = revocation_shape(&service, "fitbit").expect("fitbit revokes upstream");
    assert_eq!(
        shape,
        RevocationShape::TokenRevocation {
            revoke_url: format!("{}/oauth2/revoke", upstream.base_url),
            token_type_hint: false,
        },
        "the Fitbit arm reads its endpoint from the Fitbit API config and sends no hint"
    );

    let (user_id, tenant_id) = seed_user_with_tenant(&resources).await;
    let token = token_row(
        user_id,
        tenant_id,
        "fitbit",
        "fitbit-access-do-not-log",
        Some("fitbit-refresh-do-not-log"),
        Utc::now() + Duration::hours(8),
    );
    let creds = client_credentials("fitbit-test-client", "fitbit-test-secret");
    revoke_upstream_grant(&shape, &creds, token, user_id, tenant_id, "fitbit").await;

    let request = upstream.next_request("Fitbit revocation").await;
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/oauth2/revoke");
    assert_eq!(
        request.header("authorization"),
        Some(basic_auth_header("fitbit-test-client", "fitbit-test-secret").as_str())
    );
    assert_eq!(
        request.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        request.body, "token=fitbit-refresh-do-not-log",
        "Fitbit's body is the refresh token alone — no token_type_hint"
    );
}

/// Terra deauthenticates per user: `DELETE /v2/auth/deauthenticateUser` with
/// the Terra user id (which the platform stores as the access token) as the
/// `user_id` query param, authenticated by the developer credentials as
/// `dev-id` + `x-api-key` — no bearer, no basic auth. The default test build
/// does not register the Terra provider, so the arm is driven through
/// `revoke_upstream_grant`.
#[tokio::test]
async fn terra_backend_deauthenticates_user_with_developer_headers() {
    let mut upstream = ScriptedUpstream::serve(vec![OK_200.to_owned()]).await;
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id) = seed_user_with_tenant(&resources).await;

    let shape = RevocationShape::DeveloperKeyDeregistration {
        revoke_url: format!("{}/v2/auth/deauthenticateUser", upstream.base_url),
    };
    let token = token_row(
        user_id,
        tenant_id,
        "terra",
        "7f1c0d2e-terra-user-id",
        None,
        Utc::now() + Duration::days(365),
    );
    let creds = client_credentials("terra-dev-id", "terra-api-key-do-not-log");
    revoke_upstream_grant(&shape, &creds, token, user_id, tenant_id, "terra").await;

    let request = upstream.next_request("Terra deauthentication").await;
    assert_eq!(request.method, "DELETE");
    assert_eq!(
        request.target,
        "/v2/auth/deauthenticateUser?user_id=7f1c0d2e-terra-user-id"
    );
    assert_eq!(request.header("dev-id"), Some("terra-dev-id"));
    assert_eq!(
        request.header("x-api-key"),
        Some("terra-api-key-do-not-log")
    );
    assert_eq!(
        request.header("authorization"),
        None,
        "Terra authenticates by developer headers, never a bearer or basic header"
    );
    assert_eq!(request.body, "");
}

/// Backends with no upstream grant stay local-only, and say so through the
/// dispatch table rather than by accident: `intervals_icu` links by an API
/// key, `sciotte`/`sciotte_garmin` are scrape sessions, and `coros` is the
/// registered LIMITATION(registre#50) residue. A sciotte-backed "Strava"
/// disconnect must not touch Strava's revoke endpoint either.
#[tokio::test]
async fn local_only_backends_send_nothing_upstream() {
    let mut strava_upstream = ScriptedUpstream::serve(vec![OK_200.to_owned()]).await;
    let resources = create_test_server_resources().await.unwrap();
    let mut config = (*resources.common.config).clone();
    config.external_services.strava_api.revoke_url =
        format!("{}/oauth/revoke", strava_upstream.base_url);
    let service = oauth_service(&resources, config);

    for backend in ["intervals_icu", "sciotte", "sciotte_garmin", "coros"] {
        assert_eq!(
            revocation_shape(&service, backend),
            None,
            "{backend} holds no upstream grant this service can withdraw"
        );
    }

    // A sciotte session mirrors "strava" at the chokepoint: disconnecting
    // Strava deletes the sciotte rows and never calls Strava's revoke.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "sciotte",
        "sciotte-cookie-jar-do-not-log",
        None,
        Utc::now() + Duration::days(30),
    )
    .await;
    service
        .disconnect_provider(user_id, "strava", Some(tenant_id.as_uuid()))
        .await
        .expect("disconnect succeeds");
    strava_upstream.assert_silent("a sciotte-backed Strava disconnect");
    assert_locally_disconnected(&resources, user_id, tenant_id, "sciotte").await;

    // Intervals.icu: the API key row is the whole link; deleting it is the disconnect.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "intervals_icu",
        "intervals-api-key-do-not-log",
        None,
        Utc::now() + Duration::days(365),
    )
    .await;
    service
        .disconnect_provider(user_id, "intervals_icu", Some(tenant_id.as_uuid()))
        .await
        .expect("disconnect succeeds");
    strava_upstream.assert_silent("an Intervals.icu disconnect");
    assert_locally_disconnected(&resources, user_id, tenant_id, "intervals_icu").await;
}
