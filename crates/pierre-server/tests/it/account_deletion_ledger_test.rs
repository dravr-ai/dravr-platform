// ABOUTME: An account deletion is one transaction, leaves a chained receipt and tombstones
// ABOUTME: A failed deletion keeps everything; a tampered receipt is named; a tombstoned identity waits to be cleared
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! An account deletion removes the provider data and the account in one
//! transaction, appends a hash-chained receipt that proves it happened, evicts
//! the response caches keyed by the user, and tombstones the identities it
//! held so a webhook, a reconnect or a sign-up cannot hand the same data
//! straight back. Every test drives the real repositories and services
//! against a real database; the only stand-in is the upstream revocation,
//! which would otherwise reach a provider.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_auth::dto::auth::{LoginRequest, RegisterRequest};
use pierre_cache::{CacheKey, CacheResource};
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::{
    EvidenceRegistry, MessagingStringsRegistry, PromptRegistry, ToolDescriptionRegistry,
    TrainingCatalogueRegistry,
};
use pierre_core::admin::models::{AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::groups::{CoachingGroup, GroupDigestMode, GroupRespondMode};
use pierre_core::models::{
    ConnectionType, DataSource, DeviceType, StoredSleepSession, Tenant, TenantId, User,
    UserOAuthToken, UserStatus,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{
    email_identity_hash, provider_identity_hash, verify_receipt_chain, AccountDeletionPlan,
    DeletionActor, DeletionReceipt, PlannedBackend, TombstoneKind, GENESIS_HASH,
};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::handlers::deletion_ledger::{
    handle_list_deletion_receipts, ReceiptPageQuery,
};
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use pierre_routes_auth::AuthRoutes;
use pierre_services::analytics::{cache_user_email, resolve_user_email};
use pierre_services::import_tombstones::clear_by_operator;
use pierre_services::provider_revocation::{DisconnectReason, RevocationOutcome};
use pierre_services::user_removal::{
    remove_user, ProviderDisconnector, RemovalReport, UserRemoval,
};
use pierre_services::webhook_owner::{resolve_webhook_owner, WebhookTarget};
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use secrecy::SecretString;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use axum::body::to_bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Extension;

use crate::common::create_test_server_resources;
use crate::helpers::axum_test::AxumTestRequest;

/// The provider-side id the deleted athlete's Strava token held.
const STRAVA_ATHLETE: &str = "777001";

/// A user's rows in `sleep_sessions`, the provider data the tests seed.
const SLEEP_ROWS_SQL: &str = "SELECT COUNT(*) FROM sleep_sessions WHERE CAST(user_id AS TEXT) = $1";

/// Tamper with the first receipt in place, as someone with database access
/// would.
const TAMPER_SQL: &str = "UPDATE deletion_receipts SET actor = 'operator' WHERE seq = 1";

/// The upstream revocation, standing in for the provider: every grant is
/// confirmed withdrawn. An account removal never runs the per-provider
/// disconnect, so that path panics.
struct ConfirmsRevocation;

#[async_trait]
impl ProviderDisconnector for ConfirmsRevocation {
    async fn disconnect(
        &self,
        _: Uuid,
        _: &str,
        _: TenantId,
        _: DisconnectReason,
    ) -> AppResult<RevocationOutcome> {
        panic!("an account removal withdraws grants, it never runs the per-provider disconnect")
    }

    async fn withdraw_upstream(&self, _: Uuid, _: &str, _: TenantId) -> RevocationOutcome {
        RevocationOutcome::Revoked
    }

    fn supports(&self, provider: &str) -> bool {
        provider == "strava"
    }
}

/// An active athlete with a tenant of their own: `(id, tenant, email)`.
async fn seed_athlete(repos: &RepositoryRegistry, label: &str) -> (Uuid, TenantId, String) {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let mut user = User::new(email.clone(), "hash".to_owned(), Some(label.to_owned()));
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("{label} tenant"),
            slug: format!("{label}-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    (user_id, tenant_id, email)
}

/// A Strava connection as the OAuth callback leaves it, the token carrying
/// the athlete's Strava id.
async fn connect_strava(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    strava_id: &str,
) {
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let mut token = UserOAuthToken::new(
        user_id,
        tenant_id.to_string(),
        "strava".to_owned(),
        "access-material-do-not-log".to_owned(),
        Some("refresh-material-do-not-log".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        Some("read".to_owned()),
    );
    token.provider_user_id = Some(strava_id.to_owned());
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
}

/// One night of Strava-sourced sleep, with the data source it hangs off.
async fn seed_strava_sleep(repos: &RepositoryRegistry, user_id: Uuid, tenant_id: TenantId) {
    let source = repos
        .data_sources
        .upsert_data_source(
            &tenant_id,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: "strava".to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Band,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    let night = Utc::now() - Duration::days(1);
    repos
        .sleep
        .upsert_sleep_session(
            &tenant_id,
            &StoredSleepSession {
                id: format!("strava-sleep-{}", Uuid::new_v4()),
                user_id: user_id.to_string(),
                data_source_id: source,
                is_nap: false,
                start_datetime: night,
                end_datetime: night + Duration::hours(8),
                total_sleep_seconds: Some(25_200),
                deep_sleep_seconds: None,
                light_sleep_seconds: None,
                rem_sleep_seconds: None,
                awake_seconds: None,
                sleep_efficiency: None,
                avg_heart_rate: None,
                min_heart_rate: None,
                avg_hrv: None,
                sleep_score: None,
                stages: Vec::new(),
                source_name: "strava".to_owned(),
            },
        )
        .await
        .unwrap();
}

/// A coaching group `owner` owns: a row other people rely on, which no
/// foreign key lets the account delete take.
async fn seed_owned_group(repos: &RepositoryRegistry, owner: Uuid, tenant_id: TenantId) {
    let agent_id = repos
        .agents
        .create_system_agent(
            owner,
            tenant_id,
            &CreateSystemAgentRequest {
                title: "Rollback coach".to_owned(),
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
                name: "Rollback group".to_owned(),
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
        .unwrap();
}

/// Run one statement over the test database, binding `owner` when given.
async fn scalar(resources: &ServerContext, sql: &str, owner: Option<Uuid>) -> i64 {
    let owner = owner.map(|id| id.to_string());
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            let query = sqlx::query_scalar::<_, i64>(sql);
            let query = match &owner {
                Some(owner) => query.bind(owner),
                None => query,
            };
            query.fetch_optional(db.pool()).await.unwrap().unwrap_or(0)
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            let query = sqlx::query_scalar::<_, i64>(sql);
            let query = match &owner {
                Some(owner) => query.bind(owner),
                None => query,
            };
            query.fetch_optional(db.pool()).await.unwrap().unwrap_or(0)
        }
    }
}

/// Run a write over the test database.
async fn execute(resources: &ServerContext, sql: &str) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(sql).execute(db.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(sql).execute(db.pool()).await.unwrap();
        }
    }
}

/// The report of a removal that completed.
fn removed(removal: UserRemoval) -> RemovalReport {
    match removal {
        UserRemoval::Removed(report) => report,
        UserRemoval::Blocked(blockers) => panic!("blocked by {blockers:?}"),
        UserRemoval::Interrupted(interruption) => panic!("interrupted: {:?}", interruption.error),
    }
}

/// The receipt chain as the database holds it.
async fn receipts(repos: &RepositoryRegistry) -> Vec<DeletionReceipt> {
    repos
        .account_deletion
        .list_deletion_receipts(0, 1000)
        .await
        .unwrap()
}

/// SHA-256 of an account id, computed here rather than through the
/// production helper, as an operator holding the id would.
fn sha256_of(user_id: Uuid) -> String {
    format!("{:x}", Sha256::digest(user_id.to_string().as_bytes()))
}

/// A deletion that fails after it purged the provider's rows (a group the
/// athlete owns refuses the account row) rolls ALL of it back: the account,
/// its token and its provider data stand, and no receipt or tombstone is
/// written.
#[tokio::test]
async fn a_deletion_failing_part_way_leaves_the_account_and_its_provider_data() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (user_id, tenant_id, email) = seed_athlete(repos, "rollback").await;
    connect_strava(repos, user_id, tenant_id, STRAVA_ATHLETE).await;
    seed_strava_sleep(repos, user_id, tenant_id).await;
    seed_owned_group(repos, user_id, tenant_id).await;
    assert_eq!(scalar(&resources, SLEEP_ROWS_SQL, Some(user_id)).await, 1);

    let error = repos
        .account_deletion
        .delete_account(&AccountDeletionPlan {
            user_id,
            actor: DeletionActor::Operator,
            firebase_project: None,
            backends: vec![PlannedBackend {
                tenant_id,
                backend: "strava".to_owned(),
            }],
            revocations: "[]".to_owned(),
            not_deleted: Vec::new(),
        })
        .await
        .expect_err("the owned group refuses the account row");

    assert_eq!(error.code, ErrorCode::ResourceLocked);
    assert!(repos.users.get_global(user_id).await.unwrap().is_some());
    assert!(repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "strava")
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        scalar(&resources, SLEEP_ROWS_SQL, Some(user_id)).await,
        1,
        "the provider purge rolled back with the account delete"
    );
    assert!(
        receipts(repos).await.is_empty(),
        "no receipt for a rollback"
    );
    assert!(repos
        .import_tombstones
        .find_live(TombstoneKind::Email, None, &email_identity_hash(&email))
        .await
        .unwrap()
        .is_none());
    assert!(repos
        .import_tombstones
        .find_live(
            TombstoneKind::ProviderAccount,
            Some("strava"),
            &provider_identity_hash(STRAVA_ATHLETE),
        )
        .await
        .unwrap()
        .is_none());
}

/// Each deletion appends a receipt naming its account by the SHA-256 of the
/// id, chained to the receipt before it, recording who asked, the survivor
/// re-read, what each provider said, what could not be deleted and the
/// tombstones it left; the provider's data is gone with the account.
#[tokio::test]
async fn a_deletion_appends_a_receipt_chained_to_the_one_before() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (first, first_tenant, _) = seed_athlete(repos, "first").await;
    connect_strava(repos, first, first_tenant, STRAVA_ATHLETE).await;
    seed_strava_sleep(repos, first, first_tenant).await;
    let (second, _, _) = seed_athlete(repos, "second").await;

    let first_report = removed(
        remove_user(
            repos,
            Some(&ConfirmsRevocation),
            None,
            None,
            first,
            DeletionActor::Athlete,
        )
        .await
        .unwrap(),
    );
    let second_report = removed(
        remove_user(repos, None, None, None, second, DeletionActor::Messaging)
            .await
            .unwrap(),
    );

    let chain = receipts(repos).await;
    assert_eq!(chain.len(), 2);
    let (a, b) = (&chain[0], &chain[1]);
    assert_eq!((a.seq, a.prev_hash.as_str()), (1, GENESIS_HASH));
    assert_eq!((b.seq, b.prev_hash.as_str()), (2, a.hash.as_str()));
    assert_eq!(a.subject_hash, sha256_of(first));
    assert_eq!(b.subject_hash, sha256_of(second));
    assert_eq!(
        (a.actor.as_str(), b.actor.as_str()),
        ("athlete", "messaging")
    );
    assert_eq!(first_report.ledger.receipt_hash, a.hash);
    assert_eq!(second_report.ledger.receipt_seq, 2);

    let scan: Value = serde_json::from_str(&a.scan_result).unwrap();
    assert_eq!(scan["survivors_by_table"]["user_oauth_tokens"], 0);
    assert_eq!(scan["survivors_by_table"]["sleep_sessions"], 0);
    assert_eq!(
        scan["provider_rows_removed"][format!("strava@{first_tenant}")]["sleep_sessions"],
        1
    );
    let revocations: Value = serde_json::from_str(&a.revocations).unwrap();
    assert_eq!(revocations[0]["provider"], "strava");
    assert_eq!(revocations[0]["revocation"]["status"], "revoked");
    let not_deleted: Vec<String> = serde_json::from_str(&a.not_deleted).unwrap();
    assert!(
        not_deleted
            .iter()
            .any(|copy| copy.starts_with("posthog_person:")),
        "{not_deleted:?}"
    );
    let first_tombstones: Vec<String> = serde_json::from_str(&a.tombstone_ids).unwrap();
    assert_eq!(
        first_tombstones.len(),
        2,
        "the Strava account and the email"
    );
    assert_eq!(first_tombstones, first_report.ledger.tombstone_ids);
    let second_tombstones: Vec<String> = serde_json::from_str(&b.tombstone_ids).unwrap();
    assert_eq!(second_tombstones.len(), 1, "the email alone");

    let verification = verify_receipt_chain(&chain);
    assert_eq!(verification.first_break, None);
    assert_eq!(verification.head, Some((2, b.hash.clone())));
    assert_eq!(scalar(&resources, SLEEP_ROWS_SQL, Some(first)).await, 0);
    assert!(repos.users.get_global(first).await.unwrap().is_none());
}

