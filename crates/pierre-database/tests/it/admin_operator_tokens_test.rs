// ABOUTME: Covers what becomes of the admin tokens that act as one admin, against whichever backend DATABASE_URL names
// ABOUTME: Retired by the write that ends admin standing, kept and unnamed by the account deletion, rotated in one transaction
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A device login mints an admin token whose row names the admin who approved
//! it (`admin_tokens.operator_user_id`). When that admin is demoted or
//! suspended, every such token is deactivated in one statement. The column is
//! text on both engines and has no foreign key, so the statement is keyed by
//! the id as a string.
//!
//! The write that demotes or deactivates the account runs that statement in
//! its own transaction, with the one that ends the impersonations the
//! account opened; the write that gives an account the admin role ends the
//! impersonations open on it. The account deletion keeps the rows, in its
//! own transaction: inactive, under a name that names no one and an operator
//! that is no account, with their request log and their record of the keys
//! they provisioned. It renames a token that carries the id only in its
//! name: no `admin_tokens` row names a deleted account.
//!
//! A rotation retires a token and mints its replacement in one transaction,
//! from the row it claims: a token retired since its caller read it has no
//! replacement, and a rotation racing the deletion of the admin a device
//! login acts as leaves no row naming that admin, whichever commits first.
//!
//! These run on `SQLite` and on `PostgreSQL`: `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::thread;
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::{
    AdminAction, AdminPermission, AdminPermissions, AdminToken, AdminTokenUsage,
    CreateAdminTokenRequest, DELETED_ACCOUNT_LABEL, DELETED_ACCOUNT_OPERATOR,
    DEVICE_CLI_DELETED_ACCOUNT_SERVICE, DEVICE_CLI_SERVICE_PREFIX,
};
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_core::models::{DeviceAuthorization, User, UserStatus};
use pierre_core::permissions::impersonation::ImpersonationSession;
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::{Database, DatabaseBackend};
#[cfg(feature = "postgresql")]
use pierre_database::repositories::POSTGRES_USER_PURGE;
use pierre_database::repositories::{
    AccountDeletionPlan, DeletionActor, TokenRotation, SQLITE_USER_PURGE,
};
use pierre_database::seed_models::SeedDemoUser;
use pierre_database::RepositoryRegistry;
use pierre_test_support::account::delete_account;
use pierre_test_support::db::{create_concurrent_test_db, create_test_db};
use pierre_test_support::server::{create_test_admin_user, create_test_user};
use serde_json::Value;
use tokio::{task, time};
use uuid::Uuid;

/// A signer that only has to hand the repository a string to hash: these
/// tests never validate the token, they read its row.
struct Unsigned;

impl JwtSigner for Unsigned {
    fn sign_token(&self, claims: &Value) -> AppResult<String> {
        Ok(format!("unsigned.{claims}"))
    }
}

/// Mint a super-admin token as the device grant does, acting as `operator`,
/// or as `pierre-cli token generate` does when there is none; its id.
async fn mint(repos: &RepositoryRegistry, name: &str, operator: Option<Uuid>) -> String {
    let mut request = CreateAdminTokenRequest::super_admin(name.to_owned());
    request.operator_user_id = operator;
    repos
        .admin
        .create_token(&request, JWT_SECRET, &Unsigned)
        .await
        .unwrap()
        .token_id
}

/// The ids of the tokens still live.
async fn live(repos: &RepositoryRegistry) -> Vec<String> {
    let mut ids: Vec<String> = repos
        .admin
        .list_tokens(false)
        .await
        .unwrap()
        .into_iter()
        .map(|token| token.id)
        .collect();
    ids.sort();
    ids
}

/// Every token an admin approved is retired by the write that ends its
/// standing, together and by id; another admin's token and a service token
/// are not, and a repeat finds nothing left to retire.
#[tokio::test]
async fn an_admins_tokens_are_deactivated_together_and_no_one_elses() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let leaving = account(repos, "leaving@example.com", true).await;
    let staying = account(repos, "staying@example.com", true).await;
    let passing = account(repos, "passing@example.com", true).await;
    let first = mint(repos, "device-cli:first", Some(leaving.id)).await;
    let second = mint(repos, "device-cli:second", Some(leaving.id)).await;
    let other = mint(repos, "device-cli:other", Some(staying.id)).await;
    let service = mint(repos, "nightly-job", None).await;

    let demote = |id: Uuid| async move {
        repos
            .users
            .set_admin_status(id, false)
            .await
            .unwrap()
            .withdrawn
            .device_tokens
    };
    let mut retired = demote(leaving.id).await;
    retired.sort();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(retired, expected, "both of the admin's tokens, by id");

    let mut kept = vec![other, service];
    kept.sort();
    assert_eq!(
        live(repos).await,
        kept,
        "another admin's token and a service token stay live"
    );

    let again = demote(leaving.id).await;
    assert!(
        again.is_empty(),
        "only live tokens are retired, so a repeat reports nothing: {again:?}"
    );
    assert!(
        demote(passing.id).await.is_empty(),
        "an account that approved no login has nothing to retire"
    );
}

/// A stored account: an admin when `admin`, a plain user otherwise.
async fn account(repos: &RepositoryRegistry, email: &str, admin: bool) -> User {
    let user = if admin {
        create_test_admin_user(email, None)
    } else {
        create_test_user(email, None)
    };
    repos.users.create(&user).await.unwrap();
    user
}

/// Open an impersonation of `target` by `impersonator`; the session's id.
async fn open_session(repos: &RepositoryRegistry, impersonator: &User, target: &User) -> String {
    let session = ImpersonationSession::new(impersonator.id, target.id, None);
    repos.impersonation.create_session(&session).await.unwrap();
    session.id
}

/// Whether the session `id` is still open.
async fn is_open(repos: &RepositoryRegistry, id: &str) -> bool {
    repos
        .impersonation
        .get_session(id)
        .await
        .unwrap()
        .expect("the session row")
        .is_active
}

