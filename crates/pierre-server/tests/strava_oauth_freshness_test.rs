// ABOUTME: Pins that OAuth-connected Strava reports freshness from the activity cache, not health-sync last_sync
// ABOUTME: Regression for the "Strava perpetually Fresh" bug that suppressed the coach's get_activities directive
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Honest freshness + sync roster for OAuth-connected Strava.
//!
//! Strava is registered with the enforme orchestrator for sciotte TSB
//! scraping, but enforme background-syncs no activity data for anyone, and
//! for OAuth-connected users (Bearer token, no browser session) it can sync
//! nothing at all. Two live failure modes followed (2026-07-11):
//!
//! 1. Every no-op sync cycle stamped `oauth_tokens.last_sync`, so freshness
//!    read from that column reported Strava perpetually Fresh. The coach
//!    hint's "invoke `get_activities` before answering" directive therefore
//!    never fired, and on messaging channels the model answered activity
//!    questions from stale conversation history instead of re-fetching.
//! 2. The scheduler iterated OAuth-only Strava users every cycle, logging a
//!    misleading "Scheduled sync completed, `records_created`: 0".
//!
//! These tests pin the fixes: activity-only providers derive freshness from
//! the activity cache even when orchestrator-registered, and the scheduled
//! sync roster excludes Strava rows without a sciotte session credential.

mod common;

use std::sync::Arc;

use chrono::Utc;
use common::{create_test_server_resources, create_test_user, create_test_user_with_email};
use pierre_core::errors::AppError;
use pierre_core::models::{
    ActivityBuilder, OAuthNotification, RefreshConfig, SportType, TenantId, UserOAuthToken,
};
use pierre_database::RepositoryRegistry;
use pierre_enforme::traits::connection_store::UserConnectionStore;
use pierre_services::health_sync::PierreSyncStorage;
use pierre_services::provider_refresh::{RefreshService, SyncNotifier};
use uuid::Uuid;

/// No-op notifier: the refresh service requires a `SyncNotifier`, but these
/// tests assert on freshness classification, not SSE delivery.
struct NoopNotifier;

#[async_trait::async_trait]
impl SyncNotifier for NoopNotifier {
    async fn send_notification(
        &self,
        _user_id: Uuid,
        _notification: &OAuthNotification,
    ) -> Result<(), AppError> {
        Ok(())
    }
}

/// Seed a Strava token row. `token_type` distinguishes an OAuth exchange
/// (`"Bearer"`, opaque access token) from a sciotte browser session
/// (`"session"`, serialized session JSON) — mirroring what the OAuth
/// callback and `store_sciotte_session` write respectively.
async fn seed_strava_token(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: &str,
    token_type: &str,
    access_token: &str,
) {
    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant.to_owned(),
        provider: "strava".to_owned(),
        access_token: access_token.to_owned(),
        refresh_token: Some("refresh".to_owned()),
        token_type: token_type.to_owned(),
        expires_at: Some(now + chrono::Duration::hours(6)),
        scope: Some("activity:read_all".to_owned()),
        provider_user_id: None,
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    };
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
}

/// Resolve the tenant owned by the test user.
async fn owned_tenant(repos: &RepositoryRegistry, user_id: Uuid) -> TenantId {
    repos
        .tenants
        .get_all()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("test user should own a tenant")
        .id
}

#[tokio::test]
async fn strava_oauth_with_stamped_last_sync_but_cold_cache_is_stale() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.coach.database).await.unwrap();
    let repos = &resources.common.repos;
    let tenant_id = owned_tenant(repos, user_id).await;

    seed_strava_token(
        repos,
        user_id,
        &tenant_id.to_string(),
        "Bearer",
        "opaque-oauth-token",
    )
    .await;

    // Simulate the enforme no-op sync cycle stamping last_sync moments ago.
    // Before the fix this alone made Strava read as Fresh forever.
    repos
        .oauth_tokens
        .update_provider_last_sync(user_id, tenant_id, "strava", Utc::now())
        .await
        .unwrap();

    // Build a real orchestrator so "strava" IS registered — the fix must
    // classify it on-demand by capability, not by (non-)registration.
    let storage = Arc::new(PierreSyncStorage::new(repos));
    let orchestrator = storage.build_orchestrator();
    assert!(
        orchestrator.provider_names().contains(&"strava"),
        "test premise: strava must be registered with the orchestrator"
    );

    let service = RefreshService::new(
        &repos.auth_repos(),
        repos.activity_cache.clone(),
        Some(orchestrator),
        Arc::new(NoopNotifier) as Arc<dyn SyncNotifier>,
    );

    let status = service
        .check_and_refresh(user_id, tenant_id, &RefreshConfig::default())
        .await;

    assert!(
        status.refreshing.iter().any(|p| p == "strava"),
        "strava with a cold activity cache must be stale (in `refreshing`) even though \
         health-sync stamped last_sync just now; got refreshing={:?} fresh={:?}",
        status.refreshing,
        status.fresh
    );
    assert!(
        !status.fresh.iter().any(|p| p == "strava"),
        "strava must not be reported Fresh off the health-sync last_sync stamp"
    );

    let hint = RefreshService::build_coach_hint(&status.details)
        .expect("a stale strava must produce a coach hint");
    assert!(
        hint.contains("strava"),
        "the coach hint must name strava as stale; got {hint}"
    );
    assert!(
        hint.contains("get_activities"),
        "the coach hint must carry the get_activities re-fetch directive; got {hint}"
    );
}

