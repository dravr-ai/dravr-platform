// ABOUTME: carnet#1463 — an operator starts a whole account over: every provider disconnected, every owned row wiped
// ABOUTME: The account, its sign-in, tenants and overrides stay; no tombstone, no receipt; refusals touch nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `POST /admin/users/{id}/reset` drives the real handler against a real
//! database, with Strava's revocation endpoint on a local stub, so "revoked at
//! the provider" is read on the wire. The wipe is checked table by table
//! through the reset's own survivor read, which names every table it wipes,
//! and the account's kept rows and another account's data are read back.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use bytes::Bytes;
use chrono::{Duration, Utc};
use dravr_barrage::blob::BlobKey;
use dravr_barrage::digest::Sha256;
use dravr_barrage::store::{PageReq, RecordQuery};
use dravr_barrage::Scope;
use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_core::models::agents::AgentVisibility;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupInvite, GroupInviteKind, GroupRespondMode,
};
use pierre_core::models::{
    ActivityBuilder, AddMessageParams, ConnectionType, SessionRefreshToken, SportType, TenantId,
    UpsertMessageFeedbackParams, UserOAuthToken, UserTier,
};
use pierre_core::transport::TransportPolicy;
use pierre_database::backends::factory::DatabaseBackend;
#[cfg(feature = "postgresql")]
use pierre_database::repositories::POSTGRES_ACCOUNT_RESET;
use pierre_database::repositories::{
    email_identity_hash, AccountResetCommit, InsertClaimVerdictParams, TombstoneKind,
    UserTierOverride, SQLITE_ACCOUNT_RESET,
};
use pierre_mcp_server::a2a::client::ClientRegistrationRequest;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::archive_import::sweeps::erase_deleted_accounts;
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
use pierre_providers::upload_provider::file_sha256;
use pierre_routes_admin::handlers::types::ResetAccountRequest;
use pierre_routes_admin::handlers::user_removal::handle_reset_user;
use pierre_routes_admin::handlers::users::handle_get_user;
use pierre_routes_admin::AdminApiContext;
use pierre_test_support::fit::FitWorkout;
use sqlx::Row;
use uuid::Uuid;

use crate::admin_user_removal_test::{
    admin_context, assert_one_strava_revocation, blockers_of, body_json, connect_strava,
    conversation_mid_walk, facts_of, join_group, manage_users_token, resources, seed_agent,
    seed_group, seed_onboarded_user, seed_user, strava_token, super_admin_token, RevokeStub,
    ENV_CLIENT_ID, ENV_CLIENT_SECRET,
};
use crate::archive_import_test::{
    call, import_export, imports_serving, seed_athlete, strava_export, Athlete,
};
use crate::common;

async fn reset(
    context: Arc<AdminApiContext>,
    token: ValidatedAdminToken,
    user_id: Uuid,
) -> AppResult<Response> {
    handle_reset_user(
        State(context),
        Extension(token),
        Path(user_id.to_string()),
        Json(ResetAccountRequest {
            reason: Some("starting over".to_owned()),
            delete_owned_groups: false,
        }),
    )
    .await
}

/// The wiped tables that still hold a row for `user_id`, read with the
/// reset's own survivor statement for the engine under test.
async fn surviving(resources: &ServerContext, user_id: Uuid) -> BTreeMap<String, i64> {
    let owner = user_id.to_string();
    let rows = match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => sqlx::query(SQLITE_ACCOUNT_RESET.surviving)
            .bind(&owner)
            .fetch_all(db.pool())
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get::<String, _>("tbl"), r.get::<i64, _>("n")))
            .collect(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => sqlx::query(POSTGRES_ACCOUNT_RESET.surviving)
            .bind(&owner)
            .fetch_all(db.pool())
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get::<String, _>("tbl"), r.get::<i64, _>("n")))
            .collect(),
    };
    rows
}

/// The app-session refresh token seeded for `user_id`.
fn session_token(user_id: Uuid) -> String {
    format!("session-{user_id}")
}