/// Each of the three user writes that can take an admin out of standing
/// deactivates its device logins and ends its impersonations itself, and
/// says what it withdrew. Another admin's are untouched, and so is a service
/// token.
#[tokio::test]
async fn the_write_that_ends_admin_standing_withdraws_what_the_account_held() {
    for how in ["demote", "suspend", "rewrite"] {
        let db = create_test_db().await.unwrap();
        let repos = db.repositories();
        let leaving = account(repos, "leaving@example.com", true).await;
        let staying = account(repos, "staying@example.com", true).await;
        let athlete = account(repos, "athlete@example.com", false).await;
        let other_athlete = account(repos, "other-athlete@example.com", false).await;
        let held = mint(repos, "device-cli:leaving", Some(leaving.id)).await;
        let kept = mint(repos, "device-cli:staying", Some(staying.id)).await;
        let service = mint(repos, "nightly-job", None).await;
        let ended = open_session(repos, &leaving, &athlete).await;
        let open = open_session(repos, &staying, &other_athlete).await;

        let withdrawn = match how {
            "demote" => {
                let write = repos
                    .users
                    .set_admin_status(leaving.id, false)
                    .await
                    .unwrap();
                assert!(!write.user.role.is_admin(), "{how}");
                write.withdrawn
            }
            "suspend" => {
                let write = repos
                    .users
                    .update_status(leaving.id, UserStatus::Suspended, None)
                    .await
                    .unwrap();
                assert_eq!(write.user.user_status, UserStatus::Suspended, "{how}");
                write.withdrawn
            }
            _ => {
                let mut rewritten = leaving.clone();
                rewritten.is_active = false;
                repos.users.update(&rewritten).await.unwrap()
            }
        };

        assert_eq!(withdrawn.device_tokens, vec![held], "{how}");
        assert_eq!(withdrawn.impersonations, 1, "{how}");
        assert!(!is_open(repos, &ended).await, "{how}");
        assert!(
            is_open(repos, &open).await,
            "{how}: another admin's session"
        );
        let mut live_tokens = vec![kept, service];
        live_tokens.sort();
        assert_eq!(live(repos).await, live_tokens, "{how}");
    }
}

/// A refused write withdraws nothing: the transaction that would have taken
/// the last active admin out of standing is the one the withdrawal runs in.
/// A write that keeps the account in standing withdraws nothing either.
#[tokio::test]
async fn a_refused_or_harmless_write_withdraws_nothing() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let last = account(repos, "last@example.com", true).await;
    let athlete = account(repos, "athlete@example.com", false).await;
    let held = mint(repos, "device-cli:last", Some(last.id)).await;
    let session = open_session(repos, &last, &athlete).await;

    assert!(repos.users.set_admin_status(last.id, false).await.is_err());
    assert!(repos
        .users
        .update_status(last.id, UserStatus::Suspended, None)
        .await
        .is_err());
    let mut deactivated = last.clone();
    deactivated.is_active = false;
    assert!(repos.users.update(&deactivated).await.is_err());

    let mut renamed = last.clone();
    renamed.display_name = Some("Still the admin".to_owned());
    assert!(repos.users.update(&renamed).await.unwrap().is_empty());
    let approved = repos
        .users
        .update_status(last.id, UserStatus::Active, None)
        .await
        .unwrap();
    assert!(approved.withdrawn.is_empty());

    assert_eq!(live(repos).await, vec![held]);
    assert!(is_open(repos, &session).await);
}

/// An admin is never impersonated. The write that gives an account the admin
/// role ends the sessions open on it, and only those.
#[tokio::test]
async fn the_write_that_grants_the_role_ends_the_impersonations_of_the_account() {
    for how in ["promote", "rewrite"] {
        let db = create_test_db().await.unwrap();
        let repos = db.repositories();
        let acting = account(repos, "acting@example.com", true).await;
        let rising = account(repos, "rising@example.com", false).await;
        let bystander = account(repos, "bystander@example.com", false).await;
        let on_rising = open_session(repos, &acting, &rising).await;
        let on_bystander = open_session(repos, &acting, &bystander).await;

        if how == "promote" {
            let write = repos.users.set_admin_status(rising.id, true).await.unwrap();
            assert!(write.user.role.is_admin());
            assert!(write.withdrawn.is_empty());
        } else {
            let mut promoted = rising.clone();
            promoted.role = UserRole::Admin;
            repos.users.update(&promoted).await.unwrap();
        }

        assert!(!is_open(repos, &on_rising).await, "{how}");
        assert!(is_open(repos, &on_bystander).await, "{how}");
    }
}

/// How many rows of the three admin-token tables carry `needle` in any text
/// column, read past the repository.
const ROWS_NAMING_SQL: &str = "SELECT \
     (SELECT COUNT(*) FROM admin_tokens WHERE \
        CAST(id AS TEXT) LIKE $1 OR service_name LIKE $1 \
        OR COALESCE(service_description, '') LIKE $1 OR token_hash LIKE $1 \
        OR token_prefix LIKE $1 OR jwt_secret_hash LIKE $1 \
        OR CAST(permissions AS TEXT) LIKE $1 OR COALESCE(tenant_id, '') LIKE $1 \
        OR COALESCE(operator_user_id, '') LIKE $1 \
        OR COALESCE(CAST(last_used_ip AS TEXT), '') LIKE $1) \
   + (SELECT COUNT(*) FROM admin_provisioned_keys WHERE \
        admin_token_id LIKE $1 OR api_key_id LIKE $1 OR user_email LIKE $1 \
        OR provisioned_by_service LIKE $1) \
   + (SELECT COUNT(*) FROM admin_token_usage WHERE \
        admin_token_id LIKE $1 OR COALESCE(target_resource, '') LIKE $1 \
        OR COALESCE(user_agent, '') LIKE $1)";

/// The name every key provisioned under `token_id` was recorded with.
const PROVISIONED_BY_SQL: &str =
    "SELECT provisioned_by_service FROM admin_provisioned_keys WHERE admin_token_id = $1";

