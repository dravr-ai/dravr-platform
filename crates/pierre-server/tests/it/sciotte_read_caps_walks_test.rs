// ABOUTME: Work nobody waits on spends an athlete's background session reads, and a capped one waits instead of failing
// ABOUTME: Resume-sweep re-runs, the sign-in prefetch, the health sync, the wellness first fill and the history walk (carnet#1661)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The background side of the session-read caps, end to end (carnet#1661).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// Which caps a read of an athlete's signed-in session spends is decided where
// it is admitted, by the `in_background` scope of the task that makes it. A
// re-run the resume sweep starts and the pre-fetch a sign-in starts are work
// nobody waits on, so their Garmin session reads are counted in the athlete's
// background windows and never in the asked ones.
//
// A background read a cap refuses waits for the window to open rather than
// failing: the health sync keeps the days it read and its cursor stops at
// the refused day; the first wellness copy is counted in a day of its own,
// keeps the day it stopped at and resumes from it; a history walk pauses.
// None of them spends one of the walk's attempts on a refusal.
//
// The scraper is the Garmin stand-in the sign-in suite runs (a test double,
// per the repo's mock rule), which records every call it serves. Every
// fixture athlete has their caps lifted to the ceiling; a test spends a
// window by writing its counter, as reads would, and opens it again by
// clearing it, as the next window does: the server keeps an athlete's caps
// for a minute, so a cap written mid-test would not be seen.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_sign_in_test::{
    athlete, calls_matching, hold_grant, hold_session, server, session_read_ceiling,
    set_session_read_caps, sign_in, spawn_switchable_scraper, Calls, ScraperSwitch, LIVE_SESSION,
    SIGNED_IN_SESSION,
};
use crate::history_walk_test::{
    age_job_rows, ask_walk, job_state, paused_retries_due, walk_ended, WALK_DEADLINE,
};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use chrono::{Duration, NaiveDate, Utc};
use dravr_enforme::traits::cursor_store::SyncCursorStore;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{SessionReadCap, TenantId, UserTier};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobKind, BackfillJobState, JobProgress,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::request_budget::{
    budget_period, in_background, session_read_counter_key, ONE_DAY, PLATFORM_SCOPE,
};
use pierre_services::health_sync::PierreSyncStorage;
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use sqlx::Row;
use tokio::time::sleep;
use uuid::Uuid;

/// How many times a test lets a capped walk be claimed again: more than its
/// attempts, so a refusal that spent one would end it.
const REFUSALS: usize = 4;

/// The athlete's Garmin session reads counted under `counter` in the
/// current `window`.
async fn reads(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    counter: &str,
    window: StdDuration,
) -> i64 {
    resources
        .common
        .repos
        .usage_counters
        .get_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &session_read_counter_key(oauth_providers::SCIOTTE_GARMIN, counter, user_id),
            &budget_period(Utc::now(), window),
        )
        .await
        .unwrap()
        .value
}

/// Count `n` Garmin session reads under `counter` in the current `window`,
/// as that many reads would.
async fn spend(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    counter: &str,
    window: StdDuration,
    n: u32,
) {
    resources
        .common
        .repos
        .usage_counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &session_read_counter_key(oauth_providers::SCIOTTE_GARMIN, counter, user_id),
            &budget_period(Utc::now(), window),
            i64::from(n),
        )
        .await
        .unwrap();
}

/// Open every session-read window of the athlete again, as the next window
/// is: no repository forgets a counter, so the fixture clears its rows.
async fn fresh_windows(resources: &Arc<ServerContext>, user_id: Uuid) {
    let sql = "DELETE FROM usage_counters WHERE counter_key LIKE $1";
    let keys = format!("provider_session_reads:%:{user_id}");
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql)
                .bind(keys)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(sql)
                .bind(keys)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

/// What the ledger holds of the athlete's history walk that a step settles.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WalkRow {
    attempts: i64,
    next_attempt_ms: i64,
    leased_until_ms: i64,
    wellness_resume_day: Option<String>,
}