/// A receipt altered in the database is named as the first broken link by
/// the verifier `pierre-cli deletion verify-receipts` runs, over the receipts
/// the operator route serves; a token below super-admin reads none.
#[tokio::test]
async fn a_tampered_receipt_is_named_as_the_first_broken_link() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    for label in ["tamper-a", "tamper-b"] {
        let (user_id, _, _) = seed_athlete(repos, label).await;
        removed(
            remove_user(repos, None, None, None, user_id, DeletionActor::Athlete)
                .await
                .unwrap(),
        );
    }
    let context = admin_context(&resources);
    let intact = served_receipts(&context, true).await;
    assert_eq!(verify_receipt_chain(&intact).first_break, None);

    execute(&resources, TAMPER_SQL).await;

    let tampered = served_receipts(&context, true).await;
    let fault = verify_receipt_chain(&tampered)
        .first_break
        .expect("the altered receipt breaks the chain");
    assert_eq!(fault.seq, 1);
    assert_eq!(fault.id.as_deref(), Some(tampered[0].id.as_str()));
    assert!(fault.reason.contains("altered"), "{}", fault.reason);

    let denied = handle_list_deletion_receipts(
        State(context),
        Extension(admin_token(false)),
        Query(ReceiptPageQuery::default()),
    )
    .await
    .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
}

