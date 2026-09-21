// ABOUTME: Covers the provider OAuth token repository against whichever backend DATABASE_URL names
// ABOUTME: Stored timestamps, the encrypted refresh round trip, BYO apps freeing shared-app seats, sync stamps
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `user_oauth_tokens` holds every provider access and refresh token,
//! encrypted at rest and bound by AAD to its `(tenant, user, provider)`.
//! Two things used to differ between the backends and are pinned here:
//! the timestamps a caller hands `upsert_token` are what gets stored (the
//! `SQLite` side replaced them with the clock, which reordered `get_tokens`),
//! and a BYO OAuth app lives in `user_oauth_app_credentials`, the table the
//! seat counts consult (the `SQLite` side wrote a side table nothing read,
//! so a BYO user kept occupying a shared-app seat).
//!
//! These run on `SQLite` and on `PostgreSQL`: `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{DateTime, Utc};
use pierre_core::models::{TenantId, User, UserOAuthToken};
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// A distinct user per call; both token tables reference `users` on Postgres.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("oauth-token-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("OAuth Token Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

/// A whole-second instant, so the value survives both drivers' timestamp
/// precision unchanged and compares exactly.
fn seconds_ago(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(Utc::now().timestamp() - seconds, 0).unwrap()
}

fn token(
    user_id: Uuid,
    tenant_id: &TenantId,
    provider: &str,
    created_at: DateTime<Utc>,
) -> UserOAuthToken {
    UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: provider.to_owned(),
        access_token: format!("access-{provider}-{user_id}"),
        refresh_token: Some(format!("refresh-{provider}-{user_id}")),
        token_type: "Bearer".to_owned(),
        expires_at: Some(seconds_ago(-6 * 3600)),
        scope: Some("read,activity:read_all".to_owned()),
        provider_user_id: Some(format!("athlete-{user_id}")),
        oauth_app_client_id: None,
        created_at,
        updated_at: created_at,
    }
}

#[tokio::test]
async fn a_backdated_token_keeps_the_timestamps_it_was_handed() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;
    let tenant_id = TenantId::generate();
    let created_at = seconds_ago(3 * 86_400);

    let mut minted = token(user_id, &tenant_id, "strava", created_at);
    minted.updated_at = seconds_ago(2 * 86_400);
    repos.oauth_tokens.upsert_token(&minted).await.unwrap();

    let stored = repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "strava")
        .await
        .unwrap()
        .expect("the token just stored must read back");
    assert_eq!(stored.id, minted.id);
    assert_eq!(stored.created_at, created_at, "created_at is the caller's");
    assert_eq!(
        stored.updated_at, minted.updated_at,
        "updated_at is the caller's"
    );
    assert_eq!(
        stored.access_token, minted.access_token,
        "decrypts to what was stored"
    );
    assert_eq!(stored.refresh_token, minted.refresh_token);
    assert_eq!(stored.expires_at, minted.expires_at);
    assert_eq!(stored.scope.as_deref(), Some("read,activity:read_all"));
    assert_eq!(stored.provider_user_id, minted.provider_user_id);
    assert_eq!(stored.oauth_app_client_id, None);
    assert_eq!(stored.token_type, "Bearer");
}

#[tokio::test]
async fn get_tokens_orders_by_the_stored_created_at_not_by_insertion() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;
    let tenant_id = TenantId::generate();

    // The older token is inserted last: an ordering by insertion time would
    // put it first, an ordering by the stored created_at puts it last.
    let newer = token(user_id, &tenant_id, "garmin", seconds_ago(60));
    let older = token(user_id, &tenant_id, "strava", seconds_ago(7 * 86_400));
    repos.oauth_tokens.upsert_token(&newer).await.unwrap();
    repos.oauth_tokens.upsert_token(&older).await.unwrap();

    let in_tenant = repos
        .oauth_tokens
        .get_tokens(user_id, Some(tenant_id))
        .await
        .unwrap();
    let providers: Vec<&str> = in_tenant.iter().map(|t| t.provider.as_str()).collect();
    assert_eq!(
        providers,
        vec!["garmin", "strava"],
        "newest created_at first"
    );

    let across_tenants = repos.oauth_tokens.get_tokens(user_id, None).await.unwrap();
    let providers: Vec<&str> = across_tenants.iter().map(|t| t.provider.as_str()).collect();
    assert_eq!(providers, vec!["garmin", "strava"]);

    let for_provider = repos
        .oauth_tokens
        .get_tenant_provider_tokens(tenant_id, "strava")
        .await
        .unwrap();
    assert_eq!(for_provider.len(), 1);
    assert_eq!(for_provider[0].created_at, older.created_at);
}