/// A user with data across many of the tables a reset wipes and keeps: the
/// onboarding steps, facts and profile, a Strava grant, an intervals.icu
/// coaching-platform key, a membership in someone else's group, a private
/// agent with the user's own thread bound to it, a short link, a usage
/// counter, an app session and a tier override.
async fn seed_lived_in_account(
    repos: &pierre_database::RepositoryRegistry,
    label: &str,
) -> (Uuid, TenantId, String) {
    let (user_id, tenant_id) = seed_onboarded_user(repos, label).await;
    let user = user_id.to_string();
    let tenant = tenant_id.to_string();
    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant_id,
            "intervals_icu",
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    let (peer_id, peer_tenant, _) = seed_user(repos, &format!("{label}-peer")).await;
    let joined = seed_group(repos, peer_id, peer_tenant, "Peer squad").await;
    join_group(repos, joined, user_id, tenant_id).await;
    let agent = seed_agent(
        repos,
        user_id,
        tenant_id,
        "My own coach",
        AgentVisibility::Private,
    )
    .await;
    let thread = repos
        .chat
        .create_conversation(&user, tenant_id, "thread", "model", Some(&agent), None)
        .await
        .unwrap()
        .id;
    conversation_mid_walk(repos, user_id, tenant_id, None).await;
    let code = format!("code{}", Uuid::new_v4().simple());
    repos
        .short_links
        .create_short_link(
            &code,
            "https://example.com/plan",
            &tenant,
            &user,
            Utc::now() + Duration::days(1),
        )
        .await
        .unwrap();
    repos
        .usage_counters
        .increment_counter(&tenant, &user, "chat_turns", "2026-11", 3)
        .await
        .unwrap();
    repos
        .session_refresh_tokens
        .store_token(
            &session_token(user_id),
            &SessionRefreshToken {
                family_id: Uuid::new_v4().to_string(),
                user_id,
                tenant_id: Some(tenant.clone()),
                client_id: Some("dravr-web".to_owned()),
                created_at: Utc::now(),
                expires_at: Utc::now() + Duration::days(30),
            },
        )
        .await
        .unwrap();
    repos
        .user_tier_overrides
        .upsert(&UserTierOverride {
            user_id,
            tier: UserTier::Professional,
            note: Some("pilot".to_owned()),
            set_by: None,
            set_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    (user_id, tenant_id, thread)
}

#[tokio::test]
async fn reset_wipes_everything_the_account_owns_and_keeps_the_account() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let (user_id, tenant_id, thread) = seed_lived_in_account(repos, "reset-all").await;
    let (other_id, other_tenant, _) = seed_lived_in_account(repos, "reset-bystander").await;
    let before = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert!(
        !surviving(&resources, user_id).await.is_empty(),
        "the seed must reach the wiped tables"
    );

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("reset handler");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;

    // Every provider went through the chokepoint, the coaching platform too.
    assert_one_strava_revocation(&stub.received(), ENV_CLIENT_ID, ENV_CLIENT_SECRET);
    let mut disconnected: Vec<&str> = body["data"]["disconnected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["provider"].as_str().unwrap())
        .collect();
    disconnected.sort_unstable();
    assert_eq!(disconnected, ["intervals_icu", "strava"], "{body}");
    assert_eq!(body["data"]["not_revocable"], serde_json::json!([]));
    assert_eq!(body["data"]["still_connected"], false);
    // The erasure ran in the reset itself; nothing is left queued for a
    // sweep that would reach the live account later.
    assert_eq!(body["data"]["archive_erased"]["records"], 0, "{body}");
    assert!(
        !repos
            .archive_erasures
            .due(Utc::now() + Duration::days(365), 1000)
            .await
            .unwrap()
            .iter()
            .any(|pending| pending.erased_account_id == user_id.to_string()),
        "a reset queues nothing on the deleted-account erasure outbox"
    );
    assert!(repos
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap()
        .is_empty());

    // The wipe names what it removed; nothing it wipes holds a row any more.
    let rows: BTreeMap<String, u64> = body["data"]["rows_changed"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(table, n)| (table.clone(), n.as_u64().unwrap()))
        .collect();
    for table in [
        "user_onboarding",
        "user_facts",
        "user_profiles",
        "coaching_group_members",
        "chat_conversations",
        "agents",
        "short_links",
    ] {
        assert!(rows.contains_key(table), "{table} not wiped: {body}");
    }
    assert_eq!(rows["chat_conversations"], 2, "{body}");
    assert_eq!(surviving(&resources, user_id).await, BTreeMap::new());
    assert_eq!(facts_of(repos, user_id, tenant_id).await, []);
    assert!(repos
        .chat
        .get_conversation(&thread, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .is_none());

    // The account, its sign-in, its tenants and what operators set on it stay.
    let after = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(after.email, before.email);
    assert_eq!(after.role, before.role);
    assert_eq!(after.password_hash, before.password_hash);
    assert_eq!(
        repos
            .tenants
            .list_membership_tenant_ids(user_id)
            .await
            .unwrap(),
        vec![tenant_id]
    );
    assert_eq!(
        repos
            .user_tier_overrides
            .get(user_id)
            .await
            .unwrap()
            .map(|o| o.tier),
        Some(UserTier::Professional)
    );
    assert_eq!(
        repos
            .usage_counters
            .get_counter(
                &tenant_id.to_string(),
                &user_id.to_string(),
                "chat_turns",
                "2026-11"
            )
            .await
            .unwrap()
            .value,
        3
    );

    // The account stays signed in: its app session still exchanges.
    let session = repos
        .session_refresh_tokens
        .consume_token(&session_token(user_id), Utc::now())
        .await
        .unwrap()
        .expect("the app session survives the reset");
    assert_eq!(session.user_id, user_id);

    // A reset is not a deletion: no tombstone, no receipt.
    assert!(repos
        .import_tombstones
        .find_live(
            TombstoneKind::Email,
            None,
            &email_identity_hash(&after.email)
        )
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        repos
            .account_deletion
            .list_deletion_receipts(0, 1000)
            .await
            .unwrap()
            .len(),
        0,
        "a reset chains no deletion receipt"
    );

    // Another account's data is untouched.
    assert!(!surviving(&resources, other_id).await.is_empty());
    assert_eq!(
        repos
            .user_onboarding
            .get_onboarding_steps(&other_id.to_string())
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        repos
            .provider_connections
            .get_for_user(other_id, None)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(facts_of(repos, other_id, other_tenant).await.len(), 4);
}

#[tokio::test]
async fn a_manage_users_token_cannot_reset_an_account() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-forbidden").await;

    let response = reset(
        admin_context(&resources, &stub.url),
        manage_users_token(),
        user_id,
    )
    .await
    .expect("a refusal is a response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = body_json(response).await;
    assert_eq!(body["success"], false, "{body}");

    assert!(stub.received().is_empty(), "nothing may reach Strava");
    assert_eq!(
        repos
            .user_onboarding
            .get_onboarding_steps(&user_id.to_string())
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(facts_of(repos, user_id, tenant_id).await.len(), 4);
    assert_eq!(
        repos
            .provider_connections
            .get_for_user(user_id, None)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// An active group the account owns is somebody else's room: the reset
/// refuses with 409 naming it, before any provider is disconnected.
#[tokio::test]
async fn an_active_group_the_account_owns_refuses_the_reset_and_disconnects_nothing() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-owner").await;
    seed_group(repos, user_id, tenant_id, "Sunday long run").await;

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("a refusal is a response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    let blockers = blockers_of(&body);
    assert!(
        blockers.contains(&(
            "owns_coaching_group".to_owned(),
            "Sunday long run".to_owned()
        )),
        "{body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .starts_with("User cannot be reset"),
        "{body}"
    );

    assert!(stub.received().is_empty(), "nothing may reach Strava");
    assert_eq!(
        repos
            .provider_connections
            .get_for_user(user_id, None)
            .await
            .unwrap()
            .len(),
        1,
        "the Strava connection stands"
    );
    assert_eq!(facts_of(repos, user_id, tenant_id).await.len(), 4);
}

/// A provider this build cannot revoke is kept and named rather than
/// orphaned upstream, while every other provider is disconnected and the
/// wipe still commits.
#[tokio::test]
async fn a_provider_this_build_cannot_revoke_is_kept_and_named() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let (user_id, tenant_id, _) = seed_user(repos, "reset-unrevocable").await;
    connect_strava(repos, user_id, tenant_id, None).await;
    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant_id,
            "synthetic",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            provider: "synthetic".to_owned(),
            ..strava_token(user_id, tenant_id, None)
        })
        .await
        .unwrap();

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("reset handler");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;

    assert_one_strava_revocation(&stub.received(), ENV_CLIENT_ID, ENV_CLIENT_SECRET);
    let kept = body["data"]["not_revocable"].as_array().unwrap();
    assert_eq!(kept.len(), 1, "{body}");
    assert_eq!(kept[0]["provider"], "synthetic");
    assert_eq!(body["data"]["still_connected"], true);
    let left: Vec<String> = repos
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.provider)
        .collect();
    assert_eq!(left, vec!["synthetic".to_owned()]);
    assert_eq!(surviving(&resources, user_id).await, BTreeMap::new());
}

/// A reset of an unknown account is a 404 that touches nothing.
#[tokio::test]
async fn resetting_an_unknown_account_is_not_found() {
    let resources = resources().await;
    let stub = RevokeStub::start().await;
    let error = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        Uuid::new_v4(),
    )
    .await
    .expect_err("an unknown account");
    assert_eq!(error.code, ErrorCode::ResourceNotFound);
}

/// An A2A client's one credential is its `OAuth2` registration, keyed by the
/// client's id and naming no user: the reset removes it with the client, so
/// its secret no longer obtains a token. Another user's registration stays.
#[tokio::test]
async fn reset_removes_the_oauth_registration_of_an_a2a_client_the_account_registered() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let (user_id, _, _) = seed_user(repos, "reset-a2a-owner").await;
    let (bystander_id, _, _) = seed_user(repos, "reset-a2a-bystander").await;
    let register = |owner| {
        resources.a2a.a2a_client_manager.register_client(
            ClientRegistrationRequest {
                name: "Reset test agent".to_owned(),
                description: "Registered by an account about to be reset".to_owned(),
                capabilities: vec!["fitness-data-analysis".to_owned()],
                redirect_uris: vec!["https://example.com/callback".to_owned()],
                contact_email: "owner@example.com".to_owned(),
            },
            owner,
        )
    };
    let credentials = register(user_id).await.unwrap();
    let kept = register(bystander_id).await.unwrap();

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("reset handler");
    assert_eq!(response.status(), StatusCode::OK, "the reset succeeds");
    let body = body_json(response).await;
    assert_eq!(body["data"]["rows_changed"]["oauth2_clients"], 1, "{body}");
    assert_eq!(body["data"]["rows_changed"]["a2a_clients"], 1, "{body}");
    assert!(
        repos
            .oauth2_server
            .get_client(&credentials.client_id)
            .await
            .unwrap()
            .is_none(),
        "the reset account's A2A client no longer has a secret to authenticate"
    );
    assert!(
        repos
            .oauth2_server
            .get_client(&kept.client_id)
            .await
            .unwrap()
            .is_some(),
        "another user's A2A client keeps its registration"
    );
    assert_eq!(
        surviving(&resources, user_id).await,
        BTreeMap::new(),
        "every wiped table is empty for the account"
    );
}

/// A group `owner` ran on a private agent they wrote, with an invite they
/// created and their own copy of the room, joined by `member` when given, then
/// archived the way the app's delete does. Returns the group and invite ids.
async fn seed_archived_group_on_a_private_agent(
    repos: &pierre_database::RepositoryRegistry,
    owner: Uuid,
    tenant_id: TenantId,
    member: Option<(Uuid, TenantId)>,
) -> (Uuid, Uuid) {
    let agent_id = seed_agent(
        repos,
        owner,
        tenant_id,
        "Archived squad agent",
        AgentVisibility::Private,
    )
    .await;
    let now = Utc::now();
    let group_id = repos
        .groups
        .create_group(
            tenant_id,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: tenant_id.to_string(),
                name: "Archived squad".to_owned(),
                description: None,
                agent_id: agent_id.clone(),
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
        .id;
    let invite_id = repos
        .groups
        .create_invite(&GroupInvite {
            id: Uuid::new_v4(),
            group_id,
            tenant_id: tenant_id.to_string(),
            code: Uuid::new_v4().simple().to_string()[..8].to_owned(),
            kind: GroupInviteKind::Member,
            created_by: owner,
            expires_at: None,
            max_uses: None,
            use_count: 0,
            is_active: true,
            approves_signup: false,
            created_at: now,
        })
        .await
        .unwrap()
        .id;
    repos
        .chat
        .create_conversation(
            &owner.to_string(),
            tenant_id,
            "Archived squad",
            "model",
            Some(&agent_id),
            Some(&group_id.to_string()),
        )
        .await
        .unwrap();
    if let Some((member_id, member_tenant)) = member {
        join_group(repos, group_id, member_id, member_tenant).await;
    }
    assert!(repos
        .groups
        .delete_group(&group_id.to_string(), tenant_id)
        .await
        .unwrap());
    (group_id, invite_id)
}

/// A group the account owns and archived, that nobody else joined or talks
/// in, is the account's own leftover: the reset wipes it with its invite and
/// the account's copy of the room, so the private agent bound only to it
/// blocks nothing and goes too.
#[tokio::test]
async fn an_archived_group_only_the_account_used_is_wiped_with_its_private_agent() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-archived").await;
    let (group_id, invite_id) =
        seed_archived_group_on_a_private_agent(repos, user_id, tenant_id, None).await;

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("reset handler");
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["rows_changed"]["coaching_groups"], 1, "{body}");
    assert_eq!(body["data"]["rows_changed"]["agents"], 1, "{body}");
    assert!(
        repos
            .groups
            .get_group(&group_id.to_string(), tenant_id)
            .await
            .unwrap()
            .is_none(),
        "the archived group goes with the reset"
    );
    assert!(
        repos
            .groups
            .get_invite_by_id(&invite_id.to_string())
            .await
            .unwrap()
            .is_none(),
        "its invite cascades with it"
    );
    assert_eq!(surviving(&resources, user_id).await, BTreeMap::new());
}

