// ABOUTME: carnet#482 — an athlete deletes their own account from a signed-in session, re-proving who they are
// ABOUTME: Grants revoked upstream, no row of theirs survives, refusals touch nothing, operators and blockers refused

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `GET` / `POST /api/user/account-deletion`, driven through the real auth
//! router against a real database. Strava's revocation endpoint is a local
//! stub, so "revoked at the provider" is asserted on the wire rather than
//! inferred from the rows being gone.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::env;
use std::sync::Arc;

use axum::Router;
use chrono::{Duration, Utc};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_auth::api_keys::{ApiKeyManager, ApiKeyTier, CreateApiKeyRequest};
use pierre_core::constants::oauth_rate_limiting::PASSWORD_CONFIRM_RPM;
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    ConnectionType, CreateUserMcpTokenRequest, SessionRefreshToken, Tenant, TenantId, User,
    UserOAuthToken, UserStatus, FEDERATED_ONLY_PASSWORD_HASH,
};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::DatabaseBackend;
#[cfg(feature = "postgresql")]
use pierre_database::repositories::POSTGRES_USER_PURGE;
use pierre_database::repositories::{CreateSessionParams, UserPurge, SQLITE_USER_PURGE};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::common::create_test_server_resources;
use crate::helpers::identity_toolkit_stub::{
    Answer, IdentityToolkitStub, ACCESS_TOKEN, PROJECT as FIREBASE_PROJECT,
};
use crate::helpers::notify_capture::{capture_logs, named};
use pierre_auth::config::oauth::FirebaseConfig;
use pierre_auth::firebase_identity::FirebaseIdentityDeleter;
use pierre_database::repositories::PendingFirebaseDeletion;
use pierre_services::firebase_identity_outbox::{
    sweep_firebase_identity_deletions, ALERT_AFTER_ATTEMPTS,
};
use tracing::Level;

const PASSWORD: &str = "CorrectHorse-482";
const REFRESH_TOKEN: &str = "refresh-material-do-not-log";
const CLIENT_ID: &str = "self-deletion-test-client";
const CLIENT_SECRET: &str = "self-deletion-test-secret";

/// The rows the account delete's foreign keys cascade to rather than clear
/// themselves, plus the account row: what [`UserPurge::surviving`] does not
/// read. One statement on both engines, every id column read as text.
const CASCADED_ROWS_SQL: &str = r"
    SELECT (SELECT COUNT(*) FROM users WHERE CAST(id AS TEXT) = $1)
         + (SELECT COUNT(*) FROM tenant_users WHERE CAST(user_id AS TEXT) = $1)
         + (SELECT COUNT(*) FROM session_refresh_tokens WHERE CAST(user_id AS TEXT) = $1)
         + (SELECT COUNT(*) FROM messaging_sessions WHERE CAST(user_id AS TEXT) = $1)";

/// A stand-in for Strava's revocation endpoint that confirms every request
/// and hands the request line back. A request is queued before its response
/// is written, so once the delete has answered the request is here to read.
struct RevokeStub {
    url: String,
    requests: mpsc::UnboundedReceiver<String>,
}

impl RevokeStub {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0_u8; 8192];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if tx
                    .send(String::from_utf8_lossy(&buf[..n]).into_owned())
                    .is_err()
                {
                    return;
                }
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
                let _ = stream.shutdown().await;
            }
        });
        Self { url, requests: rx }
    }

    fn received(&mut self) -> Vec<String> {
        let mut seen = Vec::new();
        while let Ok(raw) = self.requests.try_recv() {
            seen.push(raw);
        }
        seen
    }
}

struct Setup {
    resources: Arc<ServerContext>,
    routes: Router,
    stub: RevokeStub,
    firebase: Arc<IdentityToolkitStub>,
    firebase_config: FirebaseConfig,
}

impl Setup {
    async fn new() -> Self {
        Self::with_firebase(Answer::Deleted).await
    }

