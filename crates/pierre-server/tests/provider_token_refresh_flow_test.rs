// ABOUTME: carnet#718 — the platform token refresh is descriptor-driven, single-flight and written by compare-and-swap
// ABOUTME: Drives AuthService against a mock token endpoint that rotates its refresh token on every refresh
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A provider that rotates refresh tokens honours each one once, so two
//! refreshes of one connection used to cost the athlete their grant: the
//! second presented a spent token, was refused, and flipped the connection to
//! `needs_reauth`. These tests refresh a provider this codebase knows nothing
//! about, registered from a descriptor alone, against a local token endpoint
//! that rotates like that and counts its calls.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{Duration, Utc};
use pierre_core::models::{ConnectionStatus, ConnectionType, TenantId, UserOAuthToken};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::core::ProviderConfig;
use pierre_providers::registry::ProviderRegistry;
#[cfg(feature = "provider-garmin")]
use pierre_providers::spi::GarminDescriptor;
use pierre_providers::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor,
    RefreshClientAuth,
};
use pierre_tool_runtime::protocol::auth::{AuthService, TokenData};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// A provider the platform has no client, descriptor or refresh code for:
/// everything a refresh of it needs is what the descriptor registered here
/// declares. The slug is `fitbit` because the stores a client's credentials
/// are read from only accept the slugs their `CHECK` constraints list, and
/// that one has no provider behind it.
const PROVIDER: &str = "fitbit";
const CLIENT_ID: &str = "rotato-client";
const CLIENT_SECRET: &str = "rotato-secret";

/// The pair stored before any refresh.
const FIRST_ACCESS: &str = "access-gen-0";
const FIRST_REFRESH: &str = "refresh-gen-0";

/// The pair another server instance's refresh stores.
const WINNER_ACCESS: &str = "access-of-the-winner";
const WINNER_REFRESH: &str = "refresh-of-the-winner";

/// How long the refresh waits for a winner after a refusal: the sum of the
/// pauses in `WINNER_WINDOW` (250 + 500 + 1000 ms).
const WINNER_WINDOW: StdDuration = StdDuration::from_millis(1750);

struct RotatoDescriptor;

impl ProviderDescriptor for RotatoDescriptor {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn display_name(&self) -> &'static str {
        "Rotato"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::OAUTH.union(ProviderCapabilities::ACTIVITIES)
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        Some(OAuthEndpoints {
            auth_url: "https://rotato.invalid/oauth/authorize",
            token_url: "https://rotato.invalid/oauth/token",
            revoke_url: None,
        })
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        Some(OAuthParams {
            scope_separator: " ",
            use_pkce: false,
            additional_auth_params: &[],
        })
    }

    /// Neither Strava's shape nor WHOOP's: the client in a Basic header, and a
    /// field of its own.
    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        Some(OAuthRefresh {
            client_auth: RefreshClientAuth::BasicHeader,
            extra_form: &[("audience", "rotato-api")],
        })
    }

    fn api_base_url(&self) -> &'static str {
        "https://rotato.invalid/api"
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &["read"]
    }
}

/// Where a winner's write lands, and for whom.
#[derive(Clone)]
struct Athlete {
    repos: Arc<RepositoryRegistry>,
    user_id: Uuid,
    tenant: TenantId,
    provider: &'static str,
}

impl Athlete {
    async fn stored(&self) -> UserOAuthToken {
        self.repos
            .oauth_tokens
            .get_token(self.user_id, self.tenant, self.provider)
            .await
            .unwrap()
            .expect("the athlete has a token")
    }

    /// Another server instance's refresh landing: it read the row, spent its
    /// refresh token at the vendor, and swaps its pair in.
    async fn land_a_winner(&self) {
        let row = self.stored().await;
        let landed = self
            .repos
            .oauth_tokens
            .refresh_token(
                &row,
                WINNER_ACCESS,
                Some(WINNER_REFRESH),
                Some(Utc::now() + Duration::hours(6)),
            )
            .await
            .unwrap();
        assert!(landed, "the winner swaps over the row it read");
    }

