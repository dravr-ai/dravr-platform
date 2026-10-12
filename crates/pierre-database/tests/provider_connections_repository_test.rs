// ABOUTME: Covers the provider connection repository against whichever backend DATABASE_URL names
// ABOUTME: The server-level app's live-grant count that sizes an Intervals.icu app's request pool
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `count_server_level_grants` reads the stored tokens with their
//! connections, the tenants' own apps and the users' BYO apps, so it is the
//! one statement here whose casts differ between the backends (`uuid` tenant
//! and user ids on Postgres, text on `SQLite`). These run on `SQLite` and on
//! `PostgreSQL`: `create_test_db` opens whichever `DATABASE_URL` names.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::Utc;
use pierre_core::models::{
    ConnectionType, Tenant, TenantId, TenantOAuthCredentials, User, UserOAuthToken,
    API_KEY_TOKEN_TYPE,
};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// A distinct user per call; the token table references `users` on Postgres.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("provider-connection-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Provider Connection Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

/// Store `user_id`'s link to `provider` in `tenant` for provider athlete
/// `athlete`, with a token of `token_type` and the connection that link
/// registers.
async fn link(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: &TenantId,
    provider: &str,
    athlete: Option<&str>,
    token_type: &str,
) {
    let now = Utc::now();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant_id.to_string(),
            provider: provider.to_owned(),
            access_token: format!("access-{provider}-{user_id}"),
            refresh_token: None,
            token_type: token_type.to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: athlete.map(str::to_owned),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let connection = if token_type == API_KEY_TOKEN_TYPE {
        ConnectionType::Manual
    } else {
        ConnectionType::OAuth
    };
    repos
        .provider_connections
        .register_connection(user_id, *tenant_id, provider, &connection, None)
        .await
        .unwrap();
}

/// The server-level app signs every live OAuth grant, and each provider
/// athlete counts once: one athlete granted from two tenants is one grant, a
/// pasted API key is none, a grant the provider refused is none while one
/// flagged over our own client credentials still stands, a grant whose
/// athlete id was never read counts for none, and another provider's grant
/// is that provider's.
#[tokio::test]
async fn the_server_level_app_counts_each_live_grant_athlete_once() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let tenant = TenantId::generate();
    let other_tenant = TenantId::generate();
    let provider = "intervals_icu";
    let bearer = "Bearer";

    let granted = fresh_user(repos).await;
    link(repos, granted, &tenant, provider, Some("i1"), bearer).await;
    let same_athlete = fresh_user(repos).await;
    link(
        repos,
        same_athlete,
        &other_tenant,
        provider,
        Some("i1"),
        bearer,
    )
    .await;
    let pasted_key = fresh_user(repos).await;
    link(
        repos,
        pasted_key,
        &tenant,
        provider,
        Some("i3"),
        API_KEY_TOKEN_TYPE,
    )
    .await;
    let refused = fresh_user(repos).await;
    link(repos, refused, &tenant, provider, Some("i4"), bearer).await;
    repos
        .provider_connections
        .mark_needs_reauth(refused, tenant, provider, Some("invalid_grant"), Utc::now())
        .await
        .unwrap();
    let our_client = fresh_user(repos).await;
    link(repos, our_client, &tenant, provider, Some("i5"), bearer).await;
    repos
        .provider_connections
        .mark_needs_reauth(
            our_client,
            tenant,
            provider,
            Some("invalid_client"),
            Utc::now(),
        )
        .await
        .unwrap();
    let unread = fresh_user(repos).await;
    link(repos, unread, &tenant, provider, None, bearer).await;
    let elsewhere = fresh_user(repos).await;
    link(repos, elsewhere, &tenant, "strava", Some("i7"), bearer).await;

    assert_eq!(
        repos
            .provider_connections
            .count_server_level_grants(provider)
            .await
            .unwrap(),
        2,
        "i1 once, and i5, whose grant our own client credentials cannot spoil"
    );
}

/// A tenant with an active app of its own signs its athletes' grants with
/// it, and a user with a BYO app signs theirs: neither is the server-level
/// app's. WHOOP, because both backends admit a tenant's and a user's own app
/// for it; the count asks the same question of any provider.
#[tokio::test]
async fn grants_an_own_app_signs_are_not_the_server_level_apps() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let provider = "whoop";
    let owner = fresh_user(repos).await;
    let own_app_tenant = Tenant::new(
        "Own App Tenant".to_owned(),
        format!("own-app-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        owner,
    );
    repos.tenants.create(&own_app_tenant).await.unwrap();
    repos
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id: own_app_tenant.id,
            provider: provider.to_owned(),
            client_id: "tenant-whoop-client".to_owned(),
            client_secret: "tenant-whoop-secret".to_owned(),
            redirect_uri: "https://tenant.example/callback".to_owned(),
            scopes: vec!["read:recovery".to_owned()],
            rate_limit_per_day: 1_000,
        })
        .await
        .unwrap();
    let shared_tenant = TenantId::generate();

    let in_own_app_tenant = fresh_user(repos).await;
    link(
        repos,
        in_own_app_tenant,
        &own_app_tenant.id,
        provider,
        Some("w1"),
        "Bearer",
    )
    .await;
    let byo = fresh_user(repos).await;
    link(repos, byo, &shared_tenant, provider, Some("w2"), "Bearer").await;
    repos
        .oauth_tokens
        .store_user_oauth_app(
            byo,
            provider,
            "byo-whoop-client",
            "byo-whoop-secret",
            "https://byo.example/callback",
        )
        .await
        .unwrap();
    let on_server_app = fresh_user(repos).await;
    link(
        repos,
        on_server_app,
        &shared_tenant,
        provider,
        Some("w3"),
        "Bearer",
    )
    .await;

    assert_eq!(
        repos
            .provider_connections
            .count_server_level_grants(provider)
            .await
            .unwrap(),
        1,
        "only w3's grant is signed by the server-level app"
    );
}
