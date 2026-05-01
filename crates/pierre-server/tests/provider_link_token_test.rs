// ABOUTME: Tests for short-lived signed link-tokens used by channel-initiated provider flows
// ABOUTME: Verifies HS256 mint/verify roundtrips, scope clamping, and secret isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;

use pierre_cache::CacheConfig;
use pierre_mcp_server::cache::factory::Cache;
use pierre_mcp_server::middleware::provider_link_token::{
    extract_bearer_link_token, mint_link_token, provider_scope, verify_link_token,
    MintProviderLinkTokenArgs, MintRateLimiter, NonceStore,
};
use uuid::Uuid;

const TEST_SECRET: &str = "unit-test-admin-jwt-secret-value";

fn sample_ids() -> (Uuid, Uuid) {
    (
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
        Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap(),
    )
}

/// Create a test cache (in-memory, no background cleanup).
async fn test_cache() -> Arc<Cache> {
    let config = CacheConfig {
        enable_background_cleanup: false,
        ..CacheConfig::default()
    };
    Arc::new(Cache::new(config).await.unwrap())
}

#[test]
fn mint_and_verify_roundtrip_recovers_claims() {
    let (user_id, tenant_id) = sample_ids();
    let token = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id,
            provider: "sciotte",
            target: "strava",
            channel: "slack",
            channel_thread: Some("C0123#thread-99"),
        },
        TEST_SECRET,
    )
    .unwrap();

    let claims = verify_link_token(&token, TEST_SECRET, "sciotte").unwrap();
    assert_eq!(claims.sub, user_id.to_string());
    assert_eq!(claims.tid, tenant_id.to_string());
    assert_eq!(claims.tgt, "strava");
    assert_eq!(claims.channel, "slack");
    assert_eq!(claims.channel_thread.as_deref(), Some("C0123#thread-99"));
    assert_eq!(claims.scope, "provider:sciotte:login");
}

#[test]
fn verify_rejects_wrong_scope() {
    let (user_id, tenant_id) = sample_ids();
    let token = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id,
            provider: "sciotte",
            target: "strava",
            channel: "slack",
            channel_thread: None,
        },
        TEST_SECRET,
    )
    .unwrap();

    let err = verify_link_token(&token, TEST_SECRET, "other_provider").unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("scope"), "unexpected error: {msg}");
}

#[test]
fn verify_rejects_wrong_secret() {
    let (user_id, tenant_id) = sample_ids();
    let token = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id,
            provider: "sciotte",
            target: "strava",
            channel: "slack",
            channel_thread: None,
        },
        TEST_SECRET,
    )
    .unwrap();

    assert!(verify_link_token(&token, "wrong-secret", "sciotte").is_err());
}

#[test]
fn extract_bearer_link_token_handles_prefix() {
    assert_eq!(
        extract_bearer_link_token(Some("Bearer abc.def.ghi")),
        Some("abc.def.ghi")
    );
    assert_eq!(extract_bearer_link_token(Some("abc.def.ghi")), None);
    assert_eq!(extract_bearer_link_token(None), None);
}

#[test]
fn provider_scope_is_deterministic() {
    assert_eq!(provider_scope("sciotte"), "provider:sciotte:login");
    assert_eq!(provider_scope("garmin"), "provider:garmin:login");
}

#[tokio::test]
async fn nonce_store_burns_jti_once() {
    let cache = test_cache().await;
    let store = NonceStore::new(cache);
    let jti = "unique-jti-abc-123";

    assert!(store.burn(jti).await.is_ok(), "first burn should succeed");

    let err = store.burn(jti).await.unwrap_err();
    assert!(
        format!("{err}").contains("already been used"),
        "second burn should reject: {err}"
    );
}

#[tokio::test]
async fn nonce_store_accepts_distinct_jtis() {
    let cache = test_cache().await;
    let store = NonceStore::new(cache);

    assert!(store.burn("jti-1").await.is_ok());
    assert!(store.burn("jti-2").await.is_ok());
    assert!(store.burn("jti-3").await.is_ok());

    // Each jti is independently burned — re-burning any one still fails.
    assert!(store.burn("jti-1").await.is_err());
    assert!(store.burn("jti-2").await.is_err());
    assert!(store.burn("jti-3").await.is_err());
}

#[tokio::test]
async fn mint_rate_limiter_allows_up_to_limit() {
    let cache = test_cache().await;
    let limiter = MintRateLimiter::new(3, Duration::from_hours(1), cache);
    let user = Uuid::new_v4();

    assert!(limiter.record_attempt(user).await.is_ok());
    assert!(limiter.record_attempt(user).await.is_ok());
    assert!(limiter.record_attempt(user).await.is_ok());

    let err = limiter.record_attempt(user).await.unwrap_err();
    assert!(
        format!("{err}").contains("Too many"),
        "over-limit should reject: {err}"
    );
}

#[tokio::test]
async fn mint_rate_limiter_buckets_per_user() {
    let cache = test_cache().await;
    let limiter = MintRateLimiter::new(1, Duration::from_hours(1), cache);
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    assert!(limiter.record_attempt(alice).await.is_ok());
    assert!(limiter.record_attempt(alice).await.is_err());
    // Bob is a separate bucket
    assert!(limiter.record_attempt(bob).await.is_ok());
    assert!(limiter.record_attempt(bob).await.is_err());
}

#[tokio::test]
async fn mint_rate_limiter_expires_hits() {
    let cache = test_cache().await;
    // Window must be >= 2 seconds because the rate limiter uses second-precision
    // timestamps (Utc::now().timestamp()). Sub-second windows round to 0.
    let limiter = MintRateLimiter::new(1, Duration::from_secs(2), cache);
    let user = Uuid::new_v4();

    assert!(limiter.record_attempt(user).await.is_ok());
    assert!(limiter.record_attempt(user).await.is_err());

    sleep(Duration::from_secs(3)).await;
    assert!(
        limiter.record_attempt(user).await.is_ok(),
        "window should have expired"
    );
}