impl WalkRow {
    /// Released by its step and waiting for a window that has not opened.
    fn paused(&self) -> bool {
        self.leased_until_ms == 0 && self.next_attempt_ms > Utc::now().timestamp_millis()
    }
}

/// The athlete's history walk row.
async fn walk_row(resources: &Arc<ServerContext>, user_id: Uuid) -> Option<WalkRow> {
    let sql = "SELECT attempts, next_attempt_ms, leased_until_ms, wellness_resume_day \
               FROM activity_backfill_jobs WHERE user_id = $1 AND kind = 'history_walk'";
    macro_rules! read {
        ($pool:expr) => {{
            sqlx::query(sql)
                .bind(user_id.to_string())
                .fetch_optional($pool)
                .await
                .unwrap()
                .map(|row| WalkRow {
                    attempts: row.get("attempts"),
                    next_attempt_ms: row.get("next_attempt_ms"),
                    leased_until_ms: row.get("leased_until_ms"),
                    wellness_resume_day: row.get("wellness_resume_day"),
                })
        }};
    }
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => read!(sqlite.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => read!(pg.pool()),
    }
}

/// Run the sweep until the athlete's walk is paused by a cap, and return
/// its row.
async fn walk_paused(resources: &Arc<ServerContext>, user_id: Uuid) -> WalkRow {
    let deadline = Instant::now() + WALK_DEADLINE;
    loop {
        if let Some(row) = walk_row(resources, user_id).await.filter(WalkRow::paused) {
            return row;
        }
        assert!(
            Instant::now() < deadline,
            "the walk was never paused (row: {:?})",
            walk_row(resources, user_id).await
        );
        age_job_rows(resources, user_id).await;
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        resume_backfill_jobs(runtime).await.unwrap();
        sleep(StdDuration::from_millis(50)).await;
    }
}

/// Make the paused walk due, let the sweep claim it into the same spent
/// window [`REFUSALS`] times, and return its row after the last refusal.
async fn refused_again(resources: &Arc<ServerContext>, user_id: Uuid) -> WalkRow {
    let mut row = None;
    for _ in 0..REFUSALS {
        paused_retries_due(resources, user_id).await;
        row = Some(walk_paused(resources, user_id).await);
    }
    row.expect("the walk was refused")
}

/// The days the stand-in served the session's daily summary, in call order.
fn summary_days(calls: &Calls, session: &str) -> Vec<NaiveDate> {
    calls_matching(calls, &format!("summary {session} "))
        .iter()
        .map(|call| {
            call.rsplit(' ')
                .next()
                .and_then(|day| day.parse().ok())
                .unwrap_or_else(|| panic!("a summary call names its day: {call}"))
        })
        .collect()
}

/// Wait until the stand-in served an activity read, then a moment more for
/// anything started beside it.
async fn activities_read(calls: &Calls) {
    let deadline = Instant::now() + WALK_DEADLINE;
    while calls_matching(calls, "activities").is_empty() {
        assert!(Instant::now() < deadline, "the session was never read");
        sleep(StdDuration::from_millis(50)).await;
    }
    sleep(StdDuration::from_millis(500)).await;
}

/// A chat turn's live read that the turn's instance died under is re-run by
/// the resume sweep. The conversation that asked has its follow-up coming,
/// nobody waits on the re-run, so its session read is background.
#[tokio::test]
async fn a_resume_sweep_rerun_of_an_asked_read_spends_background_reads() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let asked = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        user_id,
        provider: oauth_providers::SCIOTTE_GARMIN.to_owned(),
        kind: BackfillJobKind::LiveRead,
        after_ts: Some((Utc::now() - Duration::days(30)).timestamp()),
        before_ts: None,
        fetch_limit: Some(50),
        conversation_id: Some("conversation-that-asked".to_owned()),
        created_at_ms: 0,
        leased_until_ms: 0,
        attempts: 1,
        lease_token: None,
        progress: JobProgress::default(),
        turn_id: Some("turn-that-asked".to_owned()),
        asked_message_id: Some("message-that-asked".to_owned()),
        walk_backend: None,
        walk_sport: None,
    };
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .record_backfill_job(&asked)
            .await
            .unwrap(),
        "the dead turn's read is recorded"
    );

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        resume_backfill_jobs(runtime).await.unwrap(),
        1,
        "the sweep re-runs the read"
    );
    activities_read(&calls).await;

    assert!(
        reads(&resources, user_id, "background", ONE_DAY).await > 0,
        "the re-run's session read is counted"
    );
    assert_eq!(
        reads(&resources, user_id, "asked", ONE_DAY).await,
        0,
        "bug: a re-run nobody waits on spent the athlete's asked session reads"
    );
}

