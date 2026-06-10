// ABOUTME: Pins that on-demand providers (sciotte) report honest freshness from the activity cache
// ABOUTME: Regression for the "sciotte always Fresh" bug that made the coach claim stale data was current
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Honest freshness for on-demand providers.
//!
//! sciotte is scraped per chat request and never background-synced, so its
//! `oauth_tokens.last_sync` is stamped once at login and never moves. The
//! refresh service used to short-circuit on-demand providers to `Fresh`, so the
//! coach believed sciotte data was current even when the scraped cache was
//! hours old (the user-visible "tu parles à travers ton chapeau" bug).
//!
//! The freshness now derives from the activity cache's `synced_at` — the real
//! timestamp of the last successful scrape. These tests pin that contract:
//!  1. No cached activities -> sciotte is reported stale (in `refreshing`), and
//!     the coach hint warns about it.
//!  2. Freshly cached activities -> sciotte is reported `fresh`, and the hint is
//!     suppressed.

mod common;

use std::sync::Arc;

use chrono::Utc;
use common::{create_test_server_resources, create_test_user};
use pierre_core::errors::AppError;
use pierre_core::models::{
    ActivityBuilder, OAuthNotification, RefreshConfig, SportType, UserOAuthToken,
};
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

/// Seed a sciotte session token so `get_provider_freshness` iterates the
/// provider, mirroring what `store_sciotte_session` writes after login.
async fn seed_sciotte_token(repos: &pierre_database::RepositoryRegistry, user_id: Uuid, tenant: &str) {
    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant.to_owned(),
        provider: "sciotte".to_owned(),
        access_token: "{\"session\":\"seed\"}".to_owned(),
        refresh_token: None,
        token_type: "session".to_owned(),
        expires_at: None,
        scope: None,
        provider_user_id: None,
        created_at: now,
        updated_at: now,
    };
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
}

#[tokio::test]
async fn sciotte_with_no_cache_is_reported_stale_not_fresh() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.coach.database).await.unwrap();
    let repos = &resources.common.repos;

    let tenant = repos
        .tenants
        .get_all()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("test user should own a tenant");
    let tenant_id = tenant.id;

    seed_sciotte_token(repos, user_id, &tenant_id.to_string()).await;

    // No cached activities -> no synced_at -> stale.
    let service = RefreshService::new(
        &repos.auth_repos(),
        repos.activity_cache.clone(),
        None,
        Arc::new(NoopNotifier) as Arc<dyn SyncNotifier>,
    );

    let status = service
        .check_and_refresh(user_id, tenant_id, &RefreshConfig::default())
        .await;

    assert!(
        status.refreshing.iter().any(|p| p == "sciotte"),
        "sciotte with no cached activities must be stale (in `refreshing`), not Fresh; \
         got refreshing={:?} fresh={:?}",
        status.refreshing,
        status.fresh
    );
    assert!(
        !status.fresh.iter().any(|p| p == "sciotte"),
        "sciotte must not be reported Fresh without cached data"
    );

    let hint = RefreshService::build_coach_hint(&status.details);
    assert!(
        hint.as_deref().is_some_and(|h| h.contains("sciotte")),
        "the coach hint must warn that sciotte data is stale; got {hint:?}"
    );
}

#[tokio::test]
async fn sciotte_with_fresh_cache_is_reported_fresh() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.coach.database).await.unwrap();
    let repos = &resources.common.repos;

    let tenant = repos
        .tenants
        .get_all()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("test user should own a tenant");
    let tenant_id = tenant.id;

    seed_sciotte_token(repos, user_id, &tenant_id.to_string()).await;

    // Cache a sciotte activity now -> synced_at = now -> fresh.
    let activity = ActivityBuilder::new(
        "sc1".to_owned(),
        "morning ride".to_owned(),
        SportType::Ride,
        Utc::now() - chrono::Duration::hours(2),
        3_600,
        "sciotte".to_owned(),
    )
    .distance_meters(30_000.0)
    .build();
    let written = repos
        .activity_cache
        .upsert_activities(user_id, &tenant_id, "sciotte", &[activity])
        .await
        .unwrap();
    assert_eq!(written, 1, "one sciotte activity should be cached");

    let service = RefreshService::new(
        &repos.auth_repos(),
        repos.activity_cache.clone(),
        None,
        Arc::new(NoopNotifier) as Arc<dyn SyncNotifier>,
    );

    let status = service
        .check_and_refresh(user_id, tenant_id, &RefreshConfig::default())
        .await;

    assert!(
        status.fresh.iter().any(|p| p == "sciotte"),
        "sciotte with a just-written cache must be Fresh; got fresh={:?} refreshing={:?}",
        status.fresh,
        status.refreshing
    );

    let hint = RefreshService::build_coach_hint(&status.details);
    assert!(
        hint.as_deref().map_or(true, |h| !h.contains("sciotte")),
        "a fresh sciotte must not appear in the staleness hint; got {hint:?}"
    );
}