    /// A server whose Identity Toolkit answers every delete with `answer`.
    async fn with_firebase(answer: Answer) -> Self {
        env::set_var("STRAVA_CLIENT_ID", CLIENT_ID);
        env::set_var("STRAVA_CLIENT_SECRET", CLIENT_SECRET);
        let resources = create_test_server_resources().await.unwrap();
        let stub = RevokeStub::start().await;
        let firebase = IdentityToolkitStub::answering(answer);
        let mut context = resources.auth_routes_context();
        let mut config = (*context.config).clone();
        stub.url
            .clone_into(&mut config.external_services.strava_api.revoke_url);
        let firebase_config = firebase.serve().await;
        config.firebase = firebase_config.clone();
        context.config = Arc::new(config);
        Self {
            routes: AuthRoutes::routes(context),
            resources,
            stub,
            firebase,
            firebase_config,
        }
    }

    /// The deleter the server runs the sweep with.
    fn deleter(&self) -> FirebaseIdentityDeleter {
        FirebaseIdentityDeleter::from_config(&self.firebase_config).expect("Firebase is configured")
    }

    /// Every Firebase identity still queued for deletion, due or not.
    async fn queued(&self) -> Vec<PendingFirebaseDeletion> {
        self.resources
            .common
            .repos
            .firebase_identity_deletions
            .due(Utc::now() + Duration::days(365), 100)
            .await
            .unwrap()
    }

    /// The repositories, held apart from `self` so the stub stays mutable.
    fn repos(&self) -> Arc<RepositoryRegistry> {
        self.resources.common.repos.clone()
    }

    fn jwt(&self, user: &User) -> String {
        self.resources
            .auth
            .auth_manager
            .generate_token(user, &self.resources.auth.jwks_manager)
            .unwrap()
    }

    async fn preview(&self, user: &User) -> AxumTestResponse {
        AxumTestRequest::get("/api/user/account-deletion")
            .header("Authorization", &format!("Bearer {}", self.jwt(user)))
            .send(self.routes.clone())
            .await
    }

    async fn delete(&self, user: &User, body: &Value) -> AxumTestResponse {
        AxumTestRequest::post("/api/user/account-deletion")
            .header("Authorization", &format!("Bearer {}", self.jwt(user)))
            .json(body)
            .send(self.routes.clone())
            .await
    }

    /// Rows still held for `user_id`: every table the delete clears itself
    /// (by name), then the cascaded tables and the account row (as a total).
    async fn surviving_rows(&self, user_id: Uuid) -> (Vec<String>, i64) {
        let owner = user_id.to_string();
        match self.resources.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                let purge: UserPurge = SQLITE_USER_PURGE;
                let survivors: Vec<(String, i64)> = sqlx::query_as(purge.surviving)
                    .bind(&owner)
                    .fetch_all(db.pool())
                    .await
                    .unwrap();
                let cascaded: i64 = sqlx::query_scalar(CASCADED_ROWS_SQL)
                    .bind(&owner)
                    .fetch_one(db.pool())
                    .await
                    .unwrap();
                (survivors.into_iter().map(|(t, _)| t).collect(), cascaded)
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                let purge: UserPurge = POSTGRES_USER_PURGE;
                let survivors: Vec<(String, i64)> = sqlx::query_as(purge.surviving)
                    .bind(&owner)
                    .fetch_all(db.pool())
                    .await
                    .unwrap();
                let cascaded: i64 = sqlx::query_scalar(CASCADED_ROWS_SQL)
                    .bind(&owner)
                    .fetch_one(db.pool())
                    .await
                    .unwrap();
                (survivors.into_iter().map(|(t, _)| t).collect(), cascaded)
            }
        }
    }
}