/// The pre-fetch a Garmin sign-in starts fills the store ahead of the
/// athlete's first look, and the sign-in starts nothing else they wait on:
/// every session read it leads to is background.
#[tokio::test]
async fn a_sign_in_prefetch_spends_background_reads() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte_usr_pwd_garmin(
        &scraper,
    ))
    .await;
    let (user_id, _user, _tenant, token) = athlete(&resources).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "bug: the Garmin sign-in failed: {body}");
    activities_read(&calls).await;

    assert!(
        reads(&resources, user_id, "background", ONE_DAY).await > 0,
        "the pre-fetch's session read is counted"
    );
    assert_eq!(
        reads(&resources, user_id, "asked", ONE_DAY).await,
        0,
        "bug: the sign-in pre-fetch spent the athlete's asked session reads"
    );
}

/// A scheduled health sync of a Garmin session the background day caps
/// mid-window keeps the days it read and stops its cursor at the refused
/// day, so the next sync, once the window opens, goes on from there. The
/// refused reads reach nothing.
#[tokio::test]
async fn a_capped_health_sync_keeps_its_days_and_resumes_at_the_refused_one() {
    // Garmin's four wellness types each read a first window of 14 days,
    // sleep first, then recovery: 14 + 8 reads leave recovery 8 days in.
    const LEFT: u32 = 14 + 8;
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;
    let day = session_read_ceiling(&resources, SessionReadCap::BackgroundDaily).await;
    spend(&resources, user_id, "background", ONE_DAY, day - LEFT).await;
    let orchestrator = resources
        .sync_orchestrator()
        .expect("the test server runs the health sync")
        .clone();
    let storage = PierreSyncStorage::new(&resources.common.repos);
    let user = user_id.to_string();
    let today = Utc::now().date_naive();

    // Nobody waits on a scheduled sync.
    drop(in_background(orchestrator.sync_user(&user, "garmin")).await);

    assert_eq!(
        summary_days(&calls, SIGNED_IN_SESSION).len(),
        usize::try_from(LEFT).unwrap(),
        "bug: a read the cap refused reached the scraper"
    );
    assert_eq!(
        recovery_days(&resources, user_id, tenant).await,
        8,
        "bug: the days read before the refusal were not kept"
    );
    let cursor = storage
        .get_cursor(&user, "garmin", "recovery")
        .await
        .unwrap()
        .expect("the recovery read keeps a cursor");
    assert_eq!(
        cursor.value,
        (today - Duration::days(5)).to_string(),
        "bug: the next sync does not resume at the day the cap refused"
    );
    assert_eq!(
        reads(&resources, user_id, "asked", ONE_DAY).await,
        0,
        "a sync is background work"
    );

    fresh_windows(&resources, user_id).await;
    drop(in_background(orchestrator.sync_user(&user, "garmin")).await);
    assert_eq!(
        recovery_days(&resources, user_id, tenant).await,
        14,
        "bug: the next sync did not read the days the cap refused"
    );
    let cursor = storage
        .get_cursor(&user, "garmin", "recovery")
        .await
        .unwrap()
        .expect("the recovery read keeps a cursor");
    assert_eq!(
        cursor.value,
        today.to_string(),
        "bug: the resumed health sync did not reach today"
    );
}

/// The mornings the athlete has stored over the last fortnight.
async fn recovery_days(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) -> usize {
    resources
        .common
        .repos
        .recovery
        .get_recovery_metrics(
            user_id,
            &tenant,
            Utc::now() - Duration::days(20),
            Utc::now() + Duration::days(1),
        )
        .await
        .unwrap()
        .len()
}