    async fn connection(&self) -> ConnectionStatus {
        self.repos
            .provider_connections
            .get_for_user(self.user_id, Some(self.tenant))
            .await
            .unwrap()
            .into_iter()
            .find(|connection| connection.provider == self.provider)
            .expect("the athlete has a connection")
            .status
    }
}

/// A token endpoint that honours each refresh token once.
struct RotatingVendor {
    /// The one refresh token it honours now.
    honoured: Mutex<String>,
    /// Refreshes answered, honoured or refused.
    calls: AtomicUsize,
    /// `(authorization header, form body)` of every request.
    requests: Mutex<Vec<(String, String)>>,
    /// How long it takes to answer, so concurrent callers overlap.
    latency: StdDuration,
    /// Refuse every refresh, whatever it presents: the answer the loser of a
    /// cross-instance race gets.
    refuse_all: bool,
    /// Lands a winner's write while a refresh is being answered, when set.
    winner_during_refresh: Mutex<Option<Athlete>>,
    /// Lands a winner's write this long after a refresh was answered, when set.
    winner_after_answer: Mutex<Option<(Athlete, StdDuration)>>,
}

impl RotatingVendor {
    fn new(latency: StdDuration, refuse_all: bool) -> Self {
        Self {
            honoured: Mutex::new(FIRST_REFRESH.to_owned()),
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            latency,
            refuse_all,
            winner_during_refresh: Mutex::new(None),
            winner_after_answer: Mutex::new(None),
        }
    }

    /// Spend `presented` if it is the honoured token, and return the pair that
    /// replaces it.
    fn rotate(&self, body: &str) -> Option<(String, String)> {
        let mut honoured = self.honoured.lock().unwrap();
        if self.refuse_all || !body.contains(&format!("refresh_token={}", *honoured)) {
            return None;
        }
        let generation = self.calls.load(Ordering::SeqCst);
        let pair = (
            format!("access-gen-{generation}"),
            format!("refresh-gen-{generation}"),
        );
        honoured.clone_from(&pair.1);
        Some(pair)
    }
}

async fn serve(vendor: Arc<RotatingVendor>) -> String {
    let app = Router::new().route(
        "/oauth/token",
        post(move |headers: HeaderMap, body: String| {
            let vendor = Arc::clone(&vendor);
            async move {
                let authorization = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                vendor
                    .requests
                    .lock()
                    .unwrap()
                    .push((authorization, body.clone()));
                // A vendor finishes a refresh it has started whether or not
                // the client is still there to read the answer, and a handler
                // is dropped with its connection: so the rotation is a task.
                let answer = tokio::spawn(async move {
                    sleep(vendor.latency).await;
                    vendor.calls.fetch_add(1, Ordering::SeqCst);

                    let during = vendor.winner_during_refresh.lock().unwrap().take();
                    if let Some(athlete) = during {
                        athlete.land_a_winner().await;
                    }
                    let after = vendor.winner_after_answer.lock().unwrap().take();
                    if let Some((athlete, delay)) = after {
                        tokio::spawn(async move {
                            sleep(delay).await;
                            athlete.land_a_winner().await;
                        });
                    }
                    vendor.rotate(&body)
                });

                match answer.await.expect("the vendor answers") {
                    Some((access, refresh)) => Json(json!({
                        "access_token": access,
                        "refresh_token": refresh,
                        "token_type": "Bearer",
                        "expires_in": 86_400,
                    }))
                    .into_response(),
                    // RFC 6749 section 5.2: a spent refresh token.
                    None => (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "invalid_grant" })),
                    )
                        .into_response(),
                }
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}/oauth/token")
}

/// A server context whose provider registry also knows [`PROVIDER`], by its
/// descriptor and a token endpoint pointed at the mock, and an athlete with an
/// expired token of it over an active connection, under their own app.
async fn connected_to(vendor: &Arc<RotatingVendor>) -> (Arc<ServerContext>, Athlete) {
    connected_as(
        vendor,
        PROVIDER,
        Box::new(RotatoDescriptor),
        RotatoDescriptor.to_config(),
        None,
    )
    .await
}

