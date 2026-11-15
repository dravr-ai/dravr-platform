// ABOUTME: `POST /admin/barrage/adopt-cache` (carnet#1482): the cached rows of live sources kept in the activity store
// ABOUTME: Pins held == cached per (user, key) with every stamp kept, archives skipped, a dry run writing nothing, and the 403

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The adoption runs once per environment after the deploy that moved every
//! provider read into the activity store. Every test drives the real handler
//! with the live store the composition root wires, over a real database.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use chrono::{DateTime, Duration, SubsecRound, Utc};
use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::handlers::barrage::{handle_adopt_cache, AdoptCacheRequest};
use pierre_routes_admin::AdminApiContext;
use serde_json::Value;
use uuid::Uuid;

use crate::admin_user_removal_test::{
    admin_context, body_json, manage_users_token, resources, seed_user, super_admin_token,
};

fn activity(id: &str, provider: &str, age_days: i64) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("activity {id}"),
        SportType::Run,
        Utc::now() - Duration::days(age_days),
        3_600,
        provider.to_owned(),
    )
    .distance_meters(10_000.0)
    .build()
}

/// The admin context the composition root builds, the live store included.
fn context(resources: &Arc<ServerContext>) -> Arc<AdminApiContext> {
    let mut context = (*admin_context(resources, "http://127.0.0.1:9")).clone();
    context.live_activities = Some(Arc::clone(&resources.common.live_activities));
    Arc::new(context)
}

async fn adopt(
    context: Arc<AdminApiContext>,
    token: ValidatedAdminToken,
    request: AdoptCacheRequest,
) -> Response {
    handle_adopt_cache(State(context), Extension(token), Json(request))
        .await
        .unwrap()
        .into_response()
}

/// The report row of `provider` for `user_id`.
fn row_of<'a>(body: &'a Value, user_id: Uuid, provider: &str) -> &'a Value {
    body["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["user_id"] == user_id.to_string() && row["provider"] == provider)
        .unwrap_or_else(|| panic!("no {provider} row for {user_id}: {body}"))
}

/// An athlete whose cache holds three Strava rows from two syncs, two
/// Garmin rows and one imported Strava archive row; the stamps each sync
/// wrote, oldest first.
async fn seeded_athlete(resources: &Arc<ServerContext>) -> (Uuid, TenantId, Vec<DateTime<Utc>>) {
    let repos = &resources.common.repos;
    let (user_id, tenant_id, _) = seed_user(repos, "adopt").await;
    let cache = &repos.activity_cache;
    let older = (Utc::now() - Duration::days(2)).trunc_subsecs(6);
    let newer = (Utc::now() - Duration::hours(1)).trunc_subsecs(6);
    cache
        .upsert_activities_synced_at(
            user_id,
            &tenant_id,
            "strava",
            &[activity("101", "strava", 9), activity("102", "strava", 8)],
            older,
        )
        .await
        .unwrap();
    cache
        .upsert_activities_synced_at(
            user_id,
            &tenant_id,
            "strava",
            &[activity("103", "strava", 1)],
            newer,
        )
        .await
        .unwrap();
    cache
        .upsert_activities_synced_at(
            user_id,
            &tenant_id,
            "garmin",
            &[activity("201", "garmin", 3), activity("202", "garmin", 2)],
            newer,
        )
        .await
        .unwrap();
    cache
        .upsert_activities_synced_at(
            user_id,
            &tenant_id,
            "strava_archive",
            &[activity("301", "strava", 400)],
            older,
        )
        .await
        .unwrap();
    (user_id, tenant_id, vec![older, newer])
}

/// Every stored row of `provider`: id and stamp, oldest sync first.
async fn stored(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> Vec<(String, DateTime<Utc>)> {
    resources
        .common
        .repos
        .activity_cache
        .get_stored_activity_rows(user_id, &tenant_id, provider)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.activity.id().to_owned(), row.synced_at))
        .collect()
}