/// After the commit, every cache entry under the deleted account's id, in
/// every tenant, is gone, and so is its email from the notify-event cache;
/// another account's entry stays.
#[tokio::test]
async fn cache_entries_and_the_cached_email_are_gone_after_the_deletion() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let cache = resources.data().cache().clone();
    let (user_id, tenant_id, email) = seed_athlete(repos, "cached").await;
    let (other, other_tenant, _) = seed_athlete(repos, "kept").await;
    let key = |tenant: TenantId, user: Uuid, provider: &str| {
        CacheKey::new(
            tenant,
            user,
            provider.to_owned(),
            CacheResource::AthleteProfile,
        )
    };
    let ttl = StdDuration::from_secs(600);
    for entry in [
        key(tenant_id, user_id, "strava"),
        key(TenantId::generate(), user_id, "whoop"),
        key(other_tenant, other, "strava"),
    ] {
        cache.set(&entry, &"profile", ttl).await.unwrap();
    }
    cache_user_email(&user_id.to_string(), &email);

    let report = removed(
        remove_user(
            repos,
            None,
            None,
            Some(cache.as_ref()),
            user_id,
            DeletionActor::Operator,
        )
        .await
        .unwrap(),
    );

    assert_eq!(report.ledger.cache_entries_removed, Some(2));
    assert!(report.ledger.identity_cache_evicted);
    assert!(!cache
        .exists(&key(tenant_id, user_id, "strava"))
        .await
        .unwrap());
    assert!(cache
        .exists(&key(other_tenant, other, "strava"))
        .await
        .unwrap());
    assert_eq!(resolve_user_email(&user_id.to_string()), None);
}