/// [`connected_to`] for any registered provider: `descriptor` and `config`
/// are what the registry holds for `provider`, with the token endpoint pointed
/// at the mock, and the stored token carries `owner_id` when given.
async fn connected_as(
    vendor: &Arc<RotatingVendor>,
    provider: &'static str,
    descriptor: Box<dyn ProviderDescriptor>,
    config: ProviderConfig,
    owner_id: Option<&str>,
) -> (Arc<ServerContext>, Athlete) {
    let token_url = serve(Arc::clone(vendor)).await;
    let base = common::create_test_server_resources().await.unwrap();

    let mut registry = ProviderRegistry::new();
    registry.register_descriptor(provider, descriptor);
    registry.set_default_config(
        provider,
        ProviderConfig {
            token_url,
            ..config
        },
    );
    let mut context = (*base).clone();
    context.fitness.provider_registry = Arc::new(registry);
    let resources = Arc::new(context);

    let email = format!("rotating-{}@example.com", Uuid::new_v4());
    let (user_id, _, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let repos = Arc::clone(&resources.common.repos);
    repos
        .oauth_tokens
        .store_user_oauth_app(
            user_id,
            provider,
            CLIENT_ID,
            CLIENT_SECRET,
            "http://localhost/callback",
        )
        .await
        .unwrap();
    let mut token = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        provider.to_owned(),
        FIRST_ACCESS.to_owned(),
        Some(FIRST_REFRESH.to_owned()),
        Some(Utc::now() - Duration::hours(1)),
        Some("read".to_owned()),
    );
    token.provider_user_id = owner_id.map(str::to_owned);
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(user_id, tenant, provider, &ConnectionType::OAuth, None)
        .await
        .unwrap();

    (
        resources,
        Athlete {
            repos,
            user_id,
            tenant,
            provider,
        },
    )
}

/// One caller's lookup, through a service of its own: `AuthService` is built
/// per call in production, so nothing on it can be what callers share.
async fn lookup(resources: &Arc<ServerContext>, athlete: &Athlete) -> Option<TokenData> {
    AuthService::new(Arc::clone(resources) as Arc<dyn ToolRuntime>)
        .get_valid_token(
            athlete.user_id,
            athlete.provider,
            Some(&athlete.tenant.to_string()),
        )
        .await
        .expect("the lookup does not error")
}

/// Two callers find the same expired token at once (a tool call and the
/// capture sweep). They share one refresh: the vendor is called once, both get
/// the pair it issued, and the connection stays active. Refreshing twice would
/// present the spent refresh token the second time, and be refused.
#[tokio::test]
async fn two_concurrent_lookups_share_one_refresh_of_a_rotating_token() {
    let vendor = Arc::new(RotatingVendor::new(StdDuration::from_millis(300), false));
    let (resources, athlete) = connected_to(&vendor).await;

    let (first, second) = tokio::join!(lookup(&resources, &athlete), lookup(&resources, &athlete));

    assert_eq!(
        vendor.calls.load(Ordering::SeqCst),
        1,
        "exactly one call to the vendor"
    );
    let first = first.expect("the first caller gets a token");
    let second = second.expect("the second caller gets a token");
    assert_eq!(first.access_token, "access-gen-1");
    assert_eq!(first.refresh_token, "refresh-gen-1");
    assert_eq!(second.access_token, "access-gen-1");
    assert_eq!(second.refresh_token, "refresh-gen-1");
    assert_eq!(athlete.connection().await, ConnectionStatus::Active);
    let stored = athlete.stored().await;
    assert_eq!(stored.access_token, "access-gen-1");
    assert_eq!(stored.refresh_token.as_deref(), Some("refresh-gen-1"));
}

