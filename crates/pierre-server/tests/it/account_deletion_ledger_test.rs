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

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use bytes::Bytes;
use dravr_barrage::blob::memory::MemoryBlobStore;
use dravr_barrage::blob::{BlobId, BlobKey, BlobMeta, BlobStore};
use dravr_barrage::digest::Sha256 as BarrageSha256;
use dravr_barrage::keyed::Signer;
use dravr_barrage::record::{StreamsState, SummaryWindow, TotalsFrom};
use dravr_barrage::record_log::checkpoint::{verify_note, Anchor};
use dravr_barrage::record_log::{merkle, RecordLog};
use dravr_barrage::store::memory::MemoryImportJobStore;
use dravr_barrage::store::{ActivityRecordStore, BarrageStores};
use dravr_barrage::{ActivityId, ActivityRecord, Provenance, Scope, SourceKey};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_auth::dto::auth::{LoginRequest, RegisterRequest};
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_cache::{CacheKey, CacheResource};
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::{
    EvidenceRegistry, MessagingStringsRegistry, PromptRegistry, ToolDescriptionRegistry,
    TrainingCatalogueRegistry,
};
use pierre_core::admin::models::{AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::groups::{CoachingGroup, GroupDigestMode, GroupRespondMode};
use pierre_core::models::{
    ActivityBuilder, ConnectionType, DataSource, DeviceType, SportType, StoredSleepSession, Tenant,
    TenantId, User, UserOAuthToken, UserStatus,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{
    email_identity_hash, provider_identity_hash, verify_receipt_chain, AccountDeletion,
    AccountDeletionPlan, AccountDeletionRepository, DeletionActor, DeletionReceipt, PlannedBackend,
    ReceiptAnchor, TombstoneKind, GENESIS_HASH,
};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::handlers::deletion_ledger::{
    handle_list_deletion_receipts, ReceiptPageQuery,
};
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use pierre_routes_auth::AuthRoutes;
use pierre_services::admin_ops::AccountCaller;
use pierre_services::analytics::{cache_user_email, resolve_user_email};
use pierre_services::barrage_keys::{
    checkpoint_signer, memory_records, LocalSigner, PlatformKeyed,
};
use pierre_services::import_tombstones::clear_by_admin;
use pierre_services::provider_revocation::{DisconnectReason, RevocationOutcome};
use pierre_services::record_log_worker::{KeyedNamesPage, RecordLogWorker};
use pierre_services::user_removal::{
    remove_user, ProviderDisconnector, RemovalReport, UserRemoval,
};
use pierre_services::webhook_owner::{resolve_webhook_owner, WebhookTarget};
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use ring::signature::{UnparsedPublicKey, ED25519};
use secrecy::SecretString;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use axum::body::to_bytes;
use axum::extract::connect_info::MockConnectInfo;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{Extension, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};

use crate::common::{acting_on, create_test_server_resources};
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
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
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
        Some(Timestamp::now() + Elapsed::hours(6)),
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
    let night = Timestamp::now() - Elapsed::days(1);
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
                end_datetime: night + Elapsed::hours(8),
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
                avg_spo2: None,
                stages: Vec::new(),
                source_name: "strava".to_owned(),
            },
            Timestamp::now(),
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
    let now = Timestamp::now();
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
                chat_connection: None,
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
/// production helper, as an admin holding the id would.
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
            actor: DeletionActor::Admin,
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
            &acting_on(repos, &AccountCaller::account_holder(first), first).await,
        )
        .await
        .unwrap(),
    );
    let second_report = removed(
        remove_user(
            repos,
            None,
            None,
            None,
            &acting_on(
                repos,
                &AccountCaller::account_holder_on_messaging(second),
                second,
            )
            .await,
        )
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
    // Uploaded files and an import's recordings live in the blob store, out
    // of the transaction's reach: the receipt names them as queued for
    // erasure, never as deleted (Athlete Data Consolidation, S12).
    assert!(
        not_deleted
            .iter()
            .any(|copy| copy.starts_with("kept_originals:")
                && copy.contains("queued for their erasure")),
        "bug: the receipt must name the blob store's copies as queued: {not_deleted:?}"
    );
    // The rows of a coach's own group chat that name the account go with it
    // (carnet#1174); the copies there that name no single member are named,
    // each of them, not claimed deleted (carnet#1255).
    let group_chats: Vec<&String> = not_deleted
        .iter()
        .filter(|copy| copy.starts_with("coach_group_chats:"))
        .collect();
    assert_eq!(
        group_chats.len(),
        1,
        "the receipt names the coach's group chats once: {not_deleted:?}"
    );
    assert!(
        group_chats[0].contains("the rows drawn from the account alone go with it"),
        "the entry says what the deletion did reach: {group_chats:?}"
    );
    for kept in [
        "the coach's own questions stay",
        "a reply from a turn that read several members",
        "a reply restating the member summaries",
        "copied into the group's room transcript",
        "rows written before the chat recorded which member a row is about",
    ] {
        assert!(
            group_chats[0].contains(kept),
            "the entry names {kept:?} as a copy left behind: {group_chats:?}"
        );
    }
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
/// the admin route serves; a token below super-admin reads none.
#[tokio::test]
async fn a_tampered_receipt_is_named_as_the_first_broken_link() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    for label in ["tamper-a", "tamper-b"] {
        let (user_id, _, _) = seed_athlete(repos, label).await;
        removed(
            remove_user(
                repos,
                None,
                None,
                None,
                &acting_on(repos, &AccountCaller::account_holder(user_id), user_id).await,
            )
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
            &acting_on(repos, &AccountCaller::host_cli(None), user_id).await,
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
/// admin, the same event reaches the account that holds it now.
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
            &acting_on(repos, &AccountCaller::account_holder(deleted), deleted).await,
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

    clear_by_admin(
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
        remove_user(
            repos,
            None,
            None,
            None,
            &acting_on(repos, &AccountCaller::account_holder(deleted), deleted).await,
        )
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
            analytics_consent: None,
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

/// A super-admin token, or one with no permission at all.
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

/// A deletion writes each tombstone with its keyed digest beside the plain
/// one; a row an older binary wrote without it still refuses its identity,
/// and the backfill keys it once (carnet#1542).
#[tokio::test]
async fn a_tombstone_is_keyed_and_a_legacy_unkeyed_one_still_refuses() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (deleted, tenant_id, _) = seed_athlete(repos, "keyed").await;
    connect_strava(repos, deleted, tenant_id, STRAVA_ATHLETE).await;
    removed(
        remove_user(
            repos,
            Some(&ConfirmsRevocation),
            None,
            None,
            &acting_on(repos, &AccountCaller::account_holder(deleted), deleted).await,
        )
        .await
        .unwrap(),
    );
    let unkeyed = "SELECT COUNT(*) FROM import_tombstones WHERE identity_key IS NULL";
    let keyed_as_hash = "SELECT COUNT(*) FROM import_tombstones WHERE identity_key = identity_hash";
    assert_eq!(
        scalar(&resources, unkeyed, None).await,
        0,
        "bug: a tombstone written unkeyed"
    );
    assert_eq!(
        scalar(&resources, keyed_as_hash, None).await,
        0,
        "bug: the key is the plain hash"
    );

    execute(
        &resources,
        "UPDATE import_tombstones SET identity_key = NULL",
    )
    .await;
    let target = resolve_webhook_owner(repos, "strava", STRAVA_ATHLETE)
        .await
        .unwrap();
    assert!(
        matches!(target, WebhookTarget::Tombstoned { .. }),
        "bug: an unkeyed tombstone no longer refuses its provider account: {target:?}"
    );
    let keyed = repos
        .import_tombstones
        .backfill_identity_keys(100)
        .await
        .unwrap();
    assert_eq!(
        keyed, 2,
        "bug: the backfill keys the provider account and the email"
    );
    assert_eq!(
        repos
            .import_tombstones
            .backfill_identity_keys(100)
            .await
            .unwrap(),
        0,
        "bug: the backfill keys a tombstone twice"
    );
    let target = resolve_webhook_owner(repos, "strava", STRAVA_ATHLETE)
        .await
        .unwrap();
    assert!(
        matches!(target, WebhookTarget::Tombstoned { .. }),
        "bug: a backfilled key does not match its identity: {target:?}"
    );
}

/// The `record_log` worker anchors the receipt chain's head as leaf 0 of a
/// signed checkpoint, records the anchor, and the admin ledger serves it; an
/// account the platform no longer holds loses its log (carnet#1542).
#[tokio::test]
async fn a_checkpoint_anchors_the_receipt_chain_head() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    for label in ["anchor-a", "anchor-b"] {
        let (user_id, _, _) = seed_athlete(repos, label).await;
        removed(
            remove_user(
                repos,
                None,
                None,
                None,
                &acting_on(repos, &AccountCaller::account_holder(user_id), user_id).await,
            )
            .await
            .unwrap(),
        );
    }
    let (athlete, athlete_tenant, _) = seed_athlete(repos, "anchor-kept").await;
    let records = Arc::new(memory_records());
    let kept = Scope::new(Uuid::from(athlete_tenant), athlete);
    let orphan = Scope::new(Uuid::new_v4(), Uuid::new_v4());
    for scope in [kept, orphan] {
        let id = ActivityId::parse("anchor-ride").unwrap();
        let record = anchor_ride(scope, id);
        records.upsert(&scope, &[record]).await.unwrap();
    }
    let signer = Arc::new(LocalSigner::random().unwrap());
    let worker = RecordLogWorker::new(
        records.clone(),
        Arc::clone(&repos.import_tombstones),
        Arc::clone(&repos.account_deletion),
        Arc::clone(&repos.users),
        Some(signer.clone()),
        None,
    );

    let tick = worker.run().await.unwrap();
    assert_eq!(
        tick.orphans_swept, 1,
        "bug: a log of no account outlives it"
    );
    assert!(
        records.log_head(&orphan).await.unwrap().is_none(),
        "bug: the orphan's log head survives the sweep"
    );
    let anchor = tick.anchored.expect("a checkpoint is cut and anchored");
    let chain = receipts(repos).await;
    let head = chain.last().unwrap();
    assert_eq!(
        (anchor.receipt_seq, &anchor.receipt_hash),
        (head.seq, &head.hash),
        "bug: the anchor is not the chain head"
    );

    let (checkpoint, _, leaves) = records.athlete_checkpoint(&kept).await.unwrap().unwrap();
    let anchor_leaf = Anchor {
        seq: u64::try_from(head.seq).unwrap(),
        hash: BarrageSha256::parse_hex(&head.hash).unwrap(),
    }
    .leaf();
    assert_eq!(leaves[0], anchor_leaf, "bug: the chain head is not leaf 0");
    let path = merkle::inclusion_proof(0, &leaves).unwrap();
    assert!(
        merkle::verify_inclusion(0, checkpoint.size, &anchor_leaf, &path, &checkpoint.root),
        "bug: the chain head is not proven under the signed root"
    );
    let verify = |key: &[u8; 32], message: &[u8], sig: &[u8; 64]| {
        UnparsedPublicKey::new(&ED25519, key)
            .verify(message, sig)
            .is_ok()
    };
    let note = verify_note(
        &anchor.note,
        signer.key_name(),
        &signer.public_key(),
        &verify,
    )
    .unwrap();
    assert_eq!(
        note.root, checkpoint.root,
        "bug: the note signs another tree"
    );

    let context = admin_context(&resources);
    let response = handle_list_deletion_receipts(
        State(context),
        Extension(admin_token(true)),
        Query(ReceiptPageQuery::default()),
    )
    .await
    .unwrap();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body["data"]["anchor"]["receipt_seq"],
        json!(head.seq),
        "bug: the admin ledger serves another anchor"
    );
    assert_eq!(
        body["data"]["anchor"]["checkpoint_id"],
        json!(checkpoint.id),
        "bug: the admin ledger names another checkpoint"
    );

    let again = worker.run().await.unwrap();
    assert_eq!(
        again.anchored, None,
        "bug: an unchanged tree is signed again"
    );
}

/// The `OAuth2` router, which serves the transparency documents, as the
/// server mounts it.
fn transparency_routes(resources: &Arc<ServerContext>) -> Router {
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
        rate_limiter: Arc::new(OAuth2RateLimiter::new(
            None,
            OAuth2RateLimiter::local_window_store(),
            &resources.common.config.rate_limiting,
        )),
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))))
}