/// A push event naming a deleted account's Strava id is refused by its
/// tombstone, even once another account holds that id; cleared by an
/// operator, the same event reaches the account that holds it now.
#[tokio::test]
async fn a_webhook_for_a_tombstoned_provider_account_is_dropped_until_cleared() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (deleted, tenant_id, _) = seed_athlete(repos, "hooked").await;
    connect_strava(repos, deleted, tenant_id, STRAVA_ATHLETE).await;
    let report = removed(
        remove_user(
            repos,
            Some(&ConfirmsRevocation),
            None,
            None,
            deleted,
            DeletionActor::Athlete,
        )
        .await
        .unwrap(),
    );
    let (holder, holder_tenant, _) = seed_athlete(repos, "holder").await;
    connect_strava(repos, holder, holder_tenant, STRAVA_ATHLETE).await;

    let target = resolve_webhook_owner(repos, "strava", STRAVA_ATHLETE)
        .await
        .unwrap();
    let WebhookTarget::Tombstoned { tombstone_id } = target else {
        panic!("a tombstoned Strava account is refused, got {target:?}");
    };
    assert!(report.ledger.tombstone_ids.contains(&tombstone_id));

    clear_by_operator(
        repos.import_tombstones.as_ref(),
        &tombstone_id,
        "test-token",
    )
    .await
    .unwrap();
    match resolve_webhook_owner(repos, "strava", STRAVA_ATHLETE)
        .await
        .unwrap()
    {
        WebhookTarget::Owner(owner) => assert_eq!(owner.user_id, holder),
        other => panic!("the cleared account resolves to its holder, got {other:?}"),
    }
}

