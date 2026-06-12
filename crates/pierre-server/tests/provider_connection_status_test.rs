// ABOUTME: Integration tests for ProviderConnectionRepository::{mark_needs_reauth, mark_active}
// ABOUTME: Pins the connection-status lifecycle that surfaces a dead OAuth refresh as needs_reauth
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Pins the `provider_connections.status` lifecycle.
//!
//! A connection is born `Active`. A non-recoverable token refresh failure flips it to
//! `NeedsReauth` (so the connection stops looking healthy to the status tool, the group
//! context, and the app). A successful reconnect/refresh re-arms it to `Active` and
//! clears the notification marker. This is the persisted half of the "tell the user his
//! OAuth is disconnected, and update our state" work.

use pierre_core::models::{ConnectionStatus, ConnectionType, TenantId};
use pierre_database::backends::factory::Database;
use uuid::Uuid;

mod common;

/// Helper: resolve the user's single tenant id.
async fn sole_tenant(database: &Database, user_id: Uuid) -> TenantId {
    let tenants = database
        .repositories()
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    TenantId::from_uuid(tenants[0].id.as_uuid())
}

/// A freshly registered connection is `Active` by default — the migration's column
/// default flows through the read path onto the model.
#[tokio::test]
async fn register_connection_defaults_to_active() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = sole_tenant(&database, user_id).await;
    let repos = database.repositories();

    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let conns = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();

    assert_eq!(conns.len(), 1);
    assert_eq!(conns[0].status, ConnectionStatus::Active);
    assert!(!conns[0].status.requires_reauth());
}

/// `mark_needs_reauth` flips a live connection to `NeedsReauth`. This is what the token
/// refresh chokepoint calls when WHOOP returns a non-recoverable 400 on refresh.
#[tokio::test]
async fn mark_needs_reauth_flips_status() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = sole_tenant(&database, user_id).await;
    let repos = database.repositories();

    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    repos
        .provider_connections
        .mark_needs_reauth(user_id, tenant_id, "whoop", Some("invalid_request"))
        .await
        .unwrap();

    let conns = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert_eq!(conns[0].status, ConnectionStatus::NeedsReauth);
    assert!(conns[0].status.requires_reauth());
}

/// A successful reconnect re-arms the connection: `mark_active` returns it to `Active`.
#[tokio::test]
async fn mark_active_rearms_after_reauth() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = sole_tenant(&database, user_id).await;
    let repos = database.repositories();

    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .provider_connections
        .mark_needs_reauth(user_id, tenant_id, "whoop", Some("invalid_request"))
        .await
        .unwrap();
    repos
        .provider_connections
        .mark_active(user_id, tenant_id, "whoop")
        .await
        .unwrap();

    let conns = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert_eq!(conns[0].status, ConnectionStatus::Active);
}

/// `mark_needs_reauth` on a (user, tenant, provider) with no row is a no-op: it must not
/// error and must not insert a phantom connection. The refresh chokepoint can fire for a
/// provider that was never registered in `provider_connections`.
#[tokio::test]
async fn mark_needs_reauth_is_noop_for_unknown_provider() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = sole_tenant(&database, user_id).await;
    let repos = database.repositories();

    repos
        .provider_connections
        .mark_needs_reauth(user_id, tenant_id, "made_up_provider", None)
        .await
        .expect("mark_needs_reauth on nonexistent row must not error");

    let all = repos
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap();
    assert!(
        all.is_empty(),
        "mark_needs_reauth must not insert a row when none matches"
    );
}

/// Marking an already-`NeedsReauth` connection again is idempotent — it stays
/// `NeedsReauth`. The transition guard keeps the first failure's `status_changed_at`,
/// which the notify dedup relies on to nudge the user exactly once per disconnect.
#[tokio::test]
async fn mark_needs_reauth_is_idempotent() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = sole_tenant(&database, user_id).await;
    let repos = database.repositories();

    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    for _ in 0..3 {
        repos
            .provider_connections
            .mark_needs_reauth(user_id, tenant_id, "whoop", Some("invalid_request"))
            .await
            .unwrap();
    }

    let conns = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert_eq!(conns[0].status, ConnectionStatus::NeedsReauth);
}

/// The one-time out-of-band reconnect nudge fires exactly once per disconnect: the first
/// claim after the `needs_reauth` transition wins, the second loses. A reconnect clears the
/// marker so a future disconnect can nudge again.
#[tokio::test]
async fn claim_reauth_notification_fires_once_per_disconnect() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = sole_tenant(&database, user_id).await;
    let repos = database.repositories();
    let pc = &repos.provider_connections;

    pc.register_connection(user_id, tenant_id, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    pc.mark_needs_reauth(user_id, tenant_id, "whoop", Some("invalid_request"))
        .await
        .unwrap();

    assert!(
        pc.claim_reauth_notification(user_id, tenant_id, "whoop")
            .await
            .unwrap(),
        "first claim after disconnect must win"
    );
    assert!(
        !pc.claim_reauth_notification(user_id, tenant_id, "whoop")
            .await
            .unwrap(),
        "second claim must lose — the user is nudged once per disconnect"
    );

    // Reconnect clears the marker; a fresh disconnect can claim again.
    pc.mark_active(user_id, tenant_id, "whoop").await.unwrap();
    pc.mark_needs_reauth(user_id, tenant_id, "whoop", Some("invalid_request"))
        .await
        .unwrap();
    assert!(
        pc.claim_reauth_notification(user_id, tenant_id, "whoop")
            .await
            .unwrap(),
        "a fresh disconnect after reconnect must be claimable again"
    );
}

/// An active (healthy) connection is never claimable — only `needs_reauth` ones nudge.
#[tokio::test]
async fn claim_reauth_notification_noop_for_active_connection() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = sole_tenant(&database, user_id).await;
    let repos = database.repositories();
    let pc = &repos.provider_connections;

    pc.register_connection(user_id, tenant_id, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    assert!(
        !pc.claim_reauth_notification(user_id, tenant_id, "whoop")
            .await
            .unwrap(),
        "a healthy connection must not be claimable for a reconnect nudge"
    );
}
