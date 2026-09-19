// ABOUTME: A historical activity backfill is a ledger row before it is a task, and the resume sweep finishes what a dead instance left
// ABOUTME: Pins the spawn's record-then-run, the cross-instance dedup on the row, the sweep's claim, and the attempt cap's reap

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Before the ledger, a backfill lived only in the tokio task that ran it: a
//! scale-to-zero reclaim mid-scrape lost the job, and the athlete who had been
//! told "no need to ask again" got nothing (carnet#460). Now
//! `spawn_activity_backfill` writes a row before it spawns, a run that reaches
//! an outcome the athlete is told about deletes it, a transient failure leaves
//! it for the sweep, and the sweep claims whatever lapsed.
//!
//! The harness registers no provider fixture, so the two terminal outcomes a
//! run can reach here are the ones that need no network: a provider the
//! registry does not know fails transiently (the row stays), and a supported
//! provider with no token is a reconnect-needed outcome (the row is finished,
//! by the same rule a completed run takes). Every row assertion reads the
//! ledger table directly, so a field the spawn recorded wrong fails here.

#![cfg(feature = "tools-data")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::repositories::ActivityBackfillJobRow;
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::activity_backfill::{
    provider_tenant_id_str, spawn_activity_backfill, ActivityBackfillJob, BACKFILL_JOB_LEASE,
    MAX_BACKFILL_ATTEMPTS,
};
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::runtime::ToolRuntime;
use tokio::time::{sleep, Instant};
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};

/// A provider the registry does not know: its run fails before any fetch,
/// the transient outcome that leaves the row for the sweep.
const UNKNOWN_PROVIDER: &str = "nonesuch";

/// A supported provider with no token in the test tenant: its run is a
/// reconnect-needed outcome, which finishes the row.
const TOKENLESS_PROVIDER: &str = "strava";

/// A window deep enough to be a historical backfill — three years back.
const AFTER_TS: i64 = 1_600_000_000;
const BEFORE_TS: i64 = 1_700_000_000;
const FETCH_LIMIT: usize = 500;

/// How long a spawned run gets to reach its terminal outcome in the poll.
const RUN_BUDGET: Duration = Duration::from_secs(30);

struct Fixture {
    runtime: Arc<dyn ToolRuntime>,
    database: Arc<Database>,
    user_id: Uuid,
    tenant: TenantId,
}

async fn fixture() -> Fixture {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenants = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;
    let database = Arc::clone(&resources.agent.database);
    let runtime: Arc<dyn ToolRuntime> = resources;
    Fixture {
        runtime,
        database,
        user_id,
        tenant,
    }
}

/// The ledger row as stored, read straight from the table.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredJob {
    id: String,
    provider: String,
    after_ts: Option<i64>,
    before_ts: Option<i64>,
    fetch_limit: Option<i64>,
    conversation_id: Option<String>,
    created_at_ms: i64,
    leased_until_ms: i64,
    attempts: i64,
}

type StoredJobTuple = (
    String,
    String,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    i64,
    i64,
    i64,
);