async fn rows_naming(db: &Database, id: Uuid) -> i64 {
    let needle = format!("%{id}%");
    match db.backend() {
        DatabaseBackend::SQLite(backend) => sqlx::query_scalar(ROWS_NAMING_SQL)
            .bind(needle)
            .fetch_one(backend.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => sqlx::query_scalar(ROWS_NAMING_SQL)
            .bind(needle)
            .fetch_one(backend.pool())
            .await
            .unwrap(),
    }
}

async fn provisioned_by(db: &Database, token_id: &str) -> Vec<String> {
    match db.backend() {
        DatabaseBackend::SQLite(backend) => sqlx::query_scalar(PROVISIONED_BY_SQL)
            .bind(token_id)
            .fetch_all(backend.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => sqlx::query_scalar(PROVISIONED_BY_SQL)
            .bind(token_id)
            .fetch_all(backend.pool())
            .await
            .unwrap(),
    }
}

/// How many requests the log holds for `token_id`.
const REQUESTS_LOGGED_SQL: &str =
    "SELECT COUNT(*) FROM admin_token_usage WHERE admin_token_id = $1";

async fn requests_logged(db: &Database, token_id: &str) -> i64 {
    match db.backend() {
        DatabaseBackend::SQLite(backend) => sqlx::query_scalar(REQUESTS_LOGGED_SQL)
            .bind(token_id)
            .fetch_one(backend.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => sqlx::query_scalar(REQUESTS_LOGGED_SQL)
            .bind(token_id)
            .fetch_one(backend.pool())
            .await
            .unwrap(),
    }
}

/// The tables the account delete's survivor read finds a row naming `id`
/// in: the statement the delete runs before it commits, run here on its own.
async fn tables_naming(db: &Database, id: Uuid) -> Vec<String> {
    let rows: Vec<(String, i64)> = match db.backend() {
        DatabaseBackend::SQLite(backend) => sqlx::query_as(SQLITE_USER_PURGE.surviving)
            .bind(id.to_string())
            .fetch_all(backend.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => sqlx::query_as(POSTGRES_USER_PURGE.surviving)
            .bind(id.to_string())
            .fetch_all(backend.pool())
            .await
            .unwrap(),
    };
    rows.into_iter().map(|(table, _)| table).collect()
}

/// The address a token was last used from, in these tests (TEST-NET-3).
const LAST_USED_FROM: &str = "203.0.113.7";

/// Log one whole-provider purge made with `token_id`, as the route does.
async fn log_a_purge(repos: &RepositoryRegistry, token_id: &str) {
    repos
        .admin
        .record_token_usage(&AdminTokenUsage {
            id: None,
            admin_token_id: token_id.to_owned(),
            timestamp: Utc::now(),
            action: AdminAction::PurgeProviderData,
            target_resource: Some("provider:whoop".to_owned()),
            ip_address: None,
            user_agent: None,
            request_size_bytes: None,
            success: true,
            error_message: None,
            response_time_ms: None,
        })
        .await
        .unwrap();
}

/// The stored token `id`, revoked ones included.
async fn token(repos: &RepositoryRegistry, id: &str) -> Option<AdminToken> {
    repos.admin.get_token_by_id(id).await.unwrap()
}

/// Log one request made with `token_id` from [`LAST_USED_FROM`] by
/// [`CLI_USER_AGENT`], as the admin API's middleware records one.
async fn log_a_request_with_its_caller(repos: &RepositoryRegistry, token_id: &str) {
    repos
        .admin
        .record_token_usage(&AdminTokenUsage {
            id: None,
            admin_token_id: token_id.to_owned(),
            timestamp: Utc::now(),
            action: AdminAction::PurgeProviderData,
            target_resource: Some("provider:whoop".to_owned()),
            ip_address: Some(LAST_USED_FROM.to_owned()),
            user_agent: Some(CLI_USER_AGENT.to_owned()),
            request_size_bytes: Some(128),
            success: true,
            error_message: None,
            response_time_ms: Some(12),
        })
        .await
        .unwrap();
}

/// The user agent a logged request came from, in these tests.
const CLI_USER_AGENT: &str = "pierre-cli/0.3.0 (darwin; sentinel-agent-1589)";

/// Every request the log holds for `token_id`, as the repository reads it.
async fn requests_of(repos: &RepositoryRegistry, token_id: &str) -> Vec<AdminTokenUsage> {
    repos
        .admin
        .get_token_usage_history(
            token_id,
            Utc::now() - TimeDelta::hours(1),
            Utc::now() + TimeDelta::hours(1),
        )
        .await
        .unwrap()
}

/// A request that was authenticated just before its admin's account was
/// deleted still writes its log row and its use afterwards (carnet#1589).
/// Neither write puts the caller back under the retired login: the log row
/// says what the request did and when, without the address or the user
/// agent, and the use is not counted, so the address the deletion cleared
/// stays cleared.
///
/// Only the deletion clears a caller: the address is the deleted admin's
/// own. A service token revoked under a request in flight, and the login
/// withdrawn from an admin who was demoted and whose account remains, keep
/// the address, the user agent and the use of that request, as a live token
/// does. When a token is revoked because it leaked, that request's address
/// is the one an admin looks for.
#[tokio::test]
async fn only_a_deleted_admins_login_records_no_caller_for_a_request_in_flight() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let leaving = account(repos, "leaving@example.com", true).await;
    let demoted = account(repos, "demoted@example.com", true).await;
    account(repos, "staying@example.com", true).await;
    let label = format!("{DEVICE_CLI_SERVICE_PREFIX}{}", leaving.id);
    let demoted_label = format!("{DEVICE_CLI_SERVICE_PREFIX}{}", demoted.id);
    let retired_login = mint(repos, &label, Some(leaving.id)).await;
    let withdrawn_login = mint(repos, &demoted_label, Some(demoted.id)).await;
    let revoked = mint(repos, "revoked-job", None).await;
    let live = mint(repos, "nightly-job", None).await;
    delete_account(repos, leaving.id, None).await.unwrap();
    repos
        .users
        .set_admin_status(demoted.id, false)
        .await
        .unwrap();
    repos.admin.deactivate_token(&revoked).await.unwrap();
    for retired in [&retired_login, &withdrawn_login, &revoked] {
        let row = token(repos, retired).await.expect("the row stays");
        assert!(!row.is_active, "retired before the request is logged");
    }

    for token_id in [&retired_login, &withdrawn_login, &revoked, &live] {
        log_a_request_with_its_caller(repos, token_id).await;
        repos
            .admin
            .record_token_use(token_id, Utc::now(), Some(LAST_USED_FROM))
            .await
            .unwrap();
    }

    let requests = requests_of(repos, &retired_login).await;
    assert_eq!(requests.len(), 1, "the request is still logged");
    assert_eq!(requests[0].ip_address, None, "without its address");
    assert_eq!(requests[0].user_agent, None, "or its user agent");
    assert_eq!(requests[0].action, AdminAction::PurgeProviderData);
    assert_eq!(
        requests[0].target_resource.as_deref(),
        Some("provider:whoop")
    );
    let row = token(repos, &retired_login).await.expect("the row stays");
    assert_eq!(row.last_used_ip, None, "no address returns to the row");
    assert_eq!(row.usage_count, 0, "a deleted admin's login counts no use");

    for kept in [&withdrawn_login, &revoked, &live] {
        let requests = requests_of(repos, kept).await;
        assert_eq!(requests.len(), 1, "{kept}");
        assert_eq!(
            requests[0].ip_address.as_deref(),
            Some(LAST_USED_FROM),
            "the request keeps its address: {kept}"
        );
        assert_eq!(
            requests[0].user_agent.as_deref(),
            Some(CLI_USER_AGENT),
            "and its user agent: {kept}"
        );
        let row = token(repos, kept).await.expect("the row");
        assert_eq!(row.last_used_ip.as_deref(), Some(LAST_USED_FROM), "{kept}");
        assert_eq!(row.usage_count, 1, "the use is counted: {kept}");
    }
}

/// The request log a deleted admin's device logins keep says what each
/// request did, to what and when, and no longer where it came from
/// (carnet#1589): the deletion sets the IP address and the user agent of
/// every row under the account's logins, the live one and the one already
/// withdrawn, to `NULL` in its own transaction. Each row stays, with its
/// action, target, time and outcome. Another admin's log, and a service
/// token's, keep both.
#[tokio::test]
async fn deleting_an_account_takes_the_caller_out_of_its_kept_request_log() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let leaving = account(repos, "leaving@example.com", true).await;
    let staying = account(repos, "staying@example.com", true).await;
    let label = format!("{DEVICE_CLI_SERVICE_PREFIX}{}", leaving.id);
    let staying_label = format!("{DEVICE_CLI_SERVICE_PREFIX}{}", staying.id);

    let live_login = mint(repos, &label, Some(leaving.id)).await;
    let withdrawn_login = mint(repos, &label, Some(leaving.id)).await;
    let kept = mint(repos, &staying_label, Some(staying.id)).await;
    let service = mint(repos, "nightly-job", None).await;
    for token_id in [&live_login, &withdrawn_login, &kept, &service] {
        log_a_request_with_its_caller(repos, token_id).await;
        log_a_request_with_its_caller(repos, token_id).await;
        for request in requests_of(repos, token_id).await {
            assert_eq!(request.ip_address.as_deref(), Some(LAST_USED_FROM));
            assert_eq!(request.user_agent.as_deref(), Some(CLI_USER_AGENT));
        }
    }
    repos
        .admin
        .deactivate_token(&withdrawn_login)
        .await
        .unwrap();
    let before = requests_of(repos, &live_login).await;

    repos
        .account_deletion
        .delete_account(&AccountDeletionPlan {
            user_id: leaving.id,
            actor: DeletionActor::Operator,
            firebase_project: None,
            backends: Vec::new(),
            revocations: "[]".to_owned(),
            not_deleted: Vec::new(),
        })
        .await
        .unwrap();

    for retired in [&live_login, &withdrawn_login] {
        let requests = requests_of(repos, retired).await;
        assert_eq!(requests.len(), 2, "every row stays: {retired}");
        for request in &requests {
            assert_eq!(
                request.ip_address, None,
                "the address goes with the account: {retired}"
            );
            assert_eq!(request.user_agent, None, "and the user agent: {retired}");
            assert_eq!(request.action, AdminAction::PurgeProviderData);
            assert_eq!(request.target_resource.as_deref(), Some("provider:whoop"));
            assert!(request.success);
            assert_eq!(request.request_size_bytes, Some(128));
            assert_eq!(request.response_time_ms, Some(12));
        }
    }
    let after = requests_of(repos, &live_login).await;
    assert_eq!(
        after
            .iter()
            .map(|r| (r.id, r.timestamp))
            .collect::<Vec<_>>(),
        before
            .iter()
            .map(|r| (r.id, r.timestamp))
            .collect::<Vec<_>>(),
        "the same rows, at the times they were logged"
    );

    // The control: nobody else's log lost anything.
    for untouched in [&kept, &service] {
        let requests = requests_of(repos, untouched).await;
        assert_eq!(requests.len(), 2);
        for request in &requests {
            assert_eq!(request.ip_address.as_deref(), Some(LAST_USED_FROM));
            assert_eq!(request.user_agent.as_deref(), Some(CLI_USER_AGENT));
        }
    }
}

/// Open a `pierre-cli` login under `user_code`, approved by `approver` when
/// one is given; its device code hash.
async fn device_login_waiting(
    repos: &RepositoryRegistry,
    user_code: &str,
    approver: Option<Uuid>,
) -> String {
    let now = Utc::now().timestamp();
    let da = DeviceAuthorization {
        device_code_hash: format!("hash-{}", Uuid::new_v4()),
        user_code: user_code.to_owned(),
        status: "pending".to_owned(),
        approved_by: None,
        created_at: now,
        expires_at: now + 600,
    };
    repos
        .oauth2_server
        .create_device_authorization(&da)
        .await
        .unwrap();
    if let Some(approver) = approver {
        assert!(repos
            .oauth2_server
            .approve_device_authorization(user_code, &approver.to_string())
            .await
            .unwrap());
    }
    da.device_code_hash
}

/// Whether the device authorization `device_code_hash` is still stored.
async fn device_login_is_waiting(repos: &RepositoryRegistry, device_code_hash: &str) -> bool {
    repos
        .oauth2_server
        .get_device_authorization_by_code_hash(device_code_hash)
        .await
        .unwrap()
        .is_some()
}

/// A `pierre-cli` login an admin approved and nobody collected yet names
/// that admin (`device_authorization.approved_by`), and would mint a token
/// in its name. Deleting the account removes it, in the deletion's own
/// transaction, and the survivor read the deletion commits on covers the
/// table (carnet#1589). A login still pending and one another admin
/// approved are not the account's, and stay.
#[tokio::test]
async fn deleting_an_account_removes_the_device_logins_it_approved_and_no_one_elses() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let leaving = account(repos, "leaving@example.com", true).await;
    let staying = account(repos, "staying@example.com", true).await;
    let approved_by_leaving = device_login_waiting(repos, "UC-LEAVING", Some(leaving.id)).await;
    let approved_by_staying = device_login_waiting(repos, "UC-STAYING", Some(staying.id)).await;
    let pending = device_login_waiting(repos, "UC-PENDING", None).await;
    assert!(
        tables_naming(&db, leaving.id)
            .await
            .iter()
            .any(|table| table == "device_authorization"),
        "the survivor read finds the approval before the delete"
    );

    let deletion = repos
        .account_deletion
        .delete_account(&AccountDeletionPlan {
            user_id: leaving.id,
            actor: DeletionActor::Operator,
            firebase_project: None,
            backends: Vec::new(),
            revocations: "[]".to_owned(),
            not_deleted: Vec::new(),
        })
        .await
        .unwrap();

    assert_eq!(
        deletion.removed_from("device_authorization"),
        1,
        "the receipt counts the approval removed"
    );
    assert!(
        !device_login_is_waiting(repos, &approved_by_leaving).await,
        "nothing is left to mint a token in the deleted admin's name"
    );
    assert_eq!(tables_naming(&db, leaving.id).await, Vec::<String>::new());
    assert!(device_login_is_waiting(repos, &approved_by_staying).await);
    assert!(device_login_is_waiting(repos, &pending).await);
}

/// Deleting an account ends its device logins, in the deletion's own
/// transaction, and keeps their record: the live one and the one already
/// withdrawn both stay as rows, inactive, under the name that names no one
/// and an operator that is no account, never `NULL` (a token without an
/// operator is a service credential). The record of the keys they
/// provisioned and the log of the requests they made, a provider-data purge
/// among them, stay under those rows. A token that carries the id only in
/// its name (a replacement someone else rotated one into) is another
/// holder's credential: it keeps its state and loses the name. Nothing in
/// the three admin-token tables names the account afterwards, and another
/// admin's token is as it was.
#[tokio::test]
async fn deleting_an_account_leaves_no_token_row_that_names_it() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let leaving = account(repos, "leaving@example.com", true).await;
    let staying = account(repos, "staying@example.com", true).await;
    let label = format!("{DEVICE_CLI_SERVICE_PREFIX}{}", leaving.id);
    let staying_label = format!("{DEVICE_CLI_SERVICE_PREFIX}{}", staying.id);

    let live_login = mint(repos, &label, Some(leaving.id)).await;
    let withdrawn_login = mint(repos, &label, Some(leaving.id)).await;
    let rotated_away = mint(repos, &label, None).await;
    let rotated_away_revoked = mint(repos, &label, None).await;
    repos
        .admin
        .deactivate_token(&rotated_away_revoked)
        .await
        .unwrap();
    let kept = mint(repos, &staying_label, Some(staying.id)).await;
    let service = mint(repos, "nightly-job", None).await;
    for (token_id, key) in [
        (&live_login, "key-of-the-deleted"),
        (&rotated_away, "key-of-the-rotated"),
        (&kept, "key-of-the-staying"),
    ] {
        repos
            .admin
            .record_provisioned_key(token_id, key, "athlete@example.com", "starter", 100, "hour")
            .await
            .unwrap();
    }
    for token_id in [&live_login, &withdrawn_login, &kept] {
        log_a_purge(repos, token_id).await;
        assert_eq!(requests_logged(&db, token_id).await, 1);
        // The address the admin last used the login from, on its row.
        repos
            .admin
            .record_token_use(token_id, Utc::now(), Some(LAST_USED_FROM))
            .await
            .unwrap();
        let row = token(repos, token_id).await.expect("the row");
        assert_eq!(row.last_used_ip.as_deref(), Some(LAST_USED_FROM));
    }
    // Withdrawn after its last use: only a live token records one.
    repos
        .admin
        .deactivate_token(&withdrawn_login)
        .await
        .unwrap();
    assert!(rows_naming(&db, leaving.id).await >= 5, "the id is there");
    // The survivor read the delete commits on covers both kept tables: it
    // finds the id in them now, and would refuse a delete that left it.
    let naming = tables_naming(&db, leaving.id).await;
    for table in ["admin_tokens", "admin_provisioned_keys"] {
        assert!(naming.iter().any(|found| found == table), "{naming:?}");
    }

    let deletion = repos
        .account_deletion
        .delete_account(&AccountDeletionPlan {
            user_id: leaving.id,
            actor: DeletionActor::Operator,
            firebase_project: None,
            backends: Vec::new(),
            revocations: "[]".to_owned(),
            not_deleted: Vec::new(),
        })
        .await
        .unwrap();

    assert_eq!(
        deletion.device_logins_revoked,
        vec![live_login.clone()],
        "the login still live is the one this deletion revoked"
    );
    assert_eq!(
        deletion.removed_from("admin_tokens"),
        0,
        "the rows are kept, not removed"
    );
    assert_eq!(
        rows_naming(&db, leaving.id).await,
        0,
        "no row names the deleted account"
    );
    assert_eq!(tables_naming(&db, leaving.id).await, Vec::<String>::new());
    // The account's own logins: the rows stay and name no one.
    for retired in [&live_login, &withdrawn_login] {
        let row = token(repos, retired).await.expect("the row stays");
        assert!(!row.is_active, "{retired}");
        assert_eq!(row.service_name, DEVICE_CLI_DELETED_ACCOUNT_SERVICE);
        assert_eq!(
            row.operator_user_id,
            Some(DELETED_ACCOUNT_OPERATOR),
            "an operator that is no account, never none: {retired}"
        );
        assert_eq!(
            row.last_used_ip, None,
            "the row keeps no address its admin used it from: {retired}"
        );
        assert_eq!(
            requests_logged(&db, retired).await,
            1,
            "the request log stays under the row: {retired}"
        );
    }
    assert_eq!(
        provisioned_by(&db, &live_login).await,
        vec![DEVICE_CLI_DELETED_ACCOUNT_SERVICE.to_owned()],
        "the provisioned-keys record stays, renamed"
    );
    assert!(
        repos
            .users
            .get_global(DELETED_ACCOUNT_OPERATOR)
            .await
            .unwrap()
            .is_none(),
        "the operator they are left with is no account"
    );

    // The replacements someone else holds: renamed, otherwise as they were.
    for (id, active) in [(&rotated_away, true), (&rotated_away_revoked, false)] {
        let row = token(repos, id).await.expect("the row stays");
        assert_eq!(row.service_name, DEVICE_CLI_DELETED_ACCOUNT_SERVICE);
        assert_eq!(row.operator_user_id, None);
        assert_eq!(row.is_active, active, "its state is not the deletion's");
    }
    assert_eq!(
        provisioned_by(&db, &rotated_away).await,
        vec![DEVICE_CLI_DELETED_ACCOUNT_SERVICE.to_owned()]
    );

    // The control: another admin's login, and a service token.
    let row = token(repos, &kept)
        .await
        .expect("the staying admin's token");
    assert_eq!(row.service_name, staying_label);
    assert_eq!(row.operator_user_id, Some(staying.id));
    assert_eq!(row.last_used_ip.as_deref(), Some(LAST_USED_FROM));
    assert!(row.is_active);
    assert_eq!(provisioned_by(&db, &kept).await, vec![staying_label]);
    assert_eq!(requests_logged(&db, &kept).await, 1);
    let mut live_tokens = vec![rotated_away, kept, service];
    live_tokens.sort();
    assert_eq!(live(repos).await, live_tokens);
}

