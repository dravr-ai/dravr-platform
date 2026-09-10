// ABOUTME: compute_training_history reads the durable activity cache and never calls a provider
// ABOUTME: Pins that a cache too shallow to warm CTL writes no rows rather than a zero-seeded curve

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The rollup is computed from stored activities, and only for days the stored
//! history can stand behind.
//!
//! Two defects are pinned here, and they pull in opposite directions.
//!
//! **The one that shipped.** `compute_and_persist_history` used to fetch up to
//! 500 activities live from the provider on every call. On a scrape backend
//! that is minutes of browser work inside a chat turn bounded at 90s: on
//! 2026-09-08 a sciotte scrape returned 378 activities after 171 seconds, the
//! platform had already dropped the future at 90, and the athlete was told the
//! calculation timed out. `computes_a_full_window_from_cache_without_a_provider`
//! fails against that implementation — no provider token is registered, so the
//! live fetch it used to do could only error.
//!
//! **The one a naive fix would ship.** `ctl`/`atl`/`tsb` are plain `f64` seeded
//! at zero with no `None` arm, so computing a day without
//! `warmup_days(ctl_window)` of history behind it emits a chronic load that is
//! wrong low and looks exactly like a real one. "Read the cache and compute the
//! requested window" would therefore answer a 90-day ask from a 120-day cache
//! with 91 confident rows, ~42 of them fiction, and every assertion on row
//! count would pass. `a_shallow_cache_writes_no_row_it_cannot_warm` fails
//! against that: it asserts the early days are ABSENT.

#![cfg(feature = "tools-data")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use chrono::{Duration, NaiveDate, Utc};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::{
    ActivityBuilder, ConnectionType, DailyTrainingState, SportType, TenantId,
    UserPhysiologicalProfile,
};
use pierre_fitness_compute::training_history_compute::{warmup_days, CTL_WINDOW_DAYS};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::{
    compute_and_persist_history, default_window, fetch_history_rows, HistoryCoverage,
    TrainingHistoryComputed,
};
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};

/// The backend the seeded rows are written under. A scrape mirror, because that
/// is the class whose live fetch is expensive enough to have caused the outage.
const BACKEND: &str = "sciotte";

/// One activity every third day, oldest first, so a window's warm-up depth is a
/// function of `oldest_days_ago` alone.
const SEED_STRIDE_DAYS: i64 = 3;

struct Fixture {
    runtime: Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant: TenantId,
}

/// A user with a connected scrape backend, real physiology, and nothing cached.
///
/// No OAuth token is ever registered: every test here would fail if the compute
/// reached for the provider.
async fn fixture() -> Fixture {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, BACKEND, &ConnectionType::OAuth, None)
        .await
        .unwrap();

    // Real physiology so TSS is HR-derived and the resulting CTL is non-zero —
    // a series of zeros would pass a row-count assertion while proving nothing.
    let profile = UserPhysiologicalProfile {
        user_id,
        vo2_max: Some(52.0),
        resting_hr: Some(50),
        max_hr: Some(190),
        lactate_threshold_percentage: Some(0.85),
        age: Some(34),
        weight: Some(72.0),
        fitness_level: FitnessLevel::Advanced,
        primary_sport: SportType::Run,
        training_experience_years: Some(10),
        ftp_watts: Some(280),
        threshold_pace_sec_per_km: Some(225.0),
        hr_zones: None,
        power_zones: None,
    };
    resources
        .common
        .repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant, user_id, &profile)
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    Fixture {
        runtime,
        user_id,
        tenant,
    }
}

/// Cache a run every [`SEED_STRIDE_DAYS`] from `oldest_days_ago` up to today.
async fn seed_cache(fx: &Fixture, oldest_days_ago: i64) {
    let mut activities = Vec::new();
    let mut days_ago = oldest_days_ago;
    while days_ago >= 0 {
        activities.push(
            ActivityBuilder::new(
                format!("cached-{days_ago}"),
                format!("run {days_ago}d ago"),
                SportType::Run,
                Utc::now() - Duration::days(days_ago),
                3_600,
                BACKEND.to_owned(),
            )
            .distance_meters(12_000.0)
            .average_heart_rate(155)
            .build(),
        );
        days_ago -= SEED_STRIDE_DAYS;
    }
    fx.runtime
        .repos()
        .activity_cache
        .upsert_activities(fx.user_id, &fx.tenant, BACKEND, &activities)
        .await
        .unwrap();
}

/// The default 90-day ask, exactly as the tool and the HTTP route resolve it.
async fn ask_default_window(fx: &Fixture) -> TrainingHistoryComputed {
    let (from, to) = default_window(&fx.runtime, fx.user_id).await.unwrap();
    compute_and_persist_history(&fx.runtime, fx.tenant, fx.user_id, from, to)
        .await
        .expect("compute reads the cache and must not need a provider")
}

