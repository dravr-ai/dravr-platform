// ABOUTME: GET /api/groups/{id}/report returns numbers, never sentences — stats plus the members in fresh form
// ABOUTME: Seeds real cached activities so the fresh-form member, their form share and TSB come off the snapshot path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-groups")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use pierre_core::models::agents::CreateAgentRequest;
use pierre_core::models::groups::{CoachConsents, GroupMember, GroupRole};
use pierre_core::models::CreateGroupRequest;
use pierre_core::models::{Activity, ActivityBuilder, ConnectionType, SportType, TenantId};
use pierre_groups::strategies::tier::EnterpriseTierStrategy;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_groups::group_analytics::GroupAnalyticsRoutes;
use pierre_routes_groups::GroupRoutes;
use serde_json::{json, Value};
use uuid::Uuid;

use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;

/// A ride a day for eight weeks: `block_secs` a day until the last
/// `taper_days`, then `taper_secs` a day up to yesterday. Today's ride has
/// not happened yet.
///
/// Form today is CTL minus ATL at the end of yesterday (the Coggan /
/// TrainingPeaks convention cageux applies), so a daily rider read before
/// today's session is judged on last night's balance, not on a rest day that
/// has not happened; only a lighter final week freshens, because acute load
/// falls faster than chronic load.
fn daily_rides(block_secs: u32, taper_secs: u32, taper_days: i64) -> Vec<Activity> {
    (1..=59)
        .map(|days_ago| {
            let secs = if days_ago <= taper_days {
                taper_secs
            } else {
                block_secs
            };
            ride(days_ago, secs)
        })
        .collect()
}

/// An hour's ride a day from eight weeks ago until `stopped_days_ago` days
/// ago, then nothing: the health flags read the rider as inactive, and their
/// form as detraining rather than fresh.
fn rides_that_stopped(stopped_days_ago: i64) -> Vec<Activity> {
    (stopped_days_ago..=59)
        .map(|days_ago| ride(days_ago, 3_600))
        .collect()
}

/// A `secs`-long ride at 8 m/s, `days_ago` days ago.
fn ride(days_ago: i64, secs: u32) -> Activity {
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
    add_member_consents(res, group_id, user_id, tenant_id, role, shares, shares).await;
}

/// Add `user_id` sharing with their peers as `peer` says and with the
/// group's human coach as `coach` says.
async fn add_member_consents(
    res: &ServerContext,
    group_id: Uuid,
    user_id: Uuid,
    tenant_id: TenantId,
    role: GroupRole,
    peer: bool,
    coach: bool,
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
            peer_sharing_consent: peer,
            coach_sharing_consent: coach,
            coach_sharing_consent_at: None,
            coach_consents: CoachConsents::default(),
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
    let router =
        GroupRoutes::routes(Arc::clone(&res)).merge(GroupAnalyticsRoutes::routes(Arc::clone(&res)));

    let agent_request: CreateAgentRequest = serde_json::from_value(
        json!({"title":"Report Coach","system_prompt":"Test.","category":"training","tags":["ride"]}),
    )
    .unwrap();
    let agent_id = res
        .common
        .repos
        .agents
        .create(owner_id, tenant_id, &agent_request)
        .await
        .unwrap()
        .id
        .to_string();

    let group_request = CreateGroupRequest {
        name: "Les Rouleurs".to_owned(),
        description: None,
        agent_id,
        max_members: None,
        coach_user_id: None,
    };
    let group_id = res
        .group_service()
        .create_group(&group_request, owner_id, tenant_id, &EnterpriseTierStrategy)
        .await
        .unwrap()
        .id
        .to_string();

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
    // for the last one, up to yesterday: acute load fell faster than chronic,
    // so last night's balance reads about +17.7% of CTL, inside the fresh band.
    seed_rides(
        &fx.res,
        fx.owner_id,
        fx.tenant_id,
        &daily_rides(3_600, 2_400, 7),
    )
    .await;

    // A second member still riding an hour every day, not yet today, sits in
    // balanced form (0% of CTL: a constant load reads CTL = ATL once seeded at
    // its mean): the ride they have not done yet must not read as a rest day
    // that freshens them (+12% of CTL), so the report leaves them out.
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
    // Yesterday evening's CTL minus ATL, as a share of yesterday's CTL: the
    // same numbers a reading taken tonight, after a ride today, would give.
    //
    // An hour's ride here scores T = 51.84 TSS: cageux's pace fallback reads
    // 8 m/s (125 s/km) against cycling's 120 s/km baseline as IF
    // 120/125 x 0.75 = 0.72, and T = 1 h x 0.72^2 x 100. Forty minutes score
    // 2T/3. The snapshot reads 60 days, and the
    // series' first 42 are all hour rides, so CTL and ATL are seeded at T and
    // held there until the taper. Seven days of 2T/3 then leave
    // CTL = T(2/3 + (1/3)(41/42)^7) = 49.158 and
    // ATL = T(2/3 + (1/3)(6/7)^7) = 40.434: TSB 8.724, 17.75% of CTL.
    let hour = 0.72_f64.powi(2) * 100.0;
    let after_taper = |n: f64| {
        let keep = (1.0 - 1.0 / n).powi(7);
        hour * (2.0 / 3.0 + keep / 3.0)
    };
    let (ctl, atl) = (after_taper(42.0), after_taper(7.0));
    let expected_tsb = ctl - atl;
    let expected_pct = expected_tsb / ctl * 100.0;
    assert!(
        (form_pct - expected_pct).abs() < 0.05,
        "form_pct {form_pct} vs {expected_pct}"
    );
    assert!(
        (tsb - expected_tsb).abs() < 0.05,
        "tsb {tsb} vs {expected_tsb}"
    );
}