#[tokio::test]
async fn adoption_keeps_every_live_row_in_the_store_and_the_counts_match() {
    let resources = resources().await;
    let (user_id, tenant_id, stamps) = seeded_athlete(&resources).await;
    let strava_before = stored(&resources, user_id, tenant_id, "strava").await;
    let store = &resources.common.live_activities;
    assert_eq!(
        store.held(user_id, tenant_id, "strava").await.unwrap(),
        0,
        "test setup: the store starts empty"
    );

    let response = adopt(
        context(&resources),
        super_admin_token(),
        AdoptCacheRequest {
            tenant_id: Some(tenant_id.to_string()),
            ..AdoptCacheRequest::default()
        },
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "bug: the adoption was refused"
    );
    let body = body_json(response).await;

    assert_eq!(body["data"]["failures"], 0, "bug: a pair differs: {body}");
    for (provider, cached) in [("strava", 3), ("garmin", 2)] {
        let row = row_of(&body, user_id, provider);
        assert_eq!(
            row["outcome"], "matched",
            "bug: {provider} not adopted: {row}"
        );
        assert_eq!(row["cached"], cached, "bug: the cached count is wrong");
        assert_eq!(
            row["held_before"], 0,
            "bug: the store held rows before adoption"
        );
        assert_eq!(
            row["held_after"], cached,
            "bug: the store does not hold every cached row after"
        );
        assert_eq!(
            store.held(user_id, tenant_id, provider).await.unwrap(),
            u64::try_from(cached).unwrap(),
            "bug: the store does not hold every cached {provider} row"
        );
    }
    let archive = row_of(&body, user_id, "strava_archive");
    assert_eq!(
        archive["outcome"], "skipped",
        "bug: an archive, already in the store through its import, is adopted again"
    );
    // Projected back unchanged: every row keeps the stamp its own sync wrote,
    // so the provider's freshness does not move.
    assert_eq!(
        stored(&resources, user_id, tenant_id, "strava").await,
        strava_before,
        "bug: adoption moved a row's synced_at"
    );
    assert_eq!(
        strava_before.iter().map(|(_, at)| *at).collect::<Vec<_>>(),
        vec![stamps[0], stamps[0], stamps[1]],
        "test setup: the rows carry their seeded stamps"
    );

    // A second run finds everything held already and changes nothing.
    let again = body_json(
        adopt(
            context(&resources),
            super_admin_token(),
            AdoptCacheRequest {
                tenant_id: Some(tenant_id.to_string()),
                ..AdoptCacheRequest::default()
            },
        )
        .await,
    )
    .await;
    assert_eq!(
        again["data"]["failures"], 0,
        "bug: a re-run differs: {again}"
    );
    assert_eq!(
        row_of(&again, user_id, "strava")["held_before"],
        3,
        "bug: a re-run does not see the rows already held"
    );
    assert_eq!(
        row_of(&again, user_id, "strava")["held_after"],
        3,
        "bug: a re-run changed what the store holds"
    );
}

#[tokio::test]
async fn a_dry_run_reports_what_it_would_adopt_and_writes_nothing() {
    let resources = resources().await;
    let (user_id, tenant_id, _) = seeded_athlete(&resources).await;
    let strava_before = stored(&resources, user_id, tenant_id, "strava").await;

    let body = body_json(
        adopt(
            context(&resources),
            super_admin_token(),
            AdoptCacheRequest {
                tenant_id: Some(tenant_id.to_string()),
                dry_run: true,
                ..AdoptCacheRequest::default()
            },
        )
        .await,
    )
    .await;

    assert_eq!(
        body["data"]["dry_run"], true,
        "bug: the report does not say it was a dry run"
    );
    let row = row_of(&body, user_id, "strava");
    assert_eq!(row["outcome"], "would_adopt", "bug: {row}");
    assert_eq!(row["cached"], 3, "bug: a dry run miscounts the cache");
    assert_eq!(
        row["held_before"], 0,
        "bug: a dry run reports rows already held"
    );
    assert!(
        row["held_after"].is_null(),
        "bug: a dry run reports an after: {row}"
    );
    let store = &resources.common.live_activities;
    for provider in ["strava", "garmin"] {
        assert_eq!(
            store.held(user_id, tenant_id, provider).await.unwrap(),
            0,
            "bug: a dry run wrote {provider} records into the store"
        );
    }
    assert_eq!(
        stored(&resources, user_id, tenant_id, "strava").await,
        strava_before,
        "bug: a dry run touched the cache"
    );
}

#[tokio::test]
async fn a_key_the_store_does_not_know_is_a_failure_and_other_tenants_are_left_alone() {
    let resources = resources().await;
    let (user_id, tenant_id, _) = seeded_athlete(&resources).await;
    let (other_user, other_tenant, _) = seed_user(&resources.common.repos, "other").await;
    let cache = &resources.common.repos.activity_cache;
    cache
        .upsert_activities(
            user_id,
            &tenant_id,
            "retired_provider",
            &[activity("401", "retired_provider", 5)],
        )
        .await
        .unwrap();
    cache
        .upsert_activities(
            other_user,
            &other_tenant,
            "strava",
            &[activity("501", "strava", 1)],
        )
        .await
        .unwrap();

    let body = body_json(
        adopt(
            context(&resources),
            super_admin_token(),
            AdoptCacheRequest {
                tenant_id: Some(tenant_id.to_string()),
                ..AdoptCacheRequest::default()
            },
        )
        .await,
    )
    .await;

    let unknown = row_of(&body, user_id, "retired_provider");
    assert_eq!(
        unknown["kind"], "unknown",
        "bug: a key the store does not know is not named unknown"
    );
    assert_eq!(unknown["outcome"], "unadoptable", "bug: {unknown}");
    assert_eq!(
        body["data"]["failures"], 1,
        "bug: the unknown key is not counted: {body}"
    );
    assert!(
        body["data"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["tenant_id"] == tenant_id.to_string()),
        "bug: a tenant-scoped adoption reached another tenant: {body}"
    );
    assert_eq!(
        resources
            .common
            .live_activities
            .held(other_user, other_tenant, "strava")
            .await
            .unwrap(),
        0,
        "bug: another tenant's cache was adopted"
    );
}