/// A cache deeper than the window plus its CTL warm-up answers the whole ask,
/// with no provider connected — which is the entire point of the change.
#[tokio::test]
async fn computes_a_full_window_from_cache_without_a_provider() {
    let fx = fixture().await;
    // 200 days back covers the 90-day window plus its 72-day warm-up (162).
    seed_cache(&fx, 200).await;

    let (from, to) = default_window(&fx.runtime, fx.user_id).await.unwrap();
    let computed = ask_default_window(&fx).await;

    assert_eq!(
        computed.coverage,
        HistoryCoverage::Complete,
        "200 days of stored history warms a 90-day window"
    );
    assert_eq!(computed.from, from, "no day of the ask was dropped");
    assert_eq!(
        computed.rows_upserted,
        (to - from).num_days() as usize + 1,
        "one dense row per calendar day in the window"
    );
    assert!(
        !computed.capture_requested,
        "a complete answer must not ask the provider for anything"
    );

    let rows = fetch_history_rows(&fx.runtime.data(), fx.tenant, fx.user_id, from, to)
        .await
        .unwrap();
    assert_eq!(rows.len(), (to - from).num_days() as usize + 1);
    let last = rows.last().expect("the window has rows");
    assert!(
        last.ctl > 0.0,
        "real activities must produce a real chronic load, got ctl={}",
        last.ctl
    );
}

/// A cache that cannot warm the start of the window must leave those days
/// EMPTY, not fill them with a curve ramping up from a zero seed.
#[tokio::test]
async fn a_shallow_cache_writes_no_row_it_cannot_warm() {
    let fx = fixture().await;
    // 120 days back: enough to warm the recent part of a 90-day window, far
    // short of the 162 the whole window needs.
    seed_cache(&fx, 120).await;

    let (from, to) = default_window(&fx.runtime, fx.user_id).await.unwrap();
    let computed = ask_default_window(&fx).await;

    let warmup = warmup_days(CTL_WINDOW_DAYS);
    let expected_first = to - Duration::days(120) + Duration::days(warmup);
    assert_eq!(
        computed.coverage,
        HistoryCoverage::Partial {
            trustworthy_from: expected_first
        },
        "the honest floor is the oldest stored day plus the CTL warm-up"
    );
    assert_eq!(computed.from, expected_first);
    assert!(
        computed.from > from,
        "a 120-day cache cannot stand behind a 162-day requirement"
    );
    assert!(
        computed.capture_requested,
        "the missing depth must be asked of the capture rail"
    );

    // The load-bearing assertion: the days it could not warm carry no row at
    // all. A naive cache-read fix writes a fabricated low CTL here instead.
    let early = fetch_history_rows(
        &fx.runtime.data(),
        fx.tenant,
        fx.user_id,
        from,
        expected_first - Duration::days(1),
    )
    .await
    .unwrap();
    assert!(
        early.is_empty(),
        "days without warm-up must have NO row; found {} (first ctl={:?})",
        early.len(),
        early.first().map(|r| r.ctl)
    );

    let warmed = fetch_history_rows(
        &fx.runtime.data(),
        fx.tenant,
        fx.user_id,
        expected_first,
        to,
    )
    .await
    .unwrap();
    assert_eq!(
        warmed.len(),
        (to - expected_first).num_days() as usize + 1,
        "every day from the honest floor onward is present"
    );
}

/// No stored activities means no rollup — never a zero-seeded series that would
/// read to a coach as a real, very low chronic load.
#[tokio::test]
async fn an_empty_cache_computes_nothing_rather_than_a_zero_curve() {
    let fx = fixture().await;

    let (from, to) = default_window(&fx.runtime, fx.user_id).await.unwrap();
    let computed = ask_default_window(&fx).await;

    assert_eq!(computed.coverage, HistoryCoverage::NoStoredActivities);
    assert_eq!(computed.rows_upserted, 0, "nothing may be invented");
    assert!(
        computed.capture_requested,
        "an empty cache must ask the capture rail for the athlete's history"
    );

    let rows = fetch_history_rows(&fx.runtime.data(), fx.tenant, fx.user_id, from, to)
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "an empty cache persisted {} rows it could not justify",
        rows.len()
    );
}

/// Seed the rows the OLD provider-fetching path would have written: a confident,
/// low chronic load across the whole window, computed from a feed too short to
/// warrant it. `ctl`/`atl`/`tsb` carry no marker saying so.
async fn seed_fabricated_rows(fx: &Fixture, from: NaiveDate, to: NaiveDate) {
    let mut day = from;
    let mut rows = Vec::new();
    while day <= to {
        rows.push(DailyTrainingState {
            date: day,
            ctl: 11.5,
            atl: 9.0,
            tsb: 2.5,
            acwr: None,
            monotony: None,
            strain: None,
            ramp_rate: None,
            daily_load: 0.0,
        });
        day += Duration::days(1);
    }
    fx.runtime
        .repos()
        .training_history
        .upsert_training_history_batch(fx.tenant, fx.user_id, &rows)
        .await
        .unwrap();
}