/// `/stats` aggregates only the members who share plus the caller, whatever
/// the caller's role: otherwise the whole-roster average, minus the sharers'
/// figures the weekly digest posts into the chat, would give away the week of
/// a member who does not share. Owning the group grants no more (carnet#1166).
#[tokio::test]
async fn stats_aggregate_only_the_members_the_caller_may_see() {
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
            let body = resp.json::<Value>();
            assert_eq!(
                body["scope"]["roster_members"], 3,
                "the scope counts the whole roster: {body}"
            );
            assert_eq!(
                body["scope"]["visible_members"], body["stats"]["total_members"],
                "and the members the stats cover: {body}"
            );
            body["stats"].clone()
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
        owner["total_members"], 2,
        "the owner, who is not the group's coach, sees the sharer and themself: {owner}"
    );
    // Half an hour and an hour a day: 1.5/2 of the sharer's week. The private
    // rider's two hours would lift it to 3.5/3.
    let km = owner["avg_weekly_volume_km"].as_f64().unwrap();
    assert!(
        (km - sharer_km * 1.5 / 2.0).abs() < 0.01,
        "the owner's and the sharer's average, never the private rider's: {km}"
    );
}

/// A group whose owner is also its human coach, as onboarding and
/// `/group create` make them, with an admin who is not the coach, and four
/// athletes split by whom they share with. Each viewer has one fresh-form
/// athlete and one inactive athlete they may see, and one of each they may
/// not, so every surface has something to withhold.
struct ConsentSplit {
    fx: Fixture,
    admin_auth: String,
    /// Fresh form; ran `/group consent coach no`, still shares with peers.
    fresh_withdrew: Uuid,
    /// Inactive for three weeks; ran `/group consent coach no`, still shares
    /// with peers.
    idle_withdrew: Uuid,
    /// Fresh form; shares with the coach only.
    fresh_coach_only: Uuid,
    /// Inactive for three weeks; shares with the coach only.
    idle_coach_only: Uuid,
}

async fn consent_split() -> ConsentSplit {
    let fx = Box::pin(owner_with_group()).await;
    let group_uuid = Uuid::parse_str(&fx.group_id).unwrap();
    assert!(
        fx.res
            .common
            .repos
            .groups
            .set_group_coach_user(&fx.group_id, Some(fx.owner_id), fx.tenant_id)
            .await
            .unwrap(),
        "fixture: the owner is attached as the group's coach"
    );

    let (admin_id, admin, _) = create_test_user_with_plan(
        &fx.res.agent.database,
        "split-admin@test.com",
        "professional",
    )
    .await
    .unwrap();
    add_member_consents(
        &fx.res,
        group_uuid,
        admin_id,
        fx.tenant_id,
        GroupRole::Admin,
        false,
        false,
    )
    .await;
    let admin_auth = format!("Bearer {}", generate_test_token(&fx.res, &admin).await);

    let mut athletes = Vec::new();
    for (email, rides, peer, coach) in [
        (
            "split-fresh-withdrew@test.com",
            daily_rides(3_600, 2_400, 7),
            true,
            false,
        ),
        (
            "split-idle-withdrew@test.com",
            rides_that_stopped(21),
            true,
            false,
        ),
        (
            "split-fresh-coach@test.com",
            daily_rides(3_600, 2_400, 7),
            false,
            true,
        ),
        (
            "split-idle-coach@test.com",
            rides_that_stopped(21),
            false,
            true,
        ),
    ] {
        let (id, _, tenant) =
            create_test_user_with_plan(&fx.res.agent.database, email, "professional")
                .await
                .unwrap();
        seed_rides(&fx.res, id, tenant, &rides).await;
        add_member_consents(
            &fx.res,
            group_uuid,
            id,
            fx.tenant_id,
            GroupRole::Member,
            peer,
            coach,
        )
        .await;
        athletes.push(id);
    }

    ConsentSplit {
        fx,
        admin_auth,
        fresh_withdrew: athletes[0],
        idle_withdrew: athletes[1],
        fresh_coach_only: athletes[2],
        idle_coach_only: athletes[3],
    }
}

