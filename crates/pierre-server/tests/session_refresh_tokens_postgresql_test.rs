// ABOUTME: PostgreSQL-lane repository tests for first-party refresh tokens — the UUID user_id bind and the RETURNING decode
// ABOUTME: SQLite keeps user_id as TEXT, so only this lane proves the native-UUID bind and the TIMESTAMPTZ round trip
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `PostgreSQL` session refresh token tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "postgresql")]

use chrono::{Duration, Utc};
use pierre_core::models::{CoachingPersona, SessionRefreshToken, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use uuid::Uuid;

async fn seed_pg_user(db: &Database) -> Uuid {
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: format!("refresh-{user_id}@test.local"),
        display_name: Some("Session Refresh Test".to_owned()),
        password_hash: "hash_not_verified".to_owned(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Utc::now()),
        created_at: Utc::now(),
        last_active: Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    };
    db.repositories().users.create(&user).await.unwrap();
    user_id
}

fn record(user_id: Uuid, family_id: &str) -> SessionRefreshToken {
    let now = Utc::now();
    SessionRefreshToken {
        family_id: family_id.to_owned(),
        user_id,
        tenant_id: None,
        created_at: now,
        expires_at: now + Duration::days(30),
    }
}

/// The exchange this lane exists for: `user_id` is bound as a native UUID and
/// decoded back from `RETURNING`, the timestamps survive TIMESTAMPTZ, and a
/// NULL `tenant_id` comes back as `None` rather than failing the decode.
#[tokio::test]
async fn test_pg_exchange_round_trips_the_record() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = seed_pg_user(&db).await;

    let stored = record(user_id, "family-pg");
    repos
        .session_refresh_tokens
        .store_token("token-pg", &stored)
        .await
        .unwrap();

    let consumed = repos
        .session_refresh_tokens
        .consume_token("token-pg", Utc::now())
        .await
        .unwrap()
        .expect("a live token exchanges");
    assert_eq!(consumed.family_id, "family-pg");
    assert_eq!(consumed.user_id, user_id);
    assert_eq!(consumed.tenant_id, None);
    assert_eq!(
        consumed.expires_at.timestamp_millis(),
        stored.expires_at.timestamp_millis()
    );

    let again = repos
        .session_refresh_tokens
        .consume_token("token-pg", Utc::now())
        .await
        .unwrap();
    assert!(again.is_none(), "a token exchanges exactly once");
}

#[tokio::test]
async fn test_pg_revocation_by_family_and_by_user() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = seed_pg_user(&db).await;
    let other_user = seed_pg_user(&db).await;

    repos
        .session_refresh_tokens
        .store_token("a1", &record(user_id, "family-a"))
        .await
        .unwrap();
    repos
        .session_refresh_tokens
        .consume_token("a1", Utc::now())
        .await
        .unwrap()
        .expect("a1 exchanges");
    repos
        .session_refresh_tokens
        .store_token("a2", &record(user_id, "family-a"))
        .await
        .unwrap();
    repos
        .session_refresh_tokens
        .store_token("b1", &record(user_id, "family-b"))
        .await
        .unwrap();
    repos
        .session_refresh_tokens
        .store_token("o1", &record(other_user, "family-other"))
        .await
        .unwrap();

    // Replaying the rotated-out a1 kills the live a2 and nothing else.
    let revoked = repos
        .session_refresh_tokens
        .revoke_token_family("a1", Utc::now())
        .await
        .unwrap();
    assert_eq!(revoked, 1);
    assert!(repos
        .session_refresh_tokens
        .consume_token("a2", Utc::now())
        .await
        .unwrap()
        .is_none());

    // The user's remaining device ends with the password; the other user's
    // does not — the UUID bind on `user_id` is what scopes this UPDATE.
    let revoked = repos
        .session_refresh_tokens
        .revoke_user_tokens(user_id, Utc::now())
        .await
        .unwrap();
    assert_eq!(revoked, 1, "only b1 was still live for this user");
    assert!(repos
        .session_refresh_tokens
        .consume_token("b1", Utc::now())
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        repos
            .session_refresh_tokens
            .consume_token("o1", Utc::now())
            .await
            .unwrap()
            .map(|record| record.user_id),
        Some(other_user)
    );
}