/// A token issued without an expiry (an Intervals.icu OAuth token never
/// expires) is stored with none and read back with none: the caller gets the
/// stored pair untouched, with no expiry invented for it and no refresh.
#[tokio::test]
async fn a_stored_token_with_no_expiry_reads_back_with_no_expiry() {
    let vendor = Arc::new(RotatingVendor::new(StdDuration::ZERO, false));
    let (resources, athlete) = connected_to(&vendor).await;
    let row = athlete.stored().await;
    let landed = athlete
        .repos
        .oauth_tokens
        .refresh_token(&row, FIRST_ACCESS, Some(FIRST_REFRESH), None)
        .await
        .unwrap();
    assert!(landed, "the row now carries no expiry");

    let token = lookup(&resources, &athlete)
        .await
        .expect("the athlete gets a token");

    assert_eq!(token.expires_at, None, "no expiry is invented on read");
    assert_eq!(token.access_token, FIRST_ACCESS);
    assert_eq!(vendor.calls.load(Ordering::SeqCst), 0, "nothing to refresh");
}

/// The caller whose lookup started the refresh goes away while the vendor is
/// answering (a client that disconnected, an outer timeout). The vendor has
/// rotated all the same, so the refresh must not stop with its caller: it
/// stores the new pair, and the next caller is served it. Stopped there, the
/// row would keep a refresh token the vendor no longer honours, and the next
/// refresh would be refused and flag the connection.
#[tokio::test]
async fn a_refresh_whose_caller_was_dropped_still_stores_the_rotated_pair() {
    let vendor = Arc::new(RotatingVendor::new(StdDuration::from_millis(300), false));
    let (resources, athlete) = connected_to(&vendor).await;

    let dropped = {
        let (resources, athlete) = (Arc::clone(&resources), athlete.clone());
        tokio::spawn(async move { lookup(&resources, &athlete).await })
    };
    // The refresh has reached the vendor, which has not answered yet.
    timeout(StdDuration::from_secs(10), async {
        while vendor.requests.lock().unwrap().is_empty() {
            sleep(StdDuration::from_millis(5)).await;
        }
    })
    .await
    .expect("the refresh reaches the vendor");
    assert_eq!(vendor.calls.load(Ordering::SeqCst), 0, "still answering");
    dropped.abort();
    assert!(dropped.await.unwrap_err().is_cancelled());

    let later = lookup(&resources, &athlete)
        .await
        .expect("a later caller gets a token");

    assert_eq!(later.access_token, "access-gen-1");
    assert_eq!(later.refresh_token, "refresh-gen-1");
    assert_eq!(
        vendor.calls.load(Ordering::SeqCst),
        1,
        "the one refresh the dropped caller started, and no second one"
    );
    let stored = athlete.stored().await;
    assert_eq!(stored.access_token, "access-gen-1");
    assert_eq!(stored.refresh_token.as_deref(), Some("refresh-gen-1"));
    assert_eq!(athlete.connection().await, ConnectionStatus::Active);
}

/// A provider the platform has no code for refreshes from its descriptor
/// alone, in the shape the descriptor declares: the client in a Basic header
/// and out of the body, the declared extra field, and the token endpoint the
/// registry carries for it.
#[tokio::test]
async fn a_provider_refreshes_from_its_registered_descriptor_alone() {
    let vendor = Arc::new(RotatingVendor::new(StdDuration::ZERO, false));
    let (resources, athlete) = connected_to(&vendor).await;

    let token = lookup(&resources, &athlete)
        .await
        .expect("the refreshed token");

    assert_eq!(token.access_token, "access-gen-1");
    assert_eq!(token.provider, PROVIDER);
    let requests = vendor.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "one refresh");
    let (authorization, body) = &requests[0];
    assert_eq!(
        authorization,
        &format!(
            "Basic {}",
            BASE64.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"))
        )
    );
    assert_eq!(
        body,
        &format!("grant_type=refresh_token&refresh_token={FIRST_REFRESH}&audience=rotato-api"),
        "the grant, the stored refresh token and the declared field; no client credential"
    );
    assert_eq!(athlete.connection().await, ConnectionStatus::Active);

    // The rotated pair is the one stored, and it is unexpired: the next lookup
    // serves it without another refresh.
    let again = lookup(&resources, &athlete)
        .await
        .expect("the stored token");
    assert_eq!(again.access_token, "access-gen-1");
    assert_eq!(vendor.calls.load(Ordering::SeqCst), 1);
}

