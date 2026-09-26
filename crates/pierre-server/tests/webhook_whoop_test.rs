// ABOUTME: Pins that a signed WHOOP webhook syncs the one user whose token carries the event's WHOOP user id
// ABOUTME: Drives the real /webhooks/whoop route with a mock SyncProvider registered as "whoop"; a no-op fails it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! WHOOP webhook route suite (carnet#457).
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
//
// Until dravr-enforme 2a92afb the orchestrator's `handle_webhook` validated,
// parsed, logged and returned `()`, so the route could not act on an event.
// It now returns the events; the route maps each event's WHOOP user id to the
// platform user whose token carries it and runs that user's sync.
//
// Two layers are pinned separately, because the platform's harness affords
// no seam for enforme's WHOOP API base URL (`WHOOP_API_BASE` is a constant in
// dravr-enforme):
//
// - The real orchestrator the server builds, with the real WHOOP provider,
//   is driven directly: a body signed with `WHOOP_WEBHOOK_SECRET` comes back
//   as an event naming the WHOOP user id; a forged signature is refused.
// - The HTTP route is driven end to end with a `SyncProvider` mock
//   registered under the name "whoop", whose fetch returns one sleep
//   session. That is the strongest assertion available: the sync ran for
//   THAT user and stored a real row through the platform's stores, so
//   `last_sync` is stamped and the SSE stream carries the record count.
#![cfg(all(feature = "health-sync", feature = "provider-whoop"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::HashMap;
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::Utc;
use dravr_equilibre_sync::ContinuousMetricBatch;
use hmac::{Hmac, Mac};
use http::HeaderMap;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::models::{
    StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession, TenantId, UserOAuthToken,
};
use pierre_enforme::error::{EnformeError, EnformeResult};
use pierre_enforme::models::connection::ProviderCredentials;
use pierre_enforme::models::cursor::{SyncBatch, SyncCursor};
use pierre_enforme::models::webhook::{WebhookAlgorithm, WebhookConfig, WebhookEvent};
use pierre_enforme::providers::whoop::verify_whoop_signature;
use pierre_enforme::traits::sync_provider::{DataType, SyncProvider};
use pierre_enforme::{SyncConfig, SyncDeps, SyncOrchestrator};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::webhooks::WebhookRoutes;
use pierre_services::health_sync::PierreSyncStorage;
use serde_json::{json, Value};
use serial_test::serial;
use sha2::Sha256;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// The webhook secret the provider reads at construction.
const SECRET: &str = "whoop-webhook-secret-for-tests";

/// The WHOOP user id the seeded token carries and the events name.
const WHOOP_USER_ID: u64 = 12_345;

/// The `X-WHOOP-Signature-Timestamp` every test request carries: the time
/// the test binary first signs, well inside the provider's replay window for
/// the few seconds the tests run.
static TIMESTAMP: LazyLock<String> = LazyLock::new(|| Utc::now().timestamp_millis().to_string());

/// Environment set for the duration of one test and removed after it.
struct EnvGuard {
    keys: Vec<&'static str>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, String)]) -> Self {
        for (key, value) in vars {
            env::set_var(key, value);
        }
        Self {
            keys: vars.iter().map(|(key, _)| *key).collect(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            env::remove_var(key);
        }
    }
}

/// A WHOOP signature as WHOOP documents it: base64 of the HMAC-SHA256 of
/// [`TIMESTAMP`] followed by `body`, under `secret`, computed here with the
/// `hmac` crate rather than enforme's own helper.
fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(TIMESTAMP.as_str().as_bytes());
    mac.update(body);
    BASE64.encode(mac.finalize().into_bytes())
}

/// Headers of a request signed with `signature` at [`TIMESTAMP`].
fn signed_headers(signature: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-whoop-signature", signature.parse().unwrap());
    headers.insert("x-whoop-signature-timestamp", TIMESTAMP.parse().unwrap());
    headers
}

fn whoop_payload(event_type: &str, user_id: u64, id: &str) -> Vec<u8> {
    json!({ "type": event_type, "user_id": user_id, "id": id })
        .to_string()
        .into_bytes()
}

/// A WHOOP-shaped `SyncProvider`: the real signature check and payload shape,
/// with a fetch that answers one sleep session instead of calling WHOOP.
struct MockWhoop {
    secret: String,
    fetches: AtomicUsize,
    synced_users: Mutex<Vec<String>>,
}

