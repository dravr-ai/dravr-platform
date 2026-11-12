// ABOUTME: The one source order a history walk reads: a live usr/pwd session before the grant, never the push
// ABOUTME: And the live-read order beside it, which keeps preferring the grant (B2)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The history source order (carnet#1482).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// `history_source_order` is the one order the backends holding a provider's
// history come in. A walk reads the first live, pageable one: the usr/pwd
// session before the grant (D2), never the push. A live read is not ordered
// there: `resolve_backend` keeps preferring the grant (B2).
#![cfg(all(
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers::sciotte_mock::seed_scrape_session;
use crate::history_walk_api_test::{mock_strava, server, strava_athlete};

use chrono::Utc;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::ConnectionType;
use pierre_providers::backend_resolver::resolve_backend;
use pierre_services::training_history_read::{
    history_source_order, walk_source, HistorySource, HistorySourceRole,
};
use uuid::Uuid;

/// The one source order: a live usr/pwd session before the grant, so a walk
/// reads the session; a flagged session is passed over for the grant; a
/// Garmin athlete holding only the push has nothing to walk.
#[tokio::test]
async fn a_walk_reads_a_live_session_before_the_grant_and_never_the_push() {
    let (api_base, _mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let repos = &resources.common.repos;

    assert_eq!(
        walk_source(repos, user_id, tenant, oauth_providers::STRAVA)
            .await
            .map(|source| source.backend),
        Some(oauth_providers::STRAVA.to_owned()),
        "bug: with no session, the grant is walked"
    );
    seed_scrape_session(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        "walk-session",
    )
    .await;
    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::SCIOTTE,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        history_source_order(repos, user_id, tenant, oauth_providers::STRAVA).await,
        vec![
            HistorySource {
                backend: oauth_providers::SCIOTTE.to_owned(),
                role: HistorySourceRole::Session,
                live: true,
            },
            HistorySource {
                backend: oauth_providers::STRAVA.to_owned(),
                role: HistorySourceRole::Grant,
                live: true,
            },
        ],
        "bug: the session comes before the grant"
    );
    assert_eq!(
        walk_source(repos, user_id, tenant, oauth_providers::STRAVA)
            .await
            .map(|source| source.backend),
        Some(oauth_providers::SCIOTTE.to_owned()),
        "bug: a live session is walked before the grant (D2)"
    );
    repos
        .provider_connections
        .mark_needs_reauth(
            user_id,
            tenant,
            oauth_providers::SCIOTTE,
            Some("session_expired"),
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        walk_source(repos, user_id, tenant, oauth_providers::STRAVA)
            .await
            .map(|source| source.backend),
        Some(oauth_providers::STRAVA.to_owned()),
        "bug: a flagged session is passed over for the grant"
    );

    let (garmin_user, _user, garmin_tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("push-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    seed_scrape_session(
        &resources,
        garmin_user,
        garmin_tenant,
        oauth_providers::GARMIN,
        "push-grant",
    )
    .await;
    repos
        .provider_connections
        .register_connection(
            garmin_user,
            garmin_tenant,
            oauth_providers::GARMIN,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        walk_source(repos, garmin_user, garmin_tenant, oauth_providers::GARMIN).await,
        None,
        "bug: a push holds nothing from before the grant, so nothing is walked"
    );
}

/// A Garmin athlete holding both the usr/pwd session and the push grant: the
/// session comes first and is walked, the push after it and never walked.
#[tokio::test]
async fn a_garmin_session_is_walked_before_its_push_grant() {
    let (api_base, _mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let repos = &resources.common.repos;
    let (user_id, _user, tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("pair-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    for (backend, connection) in [
        (oauth_providers::GARMIN, ConnectionType::OAuth),
        (oauth_providers::SCIOTTE_GARMIN, ConnectionType::Manual),
    ] {
        seed_scrape_session(&resources, user_id, tenant, backend, "pair").await;
        repos
            .provider_connections
            .register_connection(user_id, tenant, backend, &connection, None)
            .await
            .unwrap();
    }

    assert_eq!(
        history_source_order(repos, user_id, tenant, oauth_providers::GARMIN).await,
        vec![
            HistorySource {
                backend: oauth_providers::SCIOTTE_GARMIN.to_owned(),
                role: HistorySourceRole::Session,
                live: true,
            },
            HistorySource {
                backend: oauth_providers::GARMIN.to_owned(),
                role: HistorySourceRole::Pushed,
                live: true,
            },
        ],
        "bug: the Garmin session comes before its push grant"
    );
    assert_eq!(
        walk_source(repos, user_id, tenant, oauth_providers::SCIOTTE_GARMIN)
            .await
            .map(|source| source.backend),
        Some(oauth_providers::SCIOTTE_GARMIN.to_owned()),
        "bug: either half of the pair walks the session"
    );
}

/// B2: sciotte-first is for walks only. A live read of Strava by an athlete
/// holding both a live session and the grant keeps reading the grant, while
/// the walk of the same provider reads the session.
#[tokio::test]
async fn a_live_read_keeps_the_grant_while_the_walk_reads_the_session() {
    let (api_base, _mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let repos = &resources.common.repos;
    seed_scrape_session(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        "live-read-session",
    )
    .await;
    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::SCIOTTE,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();

    assert_eq!(
        resolve_backend(repos, user_id, Some(tenant), oauth_providers::STRAVA).await,
        oauth_providers::STRAVA,
        "bug: a live read keeps preferring the grant (B2)"
    );
    assert_eq!(
        walk_source(repos, user_id, tenant, oauth_providers::STRAVA)
            .await
            .map(|source| source.backend),
        Some(oauth_providers::SCIOTTE.to_owned()),
        "bug: the walk of the same provider reads the session (D2)"
    );
}
