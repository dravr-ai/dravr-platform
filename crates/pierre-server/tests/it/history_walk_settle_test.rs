// ABOUTME: How a Garmin history walk settles: what it sits beside, retries, keeps, steps through and never announces
// ABOUTME: Pins the walk as a recorded property of its job, its retries, its wellness copy and the digest guard

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Settling a history walk (carnet#1482).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A walk is what its own ask recorded, never what the rows standing beside a
// job make it look like: an ordinary backfill owed before the grant stays
// ordinary when resumed, and a walk is recorded beside it instead of ending
// it at its own narrow window. A transient failure leaves the walk owed for
// the next run. A step that fills its limit reads the next one below the
// oldest activity it reached. A wellness copy that fails keeps the walk owed.
// And a walk copies the past: its write-through announces no session.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_sign_in_test::{
    athlete, calls_matching, hold_grant, hold_session, server, session_token_held, Calls,
    LIVE_SESSION, SIGNED_IN_SESSION,
};
use crate::history_walk_test::{
    age_job_rows, ask_walk, job_row, job_state, job_state_of, rows_under, spawn_switchable_scraper,
    walk_ended, ScraperSwitch, WALK_DEADLINE,
};

use std::slice;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Duration as ChronoDuration, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::{ActivityBuilder, CoachingPersona, SportType, TenantId};
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobClaim, BackfillJobKind, BackfillJobState, JobLease,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_notifications::models::NotificationCategory;
use pierre_notifications::models::TransportPolicy as PushTransportPolicy;
use pierre_notifications::{
    DispatchOutcome, DispatchRequest, NotificationEvent, PushTier, TenantId as CommTenantId,
};
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::activity_backfill::history_walk::{
    start_history_walk, WHOLE_ACCOUNT_AFTER,
};
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::activity_fetch::write_through::write_through_served_window;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use tokio::time::sleep;
use uuid::Uuid;

/// An ordinary backfill of the session, as a chat or the capture rail asks
/// for one: a one-year window, a conversation to answer.
fn ordinary_row(
    user_id: Uuid,
    tenant: TenantId,
    created_at_ms: i64,
    leased_until_ms: i64,
) -> ActivityBackfillJobRow {
    ActivityBackfillJobRow {
        after_ts: Some(Utc::now().timestamp() - 365 * 86_400),
        fetch_limit: Some(200),
        conversation_id: Some("conversation-that-asked".to_owned()),
        created_at_ms,
        leased_until_ms,
        lease_token: Some(Uuid::new_v4().to_string()),
        kind: BackfillJobKind::Backfill,
        ..job_row(user_id, tenant, created_at_ms)
    }
}

/// Claim every lapsed row as the sweep would, an hour from now.
async fn claim_all(resources: &Arc<ServerContext>) -> Vec<ActivityBackfillJobRow> {
    let now_ms = Utc::now().timestamp_millis() + 3_600_000;
    resources
        .common
        .repos
        .activity_backfill_jobs
        .claim_stale_backfill_jobs(BackfillJobClaim {
            now_ms,
            queued_older_than_ms: 0,
            lease_ms: 60_000,
            max_attempts: 10,
            limit: 10,
        })
        .await
        .unwrap()
}

/// A sign-in's walk finds an ordinary backfill already owed for the
/// session — one an earlier chat asked for. The walk is its own row beside
/// it: the ordinary job keeps its window and the conversation it answers,
/// and a second ask finds the walk already on file.
#[tokio::test]
async fn a_walk_is_recorded_beside_the_ordinary_job_owed_for_its_session() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let now = Utc::now().timestamp_millis();
    let ordinary = ordinary_row(user_id, tenant, now - 600_000, 0);
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .record_backfill_job(&ordinary)
            .await
            .unwrap(),
        "bug: the ledger records the ordinary job"
    );

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let walk = || start_history_walk(&runtime, user_id, tenant, oauth_providers::SCIOTTE_GARMIN);
    assert!(walk().await, "bug: a walk is on file for the session");
    assert!(!walk().await, "a second ask finds the walk already on file");
    assert!(
        job_state(&resources, user_id, tenant).await.is_some(),
        "bug: the walk has its own row beside the ordinary job"
    );

    let claimed: Vec<ActivityBackfillJobRow> = resources
        .common
        .repos
        .activity_backfill_jobs
        .claim_stale_backfill_jobs(BackfillJobClaim {
            now_ms: Utc::now().timestamp_millis(),
            queued_older_than_ms: 0,
            lease_ms: 60_000,
            max_attempts: 10,
            limit: 10,
        })
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.kind == BackfillJobKind::Backfill)
        .collect();
    assert_eq!(claimed.len(), 1, "bug: the ordinary job is still owed");
    let row = &claimed[0];
    assert_eq!(
        row.id, ordinary.id,
        "bug: the ordinary row is the one it was"
    );
    assert_eq!(
        row.after_ts, ordinary.after_ts,
        "bug: the ordinary job keeps its own window"
    );
    assert_eq!(
        row.conversation_id.as_deref(),
        Some("conversation-that-asked"),
        "bug: the ordinary job still answers the chat that asked"
    );
}

