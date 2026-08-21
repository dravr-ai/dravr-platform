// ABOUTME: Tests the single-flight guard that dedups background activity-cache revalidations
// ABOUTME: One in-flight refresh per (user, tenant); the slot frees when the guard drops
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Tests for [`pierre_tool_runtime::group_activity_cache::RevalidationRegistry`].
//!
//! The stale-while-revalidate path spawns a background refresh when a user's
//! cached activities are stale. Concurrent chat turns must collapse onto a
//! single refresh — the per-profile Chrome `SingletonLock` serializes scrapes,
//! so N un-deduped revalidations would queue N ~2-minute scrapes behind one
//! lock. These tests pin the claim/release behavior that prevents that.

use pierre_core::models::TenantId;
use pierre_tool_runtime::group_activity_cache::RevalidationRegistry;
use uuid::Uuid;

fn key() -> (Uuid, TenantId) {
    (Uuid::new_v4(), TenantId::generate())
}

#[test]
fn second_claim_for_same_key_is_dropped_while_first_is_held() {
    let registry = RevalidationRegistry::new();
    let k = key();

    let first = registry.try_claim(k);
    assert!(first.is_some(), "first claim should win the slot");

    let second = registry.try_claim(k);
    assert!(
        second.is_none(),
        "a concurrent claim for the same key must be dropped, not queued"
    );
}

#[test]
fn slot_frees_when_guard_drops() {
    let registry = RevalidationRegistry::new();
    let k = key();

    {
        let guard = registry.try_claim(k);
        assert!(guard.is_some(), "first claim should win the slot");
        // guard dropped at end of scope
    }

    let after = registry.try_claim(k);
    assert!(
        after.is_some(),
        "the slot must free once the in-flight revalidation finishes"
    );
}

#[test]
fn distinct_keys_do_not_block_each_other() {
    let registry = RevalidationRegistry::new();
    let a = key();
    let b = key();

    let guard_a = registry.try_claim(a);
    let guard_b = registry.try_claim(b);

    assert!(guard_a.is_some(), "user A claims its own slot");
    assert!(
        guard_b.is_some(),
        "a different (user, tenant) must claim independently"
    );
}

#[test]
fn same_user_different_tenant_is_independent() {
    let registry = RevalidationRegistry::new();
    let user = Uuid::new_v4();
    let tenant_a = (user, TenantId::generate());
    let tenant_b = (user, TenantId::generate());

    let guard_a = registry.try_claim(tenant_a);
    let guard_b = registry.try_claim(tenant_b);

    assert!(guard_a.is_some(), "same user under tenant A claims a slot");
    assert!(
        guard_b.is_some(),
        "the same user under a different tenant must not collide"
    );
}