/// An active account with its own tenant, signing in with `password_hash`.
async fn seed_account(repos: &RepositoryRegistry, password_hash: String) -> (User, TenantId) {
    let email = format!("athlete-{}@example.com", Uuid::new_v4());
    let mut user = User::new(email, password_hash, Some("Athlete".to_owned()));
    user.user_status = UserStatus::Active;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "Athlete tenant".to_owned(),
            slug: format!("athlete-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    (user, tenant_id)
}

/// An account signing in only through Google, whose Firebase user is `uid`.
async fn seed_google_account(repos: &RepositoryRegistry) -> (User, TenantId, String) {
    let (user, tenant_id) = seed_account(repos, FEDERATED_ONLY_PASSWORD_HASH.to_owned()).await;
    let uid = format!("firebase-{}", Uuid::new_v4().simple());
    let mut linked = user.clone();
    linked.firebase_uid = Some(uid.clone());
    repos.users.update(&linked).await.unwrap();
    (linked, tenant_id, uid)
}

async fn seed_password_account(repos: &RepositoryRegistry) -> (User, TenantId) {
    seed_account(repos, bcrypt::hash(PASSWORD, 4).unwrap()).await
}

/// A Strava grant as the OAuth callback writes it: connection plus token.
async fn connect_strava(repos: &RepositoryRegistry, user_id: Uuid, tenant_id: TenantId) {
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant_id.to_string(),
            provider: "strava".to_owned(),
            access_token: "access-material-do-not-log".to_owned(),
            refresh_token: Some(REFRESH_TOKEN.to_owned()),
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
}

/// A coaching group `owner` owns, with a tenant-visible agent.
async fn seed_group(repos: &RepositoryRegistry, owner: Uuid, tenant_id: TenantId) -> Uuid {
    let agent_id = repos
        .agents
        .create_system_agent(
            owner,
            tenant_id,
            &CreateSystemAgentRequest {
                title: "Track coach".to_owned(),
                description: None,
                system_prompt: "You coach the group.".to_owned(),
                category: AgentCategory::Recovery,
                tags: vec![],
                visibility: AgentVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap()
        .id
        .to_string();
    let now = Utc::now();
    repos
        .groups
        .create_group(
            tenant_id,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: tenant_id.to_string(),
                name: "Track Tuesdays".to_owned(),
                description: None,
                agent_id,
                owner_id: owner,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap()
        .id
}

async fn join_group(repos: &RepositoryRegistry, group_id: Uuid, user_id: Uuid, tenant: TenantId) {
    let now = Utc::now();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: tenant.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            coach_sharing_consent: true,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

/// Rows of the athlete's own across cleared and cascaded tables: a device
/// session, a messaging session, onboarding progress, a short link and a usage
/// counter.
async fn seed_owned_rows(repos: &RepositoryRegistry, user_id: Uuid, tenant_id: TenantId) {
    let user = user_id.to_string();
    let tenant = tenant_id.to_string();
    repos
        .session_refresh_tokens
        .store_token(
            &format!("refresh-{}", Uuid::new_v4()),
            &SessionRefreshToken {
                family_id: Uuid::new_v4().to_string(),
                user_id,
                tenant_id: Some(tenant.clone()),
                created_at: Utc::now(),
                expires_at: Utc::now() + Duration::days(30),
            },
        )
        .await
        .unwrap();
    repos
        .messaging
        .create_session(&CreateSessionParams {
            id: &Uuid::new_v4().to_string(),
            user_id: &user,
            tenant_id,
            channel_type: "telegram",
            channel_user_id: "tg-482",
            channel_conversation_id: None,
            pierre_conversation_id: None,
        })
        .await
        .unwrap();
    repos
        .user_onboarding
        .set_onboarding_step(&user, "welcome", "done", None, Some(&tenant))
        .await
        .unwrap();
    repos
        .short_links
        .create_short_link(
            &format!("code{}", Uuid::new_v4().simple()),
            "https://example.com/plan",
            &tenant,
            &user,
            Utc::now() + Duration::days(1),
        )
        .await
        .unwrap();
    repos
        .usage_counters
        .increment_counter(&tenant, &user, "chat_turns", "2026-10", 3)
        .await
        .unwrap();
}

#[tokio::test]
async fn preview_names_the_password_the_providers_and_what_blocks_the_delete() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (user, tenant_id) = seed_password_account(repos).await;
    connect_strava(repos, user.id, tenant_id).await;
    seed_group(repos, user.id, tenant_id).await;

    let response = setup.preview(&user).await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = response.json();
    assert_eq!(body["email"], user.email.as_str());
    assert_eq!(body["requires_password"], true);
    assert_eq!(body["providers"], json!(["strava"]));
    let blockers = body["blockers"].as_array().unwrap();
    assert!(
        blockers
            .iter()
            .any(|b| b["kind"] == "owns_coaching_group" && b["detail"] == "Track Tuesdays"),
        "the owned group blocks the delete: {body}"
    );
}

#[tokio::test]
async fn an_athlete_deletes_their_account_revoking_upstream_and_leaving_no_row() {
    let mut setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (owner, owner_tenant) = seed_password_account(repos).await;
    let group_id = seed_group(repos, owner.id, owner_tenant).await;
    let (user, tenant_id) = seed_password_account(repos).await;
    connect_strava(repos, user.id, tenant_id).await;
    join_group(repos, group_id, user.id, tenant_id).await;
    seed_owned_rows(repos, user.id, tenant_id).await;

    let (before, cascaded_before) = setup.surviving_rows(user.id).await;
    assert!(
        before.len() >= 6 && cascaded_before >= 4,
        "the seed must reach both kinds of table: {before:?}, {cascaded_before}"
    );
    let jwt = setup.jwt(&user);

    let response = setup
        .delete(
            &user,
            &json!({ "confirm_email": user.email.to_uppercase(), "password": PASSWORD }),
        )
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let cookies = response.header_all("set-cookie").join("; ");
    assert!(
        cookies.contains("Max-Age=0"),
        "the auth cookie is cleared: {cookies}"
    );
    let body: Value = response.json();
    assert_eq!(body["disconnected_providers"], json!(["strava"]));

    let revocations = setup.stub.received();
    assert_eq!(revocations.len(), 1, "one grant revoked at Strava");
    assert!(
        revocations[0].starts_with("POST "),
        "a revocation request: {}",
        revocations[0]
    );

    let (survivors, cascaded) = setup.surviving_rows(user.id).await;
    assert!(
        survivors.is_empty(),
        "rows outlived the account: {survivors:?}"
    );
    assert_eq!(
        cascaded, 0,
        "an account, membership or session row survived"
    );
    assert!(
        setup.firebase.calls().is_empty(),
        "a password account has no Firebase identity to delete"
    );
    assert!(repos.users.get_global(user.id).await.unwrap().is_none());
    // The group the athlete belonged to is somebody else's, and stays.
    assert!(repos
        .groups
        .get_member(&group_id.to_string(), user.id)
        .await
        .unwrap()
        .is_none());
    assert!(repos.users.get_global(owner.id).await.unwrap().is_some());

    // The JWT the athlete still holds no longer opens a session.
    let session = AxumTestRequest::get("/api/auth/session")
        .header("Authorization", &format!("Bearer {jwt}"))
        .send(setup.routes.clone())
        .await;
    assert_ne!(session.status(), 200, "a deleted account has no session");
}

#[tokio::test]
async fn a_wrong_email_or_password_is_refused_and_touches_nothing() {
    let mut setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (user, tenant_id) = seed_password_account(repos).await;
    connect_strava(repos, user.id, tenant_id).await;

    let mismatch = setup
        .delete(
            &user,
            &json!({ "confirm_email": "someone-else@example.com", "password": PASSWORD }),
        )
        .await;
    assert_eq!(mismatch.status(), 400);
    assert_eq!(mismatch.json::<Value>()["error"], "email_mismatch");

    let missing = setup
        .delete(&user, &json!({ "confirm_email": user.email }))
        .await;
    assert_eq!(missing.status(), 403);
    assert_eq!(missing.json::<Value>()["error"], "password_required");

    let wrong = setup
        .delete(
            &user,
            &json!({ "confirm_email": user.email, "password": "not-it" }),
        )
        .await;
    assert_eq!(
        wrong.status(),
        403,
        "never a 401, which signs the client out"
    );
    assert_eq!(wrong.json::<Value>()["error"], "password_incorrect");

    assert!(setup.stub.received().is_empty(), "nothing revoked");
    assert!(repos.users.get_global(user.id).await.unwrap().is_some());
    assert_eq!(
        repos
            .oauth_tokens
            .list_token_providers(user.id)
            .await
            .unwrap()
            .len(),
        1,
        "the grant stands"
    );
}

#[tokio::test]
async fn a_google_only_account_deletes_with_its_email_alone_and_its_firebase_identity_goes() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (user, tenant_id, uid) = seed_google_account(repos).await;
    seed_owned_rows(repos, user.id, tenant_id).await;

    let preview: Value = setup.preview(&user).await.json();
    assert_eq!(preview["requires_password"], false);

    let response = setup
        .delete(&user, &json!({ "confirm_email": user.email }))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let (survivors, cascaded) = setup.surviving_rows(user.id).await;
    assert!(survivors.is_empty(), "{survivors:?}");
    assert_eq!(cascaded, 0);

    let calls = setup.firebase.calls();
    assert_eq!(calls.len(), 1, "one accounts:delete: {calls:?}");
    assert_eq!(calls[0].local_id, uid, "the account's Firebase user");
    assert_eq!(calls[0].project, FIREBASE_PROJECT);
    assert_eq!(
        calls[0].authorization.as_deref(),
        Some(format!("Bearer {ACCESS_TOKEN}").as_str()),
        "authenticated as the runtime service account"
    );
    assert!(
        setup.queued().await.is_empty(),
        "a confirmed deletion leaves nothing queued"
    );
}

#[tokio::test]
async fn a_firebase_outage_retries_then_leaves_the_account_deleted() {
    let setup = Setup::with_firebase(Answer::Unavailable).await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (user, tenant_id, uid) = seed_google_account(repos).await;
    seed_owned_rows(repos, user.id, tenant_id).await;

    let response = setup
        .delete(&user, &json!({ "confirm_email": user.email }))
        .await;
    assert_eq!(
        response.status(),
        200,
        "Firebase down never fails the delete: {}",
        response.body_text()
    );

    let calls = setup.firebase.calls();
    assert_eq!(calls.len(), 3, "a 503 is retried twice: {calls:?}");
    assert!(calls.iter().all(|call| call.local_id == uid));
    assert!(
        repos.users.get_global(user.id).await.unwrap().is_none(),
        "the account stays deleted"
    );
    assert!(
        repos
            .users
            .get_by_firebase_uid(&uid)
            .await
            .unwrap()
            .is_none(),
        "nothing resurrected under the Firebase uid"
    );
    let (survivors, cascaded) = setup.surviving_rows(user.id).await;
    assert!(survivors.is_empty(), "{survivors:?}");
    assert_eq!(cascaded, 0);

    // The identity is queued, with nothing of the account but its uid.
    let queued = setup.queued().await;
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert_eq!(queued[0].firebase_uid, uid);
    assert_eq!(queued[0].firebase_project, FIREBASE_PROJECT);
    assert_eq!(queued[0].attempts, 1);
    assert!(queued[0].next_attempt_at > Utc::now(), "held for a backoff");
    assert!(queued[0]
        .last_error
        .as_deref()
        .is_some_and(|e| e.contains("503")));

    // Not yet due: a sweep now leaves it alone.
    let outbox = repos.firebase_identity_deletions.as_ref();
    let deleter = setup.deleter();
    assert_eq!(
        sweep_firebase_identity_deletions(outbox, &deleter, Utc::now())
            .await
            .unwrap(),
        0
    );
    assert_eq!(setup.firebase.calls().len(), 3, "nothing retried early");

    // Firebase recovers; the sweep after the backoff deletes the identity
    // and empties the outbox.
    setup.firebase.set_answer(Answer::Deleted);
    let later = queued[0].next_attempt_at + Duration::seconds(1);
    assert_eq!(
        sweep_firebase_identity_deletions(outbox, &deleter, later)
            .await
            .unwrap(),
        1
    );
    let calls = setup.firebase.calls();
    assert_eq!(calls.len(), 4, "{calls:?}");
    assert_eq!(calls[3].local_id, uid);
    assert!(
        setup.queued().await.is_empty(),
        "the row goes with the identity"
    );
}

#[tokio::test]
async fn an_identity_stuck_past_the_threshold_is_raised_once_and_kept() {
    let setup = Setup::with_firebase(Answer::Unavailable).await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (user, _, uid) = seed_google_account(repos).await;
    let response = setup
        .delete(&user, &json!({ "confirm_email": user.email }))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());

    let outbox = repos.firebase_identity_deletions.as_ref();
    let deleter = setup.deleter();
    let (events, _guard) = capture_logs();
    for _ in 1..=ALERT_AFTER_ATTEMPTS {
        let due_at = setup.queued().await[0].next_attempt_at;
        sweep_firebase_identity_deletions(outbox, &deleter, due_at)
            .await
            .unwrap();
    }

    let queued = setup.queued().await;
    assert_eq!(queued.len(), 1, "a stuck identity is never dropped");
    assert_eq!(queued[0].attempts, ALERT_AFTER_ATTEMPTS + 1);
    let stalled: Vec<_> = named(&events, "A deleted account's Firebase identity still cannot be deleted at Google; the sweep keeps retrying");
    assert_eq!(stalled.len(), 1, "raised once, at the threshold");
    assert_eq!(stalled[0].level, Level::ERROR);
    assert_eq!(stalled[0].field("firebase_uid"), uid);
    assert_eq!(
        stalled[0].field("event"),
        "firebase_identity.delete_stalled"
    );
}

#[tokio::test]
async fn a_queued_identity_a_new_account_signed_in_with_is_left_until_that_account_goes() {
    let setup = Setup::with_firebase(Answer::Unavailable).await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (user, _, uid) = seed_google_account(repos).await;
    let response = setup
        .delete(&user, &json!({ "confirm_email": user.email }))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let due_at = setup.queued().await[0].next_attempt_at;

    // The athlete signs in again with the same Google account before the
    // sweep reaches it: Firebase hands back the same uid.
    let (returning, _) = seed_account(repos, FEDERATED_ONLY_PASSWORD_HASH.to_owned()).await;
    let mut linked = returning.clone();
    linked.firebase_uid = Some(uid.clone());
    repos.users.update(&linked).await.unwrap();

    setup.firebase.set_answer(Answer::Deleted);
    let outbox = repos.firebase_identity_deletions.as_ref();
    let deleter = setup.deleter();
    assert_eq!(
        sweep_firebase_identity_deletions(outbox, &deleter, due_at)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        setup.firebase.calls().len(),
        3,
        "the identity behind a live account is never deleted"
    );

    // That account goes too: the waiting row is due again and settles.
    repos
        .users
        .delete(returning.id, Some(FIREBASE_PROJECT))
        .await
        .unwrap();
    let queued = setup.queued().await;
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert_eq!(queued[0].firebase_uid, uid);
    assert_eq!(
        sweep_firebase_identity_deletions(outbox, &deleter, due_at)
            .await
            .unwrap(),
        1
    );
    let calls = setup.firebase.calls();
    assert_eq!(calls.len(), 4, "{calls:?}");
    assert_eq!(calls[3].local_id, uid);
    assert!(setup.queued().await.is_empty());
}

#[tokio::test]
async fn a_firebase_user_already_gone_counts_as_deleted_without_retrying() {
    let setup = Setup::with_firebase(Answer::UserNotFound).await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (user, _, uid) = seed_google_account(repos).await;

    let response = setup
        .delete(&user, &json!({ "confirm_email": user.email }))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let calls = setup.firebase.calls();
    assert_eq!(calls.len(), 1, "USER_NOT_FOUND is final: {calls:?}");
    assert_eq!(calls[0].local_id, uid);
    assert!(repos.users.get_global(user.id).await.unwrap().is_none());
    assert!(
        setup.queued().await.is_empty(),
        "already gone settles the row"
    );
}

#[tokio::test]
async fn a_group_owner_is_refused_naming_the_group_and_nothing_is_touched() {
    let mut setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (user, tenant_id) = seed_password_account(repos).await;
    connect_strava(repos, user.id, tenant_id).await;
    seed_group(repos, user.id, tenant_id).await;

    let response = setup
        .delete(
            &user,
            &json!({ "confirm_email": user.email, "password": PASSWORD }),
        )
        .await;
    assert_eq!(response.status(), 409, "{}", response.body_text());
    let body: Value = response.json();
    assert_eq!(body["error"], "blocked");
    assert!(
        body["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["kind"] == "owns_coaching_group"),
        "{body}"
    );
    assert!(setup.stub.received().is_empty(), "nothing revoked");
    assert!(repos.users.get_global(user.id).await.unwrap().is_some());
    assert_eq!(
        repos
            .oauth_tokens
            .list_token_providers(user.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn an_operator_account_cannot_delete_itself() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (mut user, _) = seed_password_account(repos).await;
    user.role = UserRole::Admin;
    user.is_admin = true;
    repos.users.update(&user).await.unwrap();

    let preview = setup.preview(&user).await;
    assert_eq!(preview.status(), 403);
    let response = setup
        .delete(
            &user,
            &json!({ "confirm_email": user.email, "password": PASSWORD }),
        )
        .await;
    assert_eq!(response.status(), 403);
    assert!(repos.users.get_global(user.id).await.unwrap().is_some());
}

/// Only a signed-in session deletes: the athlete's API key and a personal MCP
/// token both authenticate as the athlete, yet each is refused with 403 on the
/// preview and the delete, even with the right email and password, and the
/// account and its grant stand.
#[tokio::test]
async fn an_api_key_or_mcp_token_cannot_delete_the_account() {
    let mut setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (user, tenant_id) = seed_password_account(repos).await;
    connect_strava(repos, user.id, tenant_id).await;

    let (api_key, full_key) = ApiKeyManager::new()
        .create_api_key(
            user.id,
            CreateApiKeyRequest {
                name: "deletion probe".to_owned(),
                description: None,
                tier: ApiKeyTier::Starter,
                rate_limit_requests: None,
                expires_in_days: None,
            },
        )
        .unwrap();
    repos.api_keys.create(&api_key).await.unwrap();
    let mcp_token = repos
        .user_mcp_tokens
        .create_token(
            user.id,
            &CreateUserMcpTokenRequest {
                name: "Claude Desktop".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .unwrap()
        .token_value;

    // REST reads an API key raw and a personal MCP token as a bearer token.
    for authorization in [full_key, format!("Bearer {mcp_token}")] {
        let preview = AxumTestRequest::get("/api/user/account-deletion")
            .header("Authorization", &authorization)
            .send(setup.routes.clone())
            .await;
        assert_eq!(preview.status(), 403, "{}", preview.body_text());
        let response = AxumTestRequest::post("/api/user/account-deletion")
            .header("Authorization", &authorization)
            .json(&json!({ "confirm_email": user.email, "password": PASSWORD }))
            .send(setup.routes.clone())
            .await;
        assert_eq!(response.status(), 403, "{}", response.body_text());
    }

    assert!(setup.stub.received().is_empty(), "nothing revoked");
    assert!(repos.users.get_global(user.id).await.unwrap().is_some());
    assert_eq!(
        repos
            .oauth_tokens
            .list_token_providers(user.id)
            .await
            .unwrap()
            .len(),
        1,
        "the grant stands"
    );
}

/// Every password re-confirmation spends the account's window, right or
/// wrong: past it the delete is refused with 429 and a `Retry-After`, even
/// with the right password, and the account stands.
#[tokio::test]
async fn password_guesses_past_the_window_are_refused_with_429() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (user, _) = seed_password_account(repos).await;

    for attempt in 0..PASSWORD_CONFIRM_RPM {
        let wrong = setup
            .delete(
                &user,
                &json!({ "confirm_email": user.email, "password": format!("guess-{attempt}") }),
            )
            .await;
        assert_eq!(wrong.status(), 403, "guess {attempt} is checked");
    }

    let limited = setup
        .delete(
            &user,
            &json!({ "confirm_email": user.email, "password": PASSWORD }),
        )
        .await;
    assert_eq!(limited.status(), 429, "{}", limited.body_text());
    let retry_after: u64 = limited
        .header("retry-after")
        .expect("a Retry-After")
        .parse()
        .unwrap();
    assert!(retry_after >= 1);
    assert_eq!(limited.json::<Value>()["error"], "too_many_attempts");
    assert!(repos.users.get_global(user.id).await.unwrap().is_some());
}

/// `change-password` spends the same window: guesses there lock the account
/// deletion out too, and `change-password` itself answers 429 past it.
#[tokio::test]
async fn change_password_and_deletion_share_one_window_of_guesses() {
    let setup = Setup::new().await;
    let registry = setup.repos();
    let repos = registry.as_ref();
    let (user, _) = seed_password_account(repos).await;
    let jwt = setup.jwt(&user);

    let change = |current: String| {
        AxumTestRequest::put("/api/user/change-password")
            .header("Authorization", &format!("Bearer {jwt}"))
            .json(&json!({ "current_password": current, "new_password": "NewSecurePass456" }))
            .send(setup.routes.clone())
    };
    for attempt in 0..PASSWORD_CONFIRM_RPM {
        let wrong = change(format!("guess-{attempt}")).await;
        assert_ne!(wrong.status(), 429, "guess {attempt} is within the window");
        assert_ne!(wrong.status(), 200, "guess {attempt} is wrong");
    }

    let limited = change(PASSWORD.to_owned()).await;
    assert_eq!(limited.status(), 429, "{}", limited.body_text());
    assert!(limited.header("retry-after").is_some());

    let deletion = setup
        .delete(
            &user,
            &json!({ "confirm_email": user.email, "password": PASSWORD }),
        )
        .await;
    assert_eq!(deletion.status(), 429);
    assert!(repos.users.get_global(user.id).await.unwrap().is_some());
}