async fn stored_job(fx: &Fixture, provider: &str) -> Option<StoredJob> {
    const SQL: &str = "SELECT id, provider, after_ts, before_ts, fetch_limit, conversation_id, \
                       created_at_ms, leased_until_ms, attempts \
                       FROM activity_backfill_jobs WHERE user_id = $1 AND provider = $2";
    let user_id = fx.user_id.to_string();
    let row: Option<StoredJobTuple> = match fx.database.as_ref() {
        Database::SQLite(db) => sqlx::query_as(SQL)
            .bind(&user_id)
            .bind(provider)
            .fetch_optional(db.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => sqlx::query_as(SQL)
            .bind(&user_id)
            .bind(provider)
            .fetch_optional(db.pool())
            .await
            .unwrap(),
    };
    row.map(
        |(
            id,
            provider,
            after_ts,
            before_ts,
            fetch_limit,
            conversation_id,
            created_at_ms,
            leased_until_ms,
            attempts,
        )| StoredJob {
            id,
            provider,
            after_ts,
            before_ts,
            fetch_limit,
            conversation_id,
            created_at_ms,
            leased_until_ms,
            attempts,
        },
    )
}

/// Wait for the spawned run to settle its row, or fail loudly.
async fn wait_until_row_is_gone(fx: &Fixture, provider: &str) {
    let deadline = Instant::now() + RUN_BUDGET;
    while stored_job(fx, provider).await.is_some() {
        assert!(
            Instant::now() < deadline,
            "the {provider} run never settled its ledger row within {RUN_BUDGET:?}"
        );
        sleep(Duration::from_millis(50)).await;
    }
}

fn job(fx: &Fixture, provider: &str, conversation_id: Option<&str>) -> ActivityBackfillJob {
    ActivityBackfillJob {
        resources: Arc::clone(&fx.runtime),
        user_id: fx.user_id,
        tenant_id: fx.tenant,
        tenant_id_str: provider_tenant_id_str(fx.tenant),
        provider_name: provider.to_owned(),
        query_params: ActivityQueryParams {
            limit: Some(FETCH_LIMIT),
            offset: None,
            before: Some(BEFORE_TS),
            after: Some(AFTER_TS),
        },
        pierre_conversation_id: conversation_id.map(ToOwned::to_owned),
    }
}

/// A row as another instance would have recorded it, `age` ago, with the
/// given lease and attempt count.
fn seeded_row(
    fx: &Fixture,
    provider: &str,
    age: Duration,
    lease: Duration,
    attempts: i64,
) -> ActivityBackfillJobRow {
    let now = Utc::now().timestamp_millis();
    let age_ms = i64::try_from(age.as_millis()).unwrap();
    let lease_ms = i64::try_from(lease.as_millis()).unwrap();
    ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: fx.tenant,
        user_id: fx.user_id,
        provider: provider.to_owned(),
        after_ts: Some(AFTER_TS),
        before_ts: Some(BEFORE_TS),
        fetch_limit: Some(i64::try_from(FETCH_LIMIT).unwrap()),
        conversation_id: None,
        created_at_ms: now - age_ms,
        leased_until_ms: if lease.is_zero() { 0 } else { now + lease_ms },
        attempts,
    }
}

fn lease_ms() -> i64 {
    i64::try_from(BACKFILL_JOB_LEASE.as_millis()).unwrap()
}

/// The spawn records the window, the conversation and a first attempt under
/// a live lease before its run starts; a run that fails transiently leaves
/// that row exactly as recorded, for the sweep.
#[tokio::test]
async fn spawn_records_the_row_and_a_transient_failure_leaves_it() {
    let fx = fixture().await;
    let before_spawn = Utc::now().timestamp_millis();

    let started = spawn_activity_backfill(job(&fx, UNKNOWN_PROVIDER, Some("conv-1"))).await;
    assert!(started, "a fresh pair starts a backfill");

    let row = stored_job(&fx, UNKNOWN_PROVIDER)
        .await
        .expect("the spawn recorded a row before running");
    assert_eq!(row.provider, UNKNOWN_PROVIDER);
    assert_eq!(row.after_ts, Some(AFTER_TS), "the window's after is kept");
    assert_eq!(
        row.before_ts,
        Some(BEFORE_TS),
        "the window's before is kept"
    );
    assert_eq!(
        row.fetch_limit,
        Some(i64::try_from(FETCH_LIMIT).unwrap()),
        "the fetch limit is kept"
    );
    assert_eq!(
        row.conversation_id.as_deref(),
        Some("conv-1"),
        "the conversation the notice goes back to is kept"
    );
    assert_eq!(row.attempts, 1, "the spawn is the first attempt");
    assert!(
        row.created_at_ms >= before_spawn,
        "created_at is the spawn's moment"
    );
    assert!(
        row.leased_until_ms >= before_spawn + lease_ms()
            && row.leased_until_ms <= Utc::now().timestamp_millis() + lease_ms(),
        "the row is leased to the spawning instance for the full lease: {}",
        row.leased_until_ms
    );

    // Give the run time to fail; the failed run leaves the row untouched.
    sleep(Duration::from_millis(300)).await;
    let after_run = stored_job(&fx, UNKNOWN_PROVIDER).await;
    assert_eq!(
        after_run.as_ref(),
        Some(&row),
        "a transient failure leaves the row for the resume sweep"
    );
}

