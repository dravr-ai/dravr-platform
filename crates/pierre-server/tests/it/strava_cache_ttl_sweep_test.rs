// ABOUTME: carnet#950 — the cache TTL sweep evicts Strava copies past 30 days and keeps younger ones
// ABOUTME: Runs on the registry's declared caps, as the server boots it; Garmin copies carry no cap and stay

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava copies are held 30 days (`STRAVA_CACHE_TTL`, the product decision
//! of 2026-10-14), on both the `strava` API descriptor and the sciotte
//! Strava backend. The sweep reads the caps from the server's provider
//! registry, as `pierre-mcp-server` passes them at boot, so a descriptor that
//! stops declaring the cap fails here.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{
    ActivityBuilder, DataSource, DeviceType, SportType, StoredDailyActivity, Tenant, TenantId,
    User, UserStatus,
};
use pierre_database::repositories::WellnessWriteMode;
use pierre_database::RepositoryRegistry;
use pierre_providers::provider_terms::STRAVA_CACHE_TTL;
use pierre_services::provider_cache_sweeper::sweep_provider_caches;
use uuid::Uuid;

use crate::common::create_test_server_resources;

/// One user and a tenant they own.
async fn seed_athlete(repos: &RepositoryRegistry) -> (Uuid, TenantId) {
    let mut user = User::new(
        format!("strava-ttl-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some("athlete".to_owned()),
    );
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "strava ttl tenant".to_owned(),
            slug: format!("strava-ttl-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    (user_id, tenant_id)
}

/// One cached run under `provider`, last synced `age_days` before `now`.
async fn seed_run(
    repos: &RepositoryRegistry,
    (user_id, tenant): (Uuid, TenantId),
    provider: &str,
    age_days: i64,
    now: DateTime<Utc>,
) -> String {
    let synced_at = now - Duration::days(age_days);
    let id = format!("{provider}-run-{age_days}d");
    let run = ActivityBuilder::new(
        id.clone(),
        format!("{provider} run"),
        SportType::Run,
        synced_at - Duration::hours(1),
        3_600,
        provider.to_owned(),
    )
    .distance_meters(10_000.0)
    .build();
    repos
        .activity_cache
        .upsert_activities_synced_at(user_id, &tenant, provider, &[run], synced_at)
        .await
        .unwrap();
    id
}

/// One activity day under `provider`, last synced `age_days` before `now`,
/// filed under a data source of that key.
async fn seed_activity_day(
    repos: &RepositoryRegistry,
    (user_id, tenant): (Uuid, TenantId),
    provider: &str,
    age_days: i64,
    now: DateTime<Utc>,
) {
    let synced_at = now - Duration::days(age_days);
    let source_id = repos
        .data_sources
        .upsert_data_source(
            &tenant,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: provider.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    repos
        .daily_activity
        .upsert_daily_activity(
            &tenant,
            &StoredDailyActivity {
                id: String::new(),
                user_id: user_id.to_string(),
                data_source_id: source_id,
                date: synced_at.date_naive(),
                steps: Some(9_000),
                active_kcal: None,
                total_kcal: None,
                intensity_minutes: None,
                source_name: provider.to_owned(),
                recorded_at: synced_at,
            },
            WellnessWriteMode::PartialSummary,
            synced_at,
        )
        .await
        .unwrap();
}

/// The providers of the scope's activity days, sorted.
async fn activity_day_keys(
    repos: &RepositoryRegistry,
    (user_id, tenant): (Uuid, TenantId),
    now: DateTime<Utc>,
) -> Vec<String> {
    let mut keys: Vec<String> = repos
        .daily_activity
        .get_daily_activity(user_id, &tenant, now - Duration::days(60), now)
        .await
        .unwrap()
        .into_iter()
        .map(|day| day.source_name)
        .collect();
    keys.sort_unstable();
    keys
}

/// The ids cached under `provider` for the scope, sorted.
async fn cached_ids(
    repos: &RepositoryRegistry,
    (user_id, tenant): (Uuid, TenantId),
    provider: &str,
    now: DateTime<Utc>,
) -> Vec<String> {
    let mut ids: Vec<String> = repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant,
            Some(provider),
            now - Duration::days(60),
            now,
            10,
        )
        .await
        .unwrap()
        .into_iter()
        .map(|activity| activity.id().to_owned())
        .collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
async fn one_sweep_evicts_only_the_strava_copy_older_than_thirty_days() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let athlete = seed_athlete(repos).await;
    let now = Utc::now();

    seed_run(repos, athlete, "strava", 31, now).await;
    let young_strava = seed_run(repos, athlete, "strava", 29, now).await;
    let old_garmin = seed_run(repos, athlete, "garmin", 31, now).await;
    // T11: the athlete's own exports are never a Strava copy held under
    // Strava's terms, whatever service they were exported from.
    let strava_export = seed_run(repos, athlete, "strava_archive", 31, now).await;
    let garmin_export = seed_run(repos, athlete, "garmin_archive", 31, now).await;
    seed_activity_day(repos, athlete, "strava", 31, now).await;
    seed_activity_day(repos, athlete, "garmin_archive", 31, now).await;
    let sciotte = cfg!(feature = "provider-sciotte");
    if sciotte {
        seed_run(repos, athlete, "sciotte", 31, now).await;
    }

    // The caps the server boots the sweeper with.
    let ttls = resources.data().provider_registry().cache_ttls();
    assert!(
        ttls.contains(&("strava", STRAVA_CACHE_TTL)),
        "the Strava API descriptor declares its cap: {ttls:?}"
    );
    assert!(
        ttls.iter().all(|(provider, _)| *provider != "garmin"),
        "{ttls:?}"
    );

    let swept = sweep_provider_caches(repos.provider_data.as_ref(), &ttls, now)
        .await
        .unwrap();

    let strava = swept
        .iter()
        .find(|(provider, _)| provider == "strava")
        .map(|(_, expiry)| expiry)
        .expect("strava swept");
    assert_eq!(strava.scopes, 1, "{strava:?}");
    assert_eq!(
        strava.evicted.removed_from("cached_activities"),
        1,
        "{strava:?}"
    );

    assert_eq!(
        cached_ids(repos, athlete, "strava", now).await,
        [young_strava]
    );
    assert_eq!(
        cached_ids(repos, athlete, "garmin", now).await,
        [old_garmin]
    );
    assert_eq!(
        cached_ids(repos, athlete, "strava_archive", now).await,
        [strava_export],
        "the athlete's Strava export outlives the Strava cap"
    );
    assert_eq!(
        cached_ids(repos, athlete, "garmin_archive", now).await,
        [garmin_export]
    );
    assert_eq!(
        activity_day_keys(repos, athlete, now).await,
        ["garmin_archive"],
        "bug: the export's activity day outlives the Strava cap; Strava's own does not"
    );
    if sciotte {
        assert!(
            cached_ids(repos, athlete, "sciotte", now).await.is_empty(),
            "the sciotte Strava backend carries the same cap"
        );
    }

    let recorded = repos
        .provider_data
        .list_provider_data_purges("strava", 10)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0].reason, "cache_expired");
    let got = repos
        .provider_data
        .list_provider_data_purges("garmin", 10)
        .await
        .unwrap();
    assert!(got.is_empty(), "expected empty, got {got:?}");
}
