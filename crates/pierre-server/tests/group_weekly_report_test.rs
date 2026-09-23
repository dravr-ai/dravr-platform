// ABOUTME: GET /api/groups/{id}/report returns numbers, never sentences — stats plus the members in fresh form
// ABOUTME: Seeds real cached activities so the fresh-form member, their form share and TSB come off the snapshot path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-groups")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use pierre_core::models::groups::{GroupMember, GroupRole};
use pierre_core::models::{Activity, ActivityBuilder, ConnectionType, SportType, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_agents::build_agents_router;
use pierre_routes_groups::group_analytics::GroupAnalyticsRoutes;
use pierre_routes_groups::GroupRoutes;
use serde_json::{json, Value};
use uuid::Uuid;

use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;

/// A ride a day for eight weeks: `block_secs` a day until the last
/// `taper_days`, then `taper_secs` a day up to yesterday.
///
/// The snapshot builder reads form as of the latest ride, so rest days alone
/// never freshen it; a lighter final week does, because acute load falls
/// faster than chronic load.
fn daily_rides(block_secs: u32, taper_secs: u32, taper_days: i64) -> Vec<Activity> {
    (1..=59)
        .map(|days_ago| {
            let secs = if days_ago <= taper_days {
                taper_secs
            } else {
                block_secs
            };
            ActivityBuilder::new(
                format!("ride-{days_ago}"),
                format!("ride {days_ago}"),
                SportType::Ride,
                Utc::now() - Duration::days(days_ago),
                u64::from(secs),
                "strava".to_owned(),
            )
            .distance_meters(f64::from(secs) * 8.0)
            .build()
        })
        .collect()
}

/// Give `user_id` a Strava connection whose cached activities are `rides`.
/// The cache rows are freshly synced, so the snapshot builder serves them
/// without trying a live fetch.
async fn seed_rides(res: &ServerContext, user_id: Uuid, tenant_id: TenantId, rides: &[Activity]) {
    res.common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    res.common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant_id, "strava", rides)
        .await
        .unwrap();
}

async fn add_member(
    res: &ServerContext,
    group_id: Uuid,
    user_id: Uuid,
    tenant_id: TenantId,
    role: GroupRole,
) {
    add_member_sharing(res, group_id, user_id, tenant_id, role, true).await;
}

