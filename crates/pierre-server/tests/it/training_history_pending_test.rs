// ABOUTME: A training-history window short of stored history says which walk it waits on
// ABOUTME: Never walked asks a walk and reports history_not_walked; a running walk reports its months and ETA

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The history walk a short training-history window waits on (carnet#1482).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// `get_training_history` and `compute_training_history` answer with one
// `Coverage`. When the stored history cannot warm the whole window, its
// `pending` says what fills it: a source never walked is asked for a walk
// on the spot and reported `history_not_walked`; a walk already running is
// reported with the months it walked, the months it has to cover and the
// seconds left at the pace its steps measured. A complete window owes no
// walk, and asks none.
#![cfg(feature = "tools-data")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use chrono::{Duration, Utc};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::models::{
    ActivityBuilder, ConnectionType, DailyTrainingState, SportType, TenantId,
};
use pierre_database::repositories::{
    BackfillJobKind, BackfillJobState, JobFloor, JobLease, JobProgressStep,
};
use pierre_mcp_server::tools::implementations::endurance_history::GetTrainingHistoryTool;
use pierre_tool_runtime::activity_backfill::history_walk::WHOLE_ACCOUNT_AFTER;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::{compute_and_persist_history, default_window};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};
use crate::helpers::history_walk_ledger::owe_history_walk;
use crate::helpers::sciotte_mock::seed_scrape_session;

/// The athlete's connection: a usr/pwd session, the source a walk reads.
const BACKEND: &str = "sciotte";

struct Fixture {
    runtime: Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant: TenantId,
}

/// An athlete with a live session to walk and nothing stored yet.
async fn fixture() -> Fixture {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, BACKEND, &ConnectionType::Manual, None)
        .await
        .unwrap();
    seed_scrape_session(&resources, user_id, tenant, BACKEND, "pending-session").await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    Fixture {
        runtime,
        user_id,
        tenant,
    }
}