#[tokio::test]
async fn strava_oauth_with_fresh_activity_cache_is_fresh() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.coach.database).await.unwrap();
    let repos = &resources.common.repos;
    let tenant_id = owned_tenant(repos, user_id).await;

    seed_strava_token(
        repos,
        user_id,
        &tenant_id.to_string(),
        "Bearer",
        "opaque-oauth-token",
    )
    .await;

    // A just-fetched activity in the cache is the honest freshness signal.
    let activity = ActivityBuilder::new(
        "st1".to_owned(),
        "evening ride".to_owned(),
        SportType::Ride,
        Utc::now() - chrono::Duration::hours(2),
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(42_000.0)
    .build();
    let written = repos
        .activity_cache
        .upsert_activities(user_id, &tenant_id, "strava", &[activity])
        .await
        .unwrap();
    assert_eq!(written, 1, "one strava activity should be cached");

    let storage = Arc::new(PierreSyncStorage::new(repos));
    let orchestrator = storage.build_orchestrator();

    let service = RefreshService::new(
        &repos.auth_repos(),
        repos.activity_cache.clone(),
        Some(orchestrator),
        Arc::new(NoopNotifier) as Arc<dyn SyncNotifier>,
    );

    let status = service
        .check_and_refresh(user_id, tenant_id, &RefreshConfig::default())
        .await;

    assert!(
        status.fresh.iter().any(|p| p == "strava"),
        "strava with a just-written activity cache must be Fresh; got fresh={:?} refreshing={:?}",
        status.fresh,
        status.refreshing
    );

    let hint = RefreshService::build_coach_hint(&status.details);
    assert!(
        hint.as_deref().is_none_or(|h| !h.contains("strava")),
        "a fresh strava must not appear in the staleness hint; got {hint:?}"
    );
}

#[tokio::test]
async fn scheduled_sync_roster_excludes_oauth_only_strava_users() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;

    let (oauth_user, _) = create_test_user(&resources.coach.database).await.unwrap();
    let (sciotte_user, _) =
        create_test_user_with_email(&resources.coach.database, "sciotte-strava@example.com")
            .await
            .unwrap();
    let oauth_tenant = owned_tenant(repos, oauth_user).await;
    let sciotte_tenant = owned_tenant(repos, sciotte_user).await;

    // OAuth-connected user: opaque Bearer token the TSB scraper cannot use.
    seed_strava_token(
        repos,
        oauth_user,
        &oauth_tenant.to_string(),
        "Bearer",
        "opaque-oauth-token",
    )
    .await;
    // Sciotte-connected user: serialized browser session the scraper restores.
    seed_strava_token(
        repos,
        sciotte_user,
        &sciotte_tenant.to_string(),
        "session",
        "{\"session\":\"seed\"}",
    )
    .await;

    let storage = Arc::new(PierreSyncStorage::new(repos));
    let roster = storage.list_connected_users("strava").await.unwrap();

    let ids: Vec<&str> = roster.iter().map(|u| u.user_id.as_str()).collect();
    assert!(
        ids.contains(&sciotte_user.to_string().as_str()),
        "session-credentialed strava user must stay on the sync roster; got {ids:?}"
    );
    assert!(
        !ids.contains(&oauth_user.to_string().as_str()),
        "OAuth-only strava user must be excluded from the sync roster (guaranteed no-op \
         sync would only stamp last_sync and log a misleading completion); got {ids:?}"
    );
}
