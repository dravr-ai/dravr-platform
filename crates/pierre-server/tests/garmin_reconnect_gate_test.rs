// ABOUTME: connection_needs_reauth gate decision across active→needs_reauth→reconnect transitions
// ABOUTME: Backs the synchronous get_activities reconnect prompt (no doomed backfill on a dead session)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Verifies the gate that makes `get_activities` hand back a reconnect link
//! synchronously when the provider session is dead, instead of spawning a
//! background backfill and looping the user on "fetching, ask again shortly".

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pierre_core::models::ConnectionType;
use pierre_database::RepositoryRegistry;
use pierre_tool_runtime::implementations::data::connection_needs_reauth;

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::{create_test_db, seed_user};

#[tokio::test]
async fn gate_flips_with_connection_status_active_then_reauth_then_reconnect() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user, tenant) = seed_user(&db).await;

    // A freshly-registered Garmin (sciotte mirror) connection is Active → usable,
    // so the gate must NOT short-circuit to a reconnect prompt.
    repos
        .provider_connections
        .register_connection(user, tenant, "sciotte_garmin", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let conns = repos
        .provider_connections
        .get_for_user(user, Some(tenant))
        .await
        .unwrap();
    assert!(
        !connection_needs_reauth(&conns, "sciotte_garmin"),
        "an active connection does not need reauth"
    );

    // Session lapses → mark_needs_reauth flips the status. Now the gate must
    // short-circuit so the user gets the reconnect link instead of a doomed
    // backfill + "fetching shortly" loop.
    repos
        .provider_connections
        .mark_needs_reauth(user, tenant, "sciotte_garmin", Some("session_expired"))
        .await
        .unwrap();
    let conns = repos
        .provider_connections
        .get_for_user(user, Some(tenant))
        .await
        .unwrap();
    assert!(
        connection_needs_reauth(&conns, "sciotte_garmin"),
        "a needs_reauth connection must trigger the reconnect gate"
    );
    // A different provider with no unusable connection is unaffected.
    assert!(
        !connection_needs_reauth(&conns, "strava"),
        "the gate is provider-scoped — an unrelated provider is not gated"
    );

    // The user reconnects → mark_active clears the flag → gate stops firing, the
    // normal fetch/backfill path resumes.
    repos
        .provider_connections
        .mark_active(user, tenant, "sciotte_garmin")
        .await
        .unwrap();
    let conns = repos
        .provider_connections
        .get_for_user(user, Some(tenant))
        .await
        .unwrap();
    assert!(
        !connection_needs_reauth(&conns, "sciotte_garmin"),
        "after reconnect the gate must stop short-circuiting"
    );
}

#[test]
fn gate_is_false_for_no_connections() {
    assert!(!connection_needs_reauth(&[], "sciotte_garmin"));
}