#[tokio::test]
async fn a_refreshed_token_round_trips_its_new_pair() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;
    let tenant_id = TenantId::generate();

    let mut expired = token(user_id, &tenant_id, "strava", seconds_ago(7 * 3600));
    expired.expires_at = Some(seconds_ago(3600));
    repos.oauth_tokens.upsert_token(&expired).await.unwrap();

    let renewed_until = seconds_ago(-6 * 3600);
    repos
        .oauth_tokens
        .refresh_token(
            user_id,
            tenant_id,
            "strava",
            "access-after-refresh",
            Some("refresh-after-refresh"),
            Some(renewed_until),
        )
        .await
        .unwrap();

    let stored = repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "strava")
        .await
        .unwrap()
        .expect("the refreshed token must still be there");
    assert_eq!(stored.access_token, "access-after-refresh");
    assert_eq!(
        stored.refresh_token.as_deref(),
        Some("refresh-after-refresh")
    );
    assert_eq!(stored.expires_at, Some(renewed_until));
    assert_eq!(
        stored.created_at, expired.created_at,
        "a refresh keeps the original row"
    );
    assert!(
        stored.updated_at > expired.updated_at,
        "a refresh stamps updated_at"
    );
    assert_eq!(
        stored.scope, expired.scope,
        "a refresh leaves the grant alone"
    );

    // A provider that rotates nothing hands back no refresh token; the row
    // then holds none rather than the stale one.
    repos
        .oauth_tokens
        .refresh_token(user_id, tenant_id, "strava", "access-third", None, None)
        .await
        .unwrap();
    let stored = repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "strava")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.access_token, "access-third");
    assert_eq!(stored.refresh_token, None);
    assert_eq!(stored.expires_at, None);

    // The refresh is scoped: another tenant's row for the same user and
    // provider is untouched.
    let other_tenant = TenantId::generate();
    let other = token(user_id, &other_tenant, "strava", seconds_ago(10));
    repos.oauth_tokens.upsert_token(&other).await.unwrap();
    repos
        .oauth_tokens
        .refresh_token(user_id, tenant_id, "strava", "access-fourth", None, None)
        .await
        .unwrap();
    let untouched = repos
        .oauth_tokens
        .get_token(user_id, other_tenant, "strava")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(untouched.access_token, other.access_token);
}