/// A run that reaches an outcome the athlete is told about — here the
/// reconnect nudge a tokenless provider produces — finishes the row.
#[tokio::test]
async fn a_terminal_run_finishes_the_row() {
    let fx = fixture().await;

    let started = spawn_activity_backfill(job(&fx, TOKENLESS_PROVIDER, None)).await;
    assert!(started, "a fresh pair starts a backfill");

    wait_until_row_is_gone(&fx, TOKENLESS_PROVIDER).await;

    // The pair is free again: a fresh ask records a fresh row rather than
    // being refused as owed.
    let recorded = fx
        .runtime
        .repos()
        .activity_backfill_jobs
        .record_backfill_job(&seeded_row(
            &fx,
            TOKENLESS_PROVIDER,
            Duration::ZERO,
            BACKFILL_JOB_LEASE,
            1,
        ))
        .await
        .unwrap();
    assert!(recorded, "a finished job no longer blocks the pair");
}

/// A row another instance already owes for the pair refuses the spawn: the
/// athlete has asked before and that job is on file. The row is untouched.
#[tokio::test]
async fn a_second_spawn_for_an_owed_pair_is_refused() {
    let fx = fixture().await;
    let owed = seeded_row(
        &fx,
        UNKNOWN_PROVIDER,
        Duration::from_secs(30),
        BACKFILL_JOB_LEASE,
        1,
    );
    assert!(fx
        .runtime
        .repos()
        .activity_backfill_jobs
        .record_backfill_job(&owed)
        .await
        .unwrap());

    let started = spawn_activity_backfill(job(&fx, UNKNOWN_PROVIDER, Some("conv-2"))).await;
    assert!(!started, "a pair with a row on file does not start again");

    let row = stored_job(&fx, UNKNOWN_PROVIDER)
        .await
        .expect("the owed row");
    assert_eq!(row.id, owed.id, "the owed row is the one on file");
    assert_eq!(row.attempts, 1, "the refused spawn counted no attempt");
    assert_eq!(
        row.conversation_id, None,
        "the refused spawn did not overwrite the owed row's conversation"
    );
    assert_eq!(
        row.leased_until_ms, owed.leased_until_ms,
        "the refused spawn did not touch the lease"
    );
}

/// The sweep claims a row whose lease lapsed — counting the attempt and
/// leasing it to this instance — and runs it. A transient failure again
/// leaves it, now carrying the claim.
#[tokio::test]
async fn the_sweep_claims_a_lapsed_row_and_runs_it() {
    let fx = fixture().await;
    let lapsed = seeded_row(
        &fx,
        UNKNOWN_PROVIDER,
        Duration::from_mins(10),
        Duration::ZERO,
        1,
    );
    assert!(fx
        .runtime
        .repos()
        .activity_backfill_jobs
        .record_backfill_job(&lapsed)
        .await
        .unwrap());

    let before_sweep = Utc::now().timestamp_millis();
    let resumed = resume_backfill_jobs(Arc::clone(&fx.runtime)).await.unwrap();
    assert_eq!(resumed, 1, "the lapsed row is claimed and started");

    sleep(Duration::from_millis(300)).await;
    let row = stored_job(&fx, UNKNOWN_PROVIDER)
        .await
        .expect("a transiently failed resume leaves the row");
    assert_eq!(row.id, lapsed.id);
    assert_eq!(
        row.attempts, 2,
        "the claim counted the resume as an attempt"
    );
    assert!(
        row.leased_until_ms >= before_sweep + lease_ms(),
        "the claim leased the row to this instance: {}",
        row.leased_until_ms
    );
    assert_eq!(
        row.after_ts,
        Some(AFTER_TS),
        "the window survives the claim"
    );

    // Leased now: a second sweep in the same minute takes nothing.
    let again = resume_backfill_jobs(Arc::clone(&fx.runtime)).await.unwrap();
    assert_eq!(again, 0, "a row under a live lease is not claimed twice");
}

