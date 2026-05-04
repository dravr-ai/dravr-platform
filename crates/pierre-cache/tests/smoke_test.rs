// ABOUTME: Smoke test for pierre-cache's public API (CacheProvider, CacheKey, redaction)
// ABOUTME: Exercises the in-memory cache backend without touching Redis
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Smoke integration tests for the crate public API.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::time::Duration;

use pierre_cache::memory::InMemoryCache;
use pierre_cache::redaction::redact_url;
use pierre_cache::{CacheConfig, CacheKey, CacheProvider, CacheResource};
use pierre_core::models::TenantId;
use uuid::Uuid;

#[test]
fn redact_url_strips_credentials() {
    let redacted = redact_url("postgres://alice:s3cret@db.example.com:5432/pierre");
    assert!(
        !redacted.contains("s3cret"),
        "redaction must strip the password component: {redacted}"
    );
    assert!(
        !redacted.contains("alice"),
        "redaction must strip the username component: {redacted}"
    );
}

#[test]
fn cache_key_distinguishes_tenants() {
    let tenant_a = TenantId::new();
    let tenant_b = TenantId::new();
    let user_id = Uuid::new_v4();

    let key_a = CacheKey {
        tenant_id: tenant_a,
        user_id,
        provider: "strava".to_owned(),
        resource: CacheResource::AthleteProfile,
    };
    let key_b = CacheKey {
        tenant_id: tenant_b,
        user_id,
        provider: "strava".to_owned(),
        resource: CacheResource::AthleteProfile,
    };

    assert_ne!(
        key_a, key_b,
        "cache keys with different tenants must not compare equal"
    );
}

#[tokio::test]
async fn in_memory_cache_round_trips_a_value() {
    let cache = InMemoryCache::new(CacheConfig::default())
        .await
        .expect("in-memory cache should initialize with defaults");

    let key = CacheKey {
        tenant_id: TenantId::new(),
        user_id: Uuid::new_v4(),
        provider: "strava".to_owned(),
        resource: CacheResource::AthleteProfile,
    };
    let payload = serde_json::json!({ "name": "Pierre" });

    cache
        .set(&key, &payload, Duration::from_secs(60))
        .await
        .expect("cache set must succeed");

    let fetched = cache
        .get(&key)
        .await
        .expect("cache get must succeed for present key");

    assert_eq!(fetched, Some(payload));
}
