// ABOUTME: Covers the user MCP token repository against whichever backend DATABASE_URL names
// ABOUTME: Mints, validates, lists, reads back, revokes and sweeps tokens, holding each to its user scope
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `user_mcp_tokens` authenticates AI clients on the MCP transport. Every
//! read and write is scoped by the owning `user_id`, a `uuid` column on
//! `PostgreSQL` and `TEXT` on `SQLite`, and both backends must agree on
//! what a token looks like when it comes back.
//!
//! These run on `SQLite` and on `PostgreSQL`: `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration, Utc};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{CreateUserMcpTokenRequest, User};
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// A distinct user per call; the table's foreign key needs the row to exist.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("mcp-token-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("MCP Token Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

#[tokio::test]
async fn a_minted_token_reads_back_and_validates_to_its_owner() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = fresh_user(&repos).await;

    let created = repos
        .user_mcp_tokens
        .create_token(
            owner,
            &CreateUserMcpTokenRequest {
                name: "laptop".to_owned(),
                expires_in_days: Some(30),
            },
        )
        .await
        .unwrap();
    assert!(created.token_value.starts_with("pmcp_"));
    assert_eq!(created.token.user_id, owner);
    assert_eq!(created.token.usage_count, 0);

    let stored = repos
        .user_mcp_tokens
        .get_token(&created.token.id, owner)
        .await
        .unwrap()
        .expect("the token just minted must read back for its owner");
    assert_eq!(stored.id, created.token.id);
    assert_eq!(stored.user_id, owner);
    assert_eq!(stored.name, "laptop");
    assert_eq!(stored.token_hash, created.token.token_hash);
    assert_eq!(stored.token_prefix, created.token.token_prefix);
    assert!(!stored.is_revoked);
    assert_eq!(stored.last_used_at, None);
    let expires_at = stored.expires_at.expect("a 30-day token carries an expiry");
    let expected = Utc::now() + Duration::days(30);
    assert!(
        (expires_at - expected).num_seconds().abs() < 60,
        "expiry lands 30 days out, got {expires_at}"
    );

    let stranger = fresh_user(&repos).await;
    assert!(
        repos
            .user_mcp_tokens
            .get_token(&created.token.id, stranger)
            .await
            .unwrap()
            .is_none(),
        "another user cannot read the token by id"
    );

    assert_eq!(
        repos
            .user_mcp_tokens
            .validate_token(&created.token_value)
            .await
            .unwrap(),
        owner,
        "the raw token resolves to its owner"
    );
    let used = repos
        .user_mcp_tokens
        .get_token(&created.token.id, owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(used.usage_count, 1, "validation counts one use");
    assert!(
        used.last_used_at.is_some(),
        "validation stamps last_used_at"
    );
}

#[tokio::test]
async fn list_tokens_is_scoped_to_the_owner_and_newest_first() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = fresh_user(&repos).await;
    let other = fresh_user(&repos).await;

    for name in ["first", "second"] {
        repos
            .user_mcp_tokens
            .create_token(
                owner,
                &CreateUserMcpTokenRequest {
                    name: name.to_owned(),
                    expires_in_days: None,
                },
            )
            .await
            .unwrap();
    }
    repos
        .user_mcp_tokens
        .create_token(
            other,
            &CreateUserMcpTokenRequest {
                name: "theirs".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .unwrap();

    let listed = repos.user_mcp_tokens.list_tokens(owner).await.unwrap();
    let names: Vec<&str> = listed.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["second", "first"], "only the owner's, newest first");
    assert!(
        listed.iter().all(|t| t.expires_at.is_none()),
        "a token minted without a window never expires"
    );
}

#[tokio::test]
async fn revoke_is_scoped_to_the_owner_and_stops_validation() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = fresh_user(&repos).await;
    let stranger = fresh_user(&repos).await;

    let created = repos
        .user_mcp_tokens
        .create_token(
            owner,
            &CreateUserMcpTokenRequest {
                name: "phone".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .unwrap();

    let refused = repos
        .user_mcp_tokens
        .revoke_token(&created.token.id, stranger)
        .await
        .expect_err("another user cannot revoke the token");
    assert_eq!(refused.code, ErrorCode::ResourceNotFound);

    repos
        .user_mcp_tokens
        .revoke_token(&created.token.id, owner)
        .await
        .unwrap();
    let denied = repos
        .user_mcp_tokens
        .validate_token(&created.token_value)
        .await
        .expect_err("a revoked token no longer validates");
    assert_eq!(denied.code, ErrorCode::AuthInvalid);
    assert!(
        repos.user_mcp_tokens.list_tokens(owner).await.unwrap()[0].is_revoked,
        "the listing shows the token as revoked"
    );
}

#[tokio::test]
async fn cleanup_revokes_only_the_expired_tokens() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = fresh_user(&repos).await;

    let live = repos
        .user_mcp_tokens
        .create_token(
            owner,
            &CreateUserMcpTokenRequest {
                name: "live".to_owned(),
                expires_in_days: Some(7),
            },
        )
        .await
        .unwrap();
    let expired = repos
        .user_mcp_tokens
        .create_token(
            owner,
            &CreateUserMcpTokenRequest {
                name: "expired".to_owned(),
                expires_in_days: Some(0),
            },
        )
        .await
        .unwrap();

    let denied = repos
        .user_mcp_tokens
        .validate_token(&expired.token_value)
        .await
        .expect_err("a token past its window no longer validates");
    assert_eq!(denied.code, ErrorCode::AuthInvalid);

    assert_eq!(
        repos
            .user_mcp_tokens
            .cleanup_expired_tokens()
            .await
            .unwrap(),
        1,
        "exactly the expired token is swept"
    );
    assert_eq!(
        repos
            .user_mcp_tokens
            .cleanup_expired_tokens()
            .await
            .unwrap(),
        0,
        "a second sweep finds nothing"
    );
    assert_eq!(
        repos
            .user_mcp_tokens
            .validate_token(&live.token_value)
            .await
            .unwrap(),
        owner,
        "the live token still validates"
    );
}
