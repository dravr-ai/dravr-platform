// ABOUTME: Repository tests for first-party refresh tokens on SQLite — store, exchange once, revoke a family or a user
// ABOUTME: Pins the single-use exchange, the replay-revokes-the-family rule, and that revocation touches only live rows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, Utc};
use pierre_core::models::SessionRefreshToken;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

mod common;

/// A record in a fresh family for `user_id`, expiring 30 days out.
fn record(user_id: Uuid, family_id: &str) -> SessionRefreshToken {
    let now = Utc::now();
    SessionRefreshToken {
        family_id: family_id.to_owned(),
        user_id,
        tenant_id: Some("tenant-a".to_owned()),
        created_at: now,
        expires_at: now + Duration::days(30),
    }
}

async fn seeded() -> (Database, RepositoryRegistry, Uuid) {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();
    ((*database).clone(), repos, user_id)
}

#[tokio::test]
async fn exchange_returns_the_record_once() {
    let (_db, repos, user_id) = seeded().await;
    let stored = record(user_id, "family-1");
    repos
        .session_refresh_tokens
        .store_token("token-1", &stored)
        .await
        .unwrap();

    let consumed = repos
        .session_refresh_tokens
        .consume_token("token-1", Utc::now())
        .await
        .unwrap()
        .expect("a live token exchanges");
    assert_eq!(consumed.family_id, "family-1");
    assert_eq!(consumed.user_id, user_id);
    assert_eq!(consumed.tenant_id.as_deref(), Some("tenant-a"));

    // Consumed in the same statement that read it: the second exchange sees
    // a revoked row.
    let again = repos
        .session_refresh_tokens
        .consume_token("token-1", Utc::now())
        .await
        .unwrap();
    assert!(again.is_none(), "a token exchanges exactly once");
}

#[tokio::test]
async fn an_unknown_or_expired_token_does_not_exchange() {
    let (_db, repos, user_id) = seeded().await;

    let unknown = repos
        .session_refresh_tokens
        .consume_token("never-stored", Utc::now())
        .await
        .unwrap();
    assert!(unknown.is_none());

    let mut lapsed = record(user_id, "family-lapsed");
    lapsed.expires_at = Utc::now() - Duration::minutes(1);
    repos
        .session_refresh_tokens
        .store_token("token-lapsed", &lapsed)
        .await
        .unwrap();
    let expired = repos
        .session_refresh_tokens
        .consume_token("token-lapsed", Utc::now())
        .await
        .unwrap();
    assert!(expired.is_none(), "an expired token does not exchange");
}

#[tokio::test]
async fn revoking_a_token_family_kills_its_live_successor_only() {
    let (_db, repos, user_id) = seeded().await;
    // Family A: token-a1 exchanged, token-a2 live. Family B: token-b1 live.
    repos
        .session_refresh_tokens
        .store_token("token-a1", &record(user_id, "family-a"))
        .await
        .unwrap();
    repos
        .session_refresh_tokens
        .consume_token("token-a1", Utc::now())
        .await
        .unwrap()
        .expect("token-a1 exchanges");
    repos
        .session_refresh_tokens
        .store_token("token-a2", &record(user_id, "family-a"))
        .await
        .unwrap();
    repos
        .session_refresh_tokens
        .store_token("token-b1", &record(user_id, "family-b"))
        .await
        .unwrap();

    // Presenting the rotated-out token-a1 again is a replay: it revokes the
    // one live member of its family, and reports that it did.
    let revoked = repos
        .session_refresh_tokens
        .revoke_token_family("token-a1", Utc::now())
        .await
        .unwrap();
    assert_eq!(revoked, 1, "only the live successor was left to revoke");

    let successor = repos
        .session_refresh_tokens
        .consume_token("token-a2", Utc::now())
        .await
        .unwrap();
    assert!(
        successor.is_none(),
        "the replayed family's successor is dead"
    );

    // A second replay finds nothing live, which is how the service tells a
    // replay from a token it simply never issued.
    let again = repos
        .session_refresh_tokens
        .revoke_token_family("token-a1", Utc::now())
        .await
        .unwrap();
    assert_eq!(again, 0);

    // The other device's family was never touched.
    let other = repos
        .session_refresh_tokens
        .consume_token("token-b1", Utc::now())
        .await
        .unwrap();
    assert!(other.is_some(), "an unrelated family still exchanges");

    // A token nobody issued revokes nothing.
    let unknown = repos
        .session_refresh_tokens
        .revoke_token_family("never-stored", Utc::now())
        .await
        .unwrap();
    assert_eq!(unknown, 0);
}

#[tokio::test]
async fn revoking_a_user_ends_every_device_session_and_nobody_elses() {
    let (db, repos, user_id) = seeded().await;
    let (other_user, _) = common::create_test_user_with_email(&db, "other@example.com")
        .await
        .unwrap();

    repos
        .session_refresh_tokens
        .store_token("phone", &record(user_id, "family-phone"))
        .await
        .unwrap();
    repos
        .session_refresh_tokens
        .store_token("tablet", &record(user_id, "family-tablet"))
        .await
        .unwrap();
    repos
        .session_refresh_tokens
        .store_token("someone-else", &record(other_user, "family-other"))
        .await
        .unwrap();

    let revoked = repos
        .session_refresh_tokens
        .revoke_user_tokens(user_id, Utc::now())
        .await
        .unwrap();
    assert_eq!(revoked, 2, "both of the user's devices, nobody else's");

    for token in ["phone", "tablet"] {
        let consumed = repos
            .session_refresh_tokens
            .consume_token(token, Utc::now())
            .await
            .unwrap();
        assert!(consumed.is_none(), "{token} was revoked with its user");
    }
    let untouched = repos
        .session_refresh_tokens
        .consume_token("someone-else", Utc::now())
        .await
        .unwrap();
    assert_eq!(
        untouched.map(|record| record.user_id),
        Some(other_user),
        "another user's session still exchanges"
    );
}