/// An archived group another user was a member of is their room record: the
/// reset keeps it, so the private agent it names still refuses the reset.
#[tokio::test]
async fn an_archived_group_a_former_member_joined_is_kept_and_its_agent_blocks() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-archived-kept").await;
    let (member_id, member_tenant, _) = seed_user(repos, "reset-archived-member").await;
    let (group_id, _) = seed_archived_group_on_a_private_agent(
        repos,
        user_id,
        tenant_id,
        Some((member_id, member_tenant)),
    )
    .await;

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("a refusal is a response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    assert!(
        blockers_of(&body).contains(&(
            "authored_agent".to_owned(),
            "Archived squad agent".to_owned()
        )),
        "{body}"
    );
    assert!(repos
        .groups
        .get_group(&group_id.to_string(), tenant_id)
        .await
        .unwrap()
        .is_some());
}

/// How many rows `sql` counts for `owner`: `sql` selects `COUNT(*) AS n` with
/// the account's id bound to `$1` as text.
pub(crate) async fn count_for(resources: &ServerContext, sql: &str, owner: &str) -> i64 {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => sqlx::query(sql)
            .bind(owner)
            .fetch_one(db.pool())
            .await
            .unwrap()
            .get::<i64, _>("n"),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => sqlx::query(sql)
            .bind(owner)
            .fetch_one(db.pool())
            .await
            .unwrap()
            .get::<i64, _>("n"),
    }
}