/// A resumed run that reaches an outcome the athlete is told about finishes
/// the row, the same as the original spawn would have.
#[tokio::test]
async fn a_resumed_run_finishes_its_row() {
    let fx = fixture().await;
    let lapsed = seeded_row(
        &fx,
        TOKENLESS_PROVIDER,
        Duration::from_mins(10),
        Duration::ZERO,
        1,
    );
    assert!(fx
        .runtime
        .repos()
        .activity_backfill_jobs
        .record_backfill_job(&lapsed)
        .await
        .unwrap());

    let resumed = resume_backfill_jobs(Arc::clone(&fx.runtime)).await.unwrap();
    assert_eq!(resumed, 1, "the lapsed row is claimed and started");

    wait_until_row_is_gone(&fx, TOKENLESS_PROVIDER).await;
}

/// A row whose own spawn is presumed alive — recorded moments ago, under a
/// live lease — is not the sweep's to take.
#[tokio::test]
async fn the_sweep_leaves_a_fresh_row_to_its_own_spawn() {
    let fx = fixture().await;
    let fresh = seeded_row(&fx, UNKNOWN_PROVIDER, Duration::ZERO, BACKFILL_JOB_LEASE, 1);
    assert!(fx
        .runtime
        .repos()
        .activity_backfill_jobs
        .record_backfill_job(&fresh)
        .await
        .unwrap());

    let resumed = resume_backfill_jobs(Arc::clone(&fx.runtime)).await.unwrap();
    assert_eq!(resumed, 0, "a fresh, leased row is its own spawn's");

    let row = stored_job(&fx, UNKNOWN_PROVIDER)
        .await
        .expect("still on file");
    assert_eq!(row.attempts, 1, "no attempt was counted");
    assert_eq!(
        row.leased_until_ms, fresh.leased_until_ms,
        "the lease is untouched"
    );
}

/// A row at the attempt cap is never claimed again. Once its last runner is
/// dead too, standing there would refuse every new ask for the pair as
/// already owed, so the sweep reaps it and the athlete's next ask records
/// afresh. While its last runner is alive, it is left alone.
#[tokio::test]
async fn a_row_at_the_attempt_cap_is_not_claimed_and_is_reaped_once_its_runner_dies() {
    let fx = fixture().await;
    let ledger = Arc::clone(&fx.runtime.repos().activity_backfill_jobs);

    // Its third runner is still alive: neither claimed nor reaped.
    let running = seeded_row(
        &fx,
        UNKNOWN_PROVIDER,
        Duration::from_mins(20),
        Duration::from_mins(5),
        MAX_BACKFILL_ATTEMPTS,
    );
    assert!(ledger.record_backfill_job(&running).await.unwrap());
    let resumed = resume_backfill_jobs(Arc::clone(&fx.runtime)).await.unwrap();
    assert_eq!(resumed, 0, "a row at the cap is not claimed");
    let row = stored_job(&fx, UNKNOWN_PROVIDER)
        .await
        .expect("a capped row under a live lease stays");
    assert_eq!(row.id, running.id);
    assert_eq!(
        row.attempts, MAX_BACKFILL_ATTEMPTS,
        "no attempt was counted"
    );
    ledger.finish_backfill_job(&running.id).await.unwrap();

    // Its last runner died: the lease lapsed at the cap, so it is reaped.
    let exhausted = seeded_row(
        &fx,
        UNKNOWN_PROVIDER,
        Duration::from_mins(20),
        Duration::ZERO,
        MAX_BACKFILL_ATTEMPTS,
    );
    assert!(ledger.record_backfill_job(&exhausted).await.unwrap());
    let resumed = resume_backfill_jobs(Arc::clone(&fx.runtime)).await.unwrap();
    assert_eq!(resumed, 0, "a reaped row is not run");
    assert_eq!(
        stored_job(&fx, UNKNOWN_PROVIDER).await,
        None,
        "the exhausted row is gone"
    );

    // And the pair is free for the athlete's next ask.
    let fresh = seeded_row(&fx, UNKNOWN_PROVIDER, Duration::ZERO, BACKFILL_JOB_LEASE, 1);
    assert!(
        ledger.record_backfill_job(&fresh).await.unwrap(),
        "the next ask records a fresh job instead of being refused"
    );
}
