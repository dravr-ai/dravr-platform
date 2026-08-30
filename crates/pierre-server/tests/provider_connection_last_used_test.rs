// ABOUTME: Integration tests for ProviderConnectionRepository::{touch_last_used, resolve_most_recent}
// ABOUTME: Pins the resolver behavior that replaced the synthetic-provider silent fallback on 2026-05-23
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Pins the behavior of the per-user provider resolver that replaced the
//! synthetic-fallback bug (`Claude Outputs/Synthetic-Provider Silent Fallback
//! Bug Analysis (2026-05-23)`).
//!
//! The resolver picks "which fitness backend should serve this user's tool
//! call" when the LLM omits the `provider` argument. Order:
//! 1. Health: an `active` connection ahead of one flagged `needs_reauth`/`revoked`.
//! 2. Most-recently-used connection (`last_used_at DESC NULLS LAST`).
//! 3. Otherwise, most-recently-registered (`connected_at DESC`).
//! 4. `None` when the user has zero connections.

use std::time::Duration;

use pierre_core::models::{ConnectionStatus, ConnectionType, TenantId};
use tokio::time::sleep;

mod common;

/// `resolve_most_recent` returns `None` for a brand-new user with no
/// connections. This is the path the chat-pipeline resolver translates into
/// `AppError::no_provider_connected()` — surfacing the reconnect signal
/// instead of silently falling back to seed data.
#[tokio::test]
async fn resolve_most_recent_returns_none_when_user_has_no_connections() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();

    let resolved = repos
        .provider_connections
        .resolve_most_recent(user_id, None)
        .await
        .unwrap();

    assert!(
        resolved.is_none(),
        "fresh user with zero connections must resolve to None — caller surfaces reconnect"
    );
}

/// With a single connection, the resolver returns it regardless of whether
/// `last_used_at` has been touched yet. The `connected_at` fallback in the
/// `ORDER BY` clause is what makes this case work.
#[tokio::test]
async fn resolve_most_recent_returns_only_connection_when_one_exists() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();
    let tenants = repos.tenants.list_for_user(user_id).await.unwrap();
    let tenant_id = TenantId::from_uuid(tenants[0].id.as_uuid());

    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let resolved = repos
        .provider_connections
        .resolve_most_recent(user_id, Some(tenant_id))
        .await
        .unwrap()
        .expect("single connection must resolve");

    assert_eq!(resolved.provider, "strava");
    assert!(
        resolved.last_used_at.is_none(),
        "freshly-registered connection has no last_used_at yet"
    );
}

/// Touching a connection updates `last_used_at`. The resolver then prefers
/// the touched row over an older `connected_at`. This is the behavior that
/// makes chat tools "remember" which backend the user was actually using.
#[tokio::test]
async fn touch_last_used_promotes_provider_in_resolution_order() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();
    let tenants = repos.tenants.list_for_user(user_id).await.unwrap();
    let tenant_id = TenantId::from_uuid(tenants[0].id.as_uuid());

    // Register garmin FIRST (so it has the older connected_at)
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "garmin", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    // Wait briefly so the timestamps are distinguishable in SQLite TEXT format
    sleep(Duration::from_millis(20)).await;
    // Register sciotte SECOND (newer connected_at → would win by default)
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "sciotte", &ConnectionType::Manual, None)
        .await
        .unwrap();

    // Without any touch, sciotte wins on connected_at DESC
    let resolved = repos
        .provider_connections
        .resolve_most_recent(user_id, Some(tenant_id))
        .await
        .unwrap()
        .expect("at least one connection exists");
    assert_eq!(
        resolved.provider, "sciotte",
        "with no touches, newest connection wins by connected_at"
    );

    // Touch garmin — it should now sort ahead of sciotte (NULLS LAST clause)
    sleep(Duration::from_millis(20)).await;
    repos
        .provider_connections
        .touch_last_used(user_id, tenant_id, "garmin")
        .await
        .unwrap();

    let resolved = repos
        .provider_connections
        .resolve_most_recent(user_id, Some(tenant_id))
        .await
        .unwrap()
        .expect("at least one connection exists");
    assert_eq!(
        resolved.provider, "garmin",
        "touched connection wins via last_used_at DESC NULLS LAST"
    );
    assert!(
        resolved.last_used_at.is_some(),
        "touch_last_used must populate the timestamp"
    );
}

