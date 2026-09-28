// ABOUTME: Pins the hourly API-key sweep: it deactivates expired keys and leaves live ones alone
// ABOUTME: The sweep used to live in a HealthChecker nothing constructed, so it never ran in production
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{DateTime, Duration, Utc};
use pierre_auth::api_keys::{ApiKey, ApiKeyTier};
use pierre_core::models::User;
use pierre_database::database::test_utils::create_test_db;
use pierre_services::api_key_cleanup::sweep_expired_api_keys;
use uuid::Uuid;

fn key(user_id: Uuid, expires_at: Option<DateTime<Utc>>) -> ApiKey {
    let id = Uuid::new_v4();
    ApiKey {
        id: format!("sweep_{id}"),
        user_id,
        name: format!("Sweep Key {id}"),
        description: None,
        key_hash: format!("sweep_hash_{id}"),
        key_prefix: format!("swp_{}_", id.simple()),
        tier: ApiKeyTier::Trial,
        rate_limit_requests: 10,
        rate_limit_window_seconds: 3600,
        is_active: true,
        expires_at,
        last_used_at: None,
        created_at: Utc::now() - Duration::days(1),
    }
}

#[tokio::test]
async fn the_sweep_deactivates_expired_keys_and_keeps_live_ones() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = User::new(
        "sweep@example.com".to_owned(),
        "hash".to_owned(),
        Some("Sweep".to_owned()),
    );
    repos.users.create(&user).await.unwrap();

    let expired = key(user.id, Some(Utc::now() - Duration::hours(2)));
    let live = key(user.id, Some(Utc::now() + Duration::days(30)));
    let unbounded = key(user.id, None);
    for k in [&expired, &live, &unbounded] {
        repos.api_keys.create(k).await.unwrap();
    }

    assert_eq!(
        sweep_expired_api_keys(repos.api_keys.as_ref())
            .await
            .unwrap(),
        1
    );

    for (k, expected) in [(&expired, false), (&live, true), (&unbounded, true)] {
        let stored = repos
            .api_keys
            .get_by_id(&k.id, None)
            .await
            .unwrap()
            .expect("the key row exists");
        assert_eq!(stored.is_active, expected, "{}", k.name);
    }

    // A second pass finds nothing left to do.
    assert_eq!(
        sweep_expired_api_keys(repos.api_keys.as_ref())
            .await
            .unwrap(),
        0
    );
}