/// The operator a deleted account's logins are left with is no account, and
/// stays none: neither statement that inserts a `users` row takes the nil
/// id, the repository's create or the seeder's upsert. A token acting as it
/// is refused because no account answers to it; a row with that id would be
/// the admin every such token acts as.
#[tokio::test]
async fn no_account_is_given_the_id_that_stands_for_a_deleted_one() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let mut created = create_test_admin_user("nil-created@example.com", None);
    created.id = DELETED_ACCOUNT_OPERATOR;
    let refused = repos
        .users
        .create(&created)
        .await
        .expect_err("create must refuse the nil id");
    assert_eq!(refused.code, ErrorCode::InvalidInput, "{refused:?}");

    let refused = repos
        .seeder
        .seed_insert_demo_user(&SeedDemoUser {
            id: DELETED_ACCOUNT_OPERATOR,
            email: "nil-seeded@example.com".to_owned(),
            display_name: "Seeded".to_owned(),
            password_hash: "not-a-hash".to_owned(),
            tier: "starter".to_owned(),
            status: "active".to_owned(),
            role: UserRole::Admin,
            locale: "en".to_owned(),
            created_at: Utc::now(),
        })
        .await
        .expect_err("the seeder must refuse the nil id");
    assert_eq!(refused.code, ErrorCode::InvalidInput, "{refused:?}");

    for email in ["nil-created@example.com", "nil-seeded@example.com"] {
        assert!(
            repos.users.get_by_email(email).await.unwrap().is_none(),
            "a refused insert wrote no row: {email}"
        );
    }
    assert!(repos
        .users
        .get_global(DELETED_ACCOUNT_OPERATOR)
        .await
        .unwrap()
        .is_none());

    // The control: the same account under an id of its own.
    created.id = Uuid::new_v4();
    repos.users.create(&created).await.unwrap();
}