#[tokio::test]
async fn a_byo_app_frees_the_shared_app_seat_it_occupied() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let tenant_id = TenantId::generate();
    let pooled = fresh_user(&repos).await;
    let byo = fresh_user(&repos).await;

    let mut pooled_token = token(pooled, &tenant_id, "strava", seconds_ago(30));
    pooled_token.oauth_app_client_id = Some("201455".to_owned());
    repos
        .oauth_tokens
        .upsert_token(&pooled_token)
        .await
        .unwrap();
    let byo_token = token(byo, &tenant_id, "strava", seconds_ago(20));
    repos.oauth_tokens.upsert_token(&byo_token).await.unwrap();

    assert_eq!(
        repos
            .oauth_tokens
            .count_shared_app_seat_usage("strava")
            .await
            .unwrap(),
        2,
        "both users sit on the shared app before either registers their own"
    );

    repos
        .oauth_tokens
        .store_user_oauth_app(
            byo,
            "strava",
            "byo-client-id",
            "byo-client-secret",
            "https://byo.example/callback",
        )
        .await
        .unwrap();

    assert_eq!(
        repos
            .oauth_tokens
            .count_shared_app_seat_usage("strava")
            .await
            .unwrap(),
        1,
        "a user with their own app no longer counts against the shared app"
    );
    let mut by_app = repos
        .oauth_tokens
        .count_strava_seat_usage_by_app()
        .await
        .unwrap();
    by_app.sort();
    assert_eq!(by_app, vec![(Some("201455".to_owned()), 1)]);

    let app = repos
        .oauth_tokens
        .get_user_oauth_app(byo, "strava")
        .await
        .unwrap()
        .expect("the BYO app just registered must read back");
    assert_eq!(app.user_id, byo);
    assert_eq!(app.client_id, "byo-client-id");
    assert_eq!(app.client_secret, "byo-client-secret");
    assert_eq!(app.redirect_uri, "https://byo.example/callback");
    assert_eq!(
        app.created_at, app.updated_at,
        "a first store stamps both the same"
    );
    assert!(
        app.created_at > seconds_ago(60),
        "the stamp is the store's clock"
    );
    assert_eq!(
        repos
            .oauth_tokens
            .list_user_oauth_apps(byo)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(repos
        .oauth_tokens
        .get_user_oauth_app(pooled, "strava")
        .await
        .unwrap()
        .is_none());

    repos
        .oauth_tokens
        .remove_user_oauth_app(byo, "strava")
        .await
        .unwrap();
    assert_eq!(
        repos
            .oauth_tokens
            .count_shared_app_seat_usage("strava")
            .await
            .unwrap(),
        2,
        "removing the BYO app puts the user back on the shared app"
    );
}

#[tokio::test]
async fn a_sync_stamp_is_absent_until_written_and_scoped_to_its_row() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;
    let tenant_id = TenantId::generate();
    repos
        .oauth_tokens
        .upsert_token(&token(user_id, &tenant_id, "strava", seconds_ago(5)))
        .await
        .unwrap();
    repos
        .oauth_tokens
        .upsert_token(&token(user_id, &tenant_id, "garmin", seconds_ago(5)))
        .await
        .unwrap();

    assert_eq!(
        repos
            .oauth_tokens
            .get_provider_last_sync(user_id, tenant_id, "strava")
            .await
            .unwrap(),
        None,
        "a never-synced row reads as absent, not as an error"
    );
    assert_eq!(
        repos
            .oauth_tokens
            .get_provider_last_sync(user_id, tenant_id, "whoop")
            .await
            .unwrap(),
        None,
        "a missing row reads as absent too"
    );

    let synced_at = seconds_ago(90);
    repos
        .oauth_tokens
        .update_provider_last_sync(user_id, tenant_id, "strava", synced_at)
        .await
        .unwrap();
    assert_eq!(
        repos
            .oauth_tokens
            .get_provider_last_sync(user_id, tenant_id, "strava")
            .await
            .unwrap(),
        Some(synced_at)
    );
    assert_eq!(
        repos
            .oauth_tokens
            .get_provider_last_sync(user_id, tenant_id, "garmin")
            .await
            .unwrap(),
        None,
        "the stamp lands on one provider's row only"
    );

    let (owner, owner_tenant) = repos
        .oauth_tokens
        .find_user_by_provider_user_id("strava", &format!("athlete-{user_id}"))
        .await
        .unwrap()
        .expect("the provider-side id maps back to its owner");
    assert_eq!(owner, user_id);
    assert_eq!(owner_tenant, tenant_id.to_string());

    repos
        .oauth_tokens
        .delete_token(user_id, tenant_id, "strava")
        .await
        .unwrap();
    assert!(repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "strava")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        repos
            .oauth_tokens
            .get_tokens(user_id, Some(tenant_id))
            .await
            .unwrap()
            .len(),
        1,
        "delete_token drops one provider's row"
    );
    repos
        .oauth_tokens
        .delete_tokens(user_id, tenant_id)
        .await
        .unwrap();
    assert!(repos
        .oauth_tokens
        .get_tokens(user_id, Some(tenant_id))
        .await
        .unwrap()
        .is_empty());
}