impl ConsentSplit {
    /// GET `path` under the group as the caller `auth` names; the body.
    async fn get(&self, auth: &str, path: &str) -> Value {
        let resp = AxumTestRequest::get(&format!("/api/groups/{}/{path}", self.fx.group_id))
            .header("authorization", auth)
            .send(self.fx.router.clone())
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::OK,
            "a manager of the group may read its {path}"
        );
        let body: Value = resp.json();
        assert_eq!(
            body["scope"],
            json!({"visible_members": 3, "roster_members": 6}),
            "each viewer sees three of the six members: {body}"
        );
        body
    }
}

/// The user ids named under `rows`, sorted.
fn named(rows: &Value) -> Vec<String> {
    let mut ids: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["user_id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort_unstable();
    ids
}

/// The weekly report of the coach-owner leaves out the athlete who ran
/// `/group consent coach no`; the admin's leaves out the athlete who shares
/// with the coach alone (carnet#1166).
#[tokio::test]
async fn the_weekly_report_covers_only_the_members_who_share_with_the_viewer() {
    let split = Box::pin(consent_split()).await;

    let coach = split.get(&split.fx.owner_auth, "report").await;
    assert_eq!(
        coach["report"]["stats"]["total_members"], 3,
        "the coach-owner's report counts the coach sharers and themself: {coach}"
    );
    assert_eq!(
        named(&coach["report"]["fresh_members"]),
        [split.fresh_coach_only.to_string()],
        "the athlete who withdrew from the coach is not in the coach's report: {coach}"
    );

    let admin = split.get(&split.admin_auth, "report").await;
    assert_eq!(
        admin["report"]["stats"]["total_members"], 3,
        "the admin's report counts the peer sharers and themself: {admin}"
    );
    assert_eq!(
        named(&admin["report"]["fresh_members"]),
        [split.fresh_withdrew.to_string()],
        "the athlete sharing with the coach alone is not in the admin's report: {admin}"
    );
}

/// The health flags name a member only to a viewer they share with: an
/// inactive athlete who withdrew from the coach is flagged to the admin, never
/// to the coach-owner, and the reverse for one sharing with the coach alone.
#[tokio::test]
async fn the_health_flags_name_only_the_members_who_share_with_the_viewer() {
    let split = Box::pin(consent_split()).await;

    let coach = split.get(&split.fx.owner_auth, "health").await;
    assert_eq!(
        named(&coach["flags"]),
        [split.idle_coach_only.to_string()],
        "the coach-owner is flagged only the idle athlete who shares with the coach: {coach}"
    );
    assert_eq!(
        coach["flags"][0]["flag_type"], "inactive",
        "three weeks without a ride reads as inactive: {coach}"
    );

    let admin = split.get(&split.admin_auth, "health").await;
    assert_eq!(
        named(&admin["flags"]),
        [split.idle_withdrew.to_string()],
        "the admin is flagged only the idle athlete who shares with peers: {admin}"
    );
    assert_eq!(
        admin["flags"][0]["flag_type"], "inactive",
        "three weeks without a ride reads as inactive: {admin}"
    );
}

/// A manager's `/stats` covers the same members as their report: the
/// coach-owner's never counts the athletes who withdrew from the coach, and
/// the admin's never counts those who share with the coach alone.
#[tokio::test]
async fn manager_stats_count_only_the_members_who_share_with_the_viewer() {
    let split = Box::pin(consent_split()).await;

    for auth in [&split.fx.owner_auth, &split.admin_auth] {
        let body = split.get(auth, "stats").await;
        assert_eq!(
            body["stats"]["total_members"], 3,
            "a manager's stats count the members who share with them and themself: {body}"
        );
        // Of the three, the viewer has no source and one athlete stopped three
        // weeks ago: only the fresh athlete rode this week, at 40 minutes a
        // day, so the average is a third of their week. The rolling seven days
        // hold their rides of the last six days, and the seventh day's when
        // the window's edge falls before it: six or seven rides. The unseen
        // fresh athlete's week would double that.
        let km = body["stats"]["avg_weekly_volume_km"].as_f64().unwrap();
        let rides_this_week = km * 3.0 / 19.2;
        assert!(
            (rides_this_week - rides_this_week.round()).abs() < 1e-6
                && (6.0..=7.0).contains(&rides_this_week.round()),
            "a third of one athlete's six or seven 40-minute rides, the inactive and unseen \
             athletes adding nothing: {km}"
        );
    }
}