#[tokio::test]
async fn one_pair_is_adopted_alone_when_named() {
    let resources = resources().await;
    let (user_id, tenant_id, _) = seeded_athlete(&resources).await;

    let body = body_json(
        adopt(
            context(&resources),
            super_admin_token(),
            AdoptCacheRequest {
                tenant_id: Some(tenant_id.to_string()),
                user_id: Some(user_id),
                provider: Some("garmin".to_owned()),
                dry_run: false,
            },
        )
        .await,
    )
    .await;

    let rows = body["data"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "bug: a named pair adopted more: {body}");
    assert_eq!(
        rows[0]["outcome"], "matched",
        "bug: the named pair was not adopted"
    );
    let store = &resources.common.live_activities;
    assert_eq!(
        store.held(user_id, tenant_id, "garmin").await.unwrap(),
        2,
        "bug: the store does not hold the named pair's rows"
    );
    assert_eq!(
        store.held(user_id, tenant_id, "strava").await.unwrap(),
        0,
        "bug: a pair named garmin adopted strava too"
    );
}

#[tokio::test]
async fn only_a_super_admin_adopts() {
    let resources = resources().await;
    let (user_id, tenant_id, _) = seeded_athlete(&resources).await;

    let response = adopt(
        context(&resources),
        manage_users_token(),
        AdoptCacheRequest::default(),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "bug: a non-super-admin was let adopt"
    );
    assert_eq!(
        resources
            .common
            .live_activities
            .held(user_id, tenant_id, "strava")
            .await
            .unwrap(),
        0,
        "bug: a refused adoption wrote"
    );
}

/// Every key the platform caches activities under is one the store keeps:
/// a sciotte capture, and the synthetic provider the dev seeds read through.
#[tokio::test]
async fn every_key_the_platform_caches_under_is_kept_by_the_store() {
    let resources = resources().await;
    let (user_id, tenant_id, _) = seed_user(&resources.common.repos, "keys").await;
    let live = &resources.common.live_activities;
    for (i, key) in [
        "sciotte_garmin",
        "sciotte_coros",
        "sciotte_trainingpeaks",
        "synthetic",
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("{}", 900 + i);
        let kept = live
            .ingest(
                user_id,
                tenant_id,
                key,
                &[activity(&id, key, 1)],
                Utc::now(),
            )
            .await
            .unwrap_or_else(|e| panic!("bug: the store refuses {key}: {e}"));
        assert_eq!(kept.stored, 1, "bug: the store does not keep {key}");
        assert_eq!(
            live.held(user_id, tenant_id, key).await.unwrap(),
            1,
            "bug: the store holds no {key} record"
        );
        assert_eq!(
            stored(&resources, user_id, tenant_id, key).await.len(),
            1,
            "bug: a {key} activity is not served from the cache"
        );
    }
}

/// An amendment of a row cached before the store kept its provider (one the
/// adoption has not reached) lands in both: the store adopts the row at its
/// own receipt, and the cache row keeps that receipt.
#[tokio::test]
async fn an_amendment_of_a_row_the_store_never_held_adopts_it() {
    let resources = resources().await;
    let (user_id, tenant_id, _) = seed_user(&resources.common.repos, "amend").await;
    let received = (Utc::now() - Duration::days(1)).trunc_subsecs(6);
    let start = (Utc::now() - Duration::days(2)).trunc_subsecs(0);
    let ride = |name: &str| {
        ActivityBuilder::new("601", name, SportType::Run, start, 3_600, "garmin")
            .distance_meters(10_000.0)
            .build()
    };
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities_synced_at(
            user_id,
            &tenant_id,
            "garmin",
            &[ride("Morning ride")],
            received,
        )
        .await
        .unwrap();
    let live = &resources.common.live_activities;
    assert_eq!(
        live.held(user_id, tenant_id, "garmin").await.unwrap(),
        0,
        "test setup"
    );

    let amended = ride("Amended ride");
    assert!(
        live.amend(user_id, tenant_id, "garmin", &amended)
            .await
            .unwrap(),
        "bug: the amendment of a cached row did nothing"
    );
    assert_eq!(
        live.held(user_id, tenant_id, "garmin").await.unwrap(),
        1,
        "bug: the store did not adopt the amended row"
    );
    let rows = stored(&resources, user_id, tenant_id, "garmin").await;
    assert_eq!(
        rows,
        vec![("601".to_owned(), received)],
        "bug: the row's receipt moved"
    );
    let served = resources
        .common
        .repos
        .activity_cache
        .get_cached_activity(user_id, &tenant_id, "garmin", "601")
        .await
        .unwrap()
        .expect("the row is served");
    assert_eq!(
        served.name(),
        "Amended ride",
        "bug: the cache row was not amended"
    );
    assert!(
        !live
            .amend(user_id, tenant_id, "garmin", &activity("602", "garmin", 2))
            .await
            .unwrap(),
        "bug: an amendment of a row nobody holds wrote one"
    );
}
