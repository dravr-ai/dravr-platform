// ABOUTME: E2E tests for carnet#33 + carnet#50 — disconnecting a provider revokes the grant upstream
// ABOUTME: Asserts each provider's documented revocation wire shape, and that rows + cached activities die

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use secrecy::SecretString;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use base64::prelude::{Engine as _, BASE64_STANDARD};
use dravr_tronc::dates::{Elapsed, Timestamp};
use futures_util::future;
use pierre_auth::oauth2_client::OAuth2Config;
use pierre_config::environment::ProviderEndpointOverride;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserOAuthToken,
    UserStatus,
};
use pierre_database::repositories::{ProviderDataPurge, ProviderPurge, PurgeReason};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::OAuthService;
use pierre_services::provider_revocation::{
    revocation_shape, revoke_for_disconnect, revoke_upstream_grant, DisconnectReason,
    RevocationShape,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::spawn_blocking;
use tokio::time::timeout;
use uuid::Uuid;

use crate::helpers::notify_capture::{capture_notify, named};

use crate::common::{
    create_test_server_resources, create_test_server_resources_with_config, set_provider_endpoints,
    test_server_config, test_server_config_with_app,
};

/// A server whose own `provider` app holds these credentials, its token and
/// revocation endpoints at `token_url` and `revoke_url`.
async fn server_revoking_at(
    provider: &str,
    client_id: &str,
    client_secret: &str,
    token_url: String,
    revoke_url: String,
) -> Arc<ServerContext> {
    let mut config = test_server_config_with_app(provider, client_id, client_secret);
    set_provider_endpoints(
        &mut config,
        provider,
        ProviderEndpointOverride {
            token_url: Some(token_url),
            revoke_url: Some(revoke_url),
            ..ProviderEndpointOverride::default()
        },
    );
    create_test_server_resources_with_config(config)
        .await
        .unwrap()
}

/// A server with no Strava app of its own, revoking Strava grants at
/// `revoke_url`.
async fn strava_revoking_at(revoke_url: String) -> Arc<ServerContext> {
    let mut config = test_server_config();
    set_provider_endpoints(
        &mut config,
        "strava",
        ProviderEndpointOverride {
            revoke_url: Some(revoke_url),
            ..ProviderEndpointOverride::default()
        },
    );
    create_test_server_resources_with_config(config)
        .await
        .unwrap()
}

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
                // The client may hang up first; the test checks what it received.
                drop(stream.write_all(response.as_bytes()).await);
                drop(stream.shutdown().await);
                if index == last {
                    // Close the port BEFORE handing the request over, so the
                    // test's next connection is refused rather than queued on
                    // a listener that no longer answers.
                    listener.take();
                }
                // The test may stop reading before the script runs out.
                drop(tx.send(CapturedRequest::parse(&raw)).await);
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
        // The client may hang up first; the test checks what it received.
        drop(stream.write_all(response.as_bytes()).await);
        // A closed receiver means the test already gave up waiting.
        drop(tx.send(request));
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
        created_at: Timestamp::now(),
        updated_at: Timestamp::now(),
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
        Timestamp::now() - Elapsed::days(1),
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
    expires_at: Timestamp,
) -> UserOAuthToken {
    UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: backend.to_owned(),
        access_token: SecretString::from(access_token.to_owned()),
        refresh_token: refresh_token.map(str::to_owned).map(SecretString::from),
        token_type: "Bearer".to_owned(),
        expires_at: Some(expires_at),
        scope: Some("read".to_owned()),
        provider_user_id: None,
        oauth_app_client_id: None,
        created_at: Timestamp::now(),
        updated_at: Timestamp::now(),
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
    expires_at: Timestamp,
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
        Timestamp::now() - Elapsed::days(1),
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
            Timestamp::now() - Elapsed::days(30),
            Timestamp::now(),
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

/// The service under test, revoking at the endpoints the context's provider
/// registry holds: where [`server_revoking_at`] pointed them, the refused
/// origin otherwise.
fn oauth_service(resources: &Arc<ServerContext>) -> OAuthService {
    OAuthService::new(resources.data(), Arc::clone(&resources.common.config))
}

fn client_credentials(client_id: &str, client_secret: &str) -> OAuth2Config {
    OAuth2Config {
        client_id: client_id.to_owned(),
        client_secret: SecretString::from(client_secret),
        auth_url: String::new(),
        token_url: String::new(),
        redirect_uri: String::new(),
        scopes: vec![],
        use_pkce: false,
    }
}

/// Disconnecting Strava must (1) revoke the grant upstream — the June-2026
/// `/oauth/revoke` shape: HTTP Basic auth with the client credentials and the
/// stored refresh token as `token` — and (2) delete the token row, the
/// connection row, AND the provider-derived cached activities (§7.4's
/// deletion obligation). A disconnect that only deleted local rows was the
/// 'disconnect that revoked nothing' stub class (carnet#33).
#[tokio::test]
async fn disconnect_revokes_upstream_and_deletes_provider_data() {
    // Client credentials resolve through the server's own app, as the real
    // authorize/exchange path resolves them, and the revocation endpoint
    // through the provider registry, as every Strava call's does.
    let (revoke_url, captured) = capture_one_request().await;
    let mut config =
        test_server_config_with_app("strava", "revoke-test-client", "revoke-test-secret");
    set_provider_endpoints(
        &mut config,
        "strava",
        ProviderEndpointOverride {
            revoke_url: Some(revoke_url.clone()),
            ..ProviderEndpointOverride::default()
        },
    );
    let resources = create_test_server_resources_with_config(config)
        .await
        .unwrap();
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
            access_token: SecretString::from("access-material-do-not-log"),
            refresh_token: Some(SecretString::from("refresh-material-do-not-log")),
            token_type: "Bearer".to_owned(),
            expires_at: Some(Timestamp::now() + Elapsed::hours(6)),
            scope: Some("read,activity:read_all".to_owned()),
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
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

    let oauth_service = oauth_service(&resources);
    oauth_service
        .disconnect_provider(
            user_id,
            "strava",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
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
            Timestamp::now() - Elapsed::days(30),
            Timestamp::now(),
            100,
        )
        .await
        .unwrap();
    assert!(
        cached.is_empty(),
        "cached Strava activities must be deleted on disconnect, got {} rows",
        cached.len()
    );
}

/// Disconnecting WHOOP through the chokepoint deregisters the user with
/// WHOOP's documented `DELETE /developer/v2/user/access` — the stored access
/// token as `Bearer`, nothing else — and, whatever WHOOP answers, deletes
/// the local rows. Four users walk the four scenarios against one stub:
/// a live token, an expired token (refreshed at the token endpoint first,
/// then spent), a 500, and an unreachable upstream. The server's WHOOP
/// endpoints point at one stub, which every scenario shares.
#[tokio::test]
async fn disconnect_whoop_deregisters_user_and_survives_upstream_failure() {
    let mut upstream = ScriptedUpstream::serve(vec![
        OK_204.to_owned(),
        refresh_response("whoop-access-fresh-do-not-log"),
        OK_204.to_owned(),
        SERVER_ERROR_500.to_owned(),
    ])
    .await;
    let resources = server_revoking_at(
        "whoop",
        "whoop-test-client",
        "whoop-test-secret",
        format!("{}/oauth/oauth2/token", upstream.base_url),
        format!("{}/developer/v2/user/access", upstream.base_url),
    )
    .await;
    let service = oauth_service(&resources);

    // 1. A live access token is spent as-is.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "whoop",
        "whoop-access-live-do-not-log",
        Some("whoop-refresh-do-not-log"),
        Timestamp::now() + Elapsed::hours(1),
    )
    .await;
    service
        .disconnect_provider(
            user_id,
            "whoop",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
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
        Timestamp::now() - Elapsed::hours(2),
    )
    .await;
    service
        .disconnect_provider(
            user_id,
            "whoop",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
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
        Timestamp::now() + Elapsed::hours(1),
    )
    .await;
    service
        .disconnect_provider(
            user_id,
            "whoop",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
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
        Timestamp::now() + Elapsed::hours(1),
    )
    .await;
    service
        .disconnect_provider(
            user_id,
            "whoop",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("disconnect succeeds with the provider unreachable");
    assert_locally_disconnected(&resources, user_id, tenant_id, "whoop").await;
}

/// Garmin Health API has no token-revocation endpoint: consent withdrawal is
/// `DELETE /wellness-api/rest/user/registration` with the access token as
/// `Bearer`, refreshed first when expired. The `garmin` OAuth backend is
/// exercised through `revoke_for_disconnect` directly, with the endpoints the
/// provider registry holds (`PIERRE_GARMIN_REVOKE_URL` /
/// `PIERRE_GARMIN_TOKEN_URL` in a deployment); the card's own disconnect is
/// pinned below.
#[tokio::test]
async fn garmin_backend_deregisters_user_with_bearer_delete() {
    let mut upstream = ScriptedUpstream::serve(vec![
        OK_204.to_owned(),
        refresh_response("garmin-access-fresh-do-not-log"),
        OK_204.to_owned(),
    ])
    .await;
    let resources = server_revoking_at(
        "garmin",
        "garmin-test-client",
        "garmin-test-secret",
        format!("{}/di-oauth2-service/oauth/token", upstream.base_url),
        format!("{}/wellness-api/rest/user/registration", upstream.base_url),
    )
    .await;
    let service = oauth_service(&resources);

    assert_eq!(
        revocation_shape(&service, "garmin"),
        Some(RevocationShape::BearerDeregistration {
            revoke_url: format!("{}/wellness-api/rest/user/registration", upstream.base_url),
            token_url: format!("{}/di-oauth2-service/oauth/token", upstream.base_url),
        }),
        "the Garmin arm reads its endpoints from the provider registry"
    );

    // 1. Live token: one DELETE, bearer-authenticated, no body.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "garmin",
        "garmin-access-live-do-not-log",
        Some("garmin-refresh-do-not-log"),
        Timestamp::now() + Elapsed::hours(12),
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
        Timestamp::now() - Elapsed::days(1),
    )
    .await;
    revoke_for_disconnect(&service, user_id, tenant_id, "garmin").await;
    let refresh = upstream.next_request("Garmin token refresh").await;
    assert_eq!(refresh.method, "POST");
    assert_eq!(refresh.target, "/di-oauth2-service/oauth/token");
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
}