/// `/.well-known/dravr-checkpoint` serves the newest anchored checkpoint as
/// a C2SP signed note, and `/.well-known/did.json` the key it is signed
/// with: a verifier holding only the two documents checks the signature
/// and finds the receipt chain head proven under the signed root
/// (carnet#1542).
#[tokio::test]
async fn the_well_known_documents_publish_a_checkpoint_its_key_verifies() {
    use base64::Engine as _;

    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let routes = transparency_routes(&resources);
    AxumTestRequest::get("/.well-known/dravr-checkpoint")
        .send(routes.clone())
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // The signer the server boots with: no KMS in tests, so the local key
    // derived from this database, published for did.json.
    let signer = checkpoint_signer(&PlatformKeyed::of(&resources.agent.database))
        .await
        .unwrap()
        .expect("premise: a deployment without KMS signs locally");
    let (gone, _, _) = seed_athlete(repos, "well-known-gone").await;
    removed(
        remove_user(
            repos,
            None,
            None,
            None,
            &acting_on(repos, &AccountCaller::account_holder(gone), gone).await,
        )
        .await
        .unwrap(),
    );
    let (athlete, athlete_tenant, _) = seed_athlete(repos, "well-known-kept").await;
    let records = Arc::new(memory_records());
    let kept = Scope::new(Uuid::from(athlete_tenant), athlete);
    records
        .upsert(
            &kept,
            &[anchor_ride(
                kept,
                ActivityId::parse("well-known-ride").unwrap(),
            )],
        )
        .await
        .unwrap();
    let anchored = RecordLogWorker::new(
        records.clone(),
        Arc::clone(&repos.import_tombstones),
        Arc::clone(&repos.account_deletion),
        Arc::clone(&repos.users),
        Some(signer),
        None,
    )
    .run()
    .await
    .unwrap()
    .anchored
    .expect("premise: a checkpoint is cut and anchored");

    let did = AxumTestRequest::get("/.well-known/did.json")
        .send(routes.clone())
        .await
        .assert_status(StatusCode::OK);
    assert!(
        did.header("content-type")
            .is_some_and(|value| value.starts_with("application/json")),
        "bug: did.json is not served as JSON: {:?}",
        did.header("content-type")
    );
    let document: Value = did.json();
    let method = &document["verificationMethod"][0];
    assert_eq!(
        (
            &method["type"],
            &method["publicKeyJwk"]["kty"],
            &method["publicKeyJwk"]["crv"]
        ),
        (&json!("JsonWebKey2020"), &json!("OKP"), &json!("Ed25519")),
        "bug: did.json does not name an Ed25519 JWK"
    );
    let public_key: [u8; 32] = URL_SAFE_NO_PAD
        .decode(
            method["publicKeyJwk"]["x"]
                .as_str()
                .expect("bug: the JWK has no x"),
        )
        .unwrap()
        .try_into()
        .expect("bug: the published key is not 32 bytes");
    let key_name = method["c2spKeyName"]
        .as_str()
        .expect("bug: did.json names no C2SP key name");

    let checkpoint = AxumTestRequest::get("/.well-known/dravr-checkpoint")
        .send(routes)
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        checkpoint.header("content-type"),
        Some("text/plain; charset=utf-8"),
        "bug: the checkpoint is not served as signed-note text"
    );
    let note_text = checkpoint.text();
    assert_eq!(
        note_text, anchored.note,
        "bug: another note than the newest anchor's"
    );
    let lines: Vec<&str> = note_text.lines().collect();
    assert!(
        lines.len() >= 5
            && lines[1] == anchored.tree_size.to_string()
            && lines[3].is_empty()
            && lines[4].starts_with(&format!("\u{2014} {key_name} ")),
        "bug: the checkpoint is not a C2SP note (origin, size, root, blank, signature): {note_text}"
    );
    let verify = |key: &[u8; 32], message: &[u8], sig: &[u8; 64]| {
        UnparsedPublicKey::new(&ED25519, key)
            .verify(message, sig)
            .is_ok()
    };
    let note = verify_note(&note_text, key_name, &public_key, &verify)
        .expect("bug: the served note does not verify against the published key");
    let (stored, _, leaves) = records.athlete_checkpoint(&kept).await.unwrap().unwrap();
    assert_eq!(
        (note.root, note.size),
        (stored.root, stored.size),
        "bug: the note signs another tree"
    );
    let head = receipts(repos).await.last().cloned().unwrap();
    let anchor_leaf = Anchor {
        seq: u64::try_from(head.seq).unwrap(),
        hash: BarrageSha256::parse_hex(&head.hash).unwrap(),
    }
    .leaf();
    let path = merkle::inclusion_proof(0, &leaves).unwrap();
    assert!(
        merkle::verify_inclusion(0, note.size, &anchor_leaf, &path, &note.root),
        "bug: the receipt chain head is not proven under the published root"
    );
    let mut forged = public_key;
    forged[0] ^= 1;
    assert!(
        verify_note(&note_text, key_name, &forged, &verify).is_err(),
        "bug: the note verifies against another key"
    );
}