/// A service name is free text, and `pierre-cli token generate --service`
/// takes any: a token named after an account carries its id without being
/// its device login. The delete takes the id out of that name and out of the
/// copy each key it provisioned was recorded under, and leaves the rest of
/// the name, the token's state and its holder alone: what the survivor read
/// counts as naming the account is exactly what the delete rewrites, so such
/// a token never stands between an account and its deletion.
#[tokio::test]
async fn a_token_named_with_an_accounts_id_loses_the_id_and_blocks_no_deletion() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let leaving = account(repos, "leaving@example.com", false).await;
    account(repos, "staying@example.com", true).await;
    let export = mint(repos, &format!("export-{}-nightly", leaving.id), None).await;
    let revoked = mint(repos, &format!("import-{}", leaving.id), None).await;
    repos.admin.deactivate_token(&revoked).await.unwrap();
    let unrelated = mint(repos, "nightly-job", None).await;
    for (token_id, key) in [
        (&export, "key-of-the-export"),
        (&unrelated, "key-of-the-job"),
    ] {
        repos
            .admin
            .record_provisioned_key(token_id, key, "athlete@example.com", "starter", 100, "hour")
            .await
            .unwrap();
    }
    // The survivor read counts the id inside a name, in both kept tables
    // and nowhere else (the order of its rows is the engine's).
    let mut naming = tables_naming(&db, leaving.id).await;
    naming.sort();
    assert_eq!(naming, vec!["admin_provisioned_keys", "admin_tokens"]);

    let deletion = repos
        .account_deletion
        .delete_account(&AccountDeletionPlan {
            user_id: leaving.id,
            actor: DeletionActor::Operator,
            firebase_project: None,
            backends: Vec::new(),
            revocations: "[]".to_owned(),
            not_deleted: Vec::new(),
        })
        .await
        .expect("a token's name does not refuse the deletion");

    assert_eq!(
        deletion.device_logins_revoked,
        Vec::<String>::new(),
        "the account approved no device login"
    );
    assert_eq!(rows_naming(&db, leaving.id).await, 0);
    assert_eq!(tables_naming(&db, leaving.id).await, Vec::<String>::new());
    let renamed = format!("export-{DELETED_ACCOUNT_LABEL}-nightly");
    let row = token(repos, &export).await.expect("the row stays");
    assert_eq!(row.service_name, renamed, "only the id is rewritten");
    assert!(row.is_active, "its state is not the deletion's");
    assert_eq!(row.operator_user_id, None);
    assert_eq!(provisioned_by(&db, &export).await, vec![renamed]);
    let row = token(repos, &revoked).await.expect("the row stays");
    assert_eq!(
        row.service_name,
        format!("import-{DELETED_ACCOUNT_LABEL}"),
        "a revoked token's name is rewritten too"
    );
    assert!(!row.is_active);

    // The control: a token that never carried the id.
    let row = token(repos, &unrelated).await.expect("the row stays");
    assert_eq!(row.service_name, "nightly-job");
    assert!(row.is_active);
    assert_eq!(provisioned_by(&db, &unrelated).await, vec!["nightly-job"]);
}