impl MockWhoop {
    fn new() -> Self {
        Self {
            secret: env::var("WHOOP_WEBHOOK_SECRET").unwrap_or_default(),
            fetches: AtomicUsize::new(0),
            synced_users: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl SyncProvider for MockWhoop {
    fn name(&self) -> &'static str {
        "whoop"
    }

    fn supported_data_types(&self) -> &[DataType] {
        &[DataType::Sleep]
    }

    async fn fetch_sleep(
        &self,
        creds: &ProviderCredentials,
        _cursor: Option<&SyncCursor>,
    ) -> EnformeResult<SyncBatch<StoredSleepSession>> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        self.synced_users
            .lock()
            .unwrap()
            .push(creds.user_id.clone());
        let now = Utc::now();
        let mut cursor = SyncCursor::new(creds.user_id.clone(), "whoop", "sleep");
        cursor.complete("sleep-cursor-1", 1);
        Ok(SyncBatch {
            records: vec![StoredSleepSession {
                id: Uuid::new_v4().to_string(),
                user_id: creds.user_id.clone(),
                data_source_id: String::new(),
                is_nap: false,
                start_datetime: now - chrono::Duration::hours(8),
                end_datetime: now,
                total_sleep_seconds: Some(27_000),
                deep_sleep_seconds: Some(5_400),
                light_sleep_seconds: Some(14_400),
                rem_sleep_seconds: Some(7_200),
                awake_seconds: Some(1_800),
                sleep_efficiency: Some(93.5),
                avg_heart_rate: Some(52.0),
                min_heart_rate: Some(46),
                avg_hrv: Some(71.0),
                sleep_score: Some(88),
                stages: Vec::new(),
                source_name: "whoop".to_owned(),
            }],
            cursor,
            has_more: false,
        })
    }

    async fn fetch_recovery(
        &self,
        creds: &ProviderCredentials,
        _cursor: Option<&SyncCursor>,
    ) -> EnformeResult<SyncBatch<StoredRecoveryMetrics>> {
        Ok(SyncBatch::empty(SyncCursor::new(
            creds.user_id.clone(),
            "whoop",
            "recovery",
        )))
    }

    async fn fetch_health(
        &self,
        creds: &ProviderCredentials,
        _cursor: Option<&SyncCursor>,
    ) -> EnformeResult<SyncBatch<StoredHealthMetrics>> {
        Ok(SyncBatch::empty(SyncCursor::new(
            creds.user_id.clone(),
            "whoop",
            "health",
        )))
    }

    async fn fetch_continuous(
        &self,
        creds: &ProviderCredentials,
        _cursor: Option<&SyncCursor>,
    ) -> EnformeResult<SyncBatch<ContinuousMetricBatch>> {
        Ok(SyncBatch::empty(SyncCursor::new(
            creds.user_id.clone(),
            "whoop",
            "continuous",
        )))
    }

    async fn on_connected(
        &self,
        _creds: &ProviderCredentials,
        _webhook_url: &str,
    ) -> EnformeResult<()> {
        Ok(())
    }

    async fn on_disconnected(&self, _creds: &ProviderCredentials) -> EnformeResult<()> {
        Ok(())
    }

    fn webhook_config(&self) -> Option<WebhookConfig> {
        Some(WebhookConfig {
            signature_header: "x-whoop-signature",
            algorithm: WebhookAlgorithm::HmacSha256,
            needs_verification: true,
        })
    }

    async fn validate_webhook(&self, headers: &HeaderMap, body: &[u8]) -> EnformeResult<bool> {
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| EnformeError::WebhookValidationFailed {
                    provider: "whoop".to_owned(),
                    reason: format!("missing {name} header"),
                })
        };
        // The real WHOOP check, so the route is exercised against it.
        verify_whoop_signature(
            self.secret.as_bytes(),
            header("x-whoop-signature-timestamp")?,
            body,
            header("x-whoop-signature")?,
        )
    }

    // WHOOP's own shape is one object per body; a JSON array of them is
    // accepted too so the route's per-payload owner dedupe can be exercised.
    async fn parse_webhook(&self, body: &[u8]) -> EnformeResult<Vec<WebhookEvent>> {
        let payload: Value =
            serde_json::from_slice(body).map_err(|e| EnformeError::serialization(e.to_string()))?;
        let objects = match payload {
            Value::Array(items) => items,
            single => vec![single],
        };
        Ok(objects
            .iter()
            .map(|event| {
                WebhookEvent::new(
                    "whoop",
                    event["type"].as_str().unwrap_or_default(),
                    event["user_id"].as_u64().unwrap_or_default().to_string(),
                    event["id"].as_str().unwrap_or_default(),
                )
            })
            .collect())
    }
}