/// The account's claim verdicts, wherever they were written.
const VERDICTS_OF: &str =
    "SELECT COUNT(*) AS n FROM claim_verdicts WHERE CAST(user_id AS TEXT) = $1";

/// The ratings the account left, in any thread.
const RATINGS_OF: &str =
    "SELECT COUNT(*) AS n FROM chat_message_feedback WHERE CAST(user_id AS TEXT) = $1";

/// One assistant reply `author` posts in `conversation`; its id.
async fn post_reply(
    repos: &pierre_database::RepositoryRegistry,
    tenant_id: TenantId,
    conversation: &str,
    author: &str,
) -> String {
    repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: conversation,
            user_id: author,
            role: "assistant",
            content: "Here is your plan.",
            token_count: Some(10),
            finish_reason: Some("stop"),
            prompt_tokens: Some(5),
            model: Some("model"),
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        })
        .await
        .unwrap()
        .id
}

/// A claim verdict said to `user`, outside any conversation: what an MCP
/// client's `verify_claim` call writes when it names no thread.
async fn verdict_without_a_thread(
    repos: &pierre_database::RepositoryRegistry,
    tenant_id: TenantId,
    user: &str,
) {
    repos
        .claim_verdicts
        .insert_claim_verdict(&InsertClaimVerdictParams {
            tenant_id,
            user_id: user,
            agent_id: None,
            conversation_id: None,
            message_id: None,
            claim_text: "Your threshold pace is 4:05/km",
            category: ClaimCategory::Physiological,
            status: ClaimStatus::Contradicted,
            evidence_strength: EvidenceStrength::Strong,
            confidence: 0.9,
            layer_fired: VerdictLayer::Evidence,
            explanation: None,
            evidence_refs: None,
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await
        .unwrap();
}

/// The account's verdicts and ratings go wherever they were written: a
/// verdict with no conversation, and a rating it left on a reply in someone
/// else's thread it took part in. The other person's thread and reply stay.
#[tokio::test]
async fn reset_wipes_the_accounts_verdicts_and_ratings_outside_its_own_threads() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-verdicts").await;
    let (peer_id, _, _) = seed_user(repos, "reset-verdicts-peer").await;
    let user = user_id.to_string();
    let peer = peer_id.to_string();
    let theirs = repos
        .chat
        .create_conversation(&peer, tenant_id, "peer thread", "model", None, None)
        .await
        .unwrap()
        .id;
    repos
        .chat
        .add_participant(&theirs, tenant_id, &user, &peer)
        .await
        .unwrap();
    let reply = post_reply(repos, tenant_id, &theirs, &peer).await;
    repos
        .chat
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id,
            conversation_id: &theirs,
            message_id: &reply,
            user_id: &user,
            rating: "down",
            comment: Some("too vague"),
        })
        .await
        .unwrap();
    verdict_without_a_thread(repos, tenant_id, &user).await;
    assert_eq!(count_for(&resources, RATINGS_OF, &user).await, 1);
    assert_eq!(count_for(&resources, VERDICTS_OF, &user).await, 1);

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("reset handler");
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(
        count_for(&resources, RATINGS_OF, &user).await,
        0,
        "bug: the rating the account left in another person's thread survives the reset"
    );
    assert_eq!(
        count_for(&resources, VERDICTS_OF, &user).await,
        0,
        "bug: a verdict said to the account outside a thread survives the reset"
    );
    assert!(repos
        .chat
        .get_conversation(&theirs, &peer, tenant_id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        repos
            .chat
            .get_messages(&theirs, &peer, tenant_id)
            .await
            .unwrap()
            .len(),
        1,
        "the other person's reply stays"
    );
    assert_eq!(surviving(&resources, user_id).await, BTreeMap::new());
}