/// A deletion that fails leaves the tokens as they were: they end with the
/// account or not at all. The last active admin is refused, and its login
/// stays live under its own name.
#[tokio::test]
async fn a_refused_deletion_touches_no_token() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let last = account(repos, "last@example.com", true).await;
    let label = format!("{DEVICE_CLI_SERVICE_PREFIX}{}", last.id);
    let held = mint(repos, &label, Some(last.id)).await;
    let rotated_away = mint(repos, &label, None).await;

    let refused = repos
        .account_deletion
        .delete_account(&AccountDeletionPlan {
            user_id: last.id,
            actor: DeletionActor::Operator,
            firebase_project: None,
            backends: Vec::new(),
            revocations: "[]".to_owned(),
            not_deleted: Vec::new(),
        })
        .await;
    assert!(refused.is_err(), "the last active admin is not deleted");

    for id in [&held, &rotated_away] {
        let row = token(repos, id).await.expect("the row stays");
        assert_eq!(row.service_name, label);
        assert!(row.is_active);
    }
    assert_eq!(
        token(repos, &held).await.unwrap().operator_user_id,
        Some(last.id)
    );
}

/// The secret every token in these tests is minted under.
const JWT_SECRET: &str = "test-admin-jwt-secret";

/// How many tokens the table holds, revoked ones included.
async fn stored(repos: &RepositoryRegistry) -> usize {
    repos.admin.list_tokens(true).await.unwrap().len()
}

/// What a revoked token's rotation is refused with: the words of the one
/// definition of a rotation, asked of `revoked` itself.
fn revoked_refusal(revoked: &AdminToken) -> String {
    assert!(!revoked.is_active, "the token asked about is revoked");
    CreateAdminTokenRequest::rotation_of(revoked.clone(), 30, None)
        .expect_err("a revoked token has no replacement")
        .message
}

