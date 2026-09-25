// ABOUTME: Retention of RFC 7591 dynamic client registrations — the sweep, its cascade, and the pending ceiling
// ABOUTME: Runs on whichever backend DATABASE_URL names, so the PostgreSQL lane covers the same assertions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `POST /oauth2/register` is anonymous, and every row it wrote used to be
//! kept for good (carnet#483). These pin the two halves of the fix to exact
//! row counts: the sweep deletes a registration past its expiry grace with
//! everything issued through it, and one no user authorized within a day,
//! while it keeps every other row; and registration refuses the client past
//! the pending ceiling with a 429 and an RFC 7591 §3.2.2 body.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Request as HttpRequest, StatusCode};
use chrono::{DateTime, Duration, Utc};
use pierre_auth::config::{ClientRetentionConfig, OAuth2ServerConfig, RateLimitConfig};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::{
    ClientRegistrationRequest, ClientRegistrationResponse, OAuth2Error,
};
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::{
    OAuth2AuthCode, OAuth2Client, OAuth2ClientSweep, OAuth2RefreshToken, OAuth2State,
    OAuthClientGrant,
};
use pierre_database::backends::factory::Database;
use pierre_database::database::{generate_encryption_key, test_utils::create_test_db_with_key};
use pierre_mcp_server::start_oauth2_client_sweeper;
use pierre_routes_identity::oauth2::OAuth2Context;
use pierre_routes_identity::OAuth2Routes;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

/// What a client refused by the pending ceiling reads.
const CEILING_REFUSAL: &str = "Too many registered clients are awaiting authorization; retry later";

const COUNT_AUTH_CODES_SQL: &str = "SELECT COUNT(*) FROM oauth2_auth_codes WHERE client_id = $1";
const COUNT_REFRESH_TOKENS_SQL: &str =
    "SELECT COUNT(*) FROM oauth2_refresh_tokens WHERE client_id = $1";
const COUNT_STATES_SQL: &str = "SELECT COUNT(*) FROM oauth2_states WHERE client_id = $1";
const COUNT_GRANTS_SQL: &str = "SELECT COUNT(*) FROM oauth_client_grants WHERE client_id = $1";
const COUNT_CLIENTS_SQL: &str = "SELECT COUNT(*) FROM oauth2_clients";
const COUNT_PENDING_SQL: &str = "SELECT COUNT(*) FROM oauth2_clients \
     WHERE expires_at IS NOT NULL AND last_authorized_at IS NULL";
const BACKDATE_SQL: &str =
    "UPDATE oauth2_clients SET created_at = $1, expires_at = $2 WHERE client_id = $3";
const LAST_AUTHORIZED_SQL: &str =
    "SELECT last_authorized_at FROM oauth2_clients WHERE client_id = $1";

/// A fresh database under a fresh encryption key, migrated by the factory.
async fn database() -> Arc<Database> {
    Arc::new(
        create_test_db_with_key(generate_encryption_key().to_vec())
            .await
            .unwrap(),
    )
}

fn manager(database: &Database) -> ClientRegistrationManager {
    ClientRegistrationManager::new(Arc::clone(&database.repositories().oauth2_server))
}

fn registration(name: &str) -> ClientRegistrationRequest {
    ClientRegistrationRequest {
        redirect_uris: vec!["https://claude.ai/api/mcp/auth_callback".to_owned()],
        client_name: Some(name.to_owned()),
        client_uri: None,
        grant_types: None,
        response_types: None,
        scope: None,
    }
}

async fn register(
    database: &Database,
    name: &str,
    ceiling: u64,
) -> Result<ClientRegistrationResponse, OAuth2Error> {
    manager(database)
        .register_client(registration(name), ceiling)
        .await
}

async fn register_ok(database: &Database, name: &str) -> String {
    register(database, name, MAX_PENDING_REGISTRATIONS)
        .await
        .unwrap()
        .client_id
}