/// Another server instance's refresh lands while this one is at the vendor.
/// This one's write is a compare-and-swap on the row it read, so it loses: the
/// winner's pair stays stored, the caller gets the winner's token, and the
/// connection stays active.
#[tokio::test]
async fn a_refresh_that_loses_the_swap_returns_the_winners_token() {
    let vendor = Arc::new(RotatingVendor::new(StdDuration::ZERO, false));
    let (resources, athlete) = connected_to(&vendor).await;
    *vendor.winner_during_refresh.lock().unwrap() = Some(athlete.clone());

    let token = lookup(&resources, &athlete)
        .await
        .expect("the winner's token is usable");

    assert_eq!(vendor.calls.load(Ordering::SeqCst), 1, "the refresh ran");
    assert_eq!(token.access_token, WINNER_ACCESS);
    assert_eq!(token.refresh_token, WINNER_REFRESH);
    let stored = athlete.stored().await;
    assert_eq!(
        stored.access_token, WINNER_ACCESS,
        "the pair that lost the swap is not written over the winner's"
    );
    assert_eq!(stored.refresh_token.as_deref(), Some(WINNER_REFRESH));
    assert_eq!(athlete.connection().await, ConnectionStatus::Active);
}

/// The loser of a race between two server instances: the vendor refuses the
/// refresh token the winner already spent, and the winner's write has not
/// landed yet. The refusal is not believed at once; the winner lands inside
/// the window, its token is adopted and the connection is never flagged.
#[tokio::test]
async fn a_refused_refresh_adopts_a_winner_that_lands_inside_the_window() {
    let vendor = Arc::new(RotatingVendor::new(StdDuration::ZERO, true));
    let (resources, athlete) = connected_to(&vendor).await;
    *vendor.winner_after_answer.lock().unwrap() =
        Some((athlete.clone(), StdDuration::from_millis(400)));

    let token = lookup(&resources, &athlete)
        .await
        .expect("the winner's token is adopted");

    assert_eq!(vendor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(token.access_token, WINNER_ACCESS);
    assert_eq!(
        athlete.connection().await,
        ConnectionStatus::Active,
        "the refusal of a refresh token the winner spent is no verdict on the grant"
    );
}

/// A refusal no winner explains is a dead grant: once the window has passed
/// over a row still as it was read, the connection needs re-authorizing and
/// the caller gets no token.
#[tokio::test]
async fn a_refused_refresh_no_winner_explains_needs_a_reconnect() {
    let vendor = Arc::new(RotatingVendor::new(StdDuration::ZERO, true));
    let (resources, athlete) = connected_to(&vendor).await;

    let started = Instant::now();
    let token = lookup(&resources, &athlete).await;
    let waited = started.elapsed();

    assert!(token.is_none(), "no usable token");
    assert_eq!(athlete.connection().await, ConnectionStatus::NeedsReauth);
    assert!(
        waited >= WINNER_WINDOW,
        "a winner was given the whole window before the flip: {waited:?}"
    );
    assert_eq!(
        athlete.stored().await.access_token,
        FIRST_ACCESS,
        "the row is left as it was"
    );

    // The connection is flagged now, so a later refusal is not waited on:
    // every call on a dead connection would otherwise pay the window.
    let started = Instant::now();
    assert!(lookup(&resources, &athlete).await.is_none());
    assert!(
        started.elapsed() < WINNER_WINDOW,
        "a connection already flagged is not waited on again: {:?}",
        started.elapsed()
    );
    assert_eq!(vendor.calls.load(Ordering::SeqCst), 2);
    assert_eq!(athlete.connection().await, ConnectionStatus::NeedsReauth);
}

/// Garmin refreshes from its descriptor (carnet#737), with exactly the request
/// Garmin's "OAuth2.0 PKCE Specification" gives: `client_id`, `client_secret`,
/// `grant_type=refresh_token` and the refresh token in the form body, no
/// Basic header and no other field. Garmin returns a new refresh token every
/// time, so the rotated one is what is stored, and the next expiry spends it.
#[cfg(feature = "provider-garmin")]
#[tokio::test]
async fn garmin_refreshes_with_the_spec_form_and_keeps_the_rotated_token() {
    const GARMIN_USER: &str = "garmin-user-737";
    let vendor = Arc::new(RotatingVendor::new(StdDuration::ZERO, false));
    let (resources, athlete) = connected_as(
        &vendor,
        "garmin",
        Box::new(GarminDescriptor),
        GarminDescriptor.to_config(),
        Some(GARMIN_USER),
    )
    .await;

    let token = lookup(&resources, &athlete)
        .await
        .expect("the expired Garmin token is refreshed");

    let requests = vendor.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "one refresh");
    let (authorization, body) = &requests[0];
    assert_eq!(authorization, "", "Garmin takes the client in the body");
    assert_eq!(
        body,
        &format!(
            "client_id={CLIENT_ID}&client_secret={CLIENT_SECRET}\
             &grant_type=refresh_token&refresh_token={FIRST_REFRESH}"
        ),
        "the spec's refresh form, and nothing else"
    );
    assert_eq!(token.access_token, "access-gen-1");
    assert_eq!(token.refresh_token, "refresh-gen-1");
    assert_eq!(token.provider_user_id.as_deref(), Some(GARMIN_USER));
    let stored = athlete.stored().await;
    assert_eq!(stored.access_token, "access-gen-1");
    assert_eq!(
        stored.refresh_token.as_deref(),
        Some("refresh-gen-1"),
        "the rotated refresh token is stored"
    );
    assert_eq!(stored.provider_user_id.as_deref(), Some(GARMIN_USER));

    // The next expiry spends the rotated token: the vendor no longer honours
    // the first one, so a refresh that kept it would be refused here.
    let landed = athlete
        .repos
        .oauth_tokens
        .refresh_token(
            &stored,
            &stored.access_token,
            stored.refresh_token.as_deref(),
            Some(Utc::now() - Duration::hours(1)),
        )
        .await
        .unwrap();
    assert!(landed, "the row is expired again");
    let again = lookup(&resources, &athlete)
        .await
        .expect("the rotated refresh token is honoured");
    assert_eq!(again.access_token, "access-gen-2");
    assert_eq!(again.refresh_token, "refresh-gen-2");
    let (_, second_body) = vendor.requests.lock().unwrap()[1].clone();
    assert!(
        second_body.ends_with("&refresh_token=refresh-gen-1"),
        "the second refresh presents the rotated token: {second_body}"
    );
    assert_eq!(athlete.connection().await, ConnectionStatus::Active);
}