/// A sign-up with a deleted account's email waits Pending even with a
/// standing pre-approval. Only the account that proved the address can clear
/// the tombstone, which then admits it; another account cannot.
#[tokio::test]
async fn a_deleted_email_is_not_auto_approved_until_its_holder_clears_it() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (deleted, _, email) = seed_athlete(repos, "returning").await;
    removed(
        remove_user(repos, None, None, None, deleted, DeletionActor::Athlete)
            .await
            .unwrap(),
    );
    let tombstone = repos
        .import_tombstones
        .find_live(TombstoneKind::Email, None, &email_identity_hash(&email))
        .await
        .unwrap()
        .expect("the deletion tombstoned the email");
    repos
        .pre_approved_emails
        .allow(&email, None, Some("standing allow"))
        .await
        .unwrap();

    let auth = resources.auth_routes_context().account_service();
    let registered = auth
        .register(RegisterRequest {
            email: email.clone(),
            password: SecretString::from("CorrectHorse-482"),
            display_name: None,
        })
        .await
        .unwrap();
    assert_eq!(
        registered.user_status,
        UserStatus::Pending,
        "a deleted email is not re-admitted straight to Active"
    );
    let newcomer = repos.users.get_by_email(&email).await.unwrap().unwrap();
    let routes = AuthRoutes::routes(resources.auth_routes_context());
    let clear = |user: &User, body: Value| {
        let jwt = resources
            .auth
            .auth_manager
            .generate_token(user, &resources.auth.jwks_manager)
            .unwrap();
        AxumTestRequest::post("/api/user/import-tombstones/clear")
            .header("Authorization", &format!("Bearer {jwt}"))
            .json(&body)
            .send(routes.clone())
    };

    // The pending screen reads why the account waits off the sign-in it lands
    // from (a Pending account restores no session; the middleware holds it).
    let signed_in = || async {
        let login = auth
            .login(LoginRequest {
                email: email.clone(),
                password: SecretString::from("CorrectHorse-482"),
                timezone: None,
            })
            .await
            .unwrap();
        serde_json::to_value(login.user).unwrap()
    };
    let pending = signed_in().await;
    assert_eq!(pending["user_status"], "pending");
    assert_eq!(
        pending["email_tombstone"]["tombstone_id"],
        tombstone.id.as_str(),
        "the pending screen learns why the account waits: {pending}"
    );
    assert_eq!(
        pending["email_tombstone"]["deleted_at"],
        tombstone.created_at.to_rfc3339().as_str()
    );

    let unproven = clear(&newcomer, json!({})).await;
    assert_eq!(unproven.status(), 400, "an unproven address clears nothing");

    let (_, _, bystander_email) = seed_athlete(repos, "bystander").await;
    let bystander = repos
        .users
        .get_by_email(&bystander_email)
        .await
        .unwrap()
        .unwrap();
    repos
        .email_verification
        .mark_verified(bystander.id)
        .await
        .unwrap();
    let foreign = clear(&bystander, json!({ "tombstone_id": tombstone.id })).await;
    assert_eq!(foreign.status(), 404, "another account cannot clear it");

    repos
        .email_verification
        .mark_verified(newcomer.id)
        .await
        .unwrap();
    let cleared = clear(&newcomer, json!({})).await;
    assert_eq!(cleared.status(), 200);
    let body: Value = cleared.json();
    assert_eq!(body["cleared"][0]["tombstone_id"], tombstone.id.as_str());
    assert_eq!(body["user_status"], "active", "{body}");
    assert_eq!(
        repos
            .users
            .get_global(newcomer.id)
            .await
            .unwrap()
            .unwrap()
            .user_status,
        UserStatus::Active
    );

    let admitted = signed_in().await;
    assert_eq!(admitted["user_status"], "active");
    assert!(
        admitted.get("email_tombstone").is_none(),
        "a cleared, admitted account names no tombstone: {admitted}"
    );

    let again = clear(&newcomer, json!({ "tombstone_id": tombstone.id })).await;
    assert_eq!(again.status(), 200, "clearing again is idempotent");
}