/// A server context whose sync orchestrator runs the mock WHOOP provider over
/// the platform's real stores.
///
/// The secret guard must be alive before this call: both the real provider
/// (built inside `ServerContext::new`) and the mock read the secret at
/// construction, the way production does.
async fn context_with_mock_whoop() -> (Arc<ServerContext>, Arc<MockWhoop>) {
    let context = common::create_test_server_resources().await.unwrap();
    let mut context = Arc::try_unwrap(context)
        .map_err(|_| "the fresh context has one owner")
        .unwrap();

    let mock = Arc::new(MockWhoop::new());
    let storage = Arc::new(PierreSyncStorage::new(&context.common.repos));
    let deps = Arc::new(SyncDeps {
        sleep: Arc::clone(&storage) as _,
        recovery: Arc::clone(&storage) as _,
        health: Arc::clone(&storage) as _,
        time_series: Arc::clone(&storage) as _,
        data_sources: Arc::clone(&storage) as _,
        cursors: Arc::clone(&storage) as _,
        credentials: Arc::clone(&storage) as _,
        connections: storage as _,
    });
    let mut providers: HashMap<String, Box<dyn SyncProvider>> = HashMap::new();
    providers.insert(
        "whoop".to_owned(),
        Box::new(ProviderHandle(Arc::clone(&mock))),
    );
    context.fitness.sync_orchestrator = Some(Arc::new(SyncOrchestrator::new(
        deps,
        providers,
        SyncConfig::from_env(),
    )));
    (Arc::new(context), mock)
}

/// The orchestrator owns its providers as `Box<dyn SyncProvider>`; this
/// handle lets the test keep an `Arc` to the same mock for its counters.
struct ProviderHandle(Arc<MockWhoop>);

#[async_trait]
impl SyncProvider for ProviderHandle {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn supported_data_types(&self) -> &[DataType] {
        self.0.supported_data_types()
    }
    async fn fetch_sleep(
        &self,
        creds: &ProviderCredentials,
        cursor: Option<&SyncCursor>,
    ) -> EnformeResult<SyncBatch<StoredSleepSession>> {
        self.0.fetch_sleep(creds, cursor).await
    }
    async fn fetch_recovery(
        &self,
        creds: &ProviderCredentials,
        cursor: Option<&SyncCursor>,
    ) -> EnformeResult<SyncBatch<StoredRecoveryMetrics>> {
        self.0.fetch_recovery(creds, cursor).await
    }
    async fn fetch_health(
        &self,
        creds: &ProviderCredentials,
        cursor: Option<&SyncCursor>,
    ) -> EnformeResult<SyncBatch<StoredHealthMetrics>> {
        self.0.fetch_health(creds, cursor).await
    }
    async fn fetch_continuous(
        &self,
        creds: &ProviderCredentials,
        cursor: Option<&SyncCursor>,
    ) -> EnformeResult<SyncBatch<ContinuousMetricBatch>> {
        self.0.fetch_continuous(creds, cursor).await
    }
    async fn on_connected(
        &self,
        creds: &ProviderCredentials,
        webhook_url: &str,
    ) -> EnformeResult<()> {
        self.0.on_connected(creds, webhook_url).await
    }
    async fn on_disconnected(&self, creds: &ProviderCredentials) -> EnformeResult<()> {
        self.0.on_disconnected(creds).await
    }
    fn webhook_config(&self) -> Option<WebhookConfig> {
        self.0.webhook_config()
    }
    async fn validate_webhook(&self, headers: &HeaderMap, body: &[u8]) -> EnformeResult<bool> {
        self.0.validate_webhook(headers, body).await
    }
    async fn parse_webhook(&self, body: &[u8]) -> EnformeResult<Vec<WebhookEvent>> {
        self.0.parse_webhook(body).await
    }
}

