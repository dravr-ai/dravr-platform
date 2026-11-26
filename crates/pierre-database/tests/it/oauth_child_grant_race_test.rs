// ABOUTME: carnet#1544 item 9 — a step-up child is never left live under a parent a concurrent revoke or withdrawal ended
// ABOUTME: Holds the ending transaction open across the child's write, on whichever backend DATABASE_URL names

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `store_child_grant` reads its parent and inserts the child; a revoke or a
//! withdrawal of that parent ends the parent and its children. Run
//! concurrently, the two must serialize on the parent row: either the child
//! is written first and the ending revokes it, or the ending commits first
//! and the child is refused. Each test holds the ending's transaction open,
//! uncommitted, while the other side runs, then commits it.

use std::time::Duration;

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{
    ChildGrant, GrantApproval, HostProfileBasis, OAuth2Client, OAuthClientGrant, StepUpTerms,
    OWNER_VERIFIED_ACR,
};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use tokio::time::sleep;
use uuid::Uuid;

const TENANT: &str = "tenant-child-race";
const SCOPE: &str = "fitness:read";
const CHILD_SCOPE: &str = "fitness:read pdpp:preset:one_off_question";

/// What the ending side leaves its transaction open across.
const HOLD: Duration = Duration::from_millis(400);

/// Run `$body` with `$pool` bound to the test database's pool, whichever
/// engine it is.
macro_rules! on_pool {
    ($db:expr, |$pool:ident| $body:expr) => {
        match $db.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                let $pool = sqlite.pool();
                $body
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(postgres) => {
                let $pool = postgres.pool();
                $body
            }
        }
    };
}

async fn fresh_client(repos: &RepositoryRegistry) -> OAuth2Client {
    let client = OAuth2Client {
        id: Uuid::new_v4().to_string(),
        client_id: format!("client-{}", Uuid::new_v4()),
        client_secret_hash: "argon2-hash-placeholder".to_owned(),
        redirect_uris: vec!["https://app.example/callback".to_owned()],
        grant_types: vec!["authorization_code".to_owned(), "refresh_token".to_owned()],
        response_types: vec!["code".to_owned()],
        client_name: Some("Child Race Client".to_owned()),
        client_uri: None,
        scope: Some("read".to_owned()),
        created_at: Timestamp::now(),
        expires_at: None,
    };
    assert!(
        repos
            .oauth2_server
            .store_client_within_ceiling(&client, u64::MAX)
            .await
            .unwrap(),
        "premise: an unbounded ceiling stores the client"
    );
    client
}

async fn parent_grant(repos: &RepositoryRegistry, user: Uuid, client: &str) -> String {
    repos
        .oauth2_server
        .store_client_grant(
            &OAuthClientGrant {
                id: Uuid::new_v4().to_string(),
                user_id: user.to_string(),
                tenant_id: TENANT.to_owned(),
                client_id: client.to_owned(),
                scope: SCOPE.to_owned(),
                granted_at: Timestamp::now(),
                revoked_at: None,
                consent_version: None,
                consent_locale: None,
                authorization_details: Some("{}".to_owned()),
                expires_at: None,
            },
            &GrantApproval::by_user(),
        )
        .await
        .unwrap()
}

fn child_of(user: Uuid, client: &str, parent: &str) -> ChildGrant {
    ChildGrant {
        grant: OAuthClientGrant {
            id: Uuid::new_v4().to_string(),
            user_id: user.to_string(),
            tenant_id: TENANT.to_owned(),
            client_id: client.to_owned(),
            scope: CHILD_SCOPE.to_owned(),
            granted_at: Timestamp::now(),
            revoked_at: None,
            consent_version: None,
            consent_locale: None,
            authorization_details: Some("{}".to_owned()),
            expires_at: Some(Timestamp::now() + Elapsed::hours(1)),
        },
        terms: StepUpTerms {
            parent_grant_id: parent.to_owned(),
            auth_time: Timestamp::now(),
            acr: OWNER_VERIFIED_ACR.to_owned(),
            host_profile_basis: HostProfileBasis::DeclaredCimd,
            host_profile_tier: "T2".to_owned(),
        },
    }
}

/// How many children of `parent` are live.
async fn live_children(db: &Database, parent: &str) -> i64 {
    const SQL: &str = "SELECT COUNT(*) FROM oauth_client_grants \
                       WHERE parent_grant_id = $1 AND revoked_at IS NULL";
    on_pool!(db, |pool| sqlx::query_scalar::<_, i64>(SQL)
        .bind(parent)
        .fetch_one(pool)
        .await
        .unwrap())
}

