// ABOUTME: Verifies the Strava shared-app OAuth pool repository — CRUD, encrypted-secret roundtrip, per-app seat counting
// ABOUTME: Pins that pool app secrets are recoverable and that seat usage is grouped by issuing app (None = env default)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use chrono::Utc;
use common::{create_test_server_resources, create_test_user};
use pierre_core::models::UserOAuthToken;

#[tokio::test]
async fn strava_pool_app_crud_and_secret_roundtrip() {
    let resources = create_test_server_resources().await.unwrap();
    let repo = &resources.common.repos.oauth_tokens;

    // Insert a pool app; the secret must be stored ENCRYPTED and recoverable.
    let secret = "0f2b184c076e60a35e8ced43db9c3c20c5fcf4f3"; // 40-char Strava-shaped secret
    repo.upsert_strava_pool_app("201455", secret, 10, Some("dravr-app-2"))
        .await
        .expect("upsert pool app");

    let apps = repo
        .list_strava_pool_apps(true)
        .await
        .expect("list enabled");
    assert_eq!(apps.len(), 1, "one enabled pool app");
    assert_eq!(apps[0].client_id, "201455");
    assert_eq!(apps[0].seat_cap, 10);
    assert!(apps[0].enabled);
    assert_eq!(apps[0].label.as_deref(), Some("dravr-app-2"));

    // Secret decrypts back to the original (AES-256-GCM roundtrip, AAD-bound).
    let got = repo
        .get_strava_pool_app_secret("201455")
        .await
        .expect("get secret");
    assert_eq!(
        got.as_deref(),
        Some(secret),
        "secret roundtrips through encryption"
    );
    assert!(
        repo.get_strava_pool_app_secret("nope")
            .await
            .unwrap()
            .is_none(),
        "unknown client_id yields None"
    );

    // Update: new secret + cap should replace, not duplicate.
    repo.upsert_strava_pool_app(
        "201455",
        "newsecretvaluewithlength30chars",
        20,
        Some("dravr-app-2"),
    )
    .await
    .expect("update pool app");
    let apps = repo.list_strava_pool_apps(true).await.unwrap();
    assert_eq!(apps.len(), 1, "upsert updates in place");
    assert_eq!(apps[0].seat_cap, 20);
    assert_eq!(
        repo.get_strava_pool_app_secret("201455")
            .await
            .unwrap()
            .as_deref(),
        Some("newsecretvaluewithlength30chars")
    );

    // Disable hides it from the enabled list but not the full list.
    repo.set_strava_pool_app_enabled("201455", false)
        .await
        .unwrap();
    assert!(repo.list_strava_pool_apps(true).await.unwrap().is_empty());
    let all = repo.list_strava_pool_apps(false).await.unwrap();
    assert_eq!(all.len(), 1);
    assert!(!all[0].enabled);

    // Delete removes it entirely.
    repo.delete_strava_pool_app("201455").await.unwrap();
    assert!(repo.list_strava_pool_apps(false).await.unwrap().is_empty());
}

#[tokio::test]
async fn strava_seat_usage_is_grouped_by_issuing_app() {
    let resources = create_test_server_resources().await.unwrap();
    let repo = &resources.common.repos.oauth_tokens;
    let (user_id, _u) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant = resources
        .common
        .repos
        .tenants
        .get_all()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("test user owns a tenant");

    // A strava token attributed to a specific pool app.
    let pool_token = UserOAuthToken::new(
        user_id,
        tenant.id.to_string(),
        "strava".to_owned(),
        "acc".to_owned(),
        Some("ref".to_owned()),
        Some(Utc::now() + chrono::Duration::days(3650)),
        Some("read".to_owned()),
    )
    .with_oauth_app_client_id(Some("201455".to_owned()));
    repo.upsert_token(&pool_token).await.unwrap();

    let usage = repo
        .count_strava_seat_usage_by_app()
        .await
        .expect("count by app");
    // The athlete is attributed to app 201455 (its own bucket), not the env/None
    // bucket. Robust against any other strava tokens the harness may have seeded.
    let app_bucket = usage
        .iter()
        .find(|(app, _)| app.as_deref() == Some("201455"));
    assert_eq!(
        app_bucket.map(|(_, n)| *n),
        Some(1),
        "pool app 201455 has exactly one athlete; buckets = {usage:?}"
    );
}