/// A thread the account shared with someone else is their history too, as an
/// onboarding reset already holds: the reset keeps it, with what the other
/// participant posted and their seat, while the account's unshared threads go.
#[tokio::test]
async fn a_thread_the_account_shared_is_kept_with_what_the_others_posted() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-shared-thread").await;
    let (peer_id, _, _) = seed_user(repos, "reset-shared-thread-peer").await;
    let user = user_id.to_string();
    let peer = peer_id.to_string();
    let own = conversation_mid_walk(repos, user_id, tenant_id, None).await;
    let shared = repos
        .chat
        .create_conversation(&user, tenant_id, "shared", "model", None, None)
        .await
        .unwrap()
        .id;
    repos
        .chat
        .add_participant(&shared, tenant_id, &peer, &user)
        .await
        .unwrap();
    post_reply(repos, tenant_id, &shared, &peer).await;

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("reset handler");
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"]["rows_changed"]["chat_conversations"], 1,
        "{body}"
    );

    assert!(
        repos
            .chat
            .get_conversation(&own, &user, tenant_id)
            .await
            .unwrap()
            .is_none(),
        "the account's own unshared thread goes"
    );
    assert!(
        repos
            .chat
            .get_conversation(&shared, &peer, tenant_id)
            .await
            .unwrap()
            .is_some(),
        "bug: the reset deleted a thread another participant posts in"
    );
    assert_eq!(
        repos
            .chat
            .get_messages(&shared, &peer, tenant_id)
            .await
            .unwrap()
            .len(),
        1,
        "what the other participant posted stays"
    );
    assert!(
        repos
            .chat
            .get_conversation(&shared, &user, tenant_id)
            .await
            .unwrap()
            .is_some(),
        "the owner keeps its seat in the thread it shared"
    );
    assert_eq!(surviving(&resources, user_id).await, BTreeMap::new());
}