/// Seed a WHOOP-connected user whose token carries `whoop_user_id`.
async fn seed_linked_user(
    resources: &ServerContext,
    email: &str,
    whoop_user_id: Option<u64>,
) -> (Uuid, TenantId) {
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&resources.agent.database, email, "starter")
            .await
            .unwrap();
    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: "whoop".to_owned(),
        access_token: "whoop_access_token".to_owned(),
        refresh_token: Some("whoop_refresh".to_owned()),
        token_type: "Bearer".to_owned(),
        expires_at: Some(now + chrono::Duration::hours(1)),
        scope: Some("read:sleep read:recovery".to_owned()),
        provider_user_id: whoop_user_id.map(|id| id.to_string()),
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    };
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();
    // Linking WHOOP records the owner authorization every account gives
    // before its OAuth flow; health sync keeps no WHOOP record without it.
    resources
        .common
        .repos
        .users
        .record_provider_terms(
            user_id,
            "whoop",
            provider_terms_version("whoop").expect("WHOOP carries a notice"),
        )
        .await
        .unwrap();
    (user_id, tenant_id)
}

async fn post_signed(
    resources: &Arc<ServerContext>,
    body: Vec<u8>,
    signature: Option<&str>,
) -> StatusCode {
    let mut request = Request::builder()
        .method("POST")
        .uri("/webhooks/whoop")
        .header("content-type", "application/json");
    if let Some(signature) = signature {
        request = request
            .header("x-whoop-signature", signature)
            .header("x-whoop-signature-timestamp", TIMESTAMP.as_str());
    }
    WebhookRoutes::routes(Arc::clone(resources))
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap()
        .status()
}

/// Wait for every turn the webhook spawned on the drain tracker to finish.
async fn await_spawned_turns(resources: &ServerContext) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !resources.common.turns.is_empty() {
        assert!(
            Instant::now() < deadline,
            "webhook-spawned turn did not finish within 15s"
        );
        sleep(Duration::from_millis(25)).await;
    }
}

async fn sleep_rows(resources: &ServerContext, user_id: Uuid, tenant_id: &TenantId) -> usize {
    resources
        .common
        .repos
        .sleep
        .get_sleep_sessions(
            user_id,
            tenant_id,
            Utc::now() - chrono::Duration::days(2),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap()
        .len()
}

/// The server's own orchestrator, with the real enforme WHOOP provider,
/// hands a correctly signed body back as an event naming the WHOOP user —
/// the contract the route acts on — and refuses a forged signature before
/// parsing anything.
#[tokio::test]
#[serial]
async fn real_whoop_provider_returns_the_validated_event_and_refuses_a_forgery() {
    let _secret = EnvGuard::set(&[("WHOOP_WEBHOOK_SECRET", SECRET.to_owned())]);
    let resources = common::create_test_server_resources().await.unwrap();
    let orchestrator = resources
        .fitness
        .sync_orchestrator
        .clone()
        .expect("the server builds a sync orchestrator");
    assert!(
        orchestrator.provider_names().contains(&"whoop"),
        "premise: the real WHOOP provider is registered"
    );

    let body = whoop_payload("workout.updated", WHOOP_USER_ID, "9d3c1b2a");
    let headers = signed_headers(&sign(SECRET, &body));

    let events = orchestrator
        .handle_webhook("whoop", &headers, &body)
        .await
        .expect("a body signed with the configured secret validates");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].provider, "whoop");
    assert_eq!(events[0].event_type, "workout.updated");
    assert_eq!(
        events[0].user_id,
        WHOOP_USER_ID.to_string(),
        "the event carries the WHOOP-side user id the route maps to a user"
    );
    assert_eq!(events[0].resource_id, "9d3c1b2a");

    let forged = signed_headers(&sign("another-secret", &body));
    let err = orchestrator
        .handle_webhook("whoop", &forged, &body)
        .await
        .unwrap_err();
    assert!(
        matches!(err, EnformeError::WebhookValidationFailed { .. }),
        "a forged signature is refused, got {err:?}"
    );

    // A hex HMAC of the body alone is not how WHOOP signs, and is refused.
    let mut body_only = HmacSha256::new_from_slice(SECRET.as_bytes()).unwrap();
    body_only.update(&body);
    let wrong_scheme = signed_headers(&hex::encode(body_only.finalize().into_bytes()));
    assert!(orchestrator
        .handle_webhook("whoop", &wrong_scheme, &body)
        .await
        .is_err());

    // A capture replayed ten minutes later is refused however well signed.
    let stale = (Utc::now() - chrono::Duration::minutes(10))
        .timestamp_millis()
        .to_string();
    let mut replay_mac = HmacSha256::new_from_slice(SECRET.as_bytes()).unwrap();
    replay_mac.update(stale.as_bytes());
    replay_mac.update(&body);
    let mut replayed = HeaderMap::new();
    replayed.insert(
        "x-whoop-signature",
        BASE64
            .encode(replay_mac.finalize().into_bytes())
            .parse()
            .unwrap(),
    );
    replayed.insert("x-whoop-signature-timestamp", stale.parse().unwrap());
    assert!(orchestrator
        .handle_webhook("whoop", &replayed, &body)
        .await
        .is_err());
}