/// A rotation retires the token and mints exactly one replacement, built
/// from the row: its name, description and permissions, without the flag it
/// never had, on the lifetime asked for. The same rotation asked again, by
/// a caller still holding the row it read while the token was live, claims
/// nothing: it is refused in the words a revoked token is refused with, and
/// nothing is minted. An unknown token is not found.
#[tokio::test]
async fn a_rotation_retires_the_token_and_mints_one_replacement() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let old = repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: "trail-reader".to_owned(),
                service_description: Some("reads the trail".to_owned()),
                permissions: Some(vec![AdminPermission::ViewAuditLogs]),
                expires_in_days: Some(90),
                is_super_admin: false,
                tenant_id: None,
                operator_user_id: None,
            },
            JWT_SECRET,
            &Unsigned,
        )
        .await
        .unwrap()
        .token_id;
    let bystander = mint(repos, "nightly-job", None).await;
    let read_while_live = token(repos, &old).await.expect("the token");
    let rotation = TokenRotation {
        checked: &read_while_live,
        expires_in_days: 7,
        rotated_by: None,
    };

    let replacement = repos
        .admin
        .rotate_token(&rotation, JWT_SECRET, &Unsigned)
        .await
        .unwrap();

    assert_ne!(replacement.token_id, old);
    assert_eq!(stored(repos).await, 3, "exactly one row was inserted");
    assert!(
        !token(repos, &old).await.expect("the old row").is_active,
        "the rotated token is retired"
    );
    let row = token(repos, &replacement.token_id)
        .await
        .expect("the replacement's row");
    assert!(row.is_active);
    assert_eq!(row.service_name, "trail-reader");
    assert_eq!(row.service_description.as_deref(), Some("reads the trail"));
    assert_eq!(
        row.permissions,
        AdminPermissions::new(vec![AdminPermission::ViewAuditLogs]),
        "the permissions the token held, not a new token's defaults"
    );
    assert!(!row.is_super_admin);
    assert_eq!(row.operator_user_id, None);
    let lifetime = row.expires_at.expect("it expires") - Utc::now();
    assert!(
        (6..=7).contains(&lifetime.num_days()),
        "the lifetime asked for, not the old token's: {lifetime}"
    );
    let mut live_tokens = vec![bystander, replacement.token_id];
    live_tokens.sort();
    assert_eq!(live(repos).await, live_tokens);

    // The caller's row still says live; the table says retired.
    let refused = repos
        .admin
        .rotate_token(&rotation, JWT_SECRET, &Unsigned)
        .await
        .expect_err("a retired token is not claimed");
    assert_eq!(refused.code, ErrorCode::InvalidInput, "{refused:?}");
    let retired = token(repos, &old).await.expect("the old row");
    assert_eq!(refused.message, revoked_refusal(&retired));
    assert_eq!(stored(repos).await, 3, "a refused rotation mints nothing");

    let mut unknown = read_while_live.clone();
    unknown.id = "admin_no_such_token".to_owned();
    let missing = repos
        .admin
        .rotate_token(
            &TokenRotation {
                checked: &unknown,
                ..rotation
            },
            JWT_SECRET,
            &Unsigned,
        )
        .await
        .expect_err("an unknown token is not rotated");
    assert_eq!(missing.code, ErrorCode::ResourceNotFound, "{missing:?}");
    assert_eq!(stored(repos).await, 3);
}

/// A rotation's terms print the token they were checked on by its id. The
/// row's two hashes, the one a presented token is verified against and the
/// one of the secret it was signed under, appear in no debug line.
#[tokio::test]
async fn a_rotations_terms_print_the_token_id_and_neither_hash() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let id = mint(repos, "nightly-job", None).await;
    let row = token(repos, &id).await.expect("the token");
    assert!(
        row.token_hash.len() >= 32 && row.jwt_secret_hash.len() >= 32,
        "the row carries both hashes: {} and {} characters",
        row.token_hash.len(),
        row.jwt_secret_hash.len()
    );

    let printed = format!(
        "{:?}",
        TokenRotation {
            checked: &row,
            expires_in_days: 30,
            rotated_by: None,
        }
    );

    assert!(printed.contains(&id), "{printed}");
    assert!(printed.contains("expires_in_days: 30"), "{printed}");
    assert!(
        !printed.contains(&row.token_hash),
        "the token's hash is printed"
    );
    assert!(
        !printed.contains(&row.jwt_secret_hash),
        "the signing secret's hash is printed"
    );
}

/// The replacement is minted at the level the caller checked, or not at
/// all. A caller that checked a narrower row than the one the rotation
/// claims, or one without the super-admin flag, is refused as a conflict,
/// and the refusal undoes the claim with everything else: the token is
/// still live and nothing was minted. The same token rotates for a caller
/// that checked it as it is.
#[tokio::test]
async fn a_rotation_mints_no_more_than_the_caller_checked() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let wide = repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: "user-manager".to_owned(),
                service_description: None,
                permissions: Some(vec![
                    AdminPermission::ListKeys,
                    AdminPermission::ManageUsers,
                ]),
                expires_in_days: Some(90),
                is_super_admin: false,
                tenant_id: None,
                operator_user_id: None,
            },
            JWT_SECRET,
            &Unsigned,
        )
        .await
        .unwrap()
        .token_id;
    let as_stored = token(repos, &wide).await.expect("the token");
    let mut narrower = as_stored.clone();
    narrower.permissions = AdminPermissions::new(vec![AdminPermission::ListKeys]);
    let mut flagged = as_stored.clone();
    flagged.is_super_admin = true;

    for (checked, what) in [(&narrower, "permissions"), (&flagged, "flag")] {
        let refused = repos
            .admin
            .rotate_token(
                &TokenRotation {
                    checked,
                    expires_in_days: 30,
                    rotated_by: None,
                },
                JWT_SECRET,
                &Unsigned,
            )
            .await
            .expect_err("the row claimed is not the row checked");
        assert_eq!(
            refused.code,
            ErrorCode::ResourceLocked,
            "{what}: {refused:?}"
        );
        assert!(
            token(repos, &wide).await.expect("the row").is_active,
            "{what}: the refusal rolled the claim back"
        );
        assert_eq!(stored(repos).await, 1, "{what}: nothing was minted");
    }

    let replacement = repos
        .admin
        .rotate_token(
            &TokenRotation {
                checked: &as_stored,
                expires_in_days: 30,
                rotated_by: None,
            },
            JWT_SECRET,
            &Unsigned,
        )
        .await
        .expect("the row claimed is the row checked");
    assert_eq!(replacement.permissions, as_stored.permissions);
    assert!(!token(repos, &wide).await.expect("the row").is_active);
    assert_eq!(stored(repos).await, 2);
}