/// An ordinary backfill owed for the session since before the grant is
/// resumed as what it was: it is not taken for a walk because a grant now
/// stands beside it, so its end retires nothing.
#[tokio::test]
async fn a_resumed_ordinary_backfill_beside_a_grant_is_no_walk() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let long_ago = Utc::now().timestamp_millis() - 600_000;
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .record_backfill_job(&ordinary_row(user_id, tenant, long_ago, 0))
            .await
            .unwrap(),
        "bug: the ledger records the ordinary job"
    );

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        resume_backfill_jobs(runtime).await.unwrap(),
        1,
        "bug: the sweep resumes the owed backfill"
    );
    let deadline = Instant::now() + WALK_DEADLINE;
    while job_state_of(&resources, user_id, tenant, BackfillJobKind::Backfill)
        .await
        .is_some()
    {
        assert!(
            Instant::now() < deadline,
            "the resumed backfill never ended"
        );
        sleep(Duration::from_millis(50)).await;
    }

    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: an ordinary backfill never retires the session"
    );
    assert!(
        calls_matching(&calls, &format!("activities {LIVE_SESSION}"))
            .iter()
            .all(|call| !call.contains(&WHOLE_ACCOUNT_AFTER.to_string())),
        "bug: the ordinary backfill keeps its own window"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
}

/// The scraper fails the walk's read once: the job stays owed for the
/// resume sweep and the session stays for it — nothing is dropped or marked
/// incomplete on a hiccup.
#[tokio::test]
async fn a_transient_failure_keeps_the_walk_owed_and_the_session() {
    let switch = ScraperSwitch::default();
    switch.fail_next.store(true, Ordering::SeqCst);
    let (scraper, calls) = spawn_switchable_scraper(switch).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;
    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: the session is walked"
    );
    let deadline = Instant::now() + WALK_DEADLINE;
    while calls_matching(&calls, &format!("activities {SIGNED_IN_SESSION}")).is_empty() {
        assert!(Instant::now() < deadline, "the walk never read");
        sleep(Duration::from_millis(50)).await;
    }
    sleep(Duration::from_millis(500)).await;

    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: a transient failure leaves the walk owed"
    );
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: a transient failure never retires the session"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
}

/// A history walk copies the past: its write-through announces no session,
/// so an athlete with an armed per-session digest and pushes held for it gets
/// no digest from the copy — while an ordinary write of a new session still
/// sends one.
#[tokio::test]
async fn a_walk_never_sends_the_per_session_digest() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = common::create_test_server_resources_with_config(
        common::test_server_config_with_sciotte(&scraper),
    )
    .await
    .unwrap();
    resources
        .fitness
        .persona_contract_registry
        .apply_overlay(
            "version: 1\npersonas:\n  power_athlete:\n    notification:\n      tier_floor: P2\n      digest: per_session\n",
        )
        .unwrap();
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    let repos = &resources.common.repos;
    repos
        .users
        .set_coaching_persona(user_id, CoachingPersona::PowerAthlete)
        .await
        .unwrap();
    repos
        .feature_flags
        .set_user_override(user_id, FeatureKey::PersonaNotificationPolicy, true, None)
        .await
        .unwrap();
    let service = resources
        .common
        .notification_service
        .clone()
        .expect("the server wires a notification service");
    let held = service
        .dispatch_with_tier(
            &DispatchRequest {
                user_id,
                tenant_id: CommTenantId(tenant.as_uuid()),
                category: NotificationCategory::Training,
                notification_type: "training_load_alert".to_owned(),
                title: "Training load elevated".to_owned(),
                body: "held for the digest".to_owned(),
                data: Some(json!({ "screen": "coach" })),
                image_url: None,
                actions: None,
                bypass_frequency_cap: false,
                transport_policy: PushTransportPolicy::AnyTransport,
            },
            PushTier::P3,
        )
        .await
        .unwrap();
    assert!(
        matches!(held, DispatchOutcome::PersistedNoDevices { .. }),
        "the armed gate holds the push: {held:?}"
    );
    let digests = || {
        let service = service.clone();
        async move {
            let (rows, _, _) = service
                .list_notifications(user_id, CommTenantId(tenant.as_uuid()), 50, 0, None, false)
                .await
                .unwrap();
            rows.iter()
                .filter(|n| n.notification_type == NotificationEvent::PersonaSessionDigest.wire())
                .count()
        }
    };

    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;
    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: the session is walked"
    );
    walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "the copy landed, its ride three days old the newest in the cache"
    );
    sleep(Duration::from_millis(500)).await;
    assert_eq!(digests().await, 0, "bug: a history walk sends no digest");

    // Control: a new session written by an ordinary read does send it.
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ride = ActivityBuilder::new(
        "new-ride",
        "Morning ride",
        SportType::Ride,
        Utc::now() - chrono::Duration::minutes(10),
        3_600,
        oauth_providers::GARMIN,
    )
    .build();
    write_through_served_window(
        &runtime,
        user_id,
        &tenant,
        oauth_providers::GARMIN,
        &ActivityQueryParams::default(),
        slice::from_ref(&ride),
    )
    .await;
    let deadline = Instant::now() + WALK_DEADLINE;
    while digests().await == 0 {
        assert!(
            Instant::now() < deadline,
            "the control session never sent its digest"
        );
        sleep(Duration::from_millis(50)).await;
    }
}