/// A signed event for a linked user runs that user's sync: the mock fetch is
/// called for THAT user, the sleep session it returns is stored through the
/// platform's sleep store, `last_sync` is stamped and the SSE stream carries
/// the record count. A handler that validates and logs leaves all of it
/// untouched.
#[tokio::test]
#[serial]
async fn signed_event_syncs_the_owning_user() {
    let _secret = EnvGuard::set(&[("WHOOP_WEBHOOK_SECRET", SECRET.to_owned())]);
    let (resources, mock) = context_with_mock_whoop().await;
    let (user_id, tenant_id) =
        seed_linked_user(&resources, "whoop-owner@example.com", Some(WHOOP_USER_ID)).await;
    // A second linked user who must NOT be synced by someone else's event.
    let (bystander_id, bystander_tenant) = seed_linked_user(
        &resources,
        "whoop-bystander@example.com",
        Some(WHOOP_USER_ID + 1),
    )
    .await;
    assert_eq!(sleep_rows(&resources, user_id, &tenant_id).await, 0);
    let mut sse = resources
        .sse
        .sse_manager
        .register_notification_stream(user_id)
        .await;

    let body = whoop_payload("sleep.updated", WHOOP_USER_ID, "sleep-uuid-1");
    let signature = sign(SECRET, &body);
    let status = post_signed(&resources, body, Some(&signature)).await;
    assert_eq!(status, StatusCode::OK, "WHOOP is acknowledged");

    await_spawned_turns(&resources).await;

    assert_eq!(mock.fetches.load(Ordering::SeqCst), 1, "one sync ran");
    assert_eq!(
        *mock.synced_users.lock().unwrap(),
        vec![user_id.to_string()],
        "the sync ran for the user whose token carries the WHOOP id, and only them"
    );
    assert_eq!(
        sleep_rows(&resources, user_id, &tenant_id).await,
        1,
        "the fetched sleep session is stored"
    );
    assert_eq!(
        sleep_rows(&resources, bystander_id, &bystander_tenant).await,
        0,
        "the bystander is untouched"
    );

    let last_sync = resources
        .common
        .repos
        .oauth_tokens
        .get_provider_last_sync(user_id, tenant_id, "whoop")
        .await
        .unwrap()
        .expect("last_sync is stamped after a sync that created records");
    assert!(Utc::now() - last_sync < chrono::Duration::minutes(1));

    let message = sse
        .try_recv()
        .expect("the owner's SSE stream carries the sync notification");
    let payload: Value = serde_json::from_str(message.trim_start_matches("data: ").trim()).unwrap();
    assert_eq!(payload["provider"], "whoop");
    assert_eq!(payload["message"], "WHOOP data synced (1 records)");
}