/// The private agent a kept shared thread runs on is relied on by the other
/// participant, so it refuses the reset like any other shared agent.
#[tokio::test]
async fn a_private_agent_a_shared_thread_runs_on_refuses_the_reset() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-shared-agent").await;
    let (peer_id, _, _) = seed_user(repos, "reset-shared-agent-peer").await;
    let user = user_id.to_string();
    let agent = seed_agent(
        repos,
        user_id,
        tenant_id,
        "Shared thread coach",
        AgentVisibility::Private,
    )
    .await;
    let shared = repos
        .chat
        .create_conversation(&user, tenant_id, "shared", "model", Some(&agent), None)
        .await
        .unwrap()
        .id;
    repos
        .chat
        .add_participant(&shared, tenant_id, &peer_id.to_string(), &user)
        .await
        .unwrap();

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("a refusal is a response");
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        blockers_of(&body).contains(&(
            "authored_agent".to_owned(),
            "Shared thread coach".to_owned()
        )),
        "{body}"
    );
    assert!(stub.received().is_empty(), "nothing may reach Strava");
    assert!(repos
        .chat
        .get_conversation(&shared, &user, tenant_id)
        .await
        .unwrap()
        .is_some());
}

/// The blockers are read again inside the wipe's transaction, so a share made
/// after the service's first read (here the wipe runs with no first read at
/// all) refuses with nothing wiped, rather than deleting an agent other people
/// now rely on: nothing that binds an agent is a foreign key that would stop
/// the delete.
#[tokio::test]
async fn the_wipe_reads_the_blockers_again_and_wipes_nothing_over_one() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-late-share").await;
    let agent = seed_agent(
        repos,
        user_id,
        tenant_id,
        "Published late",
        AgentVisibility::Tenant,
    )
    .await;

    let committed = repos
        .account_reset
        .reset_account(user_id)
        .await
        .expect("the wipe answers");
    let AccountResetCommit::Blocked(blockers) = committed else {
        panic!("bug: the wipe deleted an agent shared after the first blocker read: {committed:?}");
    };
    assert!(
        blockers
            .iter()
            .any(|b| b.kind.as_str() == "authored_agent" && b.detail == "Published late"),
        "{blockers:?}"
    );
    assert!(repos
        .agents
        .get_by_id(&agent, user_id, tenant_id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(facts_of(repos, user_id, tenant_id).await.len(), 4);
}

/// The athlete's archive-import scope in its tenant.
fn scope_of(athlete: &Athlete) -> Scope {
    Scope::new(athlete.tenant.as_uuid(), athlete.user_id)
}

/// The reset erases what the activity store kept of the account's provider
/// reads, where imports do not run too: a source no longer connected (so no
/// disconnect purges it) would otherwise keep its records, and the next sync
/// after the athlete reconnects would serve the history they started over
/// from.
#[tokio::test]
async fn reset_erases_the_activity_stores_provider_reads_where_imports_do_not_run() {
    let resources = resources().await;
    assert!(
        !resources.common.archive_imports.is_enabled(),
        "test setup: imports do not run here"
    );
    let stub = RevokeStub::start().await;
    let repos = &resources.common.repos;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-live-store").await;
    let live = &resources.common.live_activities;
    let ride = ActivityBuilder::new(
        "g-7001",
        "Morning ride",
        SportType::Ride,
        Utc::now() - Duration::days(3),
        3_600,
        "garmin",
    )
    .build();
    let kept = live
        .ingest(user_id, tenant_id, "garmin", &[ride], Utc::now())
        .await
        .unwrap();
    assert_eq!(kept.stored, 1, "test setup: the store holds the read");

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        user_id,
    )
    .await
    .expect("reset handler");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "bug: the reset was refused"
    );
    let body = body_json(response).await;

    assert_eq!(
        live.held(user_id, tenant_id, "garmin").await.unwrap(),
        0,
        "bug: the reset left the account's provider reads in the activity store: {body}"
    );
    assert_eq!(body["data"]["archive_erased"]["records"], 1, "{body}");
}