/// The admin context the composition root builds, over this server.
fn admin_context(resources: &Arc<ServerContext>) -> Arc<AdminApiContext> {
    Arc::new(AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: "test_admin_jwt_secret_for_deletion_ledger".to_owned(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        guardian_config_registry: Arc::new(GuardianConfigRegistry::bootstrap()),
        prompt_registry: Arc::new(PromptRegistry::new()),
        tool_description_registry: Arc::new(ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(EvidenceRegistry::new()),
        messaging_strings_registry: Arc::new(MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        training_catalogue_registry: Arc::new(TrainingCatalogueRegistry::new()),
        contremaitre_config: None,
        app_behavior: resources.common.config.app_behavior.clone(),
    }))
}

/// A super-admin admin token, or one with no permission at all.
fn admin_token(super_admin: bool) -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("test:{}", Uuid::new_v4()),
        service_name: "deletion-ledger-test".to_owned(),
        permissions: if super_admin {
            AdminPermissions::super_admin()
        } else {
            AdminPermissions::new(vec![])
        },
        is_super_admin: super_admin,
        tenant_id: None,
        user_info: None,
        operator_user_id: None,
    }
}

/// The receipts `GET /admin/deletion-receipts` serves.
async fn served_receipts(
    context: &Arc<AdminApiContext>,
    super_admin: bool,
) -> Vec<DeletionReceipt> {
    let response = handle_list_deletion_receipts(
        State(context.clone()),
        Extension(admin_token(super_admin)),
        Query(ReceiptPageQuery::default()),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    serde_json::from_value(body["data"]["receipts"].clone()).unwrap()
}