/// A revoke of the parent, its children ended in the same transaction, holds
/// that transaction open while the step-up writes its child: the child is
/// refused once the revoke commits, and none is left live.
#[tokio::test]
async fn a_step_up_racing_its_parents_revoke_leaves_no_live_child() {
    const REVOKE_PARENT: &str =
        "UPDATE oauth_client_grants SET revoked_at = CURRENT_TIMESTAMP WHERE id = $1";
    const REVOKE_CHILDREN: &str = "UPDATE oauth_client_grants SET revoked_at = CURRENT_TIMESTAMP \
                                   WHERE parent_grant_id = $1 AND revoked_at IS NULL";
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let client = fresh_client(repos).await;
    let user = Uuid::new_v4();
    let parent = parent_grant(repos, user, &client.client_id).await;
    let child = child_of(user, &client.client_id, &parent);
    let approval = GrantApproval::by_user();

    let stored = on_pool!(db, |pool| {
        let mut revoke = pool.begin().await.unwrap();
        sqlx::query(REVOKE_PARENT)
            .bind(&parent)
            .execute(&mut *revoke)
            .await
            .unwrap();
        sqlx::query(REVOKE_CHILDREN)
            .bind(&parent)
            .execute(&mut *revoke)
            .await
            .unwrap();
        let (stored, ()) = tokio::join!(
            repos
                .oauth2_server
                .store_child_grant(&child, &approval),
            async {
                sleep(HOLD).await;
                revoke.commit().await.unwrap();
            }
        );
        stored
    });

    assert_eq!(
        live_children(&db, &parent).await,
        0,
        "bug: a child is live under the parent a concurrent revoke ended"
    );
    assert!(
        matches!(stored, Ok(None)),
        "bug: the step-up under a parent revoked meanwhile was not refused cleanly: {stored:?}"
    );
}

/// A step-up that holds the parent (its row lock, or the one write lock)
/// and writes its child while a withdrawal of the client runs: the
/// withdrawal ends the child the step-up committed, not only the children it
/// saw before.
#[tokio::test]
async fn a_withdrawal_racing_a_step_up_ends_the_child_it_commits() {
    const INSERT_CHILD: &str = "INSERT INTO oauth_client_grants \
         (id, user_id, tenant_id, client_id, scope, granted_at, revoked_at, parent_grant_id, \
          step_up_auth_time, step_up_acr, host_profile_basis, host_profile_tier) \
         VALUES ($1, $2, $3, $4, $5, CURRENT_TIMESTAMP, NULL, $6, $7, $8, 'declared_cimd', 'T2')";
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let client = fresh_client(repos).await;
    let user = Uuid::new_v4();
    let parent = parent_grant(repos, user, &client.client_id).await;
    let child_id = Uuid::new_v4().to_string();
    let user_id = user.to_string();

    let lock = match db.backend() {
        DatabaseBackend::SQLite(_) => {
            "UPDATE oauth_client_grants SET revoked_at = revoked_at WHERE id = $1"
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(_) => "SELECT id FROM oauth_client_grants WHERE id = $1 FOR UPDATE",
    };
    let withdrawn = on_pool!(db, |pool| {
        let mut step_up = pool.begin().await.unwrap();
        sqlx::query(lock)
            .bind(&parent)
            .execute(&mut *step_up)
            .await
            .unwrap();
        let (withdrawn, ()) = tokio::join!(
            repos.oauth2_server.withdraw_client_access(
                &user_id,
                TENANT,
                &client.client_id,
                SCOPE
            ),
            async {
                sleep(HOLD).await;
                sqlx::query(INSERT_CHILD)
                    .bind(&child_id)
                    .bind(user.to_string())
                    .bind(TENANT)
                    .bind(&client.client_id)
                    .bind(CHILD_SCOPE)
                    .bind(&parent)
                    .bind(Timestamp::now().epoch_seconds())
                    .bind(OWNER_VERIFIED_ACR)
                    .execute(&mut *step_up)
                    .await
                    .unwrap();
                step_up.commit().await.unwrap();
            }
        );
        withdrawn
    });

    withdrawn.unwrap();
    assert_eq!(
        live_children(&db, &parent).await,
        0,
        "bug: the child a step-up committed during the withdrawal is still live"
    );
}