/// The receipt repository, with its next anchor write failing: a real
/// store offers no way to fail one row write between a checkpoint's cut and
/// its anchor, so every call delegates to it but that one.
struct FailingAnchorWrite {
    inner: Arc<dyn AccountDeletionRepository>,
    fail_next: AtomicBool,
}

#[async_trait]
impl AccountDeletionRepository for FailingAnchorWrite {
    async fn delete_account(&self, plan: &AccountDeletionPlan) -> AppResult<AccountDeletion> {
        self.inner.delete_account(plan).await
    }

    async fn list_deletion_receipts(
        &self,
        after_seq: i64,
        limit: i64,
    ) -> AppResult<Vec<DeletionReceipt>> {
        self.inner.list_deletion_receipts(after_seq, limit).await
    }

    async fn receipt_chain_head(&self) -> AppResult<Option<(i64, String)>> {
        self.inner.receipt_chain_head().await
    }

    async fn record_anchor(&self, anchor: &ReceiptAnchor) -> AppResult<()> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(AppError::database("injected: the anchor row write failed"));
        }
        self.inner.record_anchor(anchor).await
    }

    async fn latest_anchor(&self) -> AppResult<Option<ReceiptAnchor>> {
        self.inner.latest_anchor().await
    }
}

/// A cut whose anchor row failed to write is repaired on the next tick,
/// with no new record and no new checkpoint: the row names the checkpoint
/// already cut and the chain head it commits to (carnet#1542).
#[tokio::test]
async fn a_failed_anchor_write_is_repaired_on_the_next_tick() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (user_id, _, _) = seed_athlete(repos, "repair-gone").await;
    removed(
        remove_user(
            repos,
            None,
            None,
            None,
            &acting_on(repos, &AccountCaller::account_holder(user_id), user_id).await,
        )
        .await
        .unwrap(),
    );
    let (athlete, athlete_tenant, _) = seed_athlete(repos, "repair-kept").await;
    let records = Arc::new(memory_records());
    let scope = Scope::new(Uuid::from(athlete_tenant), athlete);
    records
        .upsert(
            &scope,
            &[anchor_ride(
                scope,
                ActivityId::parse("repair-ride").unwrap(),
            )],
        )
        .await
        .unwrap();
    let failing = Arc::new(FailingAnchorWrite {
        inner: Arc::clone(&repos.account_deletion),
        fail_next: AtomicBool::new(true),
    });
    let worker = RecordLogWorker::new(
        records.clone(),
        Arc::clone(&repos.import_tombstones),
        failing,
        Arc::clone(&repos.users),
        Some(Arc::new(LocalSigner::random().unwrap())),
        None,
    );

    assert!(
        worker.run().await.is_err(),
        "premise: the injected anchor write fails the tick"
    );
    let cut = records
        .latest_checkpoint()
        .await
        .unwrap()
        .expect("premise: the checkpoint is cut before its anchor write");
    assert!(
        repos
            .account_deletion
            .latest_anchor()
            .await
            .unwrap()
            .is_none(),
        "premise: no anchor row was written"
    );

    let repaired = worker
        .run()
        .await
        .unwrap()
        .anchored
        .expect("bug: the missing anchor waits for the next change to the tree");
    let head = receipts(repos).await.last().cloned().unwrap();
    assert_eq!(
        (
            repaired.checkpoint_id,
            repaired.tree_size,
            &repaired.tree_root
        ),
        (cut.id, i64::try_from(cut.size).unwrap(), &cut.root.to_hex()),
        "bug: the repair anchors another checkpoint than the one cut"
    );
    assert_eq!(
        (repaired.receipt_seq, &repaired.receipt_hash, &repaired.note),
        (head.seq, &head.hash, &cut.note),
        "bug: the repair records another head or note than the checkpoint signs"
    );
    assert_eq!(
        records.latest_checkpoint().await.unwrap().map(|c| c.id),
        Some(cut.id),
        "bug: the repair cut a new checkpoint"
    );
    assert_eq!(
        repos
            .account_deletion
            .latest_anchor()
            .await
            .unwrap()
            .map(|a| a.checkpoint_id),
        Some(cut.id),
        "bug: the repaired anchor is not stored"
    );
    assert_eq!(
        worker.run().await.unwrap().anchored,
        None,
        "bug: a recorded anchor is recorded again"
    );
}