/// Issue a refresh token through `client_id`: what a user completing the
/// authorization-code exchange leaves behind, and so what marks the client
/// authorized.
async fn authorize(database: &Database, client_id: &str) -> DateTime<Utc> {
    let issued_at = Utc::now();
    let token = OAuth2RefreshToken {
        token: format!("refresh-{}", Uuid::new_v4()),
        client_id: client_id.to_owned(),
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4().to_string(),
        scope: Some("read:fitness".to_owned()),
        expires_at: issued_at + Duration::days(30),
        created_at: issued_at,
        revoked: false,
    };
    database
        .repositories()
        .oauth2_server
        .store_refresh_token(&token)
        .await
        .unwrap();
    issued_at
}

/// Give `client_id` an authorization code, a CSRF state and a consent grant.
async fn issue_dependents(database: &Database, client_id: &str) {
    let repos = database.repositories();
    let user_id = Uuid::new_v4();
    let tenant_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    repos
        .oauth2_server
        .store_auth_code(&OAuth2AuthCode {
            code: format!("code-{}", Uuid::new_v4()),
            client_id: client_id.to_owned(),
            user_id,
            tenant_id: tenant_id.clone(),
            redirect_uri: "https://claude.ai/api/mcp/auth_callback".to_owned(),
            scope: Some("read:fitness".to_owned()),
            expires_at: now + Duration::minutes(10),
            used: false,
            state: None,
            code_challenge: None,
            code_challenge_method: None,
        })
        .await
        .unwrap();
    repos
        .oauth2_server
        .store_state(&OAuth2State {
            state: format!("state-{}", Uuid::new_v4()),
            client_id: client_id.to_owned(),
            user_id: Some(user_id),
            tenant_id: Some(tenant_id.clone()),
            redirect_uri: "https://claude.ai/api/mcp/auth_callback".to_owned(),
            scope: Some("read:fitness".to_owned()),
            code_challenge: None,
            code_challenge_method: None,
            created_at: now,
            expires_at: now + Duration::minutes(10),
            used: false,
        })
        .await
        .unwrap();
    grant(database, client_id).await;
}

async fn grant(database: &Database, client_id: &str) {
    database
        .repositories()
        .oauth2_server
        .store_client_grant(&OAuthClientGrant {
            id: Uuid::new_v4().to_string(),
            user_id: Uuid::new_v4().to_string(),
            tenant_id: Uuid::new_v4().to_string(),
            client_id: client_id.to_owned(),
            scope: "read:fitness".to_owned(),
            granted_at: Utc::now(),
            revoked_at: None,
        })
        .await
        .unwrap();
}

/// Move a client's `created_at` and `expires_at`, on whichever backend.
async fn backdate(
    database: &Database,
    client_id: &str,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
) {
    let affected = match database {
        Database::SQLite(db) => sqlx::query(BACKDATE_SQL)
            .bind(created_at)
            .bind(expires_at)
            .bind(client_id)
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => sqlx::query(BACKDATE_SQL)
            .bind(created_at)
            .bind(expires_at)
            .bind(client_id)
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
    };
    assert_eq!(affected, 1, "{client_id} must exist to be backdated");
}