/// A step whose read fills its fetch limit has more in its window: the row
/// stays owed for the step below the oldest activity it reached, never below
/// the window, so nothing between the two is skipped. The next step is read
/// under that `before`: 36 months below the ride three days old hold
/// nothing, so that empty step ends the walk there (B3), keeping the
/// session (B1).
#[tokio::test]
async fn a_step_that_fills_its_limit_reads_the_next_below_its_oldest_activity() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let long_ago = Utc::now().timestamp_millis() - 600_000;
    let mut row = job_row(user_id, tenant, long_ago);
    // The stand-in's first step holds one ride: a limit of one fills.
    row.fetch_limit = Some(1);
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .record_backfill_job(&row)
            .await
            .unwrap(),
        "bug: the ledger records the walk"
    );
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let reads = || calls_matching(&calls, &format!("activities {LIVE_SESSION}"));

    assert_eq!(
        resume_backfill_jobs(Arc::clone(&runtime)).await.unwrap(),
        1,
        "bug: the sweep resumes the owed walk"
    );
    let deadline = Instant::now() + WALK_DEADLINE;
    let stepped = loop {
        let claimed = resources
            .common
            .repos
            .activity_backfill_jobs
            .claim_stale_backfill_jobs(BackfillJobClaim {
                now_ms: Utc::now().timestamp_millis(),
                queued_older_than_ms: 0,
                lease_ms: 0,
                max_attempts: 10,
                limit: 10,
            })
            .await
            .unwrap();
        if let Some(stepped) = claimed.into_iter().next() {
            break stepped;
        }
        assert!(Instant::now() < deadline, "the first step never landed");
        sleep(Duration::from_millis(50)).await;
    };
    let recent = resources
        .common
        .repos
        .activity_cache
        .oldest_activity_start(user_id, &tenant, oauth_providers::SCIOTTE_GARMIN)
        .await
        .unwrap()
        .expect("the first step landed its ride")
        .timestamp();
    assert_eq!(
        stepped.before_ts,
        Some(recent),
        "bug: the next step reads below the oldest activity reached, not below the window"
    );
    assert_eq!(
        stepped.kind,
        BackfillJobKind::HistoryWalk,
        "bug: a stepped row stays a history walk"
    );
    assert_eq!(reads().len(), 1, "bug: one step is one session read");

    // The claim above leased the row for no time at all: the sweep takes it
    // again, and the walk goes on to the end of the history.
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk reaches the end of the history"
    );
    let reads = reads();
    assert_eq!(
        reads.len(),
        2,
        "bug: the filled step and the empty one below it: {reads:?}"
    );
    assert!(
        reads[1].contains(&format!("before={recent}")),
        "bug: the second step is read below the first's oldest activity: {reads:?}"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        1,
        "bug: the walk ends at the first 36 empty months, below the ride it landed"
    );
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: the walked session stays (B1)"
    );
}

/// The activities land but every wellness read fails: the session that holds
/// those nights is not retired and the walk stays owed for the next run —
/// a hiccup never costs the athlete their sleep history.
#[tokio::test]
async fn a_failed_wellness_copy_keeps_the_session_for_the_next_run() {
    let switch = ScraperSwitch::default();
    switch.fail_summaries.store(true, Ordering::SeqCst);
    let (scraper, calls) = spawn_switchable_scraper(switch).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;
    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: the session is walked"
    );
    wellness_read_settled(&resources, &calls, user_id, tenant).await;

    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: a failed wellness copy never retires the session"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: the walk stays owed for the next run"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
}

