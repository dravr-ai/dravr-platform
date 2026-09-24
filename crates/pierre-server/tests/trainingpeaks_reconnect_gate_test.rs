// ABOUTME: connection_needs_reauth and backend resolution for the TrainingPeaks scrape mirror
// ABOUTME: A dead TrainingPeaks session hands back a reconnect link, never a doomed backfill
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The Garmin reconnect gate, pinned for TrainingPeaks: the gate is keyed on
//! the backend name the connection row carries, and a TrainingPeaks request
//! with no session row at all still resolves to the mirror, so the reconnect
//! signal names a slug the hosted login can mint for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use chrono::Utc;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::ConnectionType;
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver;
use pierre_tool_runtime::implementations::data_helpers::connection_needs_reauth;

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::{create_test_db, seed_user};

#[tokio::test]
async fn gate_flips_with_trainingpeaks_connection_status() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user, tenant) = seed_user(&db).await;
    let mirror = oauth_providers::SCIOTTE_TRAININGPEAKS;

    repos
        .provider_connections
        .register_connection(user, tenant, mirror, &ConnectionType::Manual, None)
        .await
        .unwrap();
    let conns = repos
        .provider_connections
        .get_for_user(user, Some(tenant))
        .await
        .unwrap();
    assert!(
        !connection_needs_reauth(&conns, mirror),
        "an active TrainingPeaks session does not need reauth"
    );

    repos
        .provider_connections
        .mark_needs_reauth(user, tenant, mirror, Some("session_expired"), Utc::now())
        .await
        .unwrap();
    let conns = repos
        .provider_connections
        .get_for_user(user, Some(tenant))
        .await
        .unwrap();
    assert!(
        connection_needs_reauth(&conns, mirror),
        "a dead TrainingPeaks session must trigger the reconnect gate"
    );
    assert!(
        !connection_needs_reauth(&conns, oauth_providers::SCIOTTE_GARMIN),
        "the gate is provider-scoped — Garmin is not gated by a TrainingPeaks death"
    );

    repos
        .provider_connections
        .mark_active(user, tenant, mirror)
        .await
        .unwrap();
    let conns = repos
        .provider_connections
        .get_for_user(user, Some(tenant))
        .await
        .unwrap();
    assert!(
        !connection_needs_reauth(&conns, mirror),
        "after reconnect the gate must stop short-circuiting"
    );
}

#[tokio::test]
async fn trainingpeaks_with_no_session_row_resolves_to_its_mirror() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());
    let (user, tenant) = seed_user(&db).await;

    let resolved = backend_resolver::resolve_backend(
        &repos.auth_repos(),
        user,
        Some(tenant),
        oauth_providers::TRAININGPEAKS,
    )
    .await;
    assert_eq!(
        resolved,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
        "the reconnect signal must carry the mirror slug, the only one the hosted login can mint for"
    );
}