/// Several events for one user in one payload run one sync; the dedupe is
/// per payload, so a later payload for the same user syncs again.
#[tokio::test]
#[serial]
async fn several_events_for_one_user_in_a_payload_sync_once() {
    let _secret = EnvGuard::set(&[("WHOOP_WEBHOOK_SECRET", SECRET.to_owned())]);
    let (resources, mock) = context_with_mock_whoop().await;
    let (user_id, _tenant_id) =
        seed_linked_user(&resources, "whoop-twice@example.com", Some(WHOOP_USER_ID)).await;

    let batch = json!([
        { "type": "sleep.updated", "user_id": WHOOP_USER_ID, "id": "sleep-1" },
        { "type": "recovery.updated", "user_id": WHOOP_USER_ID, "id": "sleep-1" },
        { "type": "workout.updated", "user_id": WHOOP_USER_ID, "id": "workout-7" }
    ])
    .to_string()
    .into_bytes();
    let signature = sign(SECRET, &batch);
    assert_eq!(
        post_signed(&resources, batch, Some(&signature)).await,
        StatusCode::OK
    );
    await_spawned_turns(&resources).await;
    assert_eq!(
        mock.fetches.load(Ordering::SeqCst),
        1,
        "three events for one user in one payload are one sync"
    );

    let again = whoop_payload("sleep.updated", WHOOP_USER_ID, "sleep-2");
    let signature = sign(SECRET, &again);
    assert_eq!(
        post_signed(&resources, again, Some(&signature)).await,
        StatusCode::OK
    );
    await_spawned_turns(&resources).await;
    assert_eq!(
        mock.fetches.load(Ordering::SeqCst),
        2,
        "a later payload syncs again"
    );
    assert_eq!(
        *mock.synced_users.lock().unwrap(),
        vec![user_id.to_string(), user_id.to_string()]
    );
}

/// A bad signature is refused with 401 and nothing is synced.
#[tokio::test]
#[serial]
async fn bad_signature_syncs_nothing() {
    let _secret = EnvGuard::set(&[("WHOOP_WEBHOOK_SECRET", SECRET.to_owned())]);
    let (resources, mock) = context_with_mock_whoop().await;
    let (user_id, tenant_id) =
        seed_linked_user(&resources, "whoop-forged@example.com", Some(WHOOP_USER_ID)).await;

    let body = whoop_payload("sleep.updated", WHOOP_USER_ID, "sleep-uuid-1");
    let forged = sign("not-the-secret", &body);
    assert_eq!(
        post_signed(&resources, body.clone(), Some(&forged)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post_signed(&resources, body, None).await,
        StatusCode::UNAUTHORIZED,
        "a missing signature is refused too"
    );
    assert_eq!(resources.common.turns.len(), 0, "no sync turn was spawned");
    assert_eq!(mock.fetches.load(Ordering::SeqCst), 0);
    assert_eq!(sleep_rows(&resources, user_id, &tenant_id).await, 0);
}

/// A validated event whose WHOOP user id no token carries is acknowledged
/// and syncs nobody — never broadcast to every connected user.
#[tokio::test]
#[serial]
async fn unknown_owner_syncs_nobody() {
    let _secret = EnvGuard::set(&[("WHOOP_WEBHOOK_SECRET", SECRET.to_owned())]);
    let (resources, mock) = context_with_mock_whoop().await;
    // A linked user whose token has NO WHOOP id: the mapping cannot hit.
    let (user_id, tenant_id) =
        seed_linked_user(&resources, "whoop-unmapped@example.com", None).await;

    let body = whoop_payload("sleep.updated", WHOOP_USER_ID, "sleep-uuid-1");
    let signature = sign(SECRET, &body);
    assert_eq!(
        post_signed(&resources, body, Some(&signature)).await,
        StatusCode::OK
    );
    await_spawned_turns(&resources).await;

    assert_eq!(mock.fetches.load(Ordering::SeqCst), 0, "nobody was synced");
    assert_eq!(sleep_rows(&resources, user_id, &tenant_id).await, 0);
    assert!(resources
        .common
        .repos
        .oauth_tokens
        .get_provider_last_sync(user_id, tenant_id, "whoop")
        .await
        .unwrap()
        .is_none());
}

/// The verification GET echoes WHOOP's challenge.
#[tokio::test]
#[serial]
async fn verification_echoes_the_challenge() {
    let _secret = EnvGuard::set(&[("WHOOP_WEBHOOK_SECRET", SECRET.to_owned())]);
    let (resources, _mock) = context_with_mock_whoop().await;
    let response = WebhookRoutes::routes(Arc::clone(&resources))
        .oneshot(
            Request::builder()
                .uri("/webhooks/whoop?challenge=hello-whoop")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(&body[..], b"hello-whoop");
}
