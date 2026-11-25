// ABOUTME: carnet#1550 — the one-time migration naming the grant of refresh tokens minted before grants were named
// ABOUTME: Runs the shipped SQL on whichever backend DATABASE_URL names: exactly one live grant links, the rest stay NULL

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Migration `20261126001550` links a live refresh token with no
//! `client_grant_id` to its grant only when exactly one live grant matches
//! its tenant, client and user (JF 2026-11-25, "backfill grant ids"). Every
//! other family stays `NULL`, admitted as before, and
//! `refresh_families_without_grant` names those for the boot log.

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{
    GrantActor, GrantApproval, OAuth2Client, OAuth2RefreshToken, OAuthClientGrant,
};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::UngrantedRefreshFamilies;
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use secrecy::SecretString;
use uuid::Uuid;

// Read as files on purpose: the test runs the shipped migration SQL itself
// (the data under test), since the test database is already past it.
const SQLITE_MIGRATION: &str =
    include_str!("../../../../migrations/20261126001550_refresh_token_grant_backfill.sql");
#[cfg(feature = "postgresql")]
const POSTGRES_MIGRATION: &str =
    include_str!("../../../../migrations_pg/20261126001550_refresh_token_grant_backfill.sql");

const TENANT: &str = "tenant-backfill";

/// Run the shipped migration, whole, on whichever engine the test database is.
async fn migrate(db: &Database) {
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::raw_sql(SQLITE_MIGRATION)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::raw_sql(POSTGRES_MIGRATION)
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

async fn fresh_client(repos: &RepositoryRegistry) -> OAuth2Client {
    let client = OAuth2Client {
        id: Uuid::new_v4().to_string(),
        client_id: format!("client-{}", Uuid::new_v4()),
        client_secret_hash: "argon2-hash-placeholder".to_owned(),
        redirect_uris: vec!["https://app.example/callback".to_owned()],
        grant_types: vec!["authorization_code".to_owned(), "refresh_token".to_owned()],
        response_types: vec!["code".to_owned()],
        client_name: Some("Backfill Test Client".to_owned()),
        client_uri: None,
        scope: Some("read write".to_owned()),
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

/// A grant `user` gave `client` in `tenant` for `scope`, expiring at
/// `expires_at`; its stored id.
async fn grant(
    repos: &RepositoryRegistry,
    client: &OAuth2Client,
    user: Uuid,
    tenant: &str,
    scope: &str,
    expires_at: Option<Timestamp>,
) -> String {
    repos
        .oauth2_server
        .store_client_grant(
            &OAuthClientGrant {
                id: Uuid::new_v4().to_string(),
                user_id: user.to_string(),
                tenant_id: tenant.to_owned(),
                client_id: client.client_id.clone(),
                scope: scope.to_owned(),
                granted_at: Timestamp::now() - Elapsed::days(10),
                revoked_at: None,
                consent_version: None,
                consent_locale: None,
                authorization_details: None,
                expires_at,
            },
            &GrantApproval::by_user(),
        )
        .await
        .unwrap()
}

/// A second live grant beside the one `user` gave `client` in `tenant`, as
/// an older binary left it: the write path now revokes every other live
/// grant of the client on an approval, so it goes straight into the table.
async fn legacy_live_grant(
    db: &Database,
    client: &OAuth2Client,
    user: Uuid,
    tenant: &str,
    scope: &str,
) {
    const SQL: &str =
        "INSERT INTO oauth_client_grants (id, user_id, tenant_id, client_id, scope, granted_at) \
                       VALUES ($1, $2, $3, $4, $5, $6)";
    let id = Uuid::new_v4().to_string();
    let granted_at = Timestamp::now() - Elapsed::days(20);
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(SQL)
                .bind(&id)
                .bind(user.to_string())
                .bind(tenant)
                .bind(&client.client_id)
                .bind(scope)
                .bind(granted_at)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::query(SQL)
                .bind(&id)
                .bind(user.to_string())
                .bind(tenant)
                .bind(&client.client_id)
                .bind(scope)
                .bind(granted_at)
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

/// A refresh token of `user` for `client` in `tenant`, naming `grant`;
/// its raw value.
async fn token(
    repos: &RepositoryRegistry,
    client: &OAuth2Client,
    user: Uuid,
    tenant: &str,
    revoked: bool,
    grant: Option<String>,
) -> String {
    let raw = format!("rt-{}", Uuid::new_v4());
    repos
        .oauth2_server
        .store_refresh_token(&OAuth2RefreshToken {
            token: SecretString::from(raw.clone()),
            client_id: client.client_id.clone(),
            user_id: user,
            tenant_id: tenant.to_owned(),
            scope: Some("read".to_owned()),
            expires_at: Timestamp::now() + Elapsed::days(30),
            created_at: Timestamp::now() - Elapsed::days(5),
            revoked,
            family_id: format!("family-{}", Uuid::new_v4()),
            family_created_at: None,
            resource: None,
            client_grant_id: grant,
        })
        .await
        .unwrap();
    raw
}

async fn named(repos: &RepositoryRegistry, raw: &str) -> Option<String> {
    repos
        .oauth2_server
        .get_refresh_token_by_value(raw)
        .await
        .unwrap()
        .expect("the token reads back")
        .client_grant_id
}

/// Every case on one client, each athlete its own: one live grant links;
/// none, two, only an expired one, only one in another tenant stay NULL; a
/// revoked grant beside a live one does not count; a revoked token and one
/// already naming a grant are left alone. A rerun changes nothing, and the
/// boot read names exactly the live tokens left NULL, by tenant, client and
/// user.
#[tokio::test]
async fn the_backfill_links_a_token_to_its_one_live_grant_and_leaves_the_rest_null() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let client = fresh_client(repos).await;
    let later = Some(Timestamp::now() + Elapsed::days(30));
    let past = Some(Timestamp::now() - Elapsed::days(1));

    let one = Uuid::new_v4();
    let one_grant = grant(repos, &client, one, TENANT, "read", None).await;
    let one_token = token(repos, &client, one, TENANT, false, None).await;

    let none = Uuid::new_v4();
    let none_token = token(repos, &client, none, TENANT, false, None).await;

    let two = Uuid::new_v4();
    grant(repos, &client, two, TENANT, "read", None).await;
    legacy_live_grant(&db, &client, two, TENANT, "write").await;
    let two_token = token(repos, &client, two, TENANT, false, None).await;

    let beside = Uuid::new_v4();
    let revoked_grant = grant(repos, &client, beside, TENANT, "write", None).await;
    assert!(
        repos
            .oauth2_server
            .revoke_client_grant(
                &revoked_grant,
                &beside.to_string(),
                TENANT,
                GrantActor::User
            )
            .await
            .unwrap(),
        "premise: the athlete revokes their own live grant"
    );
    let beside_grant = grant(repos, &client, beside, TENANT, "read", later).await;
    let beside_token = token(repos, &client, beside, TENANT, false, None).await;

    let expired = Uuid::new_v4();
    grant(repos, &client, expired, TENANT, "read", past).await;
    let expired_token = token(repos, &client, expired, TENANT, false, None).await;

    let elsewhere = Uuid::new_v4();
    grant(repos, &client, elsewhere, "tenant-other", "read", None).await;
    let elsewhere_token = token(repos, &client, elsewhere, TENANT, false, None).await;

    let dead = Uuid::new_v4();
    grant(repos, &client, dead, TENANT, "read", None).await;
    let dead_token = token(repos, &client, dead, TENANT, true, None).await;

    let kept = Uuid::new_v4();
    let kept_grant = grant(repos, &client, kept, TENANT, "read", None).await;
    let other_grant = Uuid::new_v4().to_string();
    let kept_token = token(
        repos,
        &client,
        kept,
        TENANT,
        false,
        Some(other_grant.clone()),
    )
    .await;

    let expected = [
        (&one_token, Some(one_grant.clone()), "one live grant"),
        (&none_token, None, "no grant"),
        (&two_token, None, "two live grants"),
        (
            &beside_token,
            Some(beside_grant.clone()),
            "a live grant beside a revoked one",
        ),
        (&expired_token, None, "only an expired grant"),
        (&elsewhere_token, None, "a grant only in another tenant"),
        (&dead_token, None, "a revoked token"),
        (
            &kept_token,
            Some(other_grant.clone()),
            "a token already naming a grant",
        ),
    ];
    for pass in ["first", "second"] {
        migrate(&db).await;
        for (raw, grant, case) in &expected {
            assert_eq!(
                named(repos, raw).await,
                *grant,
                "bug: {pass} run, {case}: the token names the wrong grant"
            );
        }
    }
    assert_ne!(
        kept_grant, other_grant,
        "premise: the kept token names a grant other than its athlete's live one"
    );

    let mut left = repos
        .oauth2_server
        .refresh_families_without_grant(Timestamp::now())
        .await
        .unwrap();
    left.retain(|group| group.client_id == client.client_id);
    let group = |user: Uuid| UngrantedRefreshFamilies {
        tenant_id: TENANT.to_owned(),
        client_id: client.client_id.clone(),
        user_id: user.to_string(),
        families: 1,
    };
    let mut want = vec![group(none), group(two), group(expired), group(elsewhere)];
    want.sort_by(|a, b| a.user_id.cmp(&b.user_id));
    assert_eq!(
        left, want,
        "bug: the boot read does not name exactly the live tokens the backfill left NULL"
    );
}
