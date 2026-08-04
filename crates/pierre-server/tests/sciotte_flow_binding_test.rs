// ABOUTME: Integration tests pinning that a sciotte 2FA continuation must name the caller's own parked login flow
// ABOUTME: Covers the refusal when no live flow exists, tenant scoping, TTL expiry, and recall across pods
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `POST /api/providers/sciotte/{submit-otp,select-2fa}` authenticate the caller
//! but the *`flow_id`* is the only thing binding the call to the login that
//! caller started: the scraper service resumes its sole pending flow when a
//! continuation carries no id. A caller with no server-side flow of their own
//! therefore used to resume whichever browser happened to be parked — and the
//! exported session was persisted under the *caller's* `user_id`/`tenant_id`,
//! handing one athlete's provider cookies to another account.
//!
//! These tests pin the platform half of the fix: a continuation is refused
//! outright when the caller has no live remembered flow, an entry past the
//! service's parked-flow TTL counts as no entry (the service's reaper has
//! already released that browser, so the id names nothing), and an entry is
//! reachable only by the `(tenant, user)` pair that parked it.
//!
//! The registry is the shared cache rather than a process-local map, which is
//! what keeps the refusal from also refusing *legitimate* continuations: a 2FA
//! wait runs to 240s, Cloud Run serves the API from up to three pods with no
//! session affinity, and a revision deploy or pod restart lands mid-wait — so
//! the pod that serves the continuation is routinely not the one that served
//! the login.

mod common;
mod helpers;

use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use helpers::axum_test::AxumTestRequest;
use pierre_cache::{Cache, CacheConfig};
use pierre_core::models::{TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{mint_link_token, MintProviderLinkTokenArgs};
use pierre_routes_auth::{
    forget_remote_flow, remember_remote_flow, remember_remote_flow_for, require_remote_flow,
    AuthRoutes,
};
use tokio::time::sleep;
use uuid::Uuid;

/// Copy the refusal renders. `InvalidInput` is echoed verbatim to the client
/// (see `AppError::sanitized_message`), and both login UIs drop the user back
/// on the credential screen from their error phase — so the copy names the one
/// action that works.
const RESTART_HINT: &str = "start the sign-in again";

async fn test_setup() -> (Arc<ServerContext>, Uuid, TenantId) {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _) = common::create_test_user(&resources.coach.database)
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;
    (resources, user_id, tenant_id)
}

/// A cache standing in for the deployed Memorystore. Background cleanup is off
/// so expiry is observed through the entry's own TTL on read, which is the
/// behaviour the registry relies on.
async fn test_cache() -> Cache {
    Cache::new(CacheConfig {
        enable_background_cleanup: false,
        ..CacheConfig::default()
    })
    .await
    .expect("build cache")
}

/// A sciotte-scoped provider link-token — the hosted login page's credential,
/// and the cheapest way to authenticate these handlers in-process.
fn sciotte_token(resources: &Arc<ServerContext>, user_id: Uuid, tenant_id: TenantId) -> String {
    mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id: tenant_id.as_uuid(),
            provider: "sciotte",
            target: "strava",
            channel: "slack",
            channel_thread: None,
        },
        &resources.auth.admin_jwt_secret,
    )
    .expect("mint sciotte link token")
}

// ── HTTP surface: a continuation with no flow of the caller's own ───────────

#[tokio::test]
async fn submit_otp_without_a_remembered_flow_is_refused() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let token = sciotte_token(&resources, user_id, tenant_id);
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::post("/api/providers/sciotte/submit-otp")
        .header("authorization", &format!("Bearer {token}"))
        .json(&serde_json::json!({ "code": "123456" }))
        .send(app)
        .await;

    let status = resp.status();
    let body = resp.text();
    assert_eq!(
        status, 400,
        "a continuation with no flow of the caller's own is a client error, \
         not a blind resume of whichever browser is parked: {body}"
    );
    assert!(
        body.contains(RESTART_HINT),
        "refusal must tell the user how to recover: {body}"
    );
}

#[tokio::test]
async fn select_2fa_without_a_remembered_flow_is_refused() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let token = sciotte_token(&resources, user_id, tenant_id);
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::post("/api/providers/sciotte/select-2fa")
        .header("authorization", &format!("Bearer {token}"))
        .json(&serde_json::json!({ "option_id": "sms" }))
        .send(app)
        .await;

    let status = resp.status();
    let body = resp.text();
    assert_eq!(
        status, 400,
        "select-2fa carries no secret at all — without a flow of the caller's \
         own it must refuse: {body}"
    );
    assert!(
        body.contains(RESTART_HINT),
        "refusal must tell the user how to recover: {body}"
    );
}