async fn count_for(database: &Database, sql: &str, client_id: &str) -> i64 {
    match database {
        Database::SQLite(db) => sqlx::query_scalar::<_, i64>(sql)
            .bind(client_id)
            .fetch_one(db.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => sqlx::query_scalar::<_, i64>(sql)
            .bind(client_id)
            .fetch_one(db.pool())
            .await
            .unwrap(),
    }
}

async fn count_all(database: &Database, sql: &str) -> i64 {
    match database {
        Database::SQLite(db) => sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(db.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(db.pool())
            .await
            .unwrap(),
    }
}

async fn last_authorized_at(database: &Database, client_id: &str) -> Option<DateTime<Utc>> {
    match database {
        Database::SQLite(db) => sqlx::query_scalar::<_, Option<DateTime<Utc>>>(LAST_AUTHORIZED_SQL)
            .bind(client_id)
            .fetch_one(db.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => {
            sqlx::query_scalar::<_, Option<DateTime<Utc>>>(LAST_AUTHORIZED_SQL)
                .bind(client_id)
                .fetch_one(db.pool())
                .await
                .unwrap()
        }
    }
}

async fn exists(database: &Database, client_id: &str) -> bool {
    database
        .repositories()
        .oauth2_server
        .get_client(client_id)
        .await
        .unwrap()
        .is_some()
}

async fn sweep(database: &Database, retention: &ClientRetentionConfig) -> OAuth2ClientSweep {
    manager(database)
        .sweep_stale_clients(retention, Utc::now())
        .await
        .unwrap()
}

#[tokio::test]
async fn an_expired_registration_past_its_grace_goes_with_everything_issued_through_it() {
    let database = database().await;
    let retention = ClientRetentionConfig::default();
    let now = Utc::now();
    let long_ago = now - Duration::days(400);

    let gone = register_ok(&database, "Past its grace").await;
    let in_grace = register_ok(&database, "Expired inside the grace").await;
    let live = register_ok(&database, "Not expired").await;
    for client_id in [&gone, &in_grace, &live] {
        authorize(&database, client_id).await;
    }
    issue_dependents(&database, &gone).await;
    grant(&database, &in_grace).await;
    backdate(&database, &gone, long_ago, Some(now - Duration::days(31))).await;
    backdate(
        &database,
        &in_grace,
        long_ago,
        Some(now - Duration::days(29)),
    )
    .await;
    backdate(&database, &live, long_ago, Some(now + Duration::days(30))).await;

    assert_eq!(
        sweep(&database, &retention).await,
        OAuth2ClientSweep {
            expired: 1,
            abandoned: 0,
            orphaned_grants: 1,
        }
    );

    assert!(!exists(&database, &gone).await);
    assert_eq!(count_for(&database, COUNT_AUTH_CODES_SQL, &gone).await, 0);
    assert_eq!(
        count_for(&database, COUNT_REFRESH_TOKENS_SQL, &gone).await,
        0
    );
    assert_eq!(count_for(&database, COUNT_STATES_SQL, &gone).await, 0);
    assert_eq!(count_for(&database, COUNT_GRANTS_SQL, &gone).await, 0);

    assert!(exists(&database, &in_grace).await, "the grace keeps it");
    assert_eq!(
        count_for(&database, COUNT_REFRESH_TOKENS_SQL, &in_grace).await,
        1
    );
    assert_eq!(count_for(&database, COUNT_GRANTS_SQL, &in_grace).await, 1);
    assert!(exists(&database, &live).await);
    assert_eq!(
        count_for(&database, COUNT_REFRESH_TOKENS_SQL, &live).await,
        1
    );
    assert_eq!(count_all(&database, COUNT_CLIENTS_SQL).await, 2);
}

#[tokio::test]
async fn a_registration_no_user_authorized_is_reclaimed_after_a_day() {
    let database = database().await;
    let retention = ClientRetentionConfig::default();
    let now = Utc::now();
    let expiry = Some(now + Duration::days(300));

    let abandoned = register_ok(&database, "Registered, never authorized").await;
    let recent = register_ok(&database, "Registered an hour ago").await;
    let connected = register_ok(&database, "Authorized two days ago").await;
    // A user consented but the code exchange never happened: still pending.
    grant(&database, &abandoned).await;
    authorize(&database, &connected).await;
    backdate(&database, &abandoned, now - Duration::hours(25), expiry).await;
    backdate(&database, &recent, now - Duration::hours(23), expiry).await;
    backdate(&database, &connected, now - Duration::hours(48), expiry).await;

    assert_eq!(
        sweep(&database, &retention).await,
        OAuth2ClientSweep {
            expired: 0,
            abandoned: 1,
            orphaned_grants: 1,
        }
    );
    assert!(!exists(&database, &abandoned).await);
    assert_eq!(count_for(&database, COUNT_GRANTS_SQL, &abandoned).await, 0);
    assert!(exists(&database, &recent).await, "inside its day");
    assert!(exists(&database, &connected).await, "a user authorized it");
    assert_eq!(count_all(&database, COUNT_PENDING_SQL).await, 1);

    // A second pass finds nothing left to reclaim.
    assert_eq!(
        sweep(&database, &retention).await,
        OAuth2ClientSweep::default()
    );
}

#[tokio::test]
async fn a_client_without_an_expiry_is_neither_swept_nor_held_by_the_ceiling() {
    let database = database().await;
    // Registration always stamps an expiry, so a row without one was
    // provisioned some other way — the shape a first-party client would take.
    let provisioned = OAuth2Client {
        id: Uuid::new_v4().to_string(),
        client_id: format!("first-party-{}", Uuid::new_v4()),
        client_secret_hash: "argon2-hash-of-a-provisioned-secret".to_owned(),
        redirect_uris: vec!["https://app.dravr.ai/oauth/callback".to_owned()],
        grant_types: vec!["authorization_code".to_owned()],
        response_types: vec!["code".to_owned()],
        client_name: Some("Provisioned".to_owned()),
        client_uri: None,
        scope: None,
        created_at: Utc::now() - Duration::days(5 * 365),
        expires_at: None,
    };
    let stored = database
        .repositories()
        .oauth2_server
        .store_client_within_ceiling(&provisioned, 1)
        .await
        .unwrap();
    assert!(stored);

    assert_eq!(
        sweep(&database, &ClientRetentionConfig::default()).await,
        OAuth2ClientSweep::default(),
        "five years old and never authorized, yet not a registration"
    );
    assert!(exists(&database, &provisioned.client_id).await);

    // It holds no pending slot: a ceiling of one still admits a registration.
    let registered = register(&database, "Fills the only slot", 1).await;
    assert!(registered.is_ok(), "{registered:?}");
    assert_eq!(count_all(&database, COUNT_PENDING_SQL).await, 1);
    assert_eq!(count_all(&database, COUNT_CLIENTS_SQL).await, 2);
}

#[tokio::test]
async fn a_retention_age_reaching_past_the_epoch_deletes_nothing() {
    let database = database().await;
    let client_id = register_ok(&database, "Long expired").await;
    let long_ago = Utc::now() - Duration::days(4000);
    backdate(&database, &client_id, long_ago, Some(long_ago)).await;

    // Ten thousand years is a valid chrono instant but outside what
    // PostgreSQL stores; u64::MAX is not even a valid duration.
    for age_secs in [10_000 * 365 * 24 * 60 * 60, u64::MAX] {
        let retention = ClientRetentionConfig {
            expired_grace_secs: age_secs,
            abandoned_after_secs: age_secs,
            ..ClientRetentionConfig::default()
        };
        assert_eq!(
            sweep(&database, &retention).await,
            OAuth2ClientSweep::default(),
            "age {age_secs}s"
        );
    }
    assert!(exists(&database, &client_id).await);
}

#[tokio::test]
async fn the_ceiling_refuses_the_next_registration_until_one_is_authorized() {
    let database = database().await;
    let ceiling = 3;

    let mut admitted = Vec::new();
    for n in 1..=3 {
        admitted.push(
            register(&database, &format!("Pending {n}"), ceiling)
                .await
                .unwrap()
                .client_id,
        );
    }
    let refused = register(&database, "Past the ceiling", ceiling)
        .await
        .unwrap_err();
    assert_eq!(refused.error, "too_many_requests");
    assert_eq!(refused.error_description.as_deref(), Some(CEILING_REFUSAL));
    assert_eq!(refused.registration_status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        count_all(&database, COUNT_CLIENTS_SQL).await,
        3,
        "nothing stored"
    );

    // A refresh token through one of them takes it out of the pending set.
    assert_eq!(last_authorized_at(&database, &admitted[0]).await, None);
    let issued_at = authorize(&database, &admitted[0]).await;
    let stamp = last_authorized_at(&database, &admitted[0])
        .await
        .expect("issuing a refresh token stamps its client");
    assert!(
        (stamp - issued_at).num_milliseconds().abs() < 1,
        "stamped with the token's issue time: {stamp} vs {issued_at}"
    );
    assert_eq!(count_all(&database, COUNT_PENDING_SQL).await, 2);

    assert!(register(&database, "Takes the freed slot", ceiling)
        .await
        .is_ok());
    assert!(register(&database, "Past it again", ceiling).await.is_err());
    assert_eq!(count_all(&database, COUNT_PENDING_SQL).await, 3);
    assert_eq!(count_all(&database, COUNT_CLIENTS_SQL).await, 4);
}

#[tokio::test]
async fn a_ceiling_of_zero_refuses_every_registration() {
    let database = database().await;
    let refused = register(&database, "Registration switched off", 0)
        .await
        .unwrap_err();
    assert_eq!(refused.error, "too_many_requests");
    assert_eq!(count_all(&database, COUNT_CLIENTS_SQL).await, 0);
}

/// `POST /oauth2/register` through the real router, with the ceiling at one.
async fn post_register(router: &axum::Router, name: &str) -> (StatusCode, serde_json::Value) {
    let body = serde_json::json!({
        "redirect_uris": ["https://claude.ai/api/mcp/auth_callback"],
        "client_name": name,
    });
    let mut request = HttpRequest::builder()
        .method("POST")
        .uri("/oauth2/register")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 7], 50_000))));
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn the_register_endpoint_answers_past_the_ceiling_with_429_and_an_rfc7591_body() {
    let database = database().await;
    let repos = database.repositories();
    let mut config = OAuth2ServerConfig::default();
    config.client_retention.max_pending_registrations = 1;
    let router = OAuth2Routes::routes(OAuth2Context {
        database: Arc::clone(&database),
        oauth2_server: Arc::clone(&repos.oauth2_server),
        tenants: Arc::clone(&repos.tenants),
        users: Arc::clone(&repos.users),
        auth_manager: common::create_test_auth_manager(),
        jwks_manager: common::get_shared_test_jwks(),
        config: Arc::new(config),
        rate_limiter: Arc::new(OAuth2RateLimiter::new(
            None,
            OAuth2RateLimiter::local_window_store(),
            &RateLimitConfig::default(),
        )),
    });

    let (status, admitted) = post_register(&router, "First").await;
    assert_eq!(status, StatusCode::CREATED, "{admitted}");
    assert!(admitted["client_id"]
        .as_str()
        .is_some_and(|id| id.starts_with("mcp_client_")));

    let (status, refused) = post_register(&router, "Second").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        refused,
        serde_json::json!({
            "error": "too_many_requests",
            "error_description": CEILING_REFUSAL,
        })
    );
    assert_eq!(count_all(&database, COUNT_CLIENTS_SQL).await, 1);
}

#[tokio::test]
async fn the_sweeper_reclaims_an_abandoned_registration_on_its_first_tick() {
    let database = database().await;
    let repos = database.repositories();
    let client_id = register_ok(&database, "Abandoned before boot").await;
    let now = Utc::now();
    backdate(
        &database,
        &client_id,
        now - Duration::days(2),
        Some(now + Duration::days(300)),
    )
    .await;

    // A never-run worker ticks within its period (capped) of starting.
    start_oauth2_client_sweeper(
        Arc::clone(&repos.oauth2_server),
        Arc::clone(&repos.worker_runs),
        ClientRetentionConfig {
            sweep_interval_secs: 1,
            ..ClientRetentionConfig::default()
        },
    );

    for _ in 0..100 {
        if !exists(&database, &client_id).await {
            return;
        }
        sleep(StdDuration::from_millis(100)).await;
    }
    panic!("the sweeper did not delete the abandoned registration within 10s");
}