/// Store a run every third day from `oldest_days_ago` up to today.
async fn store_runs(fx: &Fixture, oldest_days_ago: i64) {
    let mut runs = Vec::new();
    let mut days_ago = oldest_days_ago;
    while days_ago >= 0 {
        runs.push(
            ActivityBuilder::new(
                format!("run-{days_ago}"),
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
        days_ago -= 3;
    }
    fx.runtime
        .repos()
        .activity_cache
        .upsert_activities(fx.user_id, &fx.tenant, BACKEND, &runs)
        .await
        .unwrap();
}

/// `get_training_history` for the default window, as a chat turn calls it.
async fn read_history(fx: &Fixture) -> Value {
    let ctx = ToolContext::new()
        .with_user(fx.user_id.to_string())
        .with_tenant(fx.tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = GetTrainingHistoryTool
        .execute(&fx.runtime, &ctx, json!({}))
        .await;
    let result = response
        .structured_content
        .expect("tool result carries structured content");
    assert!(
        result.get("error").is_none(),
        "the read must answer: {result}"
    );
    result
}

/// What the ledger holds for the session's walk.
async fn walk_state(fx: &Fixture) -> Option<BackfillJobState> {
    fx.runtime
        .repos()
        .activity_backfill_jobs
        .backfill_job_state(fx.user_id, fx.tenant, BACKEND, BackfillJobKind::HistoryWalk)
        .await
        .unwrap()
}

/// A walk of the session that has walked 36 of its 120 months at 4 s a month.
async fn walk_a_third_down(fx: &Fixture) -> JobLease {
    let lease = owe_history_walk(fx.runtime.repos(), fx.user_id, fx.tenant, BACKEND).await;
    let step = JobProgressStep {
        before_ts: (Utc::now() - Duration::days(36 * 30)).timestamp(),
        months_walked: 36,
        rows_landed: 140,
        secs_per_unit: Some(4.0),
        floor: Some(JobFloor {
            after_ts: WHOLE_ACCOUNT_AFTER,
            months_total: 120,
        }),
    };
    assert!(
        fx.runtime
            .repos()
            .activity_backfill_jobs
            .record_job_progress(&lease, step)
            .await
            .unwrap(),
        "the lease holds the walk"
    );
    lease
}

/// The running walk [`walk_a_third_down`] records: 84 months left at 4 s.
fn a_third_down() -> Value {
    json!({
        "kind": "history_walk_running",
        "months_done": 36,
        "months_total": 120,
        "eta_secs": 84 * 4,
    })
}

/// Nothing has walked the session, and 40 days cannot warm a 90-day window:
/// the read asks a walk and says the window waits on one.
#[tokio::test]
async fn a_history_never_walked_asks_a_walk_and_says_so() {
    let fx = fixture().await;
    store_runs(&fx, 40).await;
    assert_eq!(walk_state(&fx).await, None, "nothing walked yet");

    let result = read_history(&fx).await;

    let coverage = &result["coverage"];
    assert_eq!(coverage["complete"], false, "{result}");
    assert_eq!(coverage["incomplete_reason"], "shallow_history", "{result}");
    assert_eq!(
        coverage["pending"],
        json!({ "kind": "history_not_walked" }),
        "bug: a window short of history names the walk it waits on: {result}"
    );
    assert!(
        coverage["note"]
            .as_str()
            .is_some_and(|note| note.contains("being fetched in the background")),
        "bug: the note says the history is being fetched: {result}"
    );
    assert!(
        walk_state(&fx).await.is_some(),
        "bug: a history never walked must be asked for a walk"
    );
}

/// Persist ten rollup rows ending on the default window's last day, as a
/// compute over a narrower window would have left them.
async fn persist_last_ten_days(fx: &Fixture) {
    let (_, to) = default_window(&fx.runtime, fx.user_id).await.unwrap();
    let rows: Vec<DailyTrainingState> = (0..10)
        .map(|back| DailyTrainingState::zero(to - Duration::days(back)))
        .collect();
    fx.runtime
        .repos()
        .training_history
        .upsert_training_history_batch(fx.tenant, fx.user_id, &rows)
        .await
        .unwrap();
}

/// The rollup holds only the last ten days, as a narrower compute left it,
/// but the store warms the whole window: the read answers every day of it
/// from the stored sessions, never `complete` beside days that start late.
#[tokio::test]
async fn rows_that_start_late_over_a_deep_store_answer_the_whole_window() {
    let fx = fixture().await;
    store_runs(&fx, 200).await;
    persist_last_ten_days(&fx).await;
    let (from, to) = default_window(&fx.runtime, fx.user_id).await.unwrap();

    let result = read_history(&fx).await;

    let days = result["days"].as_array().expect("days");
    assert_eq!(
        days.first().and_then(|day| day["date"].as_str()),
        Some(from.format("%Y-%m-%d").to_string().as_str()),
        "bug: a complete window starts on `from`: {result}"
    );
    assert_eq!(
        i64::try_from(days.len()).unwrap(),
        (to - from).num_days() + 1,
        "{result}"
    );
    assert_eq!(result["coverage"]["complete"], true, "{result}");
    assert_eq!(
        walk_state(&fx).await,
        None,
        "bug: a complete read asked a walk"
    );
}

/// The rollup's rows start late and the store is short too: the stored
/// sessions warm no day, so the rollup's rows still answer, the stored
/// sessions say why the window is short, and the walk is asked.
#[tokio::test]
async fn rows_that_start_late_over_a_short_store_ask_the_walk() {
    let fx = fixture().await;
    store_runs(&fx, 40).await;
    persist_last_ten_days(&fx).await;

    let result = read_history(&fx).await;

    assert_eq!(
        result["days"].as_array().map(Vec::len),
        Some(10),
        "the rollup's rows still answer: {result}"
    );
    let coverage = &result["coverage"];
    assert_eq!(coverage["complete"], false, "{result}");
    assert_eq!(coverage["incomplete_reason"], "shallow_history", "{result}");
    assert_eq!(coverage["window"]["returned"], 10, "{result}");
    assert_eq!(
        coverage["pending"],
        json!({ "kind": "history_not_walked" }),
        "{result}"
    );
    assert!(walk_state(&fx).await.is_some(), "bug: no walk was asked");
}

/// A walk is running: the read reports its progress and the seconds left at
/// its measured pace, and asks no second walk.
#[tokio::test]
async fn a_running_walk_reports_its_months_and_eta() {
    let fx = fixture().await;
    store_runs(&fx, 40).await;
    walk_a_third_down(&fx).await;

    let result = read_history(&fx).await;

    assert_eq!(
        result["coverage"]["pending"],
        a_third_down(),
        "bug: the running walk's progress and ETA: {result}"
    );
    assert_eq!(
        walk_state(&fx).await,
        Some(BackfillJobState::Owed),
        "the running walk is the one owed"
    );
}

/// The compute reads the same running walk into its coverage.
#[tokio::test]
async fn the_compute_reports_the_running_walk_it_waits_on() {
    let fx = fixture().await;
    store_runs(&fx, 40).await;
    walk_a_third_down(&fx).await;
    let (from, to) = default_window(&fx.runtime, fx.user_id).await.unwrap();

    let computed = compute_and_persist_history(&fx.runtime, fx.tenant, fx.user_id, from, to)
        .await
        .unwrap();

    assert!(
        !computed.capture_requested,
        "a running walk is not asked again"
    );
    assert_eq!(
        serde_json::to_value(computed.coverage.pending).unwrap(),
        a_third_down(),
        "bug: the compute names the walk the window waits on"
    );
}

/// 200 days warm the whole 90-day window: nothing is pending and no walk is
/// asked, even of a session never walked.
#[tokio::test]
async fn a_complete_window_owes_no_walk() {
    let fx = fixture().await;
    store_runs(&fx, 200).await;

    let result = read_history(&fx).await;

    assert_eq!(result["coverage"]["complete"], true, "{result}");
    assert!(result["coverage"].get("pending").is_none(), "{result}");
    assert_eq!(
        walk_state(&fx).await,
        None,
        "bug: a complete read asked a walk"
    );
}