#[tokio::test]
async fn unauthenticated_continuation_is_rejected_before_the_flow_gate() {
    let (resources, _, _) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::post("/api/providers/sciotte/submit-otp")
        .json(&serde_json::json!({ "code": "123456" }))
        .send(app)
        .await;

    let status = resp.status();
    let body = resp.text();
    assert!(
        status == 401 || status == 403,
        "an anonymous continuation is an auth failure, expected 401/403, got {status}: {body}"
    );
    assert!(
        !body.contains(RESTART_HINT),
        "the flow gate must not run for an unauthenticated caller: {body}"
    );
}

/// A fresh login supersedes whatever the user left parked. Driven through the
/// stored-session short-circuit so the whole request stays in-process: the
/// handler clears the remembered flow *before* it reaches the scraper service,
/// and the short-circuit returns without one.
#[tokio::test]
async fn a_new_login_supersedes_the_previously_parked_flow() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let token = sciotte_token(&resources, user_id, tenant_id);

    // A stored, still-valid sciotte session makes the login short-circuit.
    let now = Utc::now();
    let session = serde_json::json!({
        "session_id": "stored-session-1",
        "cookies": [],
        "created_at": now.to_rfc3339(),
        "expires_at": (now + ChronoDuration::hours(6)).to_rfc3339(),
    })
    .to_string();
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant_id.as_uuid().to_string(),
            provider: "sciotte".to_owned(),
            access_token: session,
            refresh_token: None,
            token_type: "session".to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("seed stored sciotte session");

    // An abandoned 2FA prompt from an earlier attempt, parked in the same cache
    // the handlers read.
    remember_remote_flow(
        &resources.common.cache,
        tenant_id.as_uuid(),
        user_id,
        "abandoned-flow".to_owned(),
        "sciotte",
    )
    .await;

    let login = AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &format!("Bearer {token}"))
        .json(&serde_json::json!({
            "email": "athlete@example.com",
            "password": "correct horse battery staple",
            "target": "strava",
        }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let login_status = login.status();
    let login_body = login.text();
    assert_eq!(
        login_status, 200,
        "the stored-session short-circuit should answer the login: {login_body}"
    );
    assert!(
        login_body.contains("\"short_circuit\":true"),
        "expected the stored-session short-circuit, got: {login_body}"
    );

    assert!(
        require_remote_flow(&resources.common.cache, tenant_id.as_uuid(), user_id)
            .await
            .is_err(),
        "a new login must drop the abandoned flow instead of letting the next \
         continuation inherit an id the service has already reaped"
    );

    let resp = AxumTestRequest::post("/api/providers/sciotte/select-2fa")
        .header("authorization", &format!("Bearer {token}"))
        .json(&serde_json::json!({ "option_id": "sms" }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let status = resp.status();
    let body = resp.text();
    assert_eq!(
        status, 400,
        "the abandoned flow is gone, so a continuation has nothing to resume: {body}"
    );
    assert!(
        body.contains(RESTART_HINT),
        "refusal must tell the user how to recover: {body}"
    );
}

// ── The gate itself: live vs expired vs absent vs another tenant's ──────────

/// The gate is a binding check, not a blanket refusal: the flow a login just
/// parked is handed straight back, id and backend provider intact.
#[tokio::test]
async fn a_live_flow_is_handed_to_the_continuation() {
    let cache = test_cache().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    remember_remote_flow(
        &cache,
        tenant_id,
        user_id,
        "flow-7f3a".to_owned(),
        "sciotte_garmin",
    )
    .await;

    let (flow_id, provider) = require_remote_flow(&cache, tenant_id, user_id)
        .await
        .expect("a flow parked just now is the caller's own");
    assert_eq!(
        flow_id, "flow-7f3a",
        "the continuation must name the flow the login parked"
    );
    assert_eq!(
        provider, "sciotte_garmin",
        "the backend provider rides with the flow so a system failure alerts on \
         the right platform"
    );
}

/// The availability half of the fix. The pod that serves the OTP continuation
/// is routinely not the one that served the login — three Cloud Run pods, no
/// session affinity, and a 240s 2FA wait that a deploy or restart can land in.
/// Both hold a `Cache` over the same Memorystore, so the flow parked on one is
/// the flow the other recalls.
#[tokio::test]
async fn a_flow_parked_serving_the_login_is_recalled_serving_the_continuation() {
    let shared = test_cache().await;
    let login_pod = shared.clone();
    let continuation_pod = shared.clone();
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    remember_remote_flow(
        &login_pod,
        tenant_id,
        user_id,
        "flow-cross-pod".to_owned(),
        "sciotte",
    )
    .await;

    let (flow_id, provider) = require_remote_flow(&continuation_pod, tenant_id, user_id)
        .await
        .expect("a flow parked on one pod must be usable from another");
    assert_eq!(flow_id, "flow-cross-pod");
    assert_eq!(provider, "sciotte");

    // ...and a terminal outcome on either pod ends it everywhere.
    forget_remote_flow(&continuation_pod, tenant_id, user_id).await;
    assert!(
        require_remote_flow(&login_pod, tenant_id, user_id)
            .await
            .is_err(),
        "forgetting the flow must be visible to every pod, not just the one that did it"
    );
}

/// The registry is the cache handed to the handler, never process-global state:
/// a second, independent cache holds nothing, so nothing survives outside the
/// store the flow was written to.
#[tokio::test]
async fn the_registry_lives_in_the_cache_not_in_process_global_state() {
    let parked_in = test_cache().await;
    let unrelated = test_cache().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    remember_remote_flow(
        &parked_in,
        tenant_id,
        user_id,
        "flow-scoped-to-a-store".to_owned(),
        "sciotte",
    )
    .await;

    let error = require_remote_flow(&unrelated, tenant_id, user_id)
        .await
        .expect_err("an unrelated cache knows nothing about this flow");
    assert!(
        error.sanitized_message().contains(RESTART_HINT),
        "refusal must tell the user how to recover: {}",
        error.sanitized_message()
    );
    assert_eq!(
        require_remote_flow(&parked_in, tenant_id, user_id)
            .await
            .expect("the store it was parked in still has it")
            .0,
        "flow-scoped-to-a-store"
    );
}

/// The F3/F26 interaction: past the service's parked-flow TTL the id names a
/// browser the reaper has already released, so the entry must behave exactly
/// like no entry at all — refused, not sent to name a dead flow. Expiry is the
/// cache entry's own TTL, so the entry is gone rather than merely hidden.
#[tokio::test]
async fn a_flow_past_its_ttl_is_treated_as_absent() {
    let cache = test_cache().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    remember_remote_flow_for(
        &cache,
        tenant_id,
        user_id,
        "flow-reaped".to_owned(),
        "sciotte",
        Duration::from_millis(30),
    )
    .await;

    // Live right up to the TTL...
    assert_eq!(
        require_remote_flow(&cache, tenant_id, user_id)
            .await
            .expect("an entry inside its TTL still names a live browser")
            .0,
        "flow-reaped"
    );

    sleep(Duration::from_millis(80)).await;

    let error = require_remote_flow(&cache, tenant_id, user_id)
        .await
        .expect_err("an entry past its TTL names nothing");
    assert_eq!(
        error.sanitized_message(),
        "This sign-in is no longer active. Please start the sign-in again.",
        "an expired flow must produce the same actionable refusal as an absent one"
    );
    assert_eq!(
        error.http_status(),
        400,
        "a lapsed sign-in is a client error, not a platform fault"
    );
}

/// The key carries the tenant as well as the user, so a flow parked in one
/// tenant is unreachable from another even for the same `user_id`.
#[tokio::test]
async fn a_flow_is_scoped_to_the_tenant_that_parked_it() {
    let cache = test_cache().await;
    let parked_tenant = Uuid::new_v4();
    let other_tenant = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    remember_remote_flow(
        &cache,
        parked_tenant,
        user_id,
        "flow-tenant-a".to_owned(),
        "sciotte",
    )
    .await;

    require_remote_flow(&cache, other_tenant, user_id)
        .await
        .expect_err("another tenant must not reach this flow");
    assert_eq!(
        require_remote_flow(&cache, parked_tenant, user_id)
            .await
            .expect("the parking tenant still owns it")
            .0,
        "flow-tenant-a"
    );
}

/// A terminal outcome forgets the flow, so the next continuation is refused
/// instead of resuming a browser the service has already torn down.
#[tokio::test]
async fn a_forgotten_flow_is_refused() {
    let cache = test_cache().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    remember_remote_flow(
        &cache,
        tenant_id,
        user_id,
        "flow-terminal".to_owned(),
        "sciotte",
    )
    .await;
    forget_remote_flow(&cache, tenant_id, user_id).await;

    let error = require_remote_flow(&cache, tenant_id, user_id)
        .await
        .expect_err("a forgotten flow is gone");
    assert!(
        error.sanitized_message().contains(RESTART_HINT),
        "refusal must tell the user how to recover: {}",
        error.sanitized_message()
    );
}
