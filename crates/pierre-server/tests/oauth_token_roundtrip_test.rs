// ABOUTME: Verifies a seeded OAuth token round-trips through upsert_token -> get_tokens
// ABOUTME: Pins whether the recommender's provider gate can see a seeded provider token
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use chrono::Utc;
use common::{create_test_server_resources, create_test_user};
use pierre_core::models::UserOAuthToken;
use uuid::Uuid;

/// The personalized coach recommender's provider gate reads
/// `oauth_tokens.get_tokens`. This asserts a token seeded via `upsert_token`
/// (the path a seeder would use) is decryptable and surfaced by `get_tokens`,
/// so the gate sees the provider name.
#[tokio::test]
async fn seeded_oauth_token_roundtrips_through_get_tokens() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.coach.database).await.unwrap();

    let tenant = resources
        .common
        .repos
        .tenants
        .get_all()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("test user should own a tenant");

    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant.id.to_string(),
        provider: "strava".to_owned(),
        access_token: "seed-access-token".to_owned(),
        refresh_token: Some("seed-refresh-token".to_owned()),
        token_type: "bearer".to_owned(),
        expires_at: Some(now + chrono::Duration::days(3650)),
        scope: Some("read,activity:read_all".to_owned()),
        provider_user_id: None,
        created_at: now,
        updated_at: now,
    };

    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .expect("upsert_token should succeed");

    let tokens = resources
        .common
        .repos
        .oauth_tokens
        .get_tokens(user_id, None)
        .await
        .expect("get_tokens should not error (AAD/decrypt must match)");

    let providers: Vec<String> = tokens.iter().map(|t| t.provider.to_lowercase()).collect();
    assert!(
        providers.contains(&"strava".to_owned()),
        "get_tokens must surface the seeded strava token; got {providers:?}"
    );

    let strava = tokens.iter().find(|t| t.provider == "strava").unwrap();
    assert_eq!(
        strava.access_token, "seed-access-token",
        "decrypted access_token must match the seeded value"
    );
}

/// Intervals.icu stores the athlete id in the `provider_user_id` column (the
/// HTTP Basic username) alongside the encrypted API key. This pins that the
/// new column round-trips through `upsert_token` -> `get_token` so the serving
/// path can rebuild the credentials.
#[tokio::test]
async fn intervals_icu_token_roundtrips_provider_user_id() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.coach.database).await.unwrap();

    let tenant = resources
        .common
        .repos
        .tenants
        .get_all()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("test user should own a tenant");

    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant.id.to_string(),
        provider: "intervals_icu".to_owned(),
        access_token: "test-api-key".to_owned(),
        refresh_token: None,
        token_type: "api_key".to_owned(),
        expires_at: None,
        scope: None,
        provider_user_id: Some("i123456".to_owned()),
        created_at: now,
        updated_at: now,
    };

    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .expect("upsert_token should succeed");

    let fetched = resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant.id, "intervals_icu")
        .await
        .expect("get_token should not error")
        .expect("token should exist");

    assert_eq!(
        fetched.provider_user_id.as_deref(),
        Some("i123456"),
        "athlete id must round-trip through the provider_user_id column"
    );
    assert_eq!(
        fetched.access_token, "test-api-key",
        "decrypted API key must match the stored value"
    );
}