/// A Garmin athlete's first wellness copy (the year before their push grant)
/// is counted in a day of its own, never in the background day the walk's
/// activity reads spend. Capped five days in, it keeps the sixth on its row
/// and pauses with no attempt spent, however often it is claimed into the
/// spent day; once the window opens it copies from the sixth day, reading no
/// day twice.
#[tokio::test]
async fn a_capped_first_wellness_fill_pauses_at_its_day_and_resumes_from_it() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;
    spend(
        &resources,
        user_id,
        "wellness_first_fill",
        ONE_DAY,
        session_read_ceiling(&resources, SessionReadCap::WellnessFirstFillDaily).await - 5,
    )
    .await;

    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "the session is walked"
    );
    let paused = walk_paused(&resources, user_id).await;
    let days = summary_days(&calls, SIGNED_IN_SESSION);
    assert_eq!(days.len(), 5, "bug: the copy read past its first-fill day");
    let start = days[0];
    assert_eq!(
        days,
        (0..5)
            .map(|n| start + Duration::days(n))
            .collect::<Vec<_>>(),
        "the copy reads oldest first"
    );
    let resume_day = start + Duration::days(5);
    assert_eq!(
        paused.wellness_resume_day,
        Some(resume_day.to_string()),
        "bug: the paused copy did not keep the day the cap refused"
    );
    assert_eq!(
        reads(&resources, user_id, "background", ONE_DAY).await,
        i64::try_from(calls_matching(&calls, "activities").len()).unwrap(),
        "bug: the first fill's days were counted in the background day"
    );

    let again = refused_again(&resources, user_id).await;
    assert_eq!(
        again.attempts, paused.attempts,
        "bug: a capped copy spent the walk's attempts"
    );
    assert_eq!(
        again.wellness_resume_day,
        Some(resume_day.to_string()),
        "bug: a re-claimed capped copy lost the day it stopped at"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: a capped copy ended the walk"
    );
    assert_eq!(
        summary_days(&calls, SIGNED_IN_SESSION).len(),
        5,
        "bug: a refused day reached the scraper"
    );

    fresh_windows(&resources, user_id).await;
    paused_retries_due(&resources, user_id).await;
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk never ended once its copy could go on"
    );
    let days = summary_days(&calls, SIGNED_IN_SESSION);
    assert_eq!(
        days[5], resume_day,
        "bug: the resumed copy did not start at the day it stopped at"
    );
    let distinct: HashSet<_> = days.iter().collect();
    assert_eq!(
        distinct.len(),
        days.len(),
        "bug: the resumed copy read a day twice"
    );
    assert_eq!(
        reads(&resources, user_id, "wellness_first_fill", ONE_DAY).await,
        i64::try_from(days.len() - 5).unwrap(),
        "every day the resumed copy read is counted in the first fill's day"
    );
}

/// Record the athlete's Garmin session walk anew over its ended row, due to
/// the next sweep, as a reopen of the walk does.
async fn walk_again(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let now = Utc::now().timestamp_millis();
    let recorded = resources
        .common
        .repos
        .activity_backfill_jobs
        .record_backfill_job(&ActivityBackfillJobRow {
            id: Uuid::new_v4().to_string(),
            tenant_id: tenant,
            user_id,
            provider: oauth_providers::SCIOTTE_GARMIN.to_owned(),
            kind: BackfillJobKind::HistoryWalk,
            after_ts: Some(WHOLE_ACCOUNT_AFTER),
            before_ts: None,
            fetch_limit: Some(200),
            conversation_id: None,
            created_at_ms: now,
            leased_until_ms: 0,
            attempts: 0,
            lease_token: None,
            progress: JobProgress::default(),
            turn_id: None,
            asked_message_id: None,
            walk_backend: None,
            walk_sport: None,
        })
        .await
        .unwrap();
    assert!(recorded, "the walk is recorded anew over its ended row");
}

/// The floor a walk of the whole account reads down to (2000-01-01).
const WHOLE_ACCOUNT_AFTER: i64 = 946_684_800;