/// The platform refreshes a stored token within ten minutes of its expiry,
/// the one window every provider shares (Garmin asks for at least 600 s): a
/// token expiring in nine minutes is refreshed, one expiring in eleven is
/// served as it is.
#[tokio::test]
async fn a_token_is_refreshed_within_ten_minutes_of_its_expiry() {
    let vendor = Arc::new(RotatingVendor::new(StdDuration::ZERO, false));
    let (resources, athlete) = connected_to(&vendor).await;
    let expire_in = |minutes: i64| {
        let athlete = athlete.clone();
        async move {
            let row = athlete.stored().await;
            let landed = athlete
                .repos
                .oauth_tokens
                .refresh_token(
                    &row,
                    &row.access_token,
                    row.refresh_token.as_deref(),
                    Some(Utc::now() + Duration::minutes(minutes)),
                )
                .await
                .unwrap();
            assert!(landed);
        }
    };

    expire_in(11).await;
    let served = lookup(&resources, &athlete).await.expect("a token");
    assert_eq!(
        served.access_token, FIRST_ACCESS,
        "eleven minutes out: no refresh"
    );
    assert_eq!(vendor.calls.load(Ordering::SeqCst), 0);

    expire_in(9).await;
    let refreshed = lookup(&resources, &athlete).await.expect("a token");
    assert_eq!(
        refreshed.access_token, "access-gen-1",
        "nine minutes out: refreshed"
    );
    assert_eq!(vendor.calls.load(Ordering::SeqCst), 1);
}