/// Disconnecting the Garmin card (named for `sciotte_garmin`, as the clients
/// name it) clears whichever backend serves it: a Garmin API connection is
/// deregistered at Garmin with the bearer `DELETE`, and its token, connection
/// and cached pushes are deleted locally (carnet#1155).
#[tokio::test]
async fn disconnecting_the_garmin_card_deregisters_a_garmin_api_connection() {
    let mut upstream = ScriptedUpstream::serve(vec![OK_204.to_owned()]).await;
    let resources = server_revoking_at(
        "garmin",
        "garmin-test-client",
        "garmin-test-secret",
        format!("{}/di-oauth2-service/oauth/token", upstream.base_url),
        format!("{}/wellness-api/rest/user/registration", upstream.base_url),
    )
    .await;
    let service = oauth_service(&resources);

    let (user_id, tenant_id) = seed_connected(
        &resources,
        "garmin",
        "garmin-access-card-do-not-log",
        Some("garmin-refresh-card-do-not-log"),
        Timestamp::now() + Elapsed::hours(12),
    )
    .await;
    service
        .disconnect_provider(
            user_id,
            "sciotte_garmin",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("the Garmin card disconnects");

    let request = upstream.next_request("Garmin deregistration").await;
    assert_eq!(request.method, "DELETE");
    assert_eq!(request.target, "/wellness-api/rest/user/registration");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer garmin-access-card-do-not-log")
    );
    assert_locally_disconnected(&resources, user_id, tenant_id, "garmin").await;
    assert_locally_disconnected(&resources, user_id, tenant_id, "sciotte_garmin").await;
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
        Timestamp::now() + Elapsed::days(365),
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
/// dispatch table rather than by accident: `sciotte`/`sciotte_garmin` are
/// signed-in sessions, and `coros` is the residue registered at
/// `revocation_shape` (registre#509). An `intervals_icu` API-key link is the
/// row-level case, pinned in `intervals_icu_oauth_test`. A session-backed
/// "Strava" disconnect must not touch Strava's revoke endpoint either.
#[tokio::test]
async fn local_only_backends_send_nothing_upstream() {
    let mut strava_upstream = ScriptedUpstream::serve(vec![OK_200.to_owned()]).await;
    let resources = strava_revoking_at(format!("{}/oauth/revoke", strava_upstream.base_url)).await;
    let service = oauth_service(&resources);

    for backend in [
        "sciotte",
        "sciotte_garmin",
        "sciotte_trainingpeaks",
        "coros",
    ] {
        assert_eq!(
            revocation_shape(&service, backend),
            None,
            "{backend} holds no upstream grant this service can withdraw"
        );
    }

    // A signed-in session mirrors "strava" at the chokepoint: disconnecting
    // Strava deletes the session-read rows and never calls Strava's revoke.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "sciotte",
        "sciotte-cookie-jar-do-not-log",
        None,
        Timestamp::now() + Elapsed::days(30),
    )
    .await;
    service
        .disconnect_provider(
            user_id,
            "strava",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("disconnect succeeds");
    strava_upstream.assert_silent("a session-backed Strava disconnect");
    assert_locally_disconnected(&resources, user_id, tenant_id, "sciotte").await;

    // TrainingPeaks has no backend of its own: a disconnect names the card
    // (`trainingpeaks`), and the chokepoint must expand it to the mirror that
    // holds the session — through provider validation, which knows no
    // `trainingpeaks` factory.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "sciotte_trainingpeaks",
        "trainingpeaks-cookie-jar-do-not-log",
        None,
        Timestamp::now() + Elapsed::days(30),
    )
    .await;
    service
        .disconnect_provider(
            user_id,
            "trainingpeaks",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("a TrainingPeaks disconnect by its user-facing name succeeds");
    strava_upstream.assert_silent("a TrainingPeaks disconnect");
    assert_locally_disconnected(&resources, user_id, tenant_id, "sciotte_trainingpeaks").await;

    // Intervals.icu: the API key row is the whole link; deleting it is the disconnect.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "intervals_icu",
        "intervals-api-key-do-not-log",
        None,
        Timestamp::now() + Elapsed::days(365),
    )
    .await;
    service
        .disconnect_provider(
            user_id,
            "intervals_icu",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("disconnect succeeds");
    strava_upstream.assert_silent("an Intervals.icu disconnect");
    assert_locally_disconnected(&resources, user_id, tenant_id, "intervals_icu").await;
}

/// The Strava card is served by EITHER a native `strava` OAuth grant or the
/// `sciotte` mirror, and `get_connection_status` coalesces the pair into ONE
/// card. A client that names the card's own id must therefore clear whichever
/// backend actually holds the grant.
///
/// Resolution runs one way only — `mirror_backend_for` maps `strava` → `sciotte`
/// and never the reverse — so disconnecting by `sciotte` deleted a row that did
/// not exist, reported success, and left the live `strava` grant to fold
/// straight back onto the card. That is the "disconnect does nothing" users hit
/// on the web app and on the installed PWA: HTTP 204, still connected.
///
/// Asserting on the surviving grant is the point: a disconnect that returns Ok
/// while the token row lives is exactly the shape this regression takes.
#[tokio::test]
async fn disconnect_by_sciotte_clears_a_native_strava_grant() {
    let strava_upstream = ScriptedUpstream::serve(vec![OK_200.to_owned()]).await;
    let resources = strava_revoking_at(format!("{}/oauth/revoke", strava_upstream.base_url)).await;
    let service = oauth_service(&resources);

    // A native Strava OAuth grant — no session-read row anywhere.
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "strava",
        "strava-access-do-not-log",
        Some("strava-refresh-do-not-log"),
        Timestamp::now() + Elapsed::days(30),
    )
    .await;

    // Disconnect naming the CARD's id, which is what both clients send.
    service
        .disconnect_provider(
            user_id,
            "sciotte",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("disconnect succeeds");

    // The grant the card actually reported must be gone — token row,
    // connection row and cached activities alike.
    assert_locally_disconnected(&resources, user_id, tenant_id, "strava").await;
}

/// The success post-condition must not fire on the benign no-ops. It re-reads
/// the pair after deleting and refuses to report success when a row survives —
/// a guard that mistook "nothing was there" for "nothing was deleted" would
/// turn every second click, and every disconnect of an unconnected provider,
/// into a 500.
///
/// A repeat is also a no-op in effect, not only in outcome (carnet#1838): one
/// athlete's 23 presses each re-ran the purge of both backends and counted
/// `provider.disconnected` again. The first disconnect purges each backend of
/// the pair once and emits once; nothing after it purges or emits.
#[tokio::test]
async fn repeat_disconnect_and_unconnected_provider_still_succeed() {
    let strava_upstream = ScriptedUpstream::serve(vec![OK_200.to_owned()]).await;
    let resources = strava_revoking_at(format!("{}/oauth/revoke", strava_upstream.base_url)).await;
    let service = oauth_service(&resources);

    let (user_id, tenant_id) = seed_connected(
        &resources,
        "strava",
        "strava-access-do-not-log",
        Some("strava-refresh-do-not-log"),
        Timestamp::now() + Elapsed::days(30),
    )
    .await;
    let since = Timestamp::now() - Elapsed::minutes(1);
    let (events, _guard) = capture_notify();

    // First click clears the grant.
    service
        .disconnect_provider(
            user_id,
            "strava",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("first disconnect succeeds");
    assert_locally_disconnected(&resources, user_id, tenant_id, "strava").await;
    assert_purge_counts(
        &resources,
        user_id,
        tenant_id,
        since,
        1,
        "the first disconnect",
    )
    .await;
    assert_eq!(
        named(&events, "provider.disconnected").len(),
        1,
        "the first disconnect emits once"
    );

    // Second click has nothing left to delete — still a success, not a 500.
    service
        .disconnect_provider(
            user_id,
            "strava",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("a repeat disconnect is a no-op, not a failure");

    // Naming the card id on an already-clear pair is likewise a no-op.
    service
        .disconnect_provider(
            user_id,
            "sciotte",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("disconnecting an already-clear pair by card id is a no-op");

    // A provider this user never connected must not trip the guard either.
    service
        .disconnect_provider(
            user_id,
            "whoop",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("disconnecting a never-connected provider is a no-op");

    assert_purge_counts(&resources, user_id, tenant_id, since, 1, "the repeats").await;
    assert!(
        resources
            .common
            .repos
            .provider_data
            .purge_reasons_since(user_id, &tenant_id, "whoop", since)
            .await
            .unwrap()
            .is_empty(),
        "a never-connected provider is not purged"
    );
    assert_eq!(
        named(&events, "provider.disconnected").len(),
        1,
        "only the disconnect that cleared something counts"
    );
}

/// A disconnect whose purge fails is an error, and its retry must purge again
/// (the contract on `purge_provider_data`). The connection row is removed only
/// after the purge succeeded, so it survives the failure and marks the pair
/// as still held: the retry is not mistaken for a repeat of a finished clear.
#[tokio::test]
async fn a_failed_purge_leaves_the_connection_row_and_the_retry_purges() {
    let strava_upstream = ScriptedUpstream::serve(vec![OK_200.to_owned(), OK_200.to_owned()]).await;
    let resources = strava_revoking_at(format!("{}/oauth/revoke", strava_upstream.base_url)).await;
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "strava",
        "strava-access-do-not-log",
        Some("strava-refresh-do-not-log"),
        Timestamp::now() + Elapsed::days(30),
    )
    .await;
    let since = Timestamp::now() - Elapsed::minutes(1);
    let (events, _guard) = capture_notify();

    let failing = OAuthService::new(
        resources
            .data()
            .with_provider_purge(Arc::new(PurgeFailingFor {
                inner: Arc::clone(&resources.common.provider_purge),
                provider: "strava",
            })),
        Arc::clone(&resources.common.config),
    );
    failing
        .disconnect_provider(
            user_id,
            "strava",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect_err("a failed purge is the disconnect's error");

    let repos = &resources.common.repos;
    assert!(
        repos
            .oauth_tokens
            .get_token(user_id, tenant_id, "strava")
            .await
            .unwrap()
            .is_none(),
        "the token row went before the purge"
    );
    let connections = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert!(
        connections.iter().any(|c| c.provider == "strava"),
        "the connection row outlives a failed purge, got {connections:?}"
    );
    assert!(
        repos
            .provider_data
            .purge_reasons_since(user_id, &tenant_id, "strava", since)
            .await
            .unwrap()
            .is_empty(),
        "the failed purge attests nothing"
    );
    assert!(
        named(&events, "provider.disconnected").is_empty(),
        "a failed disconnect emits nothing"
    );

    oauth_service(&resources)
        .disconnect_provider(
            user_id,
            "strava",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("the retry succeeds");

    assert_locally_disconnected(&resources, user_id, tenant_id, "strava").await;
    assert_eq!(
        repos
            .provider_data
            .purge_reasons_since(user_id, &tenant_id, "strava", since)
            .await
            .unwrap(),
        vec!["athlete_disconnect".to_owned()],
        "the retry purges the provider"
    );
    assert_eq!(
        named(&events, "provider.disconnected").len(),
        1,
        "the retry that cleared the provider emits once"
    );
}

/// The backend of a pair the athlete does not hold is cleared first, for the
/// rows filed under its key. Should its purge fail, the held backend has not
/// been touched yet: its token and connection rows still mark the pair held,
/// so the retry clears the whole pair rather than finding nothing to do.
#[tokio::test]
async fn a_failed_purge_of_the_unheld_backend_leaves_the_held_one_for_the_retry() {
    let strava_upstream = ScriptedUpstream::serve(vec![OK_200.to_owned()]).await;
    let resources = strava_revoking_at(format!("{}/oauth/revoke", strava_upstream.base_url)).await;
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "strava",
        "strava-access-do-not-log",
        Some("strava-refresh-do-not-log"),
        Timestamp::now() + Elapsed::days(30),
    )
    .await;
    let since = Timestamp::now() - Elapsed::minutes(1);
    let (events, _guard) = capture_notify();

    let failing = OAuthService::new(
        resources
            .data()
            .with_provider_purge(Arc::new(PurgeFailingFor {
                inner: Arc::clone(&resources.common.provider_purge),
                provider: "sciotte",
            })),
        Arc::clone(&resources.common.config),
    );
    failing
        .disconnect_provider(
            user_id,
            "strava",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect_err("a failed purge is the disconnect's error");

    let repos = &resources.common.repos;
    assert!(
        repos
            .oauth_tokens
            .get_token(user_id, tenant_id, "strava")
            .await
            .unwrap()
            .is_some(),
        "the held backend is cleared after the unheld one, so its token stands"
    );
    let connections = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert!(
        connections.iter().any(|c| c.provider == "strava"),
        "the held backend's connection row stands, got {connections:?}"
    );
    assert_purge_counts(&resources, user_id, tenant_id, since, 0, "the failure").await;
    assert!(
        named(&events, "provider.disconnected").is_empty(),
        "a failed disconnect emits nothing"
    );

    oauth_service(&resources)
        .disconnect_provider(
            user_id,
            "strava",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("the retry succeeds");

    assert_locally_disconnected(&resources, user_id, tenant_id, "strava").await;
    assert_purge_counts(&resources, user_id, tenant_id, since, 1, "the retry").await;
    assert_eq!(
        named(&events, "provider.disconnected").len(),
        1,
        "the retry that cleared the pair emits once"
    );
}

/// Each backend of the Strava pair was purged exactly `expected` times since
/// `since`, read from the purge attestation rows.
async fn assert_purge_counts(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    since: Timestamp,
    expected: usize,
    after: &str,
) {
    for backend in ["strava", "sciotte"] {
        let purges = resources
            .common
            .repos
            .provider_data
            .purge_reasons_since(user_id, &tenant_id, backend, since)
            .await
            .unwrap();
        assert_eq!(
            purges.len(),
            expected,
            "{backend} purges after {after}: {purges:?}"
        );
    }
}

/// The server's purge, failing every user purge of `provider` before it
/// deletes anything: the failure a disconnect must report and survive.
#[derive(Debug)]
struct PurgeFailingFor {
    inner: Arc<dyn ProviderPurge>,
    provider: &'static str,
}

#[async_trait]
impl ProviderPurge for PurgeFailingFor {
    async fn purge_user_provider_data(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge> {
        if provider == self.provider {
            return Err(AppError::database("injected purge failure"));
        }
        self.inner
            .purge_user_provider_data(user_id, tenant_id, provider, reason)
            .await
    }

    async fn purge_user_provider_wellness(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge> {
        self.inner
            .purge_user_provider_wellness(user_id, tenant_id, provider, reason)
            .await
    }

    async fn purge_user_provider_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge> {
        self.inner
            .purge_user_provider_activities(user_id, tenant_id, provider, reason)
            .await
    }

    async fn purge_user_provider_source_wellness(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        source: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge> {
        self.inner
            .purge_user_provider_source_wellness(user_id, tenant_id, provider, source, reason)
            .await
    }

    async fn purge_user_provider_source_data(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        source: &str,
        reason: PurgeReason,
    ) -> AppResult<ProviderDataPurge> {
        self.inner
            .purge_user_provider_source_data(user_id, tenant_id, provider, source, reason)
            .await
    }

    async fn purge_provider_data(&self, provider: &str) -> AppResult<ProviderDataPurge> {
        self.inner.purge_provider_data(provider).await
    }
}

/// Wahoo (carnet#34): a disconnect `DELETE`s the athlete's permissions with
/// their access token as `Bearer` — with the portal's cleanup flags on, that
/// also removes the workouts and plans Dravr scheduled on their account — and
/// the rows and cached activities die locally.
#[cfg(feature = "provider-wahoo")]
#[tokio::test]
async fn disconnect_wahoo_deletes_the_athletes_permissions() {
    let mut upstream = ScriptedUpstream::serve(vec![OK_204.to_owned()]).await;
    let resources = server_revoking_at(
        "wahoo",
        "wahoo-test-client",
        "wahoo-test-secret",
        format!("{}/oauth/token", upstream.base_url),
        format!("{}/v1/permissions", upstream.base_url),
    )
    .await;
    let service = oauth_service(&resources);
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "wahoo",
        "wahoo-access-live-do-not-log",
        Some("wahoo-refresh-do-not-log"),
        Timestamp::now() + Elapsed::hours(1),
    )
    .await;
    service
        .disconnect_provider(
            user_id,
            "wahoo",
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("disconnect succeeds");
    let request = upstream.next_request("Wahoo deauthorization").await;
    assert_eq!(request.method, "DELETE");
    assert_eq!(request.target, "/v1/permissions");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer wahoo-access-live-do-not-log"),
        "Wahoo deauthorizes with the athlete's own access token"
    );
    assert_locally_disconnected(&resources, user_id, tenant_id, "wahoo").await;
}

/// Disconnects of one held pair arriving at the same moment must not both
/// clear it (carnet#1838). Each reads the pair held before any clears, so
/// without the clear lock every one of them revoked, purged both backends and
/// counted `provider.disconnected`. Serialised, the first clears and emits,
/// and the others wait, find nothing held and succeed as no-ops.
#[tokio::test]
async fn concurrent_disconnects_of_one_pair_clear_and_emit_once() {
    const CONCURRENT: usize = 5;
    // One answer per request, so a run without the lock fails on its counts
    // rather than on a refused connection.
    let strava_upstream = ScriptedUpstream::serve(vec![OK_200.to_owned(); CONCURRENT]).await;
    let resources = strava_revoking_at(format!("{}/oauth/revoke", strava_upstream.base_url)).await;
    let service = oauth_service(&resources);
    let (user_id, tenant_id) = seed_connected(
        &resources,
        "strava",
        "strava-access-do-not-log",
        Some("strava-refresh-do-not-log"),
        Timestamp::now() + Elapsed::days(30),
    )
    .await;
    let since = Timestamp::now() - Elapsed::minutes(1);
    let (events, _guard) = capture_notify();

    // Both halves of the coalesced card, so the card id and the backend id
    // are proven to share one lock.
    let results = future::join_all((0..CONCURRENT).map(|i| {
        let provider = if i % 2 == 0 { "strava" } else { "sciotte" };
        service.disconnect_provider(
            user_id,
            provider,
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
    }))
    .await;

    for (i, result) in results.iter().enumerate() {
        assert!(
            result.is_ok(),
            "concurrent disconnect {i} fails instead of finding nothing held: {result:?}"
        );
    }
    assert_locally_disconnected(&resources, user_id, tenant_id, "strava").await;
    assert_purge_counts(
        &resources,
        user_id,
        tenant_id,
        since,
        1,
        "concurrent disconnects",
    )
    .await;
    assert_eq!(
        named(&events, "provider.disconnected").len(),
        1,
        "concurrent disconnects count one provider.disconnected"
    );
}