/// The `record_log` worker renames an original still named by its content
/// to its keyed name, keeps the old name as the record's legacy original,
/// counts an original it cannot read, and ends the sweep reporting what
/// remains (carnet#1542, the done signal for carnet#1550).
#[tokio::test]
async fn the_worker_renames_content_named_originals_and_reports_what_remains() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (athlete, athlete_tenant, _) = seed_athlete(repos, "keyed-names").await;
    let scope = Scope::new(Uuid::from(athlete_tenant), athlete);
    let records: Arc<dyn ActivityRecordStore> = Arc::new(memory_records());
    let blobs = Arc::new(MemoryBlobStore::new());
    let bytes = Bytes::from_static(b"a FIT recording kept by an older binary");
    let digest = BarrageSha256::of(&bytes);
    let content_named = BlobKey::original(scope, digest);
    blobs
        .put(
            &content_named,
            bytes.clone(),
            &BlobMeta::new("application/octet-stream"),
        )
        .await
        .unwrap();
    let kept = ActivityId::parse("kept-ride").unwrap();
    let lost = ActivityId::parse("lost-ride").unwrap();
    let lost_digest = BarrageSha256::of(b"bytes no store holds");
    let mut kept_ride = anchor_ride(scope, kept.clone());
    kept_ride.provenance.original = Some(content_named);
    kept_ride.provenance.original_sha256 = Some(digest);
    let mut lost_ride = anchor_ride(scope, lost);
    lost_ride.provenance.original = Some(BlobKey::original(scope, lost_digest));
    lost_ride.provenance.original_sha256 = Some(lost_digest);
    records
        .upsert(&scope, &[kept_ride, lost_ride])
        .await
        .unwrap();
    let worker = RecordLogWorker::new(
        Arc::clone(&records),
        Arc::clone(&repos.import_tombstones),
        Arc::clone(&repos.account_deletion),
        Arc::clone(&repos.users),
        None,
        Some(BarrageStores {
            blobs: blobs.clone(),
            records: Arc::clone(&records),
            jobs: Arc::new(MemoryImportJobStore::new()),
        }),
    );

    let tick = worker.run().await.unwrap();

    let page = tick.keyed_names.expect("a blob store renames originals");
    assert_eq!(
        page,
        KeyedNamesPage {
            repointed: 1,
            unreadable: 1,
            remaining: 1,
            sweep_done: true,
        },
        "bug: the sweep does not report the unreadable original as what remains"
    );
    let stored = records
        .get(&scope, SourceKey::StravaArchive, &kept)
        .await
        .unwrap()
        .expect("the ride is still held");
    let keyed = stored
        .provenance
        .original
        .expect("the ride keeps an original");
    assert!(
        matches!(keyed.id, BlobId::Keyed(_)),
        "bug: the original is not renamed to its keyed name: {keyed}"
    );
    assert_eq!(
        stored.provenance.legacy_original,
        Some(content_named),
        "bug: the old name is not kept as the legacy original"
    );
    assert_eq!(
        blobs.get(&keyed).await.unwrap(),
        Some(bytes),
        "bug: the keyed name does not hold the original's bytes"
    );

    let without_blobs = RecordLogWorker::new(
        Arc::clone(&records),
        Arc::clone(&repos.import_tombstones),
        Arc::clone(&repos.account_deletion),
        Arc::clone(&repos.users),
        None,
        None,
    );
    assert_eq!(
        without_blobs.run().await.unwrap().keyed_names,
        None,
        "bug: a worker without a blob store renames originals"
    );
}

