// ABOUTME: Tests for tenant-scoped cache key generation used by provider caching
// ABOUTME: Validates cache key resource encoding and time-filter serialization
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_cache::{CacheKey, CacheResource};
use pierre_core::models::TenantId;
use uuid::Uuid;

#[test]
fn test_cache_key_generation() {
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let provider = "strava";

    let key = CacheKey::new(
        tenant_id,
        user_id,
        provider.to_owned(),
        CacheResource::AthleteProfile,
    );

    let key_str = key.to_string();
    assert!(key_str.contains(&tenant_id.to_string()));
    assert!(key_str.contains(&user_id.to_string()));
    assert!(key_str.contains(provider));
    assert!(key_str.contains("athlete_profile"));
}

#[test]
fn test_activity_list_cache_key_with_time_filters() {
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();

    let key = CacheKey::new(
        tenant_id,
        user_id,
        "strava".to_owned(),
        CacheResource::ActivityList {
            page: 1,
            per_page: 50,
            before: Some(1_700_000_000),
            after: Some(1_600_000_000),
            sport_type: None,
        },
    );

    let key_str = key.to_string();
    assert!(key_str.contains("activity_list"));
    assert!(key_str.contains("page:1"));
    assert!(key_str.contains("per_page:50"));
    assert!(key_str.contains("before:1700000000"));
    assert!(key_str.contains("after:1600000000"));
}