/// A connection flagged `needs_reauth` must never be elected over a healthy
/// sibling — not even when it is the newest, and not even when it is the one
/// most recently used. The platform already knows the connection is dead
/// (`mark_needs_reauth` wrote it); electing it anyway is what turned one expired
/// watch token into a blanked turn for an athlete whose other provider was fine.
#[tokio::test]
async fn resolve_most_recent_refuses_to_elect_a_connection_needing_reauth() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();
    let tenants = repos.tenants.list_for_user(user_id).await.unwrap();
    let tenant_id = TenantId::from_uuid(tenants[0].id.as_uuid());

    // strava first (older connected_at), whoop second (newer → wins by default).
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    sleep(Duration::from_millis(20)).await;
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    // whoop is also the one most recently used, so recency alone would elect it.
    repos
        .provider_connections
        .touch_last_used(user_id, tenant_id, "whoop")
        .await
        .unwrap();

    let elected = repos
        .provider_connections
        .resolve_most_recent(user_id, Some(tenant_id))
        .await
        .unwrap()
        .expect("two connections exist");
    assert_eq!(
        elected.provider, "whoop",
        "while healthy, the touched connection wins on recency"
    );

    // Its refresh dies non-recoverably.
    repos
        .provider_connections
        .mark_needs_reauth(user_id, tenant_id, "whoop", Some("invalid_grant"))
        .await
        .unwrap();

    let elected = repos
        .provider_connections
        .resolve_most_recent(user_id, Some(tenant_id))
        .await
        .unwrap()
        .expect("two connections exist");
    assert_eq!(
        elected.provider, "strava",
        "a healthy connection must outrank a flagged one, whatever their timestamps"
    );
    assert_eq!(
        elected.status,
        ConnectionStatus::Active,
        "the elected connection is the usable one"
    );
}

/// When every connection is flagged there is nothing healthy to fall back to —
/// the resolver still returns one so the caller can surface the reconnect signal
/// instead of the "no provider connected" refusal, which sends an athlete who
/// HAS a connection to the wrong flow.
#[tokio::test]
async fn resolve_most_recent_still_returns_a_flagged_connection_when_all_are_dead() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();
    let tenants = repos.tenants.list_for_user(user_id).await.unwrap();
    let tenant_id = TenantId::from_uuid(tenants[0].id.as_uuid());

    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    sleep(Duration::from_millis(20)).await;
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .provider_connections
        .mark_needs_reauth(user_id, tenant_id, "strava", Some("invalid_grant"))
        .await
        .unwrap();
    repos
        .provider_connections
        .mark_needs_reauth(user_id, tenant_id, "whoop", Some("invalid_grant"))
        .await
        .unwrap();

    let elected = repos
        .provider_connections
        .resolve_most_recent(user_id, Some(tenant_id))
        .await
        .unwrap()
        .expect("a flagged connection is still a connection");
    assert_eq!(
        elected.provider, "whoop",
        "with nothing healthy left, the newest flagged connection is elected"
    );
    assert_eq!(
        elected.status,
        ConnectionStatus::NeedsReauth,
        "and it is elected AS flagged, so the caller prompts for a reconnect"
    );
}

/// Touching a nonexistent (user, tenant, provider) row is a no-op — must not
/// error and must not insert anything. The serve chokepoint in `get_activities`
/// touches the provider it just served even when that name isn't a registered
/// connection (e.g., when an explicit arg names something unregistered); we
/// don't want it to blow up.
#[tokio::test]
async fn touch_last_used_is_a_noop_for_unknown_provider() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();
    let tenants = repos.tenants.list_for_user(user_id).await.unwrap();
    let tenant_id = TenantId::from_uuid(tenants[0].id.as_uuid());

    repos
        .provider_connections
        .touch_last_used(user_id, tenant_id, "made_up_provider")
        .await
        .expect("touch on nonexistent row must not error");

    let all = repos
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap();
    assert!(
        all.is_empty(),
        "touch_last_used must not insert a row when none matches"
    );
}