/// Once a Garmin athlete's first wellness fill completes, a later copy of
/// their walk (the walk recorded again) counts in the ordinary background day,
/// never again in the first fill's own allowance.
#[tokio::test]
async fn a_copy_after_the_first_fill_counts_in_the_background_day() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;

    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "the session is walked"
    );
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "the first walk ends"
    );
    let first_copy = summary_days(&calls, SIGNED_IN_SESSION).len();
    assert!(first_copy > 0, "the first walk copied wellness");
    let first_fill = reads(&resources, user_id, "wellness_first_fill", ONE_DAY).await;
    assert_eq!(
        first_fill,
        i64::try_from(first_copy).unwrap(),
        "bug: the first copy's days were not counted as the first fill"
    );
    let background = reads(&resources, user_id, "background", ONE_DAY).await;

    // A walked session is not walked again on an ask; a reopen records the
    // walk anew over its ended row, as this does.
    walk_again(&resources, user_id, tenant).await;
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "the second walk ends"
    );
    let second_copy = summary_days(&calls, SIGNED_IN_SESSION).len() - first_copy;
    assert!(second_copy > 0, "the second walk copied wellness again");
    assert_eq!(
        reads(&resources, user_id, "wellness_first_fill", ONE_DAY).await,
        first_fill,
        "bug: a copy after the first fill completed spent the first fill's allowance"
    );
    assert!(
        reads(&resources, user_id, "background", ONE_DAY).await
            >= background + i64::try_from(second_copy).unwrap(),
        "bug: a copy after the first fill completed was not counted in the background day"
    );
}

/// A background day an admin set to 0 (the kill switch) refuses every read
/// until it is raised: a walk it refuses spends its attempts and ends
/// incomplete, rather than pausing and being claimed again forever.
#[tokio::test]
async fn a_walk_a_zero_cap_refuses_ends_incomplete_within_its_attempts() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    let off = HashMap::from([(
        SessionReadCap::BackgroundDaily.config_key(&UserTier::Starter),
        json!(0),
    )]);
    set_session_read_caps(&resources, user_id, off).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;

    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "the session is walked"
    );
    // Each failed step is retried a lease from now: make every retry due at
    // once, as the sweeps over that lease would.
    let deadline = Instant::now() + WALK_DEADLINE;
    loop {
        let state = job_state(&resources, user_id, tenant).await;
        if state == Some(BackfillJobState::Incomplete) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "bug: a walk the kill switch refuses never ends (row: {state:?})"
        );
        paused_retries_due(&resources, user_id).await;
        age_job_rows(&resources, user_id).await;
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        resume_backfill_jobs(runtime).await.unwrap();
        sleep(StdDuration::from_millis(50)).await;
    }
    assert_eq!(
        calls_matching(&calls, "activities"),
        Vec::<String>::new(),
        "bug: a read the kill switch refused reached the scraper"
    );
}

/// A history walk whose session read the background day refuses pauses
/// until the day ends, however often the sweep claims it: no attempt is
/// spent, so the walk never ends incomplete over a cap. Once the window
/// opens it walks to the end.
#[tokio::test]
async fn a_capped_history_walk_step_pauses_without_spending_an_attempt() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let day = session_read_ceiling(&resources, SessionReadCap::BackgroundDaily).await;
    spend(&resources, user_id, "background", ONE_DAY, day).await;

    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "the session is walked"
    );
    let paused = walk_paused(&resources, user_id).await;
    let again = refused_again(&resources, user_id).await;

    assert_eq!(
        again.attempts, paused.attempts,
        "bug: a capped step spent the walk's attempts"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: a capped step ended the walk"
    );
    assert_eq!(
        calls_matching(&calls, "activities"),
        Vec::<String>::new(),
        "bug: a refused step reached the scraper"
    );

    fresh_windows(&resources, user_id).await;
    paused_retries_due(&resources, user_id).await;
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk never went on once the window opened"
    );
    assert_ne!(
        calls_matching(&calls, "activities"),
        Vec::<String>::new(),
        "the walk read the session once the window opened"
    );
}
