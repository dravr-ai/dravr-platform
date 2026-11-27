// ABOUTME: carnet#1544 item 6 — the step-up migration keeps the previous binary's grant arbiter beside the new parent-only one
// ABOUTME: The previous release's consent upsert, verbatim, and this one's both run against the migrated schema, with a live child

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Migration `20261126200000` must not take away the index the previous
//! binary's consent upsert names as its arbiter: during a rolling deploy, or
//! after a rollback, that binary still records consents against this schema.
//! The previous release's statement is copied here verbatim (it is the data
//! under test: no current source holds it), run beside this release's
//! repository write, with a step-up child live under the parent.

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{
    ChildGrant, GrantApproval, HostProfileBasis, OAuth2Client, OAuthClientGrant, StepUpTerms,
    OWNER_VERIFIED_ACR,
};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// `STORE_CLIENT_GRANT_SQL` as origin/main (20b3555fb) ships it: the
/// previous binary's consent upsert.
const PREVIOUS_STORE_CLIENT_GRANT_SQL: &str = r"
            INSERT INTO oauth_client_grants
                (id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                 consent_version, consent_locale, authorization_details, expires_at,
                 standing_policy_id)
            VALUES ($1, $2, $3, $4, $5, CURRENT_TIMESTAMP, NULL, $6, $7, $8, $9, $10)
            ON CONFLICT (user_id, tenant_id, client_id, scope) WHERE revoked_at IS NULL
            DO UPDATE SET consent_version = excluded.consent_version,
                          consent_locale = excluded.consent_locale,
                          standing_policy_id = excluded.standing_policy_id
            RETURNING id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                      consent_version, consent_locale, authorization_details, expires_at,
                      CURRENT_TIMESTAMP AS event_at
            ";

const TENANT: &str = "tenant-index-rollout";
const SCOPE: &str = "fitness:read";
/// A step-up child's scope: the parent's, and the one-off preset beside it.
const CHILD_SCOPE: &str = "fitness:read pdpp:preset:one_off_question";

async fn fresh_client(repos: &RepositoryRegistry) -> OAuth2Client {
    let client = OAuth2Client {
        id: Uuid::new_v4().to_string(),
        client_id: format!("client-{}", Uuid::new_v4()),
        client_secret_hash: "argon2-hash-placeholder".to_owned(),
        redirect_uris: vec!["https://app.example/callback".to_owned()],
        grant_types: vec!["authorization_code".to_owned(), "refresh_token".to_owned()],
        response_types: vec!["code".to_owned()],
        client_name: Some("Index Rollout Client".to_owned()),
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

/// Run the previous binary's upsert for (`user`, `client`, [`SCOPE`]) with
/// row id `id`; the id of the row holding the grant.
async fn previous_store(db: &Database, id: &str, user: Uuid, client: &str) -> String {
    macro_rules! run {
        ($pool:expr) => {
            sqlx::query_scalar::<_, String>(PREVIOUS_STORE_CLIENT_GRANT_SQL)
                .bind(id)
                .bind(user.to_string())
                .bind(TENANT)
                .bind(client)
                .bind(SCOPE)
                .bind(Some("digest-previous".to_owned()))
                .bind(Some("en".to_owned()))
                .bind(None::<String>)
                .bind(None::<Timestamp>)
                .bind(None::<String>)
                .fetch_one($pool)
                .await
                .expect("bug: the previous binary's consent upsert fails on the migrated schema")
        };
    }
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => run!(sqlite.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => run!(postgres.pool()),
    }
}

/// A plain grant for (`user`, `client`, [`SCOPE`]) through this release's
/// repository write; the id of the row holding it.
async fn current_store(repos: &RepositoryRegistry, user: Uuid, client: &str) -> String {
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
                consent_version: Some("digest-current".to_owned()),
                consent_locale: Some("en".to_owned()),
                authorization_details: None,
                expires_at: None,
            },
            &GrantApproval::by_user(),
        )
        .await
        .expect("bug: this release's consent write fails on the migrated schema")
}

/// A live step-up child under `parent`.
async fn store_child(repos: &RepositoryRegistry, user: Uuid, client: &str, parent: &str) {
    let child = ChildGrant {
        grant: OAuthClientGrant {
            id: Uuid::new_v4().to_string(),
            user_id: user.to_string(),
            tenant_id: TENANT.to_owned(),
            client_id: client.to_owned(),
            scope: CHILD_SCOPE.to_owned(),
            granted_at: Timestamp::now(),
            revoked_at: None,
            consent_version: Some("digest-child".to_owned()),
            consent_locale: Some("en".to_owned()),
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
    };
    let stored = repos
        .oauth2_server
        .store_child_grant(&child, &GrantApproval::by_user())
        .await
        .expect("bug: a step-up child cannot be stored beside its live parent");
    assert!(stored.is_some(), "premise: the child's parent is live");
}

/// Both binaries record and re-record a consent against the migrated
/// schema, a live child beside the parent changes nothing, and they agree on
/// the one live row.
#[tokio::test]
async fn the_previous_and_the_current_consent_upsert_both_run_on_the_migrated_schema() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let client = fresh_client(repos).await;
    let user = Uuid::new_v4();

    let first = Uuid::new_v4().to_string();
    let held = previous_store(&db, &first, user, &client.client_id).await;
    assert_eq!(
        held, first,
        "premise: the previous binary inserts a new row"
    );

    store_child(repos, user, &client.client_id, &held).await;

    let again = previous_store(&db, &Uuid::new_v4().to_string(), user, &client.client_id).await;
    assert_eq!(
        again, first,
        "bug: the previous binary's re-consent did not land on the live parent row"
    );
    let current = current_store(repos, user, &client.client_id).await;
    assert_eq!(
        current, first,
        "bug: this release's re-consent did not land on the row the previous binary wrote"
    );

    let other = Uuid::new_v4();
    let fresh = current_store(repos, other, &client.client_id).await;
    let back = previous_store(&db, &Uuid::new_v4().to_string(), other, &client.client_id).await;
    assert_eq!(
        back, fresh,
        "bug: the previous binary's re-consent did not land on the row this release wrote"
    );
}