/// The keyed-name backfill pages across ticks: a sweep longer than one page
/// resumes where the last tick stopped, ends on the page that passes the
/// last record, and the next tick starts a new sweep (carnet#1542).
#[tokio::test]
async fn the_keyed_name_backfill_resumes_across_ticks_and_restarts_after_a_sweep() {
    // One page is 500 records (barrage's LOG_PAGE): one more spills a
    // second page.
    const RIDES: u64 = 501;
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (athlete, athlete_tenant, _) = seed_athlete(repos, "keyed-pages").await;
    let scope = Scope::new(Uuid::from(athlete_tenant), athlete);
    let records: Arc<dyn ActivityRecordStore> = Arc::new(memory_records());
    let rides: Vec<ActivityRecord> = (0..RIDES)
        .map(|n| {
            let digest = BarrageSha256::of(format!("bytes no store holds {n}").as_bytes());
            let mut ride = anchor_ride(scope, ActivityId::parse(&format!("lost-{n}")).unwrap());
            ride.provenance.original = Some(BlobKey::original(scope, digest));
            ride.provenance.original_sha256 = Some(digest);
            ride
        })
        .collect();
    records.upsert(&scope, &rides).await.unwrap();
    let worker = RecordLogWorker::new(
        Arc::clone(&records),
        Arc::clone(&repos.import_tombstones),
        Arc::clone(&repos.account_deletion),
        Arc::clone(&repos.users),
        None,
        Some(BarrageStores {
            blobs: Arc::new(MemoryBlobStore::new()),
            records: Arc::clone(&records),
            jobs: Arc::new(MemoryImportJobStore::new()),
        }),
    );

    let first = worker.run().await.unwrap().keyed_names;
    assert_eq!(
        first,
        Some(KeyedNamesPage {
            repointed: 0,
            unreadable: RIDES - 1,
            remaining: RIDES,
            sweep_done: false,
        }),
        "bug: a full first page ends the sweep"
    );
    let second = worker.run().await.unwrap().keyed_names;
    assert_eq!(
        second,
        Some(KeyedNamesPage {
            repointed: 0,
            unreadable: 1,
            remaining: RIDES,
            sweep_done: true,
        }),
        "bug: the second tick does not resume after the first page"
    );
    let third = worker.run().await.unwrap().keyed_names;
    assert_eq!(
        third, first,
        "bug: the tick after a finished sweep does not start a new one"
    );
}

/// One archive ride of `scope`.
fn anchor_ride(scope: Scope, id: ActivityId) -> ActivityRecord {
    let start = Timestamp::now() - Elapsed::days(1);
    let activity = ActivityBuilder::new(
        id.as_str(),
        "Ride",
        SportType::Ride,
        start,
        3600,
        "strava_archive",
    )
    .distance_meters(30_000.0)
    .build();
    let provenance = Provenance {
        source: SourceKey::StravaArchive,
        import_id: None,
        source_activity_id: id.as_str().to_owned(),
        source_athlete_id: None,
        ingested_at: start,
        original: None,
        original_sha256: None,
        session_index: None,
        streams_state: StreamsState::None,
        source_gear: None,
        importer_version: None,
        summary_window: SummaryWindow::default(),
        totals_from: TotalsFrom::default(),
        legacy_id: None,
        legacy_original: None,
    };
    ActivityRecord::try_new(scope, id, provenance, activity).unwrap()
}