/// A ride file `athlete` uploads; the key its bytes are kept under.
async fn upload_ride(resources: &ServerContext, athlete: &Athlete, days_ago: i64) -> BlobKey {
    let upload = FitWorkout::ride(Utc::now() - Duration::days(days_ago), 600).encode();
    let sha = file_sha256(&upload);
    resources
        .common
        .originals
        .put_upload(&athlete.tenant, athlete.user_id, &sha, Bytes::from(upload))
        .await
        .expect("the upload is kept");
    BlobKey::upload(scope_of(athlete), Sha256::parse_hex(&sha).expect("a hash"))
}

/// The reset erases the imported history and uploads itself, before its wipe,
/// and queues nothing: what the athlete uploads after the reset is theirs and
/// survives every erasure pass, where a queued erasure would have deleted its
/// bytes under the row that names them.
#[tokio::test]
async fn reset_erases_the_imported_history_at_once_and_spares_what_comes_after() {
    let (imports, stores) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let stub = RevokeStub::start().await;
    let athlete = seed_athlete(&resources, "reset-archive").await;
    import_export(&resources, &athlete).await;
    let before = upload_ride(&resources, &athlete, 10).await;

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        athlete.user_id,
    )
    .await
    .expect("reset handler");
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["data"]["archive_erased"]["records"].as_u64().unwrap() > 0,
        "{body}"
    );
    let records = stores
        .records
        .list(&scope_of(&athlete), &RecordQuery::all(PageReq::first(50)))
        .await
        .expect("barrage reads");
    assert!(
        records.items.is_empty(),
        "the imported records are gone with the reset"
    );
    assert!(
        stores
            .blobs
            .get(&before)
            .await
            .expect("blobs read")
            .is_none(),
        "the file uploaded before the reset is gone with it"
    );

    let after = upload_ride(&resources, &athlete, 3).await;
    let erased = erase_deleted_accounts(&resources)
        .await
        .expect("the sweep runs");
    assert_eq!(
        erased, 0,
        "bug: the reset left the live account queued for erasure"
    );
    assert!(
        stores
            .blobs
            .get(&after)
            .await
            .expect("blobs read")
            .is_some(),
        "bug: the file uploaded after the reset was erased"
    );
}