/// A recompute that cannot stand behind the early window must REMOVE what an
/// earlier path left there, not merely decline to overwrite it.
///
/// The rollup is upsert-only everywhere else, so declining to write leaves the
/// stale row readable — and `get_training_history` serves it as current. This is
/// the difference between "the coach sees no fitness number" and "the coach sees
/// a fabricated one", which is the entire point of the change.
#[tokio::test]
async fn a_partial_recompute_clears_rows_the_old_path_fabricated() {
    let fx = fixture().await;
    let (from, to) = default_window(&fx.runtime, fx.user_id).await.unwrap();

    seed_fabricated_rows(&fx, from, to).await;
    let before = fetch_history_rows(&fx.runtime.data(), fx.tenant, fx.user_id, from, to)
        .await
        .unwrap();
    assert_eq!(
        before.len(),
        (to - from).num_days() as usize + 1,
        "the old path's rows are in place before the recompute"
    );

    // Only 120 days of stored activity — short of the 162 the window needs.
    seed_cache(&fx, 120).await;
    let computed = ask_default_window(&fx).await;

    let warmup = warmup_days(CTL_WINDOW_DAYS);
    let expected_first = to - Duration::days(120) + Duration::days(warmup);
    assert_eq!(
        computed.coverage,
        HistoryCoverage::Partial {
            trustworthy_from: expected_first
        }
    );
    assert!(
        computed.rows_cleared > 0,
        "the un-warmable span held fabricated rows and must have been cleared"
    );

    let early = fetch_history_rows(
        &fx.runtime.data(),
        fx.tenant,
        fx.user_id,
        from,
        expected_first - Duration::days(1),
    )
    .await
    .unwrap();
    assert!(
        early.is_empty(),
        "fabricated rows survived the recompute: {} rows, first ctl={:?}",
        early.len(),
        early.first().map(|r| r.ctl)
    );

    // What it could stand behind is present, and is real rather than the seeded
    // 11.5 — proof the window was recomputed, not merely truncated.
    let warmed = fetch_history_rows(
        &fx.runtime.data(),
        fx.tenant,
        fx.user_id,
        expected_first,
        to,
    )
    .await
    .unwrap();
    assert_eq!(warmed.len(), (to - expected_first).num_days() as usize + 1);
    let last = warmed.last().expect("warmed rows exist");
    assert!(
        (last.ctl - 11.5).abs() > f64::EPSILON,
        "the surviving rows must be recomputed, not the seeded placeholder"
    );
}

/// An empty cache clears the whole window: nothing is vouched for anywhere.
#[tokio::test]
async fn an_empty_cache_clears_the_whole_window() {
    let fx = fixture().await;
    let (from, to) = default_window(&fx.runtime, fx.user_id).await.unwrap();

    seed_fabricated_rows(&fx, from, to).await;
    let computed = ask_default_window(&fx).await;

    assert_eq!(computed.coverage, HistoryCoverage::NoStoredActivities);
    assert_eq!(
        computed.rows_cleared,
        (to - from).num_days() as u64 + 1,
        "every fabricated day is removed"
    );
    let rows = fetch_history_rows(&fx.runtime.data(), fx.tenant, fx.user_id, from, to)
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "an athlete with no stored activities must have no rollup, found {}",
        rows.len()
    );
}

/// A narrow ask must never delete a day outside itself.
///
/// `trustworthy_from` is derived from the warm-up shortfall, not from the ask,
/// so on a shallow cache it can sit far past `to`. The clear is bounded by the
/// ask for that reason: the days between `to` and `trustworthy_from` were never
/// requested, and a wider earlier run legitimately vouched for them.
#[tokio::test]
async fn a_narrow_ask_clears_nothing_past_its_own_end() {
    let fx = fixture().await;
    let (from, to) = default_window(&fx.runtime, fx.user_id).await.unwrap();

    // Rows across the whole default window, as a wider earlier run would leave.
    seed_fabricated_rows(&fx, from, to).await;

    // 120 days of activity: warm-up reaches only `to - 48`, so any ask ending
    // before that is entirely un-warmable.
    seed_cache(&fx, 120).await;
    let warmup = warmup_days(CTL_WINDOW_DAYS);
    let trustworthy_from = to - Duration::days(120) + Duration::days(warmup);

    let ask_to = to - Duration::days(80);
    assert!(
        trustworthy_from > ask_to,
        "fixture must exercise the shallow branch: trustworthy_from={trustworthy_from} ask_to={ask_to}"
    );

    let computed = compute_and_persist_history(&fx.runtime, fx.tenant, fx.user_id, from, ask_to)
        .await
        .expect("compute reads the cache and must not need a provider");

    assert_eq!(
        computed.rows_cleared,
        (ask_to - from).num_days() as u64 + 1,
        "exactly the requested days are cleared, no more"
    );

    // The day after the ask, and every day up to `trustworthy_from`, is outside
    // the request and must be untouched — this is what the unclamped bound took.
    let beyond = fetch_history_rows(
        &fx.runtime.data(),
        fx.tenant,
        fx.user_id,
        ask_to + Duration::days(1),
        to,
    )
    .await
    .unwrap();
    assert_eq!(
        beyond.len(),
        (to - ask_to).num_days() as usize,
        "a narrow ask deleted days it never named: expected {} rows after {ask_to}, got {}",
        (to - ask_to).num_days(),
        beyond.len()
    );
}