async fn add_member_sharing(
    res: &ServerContext,
    group_id: Uuid,
    user_id: Uuid,
    tenant_id: TenantId,
    role: GroupRole,
    shares: bool,
) {
    let now = Utc::now();
    res.common
        .repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: tenant_id.to_string(),
            role,
            peer_sharing_consent: shares,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

struct Fixture {
    res: Arc<ServerContext>,
    router: axum::Router,
    owner_auth: String,
    owner_id: Uuid,
    tenant_id: TenantId,
    group_id: String,
}

/// An owner on the Professional plan with one coaching group.
async fn owner_with_group() -> Fixture {
    let res = create_test_server_resources().await.unwrap();
    let (owner_id, owner, tenant_id) =
        create_test_user_with_plan(&res.agent.database, "report-owner@test.com", "professional")
            .await
            .unwrap();
    let owner_auth = format!("Bearer {}", generate_test_token(&res, &owner).await);
    let router = build_agents_router::<ServerContext>()
        .with_state(Arc::clone(&res))
        .merge(GroupRoutes::routes(Arc::clone(&res)))
        .merge(GroupAnalyticsRoutes::routes(Arc::clone(&res)));

    let resp = AxumTestRequest::post("/api/agents")
        .header("authorization", &owner_auth)
        .json(&json!({"title":"Report Coach","system_prompt":"Test.","category":"training","tags":["ride"]}))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let agent_id = resp.json::<Value>()["id"].as_str().unwrap().to_owned();

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &owner_auth)
        .json(&json!({ "name": "Les Rouleurs", "agent_id": agent_id }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let group_id = resp.json::<Value>()["id"].as_str().unwrap().to_owned();

    Fixture {
        res,
        router,
        owner_auth,
        owner_id,
        tenant_id,
        group_id,
    }
}

#[tokio::test]
async fn report_names_the_fresh_member_with_their_form_share_and_tsb() {
    let fx = Box::pin(owner_with_group()).await;

    // The owner rode an hour a day for seven weeks, then forty minutes a day
    // for the last one: acute load fell faster than chronic, so form reads
    // about +15% of CTL, inside the fresh band.
    seed_rides(
        &fx.res,
        fx.owner_id,
        fx.tenant_id,
        &daily_rides(3_600, 2_400, 7),
    )
    .await;

    // A second member still riding an hour every day sits in balanced form
    // (about -7% of CTL), so the report must leave them out.
    let (rider_id, _, rider_tenant) = create_test_user_with_plan(
        &fx.res.agent.database,
        "report-rider@test.com",
        "professional",
    )
    .await
    .unwrap();
    seed_rides(
        &fx.res,
        rider_id,
        rider_tenant,
        &daily_rides(3_600, 3_600, 0),
    )
    .await;
    let group_uuid = Uuid::parse_str(&fx.group_id).unwrap();
    add_member(
        &fx.res,
        group_uuid,
        rider_id,
        fx.tenant_id,
        GroupRole::Member,
    )
    .await;

    let resp = AxumTestRequest::get(&format!("/api/groups/{}/report", fx.group_id))
        .header("authorization", &fx.owner_auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: Value = resp.json();
    let report = &body["report"];

    let mut fields: Vec<&str> = report
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        ["fresh_members", "stats"],
        "the report carries numbers only — no English summary, highlights, concerns or recommendations"
    );

    assert_eq!(report["stats"]["total_members"], 2);
    assert_eq!(report["stats"]["active_members"], 2);

    let fresh = report["fresh_members"].as_array().unwrap();
    assert_eq!(
        fresh.len(),
        1,
        "only the tapering owner reads fresh: {fresh:?}"
    );
    assert_eq!(fresh[0]["user_id"], fx.owner_id.to_string());
    assert_eq!(fresh[0]["display_name"], "Test User");
    let form_pct = fresh[0]["form_pct"].as_f64().unwrap();
    let tsb = fresh[0]["tsb"].as_f64().unwrap();
    assert!((form_pct - 14.99).abs() < 0.05, "form_pct {form_pct}");
    assert!((tsb - 6.64).abs() < 0.05, "tsb {tsb}");
}

/// `/stats` is open to every member, so for a member who does not manage the
/// group it aggregates only the members who share plus the caller: otherwise
/// the whole-roster average, minus the sharers' figures the weekly digest
/// posts into the chat, would give away the week of a member who does not
/// share. The owner still sees the whole roster.
#[tokio::test]
async fn stats_aggregate_only_the_members_a_plain_member_may_see() {
    let fx = Box::pin(owner_with_group()).await;
    let group_uuid = Uuid::parse_str(&fx.group_id).unwrap();
    // The owner does not share (the default); half an hour a day.
    seed_rides(
        &fx.res,
        fx.owner_id,
        fx.tenant_id,
        &daily_rides(1_800, 1_800, 0),
    )
    .await;

    let mut riders = Vec::new();
    for (email, secs, shares) in [
        ("stats-sharer@test.com", 3_600, true),
        ("stats-private@test.com", 7_200, false),
    ] {
        let (id, user, tenant) =
            create_test_user_with_plan(&fx.res.agent.database, email, "professional")
                .await
                .unwrap();
        seed_rides(&fx.res, id, tenant, &daily_rides(secs, secs, 0)).await;
        add_member_sharing(
            &fx.res,
            group_uuid,
            id,
            fx.tenant_id,
            GroupRole::Member,
            shares,
        )
        .await;
        riders.push(format!(
            "Bearer {}",
            generate_test_token(&fx.res, &user).await
        ));
    }

    let stats_for = |auth: String| {
        let router = fx.router.clone();
        let uri = format!("/api/groups/{}/stats", fx.group_id);
        async move {
            let resp = AxumTestRequest::get(&uri)
                .header("authorization", &auth)
                .send(router)
                .await;
            assert_eq!(resp.status_code(), StatusCode::OK);
            resp.json::<Value>()["stats"].clone()
        }
    };

    // A ride a day at 8 m/s: an hour is 28.8 km. The sharer's view is their
    // own week alone, whatever number of rides the rolling window holds.
    let sharer = stats_for(riders[0].clone()).await;
    assert_eq!(
        sharer["total_members"], 1,
        "only the sharer themself: {sharer}"
    );
    let sharer_km = sharer["avg_weekly_volume_km"].as_f64().unwrap();
    assert!(
        sharer_km > 0.0 && (sharer_km / 28.8).fract().abs() < 1e-6,
        "whole hour-long rides of the sharer only: {sharer_km}"
    );

    let private = stats_for(riders[1].clone()).await;
    assert_eq!(
        private["total_members"], 2,
        "the sharer plus the caller: {private}"
    );

    let owner = stats_for(fx.owner_auth.clone()).await;
    assert_eq!(
        owner["total_members"], 3,
        "the owner sees the roster: {owner}"
    );
    // Half an hour, an hour and two hours a day: the roster averages 3.5/3 of
    // the sharer's week, which only holds if the private rider is counted.
    let km = owner["avg_weekly_volume_km"].as_f64().unwrap();
    assert!(
        (km - sharer_km * 3.5 / 3.0).abs() < 0.01,
        "the whole roster's average: {km}"
    );
}