/// An import still running at the reset is ended by it: a later step finds no
/// job to project into the wiped tables, and the source takes a fresh import
/// at once instead of refusing it as already in progress.
#[tokio::test]
async fn reset_ends_an_import_still_running() {
    let (imports, _) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let stub = RevokeStub::start().await;
    let athlete = seed_athlete(&resources, "reset-running-import").await;
    let (started, waiting) = call(
        &resources,
        "POST",
        "/api/imports/strava_archive",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(started, StatusCode::CREATED, "{waiting}");
    assert_eq!(waiting["status"], "awaiting_link", "{waiting}");

    let response = reset(
        admin_context(&resources, &stub.url),
        super_admin_token(),
        athlete.user_id,
    )
    .await
    .expect("reset handler");
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["archive_erased"]["jobs"], 1, "{body}");

    let (again, fresh) = call(
        &resources,
        "POST",
        "/api/imports/strava_archive",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(
        again,
        StatusCode::CREATED,
        "bug: the import running at the reset still holds the source: {fresh}"
    );
}

/// The user read names what would refuse a reset, so `pierre-cli user reset`
/// can say so before `--yes` is typed rather than promise a wipe the server
/// will refuse; an account with nothing shared names none.
#[tokio::test]
async fn the_user_read_names_what_would_refuse_a_reset() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let (user_id, tenant_id) = seed_onboarded_user(repos, "reset-read-blockers").await;
    let (free_id, _) = seed_onboarded_user(repos, "reset-read-free").await;
    seed_group(repos, user_id, tenant_id, "Track Tuesdays").await;

    let read = |id: Uuid| {
        let context = admin_context(&resources, &stub.url);
        async move {
            let response = handle_get_user(
                State(context),
                Extension(manage_users_token()),
                Path(id.to_string()),
            )
            .await
            .expect("get user handler");
            body_json(response.into_response()).await
        }
    };
    let body = read(user_id).await;
    assert_eq!(
        body["data"]["reset_blockers"],
        serde_json::json!([
            { "kind": "authored_agent", "detail": "Track Tuesdays coach" },
            { "kind": "owns_coaching_group", "detail": "Track Tuesdays" },
        ]),
        "{body}"
    );
    let free = read(free_id).await;
    assert_eq!(
        free["data"]["reset_blockers"],
        serde_json::json!([]),
        "{free}"
    );
}