/// How long [`Slow`] takes to sign: the time a rotation's transaction stays
/// open between its claim and its insert, which is the window a deletion
/// has to land in.
const SIGNING_PAUSE: Duration = Duration::from_millis(120);

/// The head start one racer is given in the rounds that fix the order.
const HEAD_START: Duration = Duration::from_millis(40);

/// A signer that takes [`SIGNING_PAUSE`] to sign, on its own thread's time.
struct Slow;

impl JwtSigner for Slow {
    fn sign_token(&self, claims: &Value) -> AppResult<String> {
        task::block_in_place(|| thread::sleep(SIGNING_PAUSE));
        Unsigned.sign_token(claims)
    }
}

/// How many times the race below is run: every pairing of who rotates and
/// who starts first, twice.
const ROTATION_RACE_ROUNDS: usize = 12;

/// A device login is rotated while the admin it acts as is deleted. A
/// rotation that minted from the row its caller read would, when the
/// deletion committed after that read, mint its replacement under the
/// deleted account's name and, rotated by that admin, acting as it. In one
/// transaction each, the two run one after the other:
///
/// - the deletion first: the login is retired, the rotation claims nothing
///   and is refused as a revoked token's is, and no replacement exists;
/// - the rotation first: the deletion finds the replacement. Rotated by the
///   admin itself it acts as that admin, and is retired with the admin's
///   other logins, so it authenticates no one; rotated by the host shell it
///   is no admin's and keeps its state, under the name that names no one.
///
/// Either way the deletion lands, and no `admin_tokens` row names the
/// deleted account. A third of the rounds start both at once; the others
/// give one racer a head start, so both orders are run.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rotation_racing_its_admins_deletion_leaves_no_token_naming_the_account() {
    let mut replaced = 0;
    let mut refused = 0;
    for round in 0..ROTATION_RACE_ROUNDS {
        // Connections that overlap in the engine, and a database per round.
        let db = create_concurrent_test_db().await.unwrap();
        let repos = db.repositories();
        let leaving = account(repos, "leaving@example.com", true).await;
        account(repos, "staying@example.com", true).await;
        let label = format!("{DEVICE_CLI_SERVICE_PREFIX}{}", leaving.id);
        let login = mint(repos, &label, Some(leaving.id)).await;
        let read_while_live = token(repos, &login).await.expect("the login");
        // Two connections open before the race, so neither racer connects
        // while the other runs.
        let (warm, also_warm) = tokio::join!(repos.users.list_admins(), repos.users.list_admins());
        assert_eq!(warm.unwrap().len() + also_warm.unwrap().len(), 4);

        let rotated_by = (round % 2 == 0).then_some(leaving.id);
        let (rotation_waits, deletion_waits) = match round % 3 {
            0 => (Duration::ZERO, Duration::ZERO),
            1 => (Duration::ZERO, HEAD_START),
            _ => (HEAD_START, Duration::ZERO),
        };
        let rotating = {
            let db = db.clone();
            tokio::spawn(async move {
                time::sleep(rotation_waits).await;
                db.repositories()
                    .admin
                    .rotate_token(
                        &TokenRotation {
                            checked: &read_while_live,
                            expires_in_days: 30,
                            rotated_by,
                        },
                        JWT_SECRET,
                        &Slow,
                    )
                    .await
            })
        };
        let deleting = {
            let db = db.clone();
            tokio::spawn(async move {
                time::sleep(deletion_waits).await;
                delete_account(db.repositories(), leaving.id, None).await
            })
        };
        let rotation = rotating.await.expect("the rotation ran");
        let deletion = deleting.await.expect("the deletion ran");

        let deletion = deletion.unwrap_or_else(|e| {
            panic!("round {round}: a rotation never refuses the deletion: {e:?} / {rotation:?}")
        });
        assert_eq!(
            rows_naming(&db, leaving.id).await,
            0,
            "round {round}: no row names the deleted account"
        );
        assert_eq!(
            tables_naming(&db, leaving.id).await,
            Vec::<String>::new(),
            "round {round}"
        );
        let retired = token(repos, &login).await.expect("the login's row stays");
        assert!(!retired.is_active, "round {round}");
        assert_eq!(
            retired.service_name, DEVICE_CLI_DELETED_ACCOUNT_SERVICE,
            "round {round}"
        );
        assert_eq!(
            retired.operator_user_id,
            Some(DELETED_ACCOUNT_OPERATOR),
            "round {round}"
        );

        match rotation {
            Ok(replacement) => {
                replaced += 1;
                assert_eq!(stored(repos).await, 2, "round {round}: one replacement");
                // The rotation retired the login before the deletion read
                // the live ones, so the login is not among them: what the
                // deletion revoked is the replacement when it acts as the
                // admin, and nothing when it is no admin's.
                let revoked = if rotated_by.is_some() {
                    vec![replacement.token_id.clone()]
                } else {
                    Vec::new()
                };
                assert_eq!(
                    deletion.device_logins_revoked, revoked,
                    "round {round}: {deletion:?}"
                );
                let row = token(repos, &replacement.token_id)
                    .await
                    .expect("the replacement's row");
                assert_eq!(
                    row.service_name, DEVICE_CLI_DELETED_ACCOUNT_SERVICE,
                    "round {round}: the replacement is renamed with the login"
                );
                if rotated_by.is_some() {
                    assert!(
                        !row.is_active,
                        "round {round}: the admin's own replacement is retired with it"
                    );
                    assert_eq!(
                        row.operator_user_id,
                        Some(DELETED_ACCOUNT_OPERATOR),
                        "round {round}"
                    );
                } else {
                    assert_eq!(
                        row.operator_user_id, None,
                        "round {round}: the host shell's replacement is no admin's"
                    );
                    assert!(row.is_active, "round {round}: its state is its holder's");
                }
            }
            Err(error) => {
                refused += 1;
                assert_eq!(
                    error.code,
                    ErrorCode::InvalidInput,
                    "round {round}: {error:?}"
                );
                assert_eq!(error.message, revoked_refusal(&retired), "round {round}");
                assert_eq!(stored(repos).await, 1, "round {round}: no replacement");
                assert_eq!(
                    deletion.device_logins_revoked,
                    vec![login],
                    "round {round}: the deletion is what revoked the login"
                );
            }
        }
    }
    assert!(
        replaced > 0 && refused > 0,
        "both orders were run: {replaced} rotations landed first, {refused} deletions did"
    );
}