/// One day of the year before the grant fails to read while every other
/// lands: the copy is not complete, so the session that still holds that
/// day is kept and the walk stays owed, rather than retired over a hole.
#[tokio::test]
async fn one_unread_wellness_day_keeps_the_session_for_the_next_run() {
    let switch = ScraperSwitch::default();
    let hole = Utc::now().date_naive() - ChronoDuration::days(200);
    *switch.fail_summary_on.lock().unwrap() = Some(hole);
    let (scraper, calls) = spawn_switchable_scraper(switch).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;
    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: the session is walked"
    );
    wellness_read_settled(&resources, &calls, user_id, tenant).await;

    let landed = resources
        .common
        .repos
        .recovery
        .get_recovery_metrics(
            user_id,
            &tenant,
            Utc::now() - ChronoDuration::days(400),
            Utc::now() + ChronoDuration::days(1),
        )
        .await
        .unwrap();
    assert!(
        landed.len() > 300 && landed.iter().all(|morning| morning.date != hole),
        "the other days landed, the failed one did not: {} mornings",
        landed.len()
    );
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: a copy with an unread day never retires the session"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: the walk stays owed for the next run"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
}

/// Run the walk the sweep's way until its activities landed, its wellness
/// read reached every day of the year before the grant, and its job row is
/// still owed.
async fn wellness_read_settled(
    resources: &Arc<ServerContext>,
    calls: &Calls,
    user_id: Uuid,
    tenant: TenantId,
) {
    let deadline = Instant::now() + WALK_DEADLINE;
    while calls_matching(calls, &format!("summary {SIGNED_IN_SESSION}")).len() < 365
        || rows_under(resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await < 3
        || job_state(resources, user_id, tenant).await != Some(BackfillJobState::Owed)
    {
        assert!(
            Instant::now() < deadline,
            "the walk never settled after its wellness"
        );
        age_job_rows(resources, user_id).await;
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        resume_backfill_jobs(runtime).await.unwrap();
        sleep(Duration::from_millis(50)).await;
    }
}

/// A claim the sweep cannot run is handed back with its attempt, so waiting
/// on a run already in flight never spends the job's attempts.
#[tokio::test]
async fn a_released_claim_returns_its_attempt() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    let ledger = &resources.common.repos.activity_backfill_jobs;
    assert!(
        ledger
            .record_backfill_job(&job_row(user_id, tenant, 0))
            .await
            .unwrap(),
        "bug: the ledger records the job"
    );
    let claimed = claim_all(&resources).await;
    assert_eq!(
        claimed[0].attempts, 2,
        "bug: the claim counts one more attempt"
    );
    let lease = JobLease {
        id: claimed[0].id.clone(),
        token: claimed[0].lease_token.clone().unwrap(),
    };
    assert!(
        ledger.release_backfill_job_claim(&lease).await.unwrap(),
        "bug: the lease holder hands its claim back"
    );
    assert!(
        !ledger.release_backfill_job_claim(&lease).await.unwrap(),
        "a released claim holds the row no more"
    );
    let again = claim_all(&resources).await;
    assert_eq!(again.len(), 1, "bug: a released row is claimable at once");
    assert_eq!(
        again[0].attempts, 2,
        "bug: the release returned the attempt"
    );
}

/// A row that records how one tenant's walk ended is never replaced by an
/// ask for the same user and provider from another tenant: the first
/// tenant's card keeps reading what its own walk did.
#[tokio::test]
async fn an_ask_from_another_tenant_never_replaces_an_ended_row() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    let (_other_user, _user, other_tenant, _token) = athlete(&resources).await;
    let ledger = &resources.common.repos.activity_backfill_jobs;
    let mut row = job_row(user_id, tenant, 0);
    row.lease_token = Some(Uuid::new_v4().to_string());
    assert!(
        ledger.record_backfill_job(&row).await.unwrap(),
        "bug: the ledger records the job"
    );
    let lease = JobLease {
        id: row.id.clone(),
        token: row.lease_token.clone().unwrap(),
    };
    assert!(
        ledger.end_backfill_job_incomplete(&lease).await.unwrap(),
        "bug: the lease holder ends its job incomplete"
    );

    assert!(
        !ledger
            .record_backfill_job(&job_row(user_id, other_tenant, 0))
            .await
            .unwrap(),
        "bug: an ask from another tenant replaced this tenant's ended row"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Incomplete),
        "bug: this tenant's card no longer reads its own walk's end"
    );
    assert_eq!(
        job_state(&resources, user_id, other_tenant).await,
        None,
        "bug: the other tenant took the row over"
    );

    assert!(
        ledger
            .record_backfill_job(&job_row(user_id, tenant, 0))
            .await
            .unwrap(),
        "bug: an ask from the row's own tenant replaces its ended row"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: the own tenant's new ask is owed"
    );
}
