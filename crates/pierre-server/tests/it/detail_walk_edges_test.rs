// ABOUTME: The detail walk at its edges: a ledger that refuses its row, a budget with no room, a credential gone dead
// ABOUTME: A second cut short, a sport widened, a purge mid-read, a fresh walk beside one that ended, a ride stored late

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Detail walks at their edges (carnet#1483).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// The walk's main path is `detail_walk_test`; this suite drives what it
// meets at the edges, over the same loopback Strava stand-in.
#![cfg(all(
    feature = "tools-data",
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::detail_walk_test::{
    mock_strava, ride_listed_at, rows_under, since, store_rides, walk_ended, walk_row, DAY, SESSION,
};
use crate::history_walk_api_test::{open_window, server, strava_athlete, with_window};
use crate::history_walk_handoff_test::holding_writes;
use crate::messaging_fixtures::{strings, CapturingChannel, FakeResolver};

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use pierre_contremaitre::messaging_strings::KEY_DETAIL_WALK_COMPLETE;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::coverage::{IncompleteReason, Pending};
use pierre_core::models::{Activity, ActivityBuilder, ConnectionStatus, SportType, TenantId};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::BackfillJobState;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::backfill_notifier::ServerBackfillNotifier;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use pierre_tool_runtime::activity_backfill::detail_walk::{
    ask_detail_walk, detail_walk_pending, DetailWalkAsk, DetailWalkAsked,
};
use pierre_tool_runtime::runtime::{BackfillNotifier, ToolRuntime};
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// The trigger, and on `PostgreSQL` the function behind it, that refuses
/// every detail walk row the ledger is asked to record.
const REFUSE_WALK_ROWS: &str = "refuse_detail_walk_rows";

/// Make the ledger refuse to record any detail walk row from now on.
async fn refuse_walk_rows(resources: &Arc<ServerContext>) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(&format!(
                "CREATE TRIGGER {REFUSE_WALK_ROWS} BEFORE INSERT ON activity_backfill_jobs \
                 WHEN NEW.kind = 'detail_walk' \
                 BEGIN SELECT RAISE(ABORT, 'detail walk row refused by the test'); END"
            ))
            .execute(db.pool())
            .await
            .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(&format!(
                "CREATE FUNCTION {REFUSE_WALK_ROWS}() RETURNS trigger LANGUAGE plpgsql \
                 AS $$ BEGIN RAISE EXCEPTION 'detail walk row refused by the test'; END $$"
            ))
            .execute(db.pool())
            .await
            .unwrap();
            sqlx::query(&format!(
                "CREATE TRIGGER {REFUSE_WALK_ROWS} BEFORE INSERT ON activity_backfill_jobs \
                 FOR EACH ROW WHEN (NEW.kind = 'detail_walk') \
                 EXECUTE FUNCTION {REFUSE_WALK_ROWS}()"
            ))
            .execute(db.pool())
            .await
            .unwrap();
        }
    }
}

/// Let the ledger record detail walk rows again.
async fn accept_walk_rows(resources: &Arc<ServerContext>) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(&format!("DROP TRIGGER {REFUSE_WALK_ROWS}"))
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(&format!(
                "DROP TRIGGER {REFUSE_WALK_ROWS} ON activity_backfill_jobs"
            ))
            .execute(db.pool())
            .await
            .unwrap();
        }
    }
}

/// A walk the ledger refuses to record is not started: the ask says it was
/// not recorded, and no run reads a ride for a walk with no row. Once the
/// ledger records again, the next ask starts the walk: the refused one held
/// nothing back.
#[tokio::test]
async fn a_walk_the_ledger_cannot_record_is_not_started() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    store_rides(&resources, user_id, tenant, SESSION, &[now - 5 * DAY]).await;
    open_window(&resources).await;
    refuse_walk_rows(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 10 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::NotRecorded,
        "bug: a walk the ledger refused to record reads as started"
    );
    sleep(Duration::from_millis(200)).await;
    assert!(
        mock.detail_reads().is_empty(),
        "bug: a run was spawned for a walk with no row: {:?}",
        mock.detail_reads()
    );
    assert!(
        walk_row(&resources, user_id, tenant).await.is_none(),
        "the refused walk left no row"
    );

    accept_walk_rows(&resources).await;
    assert_eq!(
        ask_detail_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            since(now - 10 * DAY),
        )
        .await,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: the refused ask still holds the pair, so the next one never starts"
    );
}

/// Mark every row the athlete's grant holds as read for its segments: a
/// detail read stored on each, its efforts list empty.
async fn grant_rows_read(resources: &Arc<ServerContext>, user_id: Uuid) {
    let sql = "UPDATE cached_activities SET detail_json = '{\"segment_efforts\":[]}' \
               WHERE user_id = $1 AND provider = 'strava'";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(sql)
                .bind(user_id.to_string())
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(sql)
                .bind(user_id.to_string())
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

/// A ride the session listed on its day, whose grant copy at its real start
/// holds its efforts, is not read again however many read rows sit between
/// it and the step's newest ride: the look for its other copies reads every
/// row of its span, never only the first thousand.
#[tokio::test]
async fn a_read_copy_past_a_thousand_rows_still_stops_its_ride_being_read() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let real_start = now - 10 * DAY;
    let day_start = real_start - real_start.rem_euclid(DAY);
    let newest = now - 2 * DAY;
    // The grant's copy of the ride, then 1,100 grant rides a minute apart
    // just above it: all read.
    let grant: Vec<i64> = (0..=1_100).map(|minute| real_start + minute * 60).collect();
    store_rides(&resources, user_id, tenant, oauth_providers::STRAVA, &grant).await;
    grant_rows_read(&resources, user_id).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    runtime
        .live_activities()
        .ingest(
            user_id,
            tenant,
            SESSION,
            &[
                ride_listed_at(SESSION, real_start, day_start),
                ride_listed_at(SESSION, newest, newest),
            ],
            Utc::now(),
        )
        .await
        .unwrap();
    open_window(&resources).await;

    assert_eq!(
        ask_detail_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            since(now - 30 * DAY),
        )
        .await,
        DetailWalkAsked::Started { units_total: 1 },
        "only the newest ride is unread: the other's grant copy holds its efforts"
    );
    let ended = walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        mock.detail_reads(),
        vec![newest],
        "bug: a ride whose read copy sat past the look's first thousand rows was read again"
    );
    assert_eq!(
        ended.progress.units_done, 1,
        "bug: the walk counted a ride read already"
    );
}

/// A step a spent day stops part-way through a second records only the
/// rides of it the next step will not read again: a ride it stored is read,
/// but one it walked past (`404`) is read again with the rest of that second,
/// and is counted once, when the second is finished.
#[tokio::test]
async fn a_second_cut_short_by_a_spent_day_counts_each_ride_once() {
    let (api_base, mock) = mock_strava().await;
    // 20 a day: a background share of 5, half of it, 2, for the walk.
    let resources = server(&api_base, 20).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let day_start = (now - 5 * DAY) - (now - 5 * DAY).rem_euclid(DAY);
    // Read newest id first within the second: the unreadable one, then one
    // that stores, then the one the spent day refuses.
    let (unreadable, stored, refused) = (
        day_start + 3 * 3_600,
        day_start + 2 * 3_600,
        day_start + 3_600,
    );
    mock.missing.lock().unwrap().insert(unreadable);
    let listed: Vec<_> = [unreadable, stored, refused]
        .iter()
        .map(|id| ride_listed_at(SESSION, *id, day_start))
        .collect();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    runtime
        .live_activities()
        .ingest(user_id, tenant, SESSION, &listed, Utc::now())
        .await
        .unwrap();
    open_window(&resources).await;

    assert_eq!(
        ask_detail_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            since(now - 10 * DAY),
        )
        .await,
        DetailWalkAsked::Started { units_total: 3 },
        "the three rides of the one second are owed"
    );
    let ended = walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        mock.detail_reads(),
        vec![unreadable, stored, unreadable, refused],
        "the cut-short second is read again, the ride it stored excepted"
    );
    assert_eq!(
        (ended.progress.units_done, ended.progress.rows_landed),
        (3, 2),
        "bug: the ride walked past in a second the spent day cut short is counted twice"
    );
}

/// A walk's ETA is paced by the athlete's real walk day, as the budget sizes
/// it for the app that signs the grant's calls: at 40 a day the walk gets 5,
/// so one ride left is a fifth of a day, never the 1,728 s of a full 50.
#[tokio::test]
async fn the_eta_is_paced_by_the_athletes_real_walk_day() {
    let (api_base, mock) = mock_strava().await;
    // 40 a day: a background share of 10, half of it, 5, for the walk.
    let resources = server(&api_base, 40).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    store_rides(&resources, user_id, tenant, SESSION, &[now - 5 * DAY]).await;
    open_window(&resources).await;

    let gate = mock.hold_next_read();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        ask_detail_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            since(now - 10 * DAY),
        )
        .await,
        DetailWalkAsked::Started { units_total: 1 },
        "the one ride is owed"
    );
    mock.read_held().await;
    let pending = detail_walk_pending(&runtime, user_id, tenant, oauth_providers::STRAVA)
        .await
        .unwrap();
    gate.notify_one();
    assert_eq!(
        pending,
        Some(Pending::DetailWalkRunning {
            units_done: 0,
            units_total: Some(1),
            eta_secs: Some(17_280),
        }),
        "bug: one ride left at the athlete's 5 a day is not 17,280 s"
    );
}

/// An app registered at under 8 calls a day leaves a walk no call at all:
/// no walk is recorded, which would only pause until each day's end,
/// forever, and the window reads as one no backend can supply efforts for.
#[tokio::test]
async fn an_app_with_no_room_for_a_walk_records_none() {
    let (api_base, mock) = mock_strava().await;
    // 7 a day: a background share of 1, half of it rounded down, 0.
    let resources = server(&api_base, 7).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    store_rides(&resources, user_id, tenant, SESSION, &[now - 5 * DAY]).await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 10 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::NoBudget,
        "bug: a walk with no call a day was asked"
    );
    assert_eq!(
        asked.segment_reason(),
        Some(IncompleteReason::ProviderUnavailable),
        "bug: a window no walk can read reads as being read, or as Strava not connected"
    );
    assert!(
        walk_row(&resources, user_id, tenant).await.is_none(),
        "bug: a walk with no call a day was recorded"
    );
    sleep(Duration::from_millis(200)).await;
    assert!(mock.detail_reads().is_empty(), "no ride was read");
}

/// A walk owed when its app's day shrank to no room for it ends incomplete
/// at its next step, never paused day after day.
#[tokio::test]
async fn an_owed_walk_whose_app_lost_its_room_ends_incomplete() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    // Twelve rides: the first step reads ten, and the walk stays owed.
    let starts: Vec<i64> = (1..=12).map(|day| now - day * DAY).collect();
    store_rides(&resources, user_id, tenant, SESSION, &starts).await;
    open_window(&resources).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        ask_detail_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            since(now - 20 * DAY),
        )
        .await,
        DetailWalkAsked::Started { units_total: 12 },
        "the twelve rides are owed"
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while walk_row(&resources, user_id, tenant)
        .await
        .is_none_or(|row| row.progress.units_done < 10)
    {
        assert!(Instant::now() < deadline, "the first step did not record");
        sleep(Duration::from_millis(20)).await;
    }

    // The app is now registered at 7 a day.
    let shrunk = with_window(&resources, 7);
    let ended = walk_ended(&shrunk, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Incomplete,
        "bug: a walk with no call a day left was not ended"
    );
    assert_eq!(
        mock.detail_reads().len(),
        10,
        "no ride was read once the app had no room"
    );
}

/// A run started at `start` as the session lists it, its Strava id its start.
fn run_listed(start: i64) -> Activity {
    ActivityBuilder::new(
        start.to_string(),
        "Morning run",
        SportType::Run,
        Utc.timestamp_opt(start, 0).unwrap(),
        1_800,
        SESSION,
    )
    .source(oauth_providers::STRAVA)
    .build()
}

/// Store `starts` as runs the session listed.
async fn store_runs(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    starts: &[i64],
) {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let runs: Vec<Activity> = starts.iter().map(|start| run_listed(*start)).collect();
    runtime
        .live_activities()
        .ingest(user_id, tenant, SESSION, &runs, Utc::now())
        .await
        .unwrap();
}

/// A walk of one sport reads that sport's activities only: the runs of its
/// window are neither counted nor read.
#[tokio::test]
async fn a_walk_of_rides_reads_no_run() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let rides = [now - 3 * DAY, now - 6 * DAY];
    let runs = [now - 2 * DAY, now - 5 * DAY];
    store_rides(&resources, user_id, tenant, SESSION, &rides).await;
    store_runs(&resources, user_id, tenant, &runs).await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        DetailWalkAsk {
            sport: Some("Ride"),
            ..since(now - 10 * DAY)
        },
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 2 },
        "bug: a ride walk counts the runs of its window"
    );
    let ended = walk_ended(&resources, user_id, tenant).await;
    let mut reads = mock.detail_reads();
    reads.sort_unstable();
    assert_eq!(
        reads,
        vec![rides[1], rides[0]],
        "bug: a ride walk read a run"
    );
    assert_eq!(
        (ended.state, ended.progress.units_done),
        (BackfillJobState::Walked, 2),
        "the ride walk ends having read its two rides"
    );
}

/// A ride walk asked again for every sport reads the runs above where its
/// cursor got to: its cursor goes back to its window's top. The step running
/// when the ask lands gives its row up rather than writing its cursor over
/// that, so the runs above it are still read.
#[tokio::test]
async fn a_walk_widened_to_every_sport_reads_the_runs_above_its_cursor() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    // Twelve rides, and a run newer than all of them.
    let rides: Vec<i64> = (2..=13).map(|day| now - day * DAY).collect();
    let run = now - DAY;
    store_rides(&resources, user_id, tenant, SESSION, &rides).await;
    store_runs(&resources, user_id, tenant, &[run]).await;
    open_window(&resources).await;

    let gate = mock.hold_next_read();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let window = since(now - 20 * DAY);
    assert_eq!(
        ask_detail_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            DetailWalkAsk {
                sport: Some("Ride"),
                ..window
            },
        )
        .await,
        DetailWalkAsked::Started { units_total: 12 },
        "the twelve rides are owed"
    );
    mock.read_held().await;
    assert_eq!(
        ask_detail_walk(&runtime, user_id, tenant, oauth_providers::STRAVA, window).await,
        DetailWalkAsked::Widened,
        "bug: an ask for every sport while a ride walk is owed records a second walk"
    );
    let row = walk_row(&resources, user_id, tenant).await.unwrap();
    assert_eq!(row.walk_sport, None, "the widened walk reads every sport");
    assert_eq!(
        row.progress.units_total,
        Some(13),
        "bug: the widened total leaves out the run above the cursor"
    );
    gate.notify_one();

    let ended = walk_ended(&resources, user_id, tenant).await;
    assert!(
        mock.detail_reads().contains(&run),
        "bug: the run above the ride walk's cursor was never read: {:?}",
        mock.detail_reads()
    );
    assert_eq!(
        (
            ended.state,
            ended.progress.units_done,
            ended.progress.rows_landed
        ),
        (BackfillJobState::Walked, 13, 13),
        "the widened walk reads the twelve rides and the run"
    );
}

/// The rides the grant already holds are detailed in place, with no copy
/// to write: the step a widen to every sport took the lease from still
/// stores them. They are read, so no later step reads or counts them, and
/// the walk counts them stored all the same: its end, and the post that
/// tells it, count every activity whose efforts it read.
#[tokio::test]
async fn rides_stored_by_a_step_a_sport_widen_stopped_are_counted() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    // Twelve rides the grant listed, and a run newer than all of them.
    let rides: Vec<i64> = (2..=13).map(|day| now - day * DAY).collect();
    let run = now - DAY;
    store_rides(&resources, user_id, tenant, oauth_providers::STRAVA, &rides).await;
    store_runs(&resources, user_id, tenant, &[run]).await;
    open_window(&resources).await;

    let gate = mock.hold_next_read();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let window = since(now - 20 * DAY);
    assert_eq!(
        ask_detail_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            DetailWalkAsk {
                sport: Some("Ride"),
                ..window
            },
        )
        .await,
        DetailWalkAsked::Started { units_total: 12 },
        "the twelve rides are owed"
    );
    mock.read_held().await;
    assert_eq!(
        ask_detail_walk(&runtime, user_id, tenant, oauth_providers::STRAVA, window).await,
        DetailWalkAsked::Widened,
        "an ask for every sport while a ride walk is owed widens it"
    );
    gate.notify_one();
    // The stopped step reads its ten rides and settles before the sweep
    // runs the walk on (which moves the ask time the credit is keyed by).
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let row = walk_row(&resources, user_id, tenant).await.unwrap();
        if row.progress.rows_landed >= 10 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "bug: the ten rides the stopped step stored are not counted on its walk: {row:?}"
        );
        sleep(Duration::from_millis(20)).await;
    }

    let ended = walk_ended(&resources, user_id, tenant).await;
    let mut reads = mock.detail_reads();
    reads.sort_unstable();
    reads.dedup();
    assert_eq!(
        reads.len(),
        13,
        "the twelve rides and the run are each read: {reads:?}"
    );
    assert_eq!(
        (
            ended.state,
            ended.progress.units_done,
            ended.progress.rows_landed
        ),
        (BackfillJobState::Walked, 13, 13),
        "bug: the rides the stopped step stored on the grant's rows are not counted stored"
    );
}

/// Ask until the run an earlier ask spawned has let the pair go: the ask
/// that finds it still running records nothing.
async fn ask_once_the_run_is_done(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant: TenantId,
    ask: DetailWalkAsk<'_>,
) -> DetailWalkAsked {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let asked = ask_detail_walk(runtime, user_id, tenant, oauth_providers::STRAVA, ask).await;
        if asked != DetailWalkAsked::NotRecorded {
            return asked;
        }
        assert!(Instant::now() < deadline, "the earlier run never ended");
        sleep(Duration::from_millis(20)).await;
    }
}

/// The grant is revoked while the walk reads a ride only the session
/// listed: the grant's copy the walk would write before storing the read
/// lands nothing under the purged grant, and the session's row keeps no
/// effort it never read.
#[tokio::test]
async fn a_purge_while_a_ride_is_read_leaves_no_grant_copy() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let ride = now - 5 * DAY;
    store_rides(&resources, user_id, tenant, SESSION, &[ride]).await;
    open_window(&resources).await;

    let gate = mock.hold_next_read();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let window = since(now - 10 * DAY);
    assert_eq!(
        ask_detail_walk(&runtime, user_id, tenant, oauth_providers::STRAVA, window).await,
        DetailWalkAsked::Started { units_total: 1 },
        "the session's ride is owed"
    );
    mock.read_held().await;
    OAuthService::new(resources.data(), Arc::clone(&resources.common.config))
        .disconnect_provider(
            user_id,
            oauth_providers::STRAVA,
            Some(tenant.as_uuid()),
            DisconnectReason::ProviderRevoked,
        )
        .await
        .unwrap();
    gate.notify_one();
    // The run that read the ride has let the pair go: it settled.
    ask_once_the_run_is_done(&runtime, user_id, tenant, window).await;

    assert!(
        rows_under(&resources, user_id, tenant, oauth_providers::STRAVA)
            .await
            .is_empty(),
        "bug: the walk wrote the grant's copy of a ride after the grant's purge"
    );
    let session = rows_under(&resources, user_id, tenant, SESSION).await;
    assert_eq!(session.len(), 1, "the session's ride is kept");
    assert!(
        !session[0].segment_read,
        "bug: the grant's read landed on the session's row"
    );
}

/// The grant is revoked after the walk's step found its row still owed and
/// before the grant's copy of a ride only the session listed landed: the
/// copy lands after the purge, and the step, reading the purge once it
/// wrote, takes the copy back. Nothing of the purged grant is left.
#[tokio::test]
async fn a_purge_landing_while_the_grant_copy_is_written_takes_it_back() {
    let (api_base, _mock) = mock_strava().await;
    let (resources, store) = holding_writes(&server(&api_base, 400).await);
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    store_rides(&resources, user_id, tenant, SESSION, &[now - 5 * DAY]).await;
    open_window(&resources).await;

    let write = store.hold_next_write(oauth_providers::STRAVA);
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        ask_detail_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            since(now - 10 * DAY),
        )
        .await,
        DetailWalkAsked::Started { units_total: 1 },
        "the session's ride is owed"
    );
    timeout(Duration::from_secs(10), write.reached.notified())
        .await
        .expect("the grant's copy never reached the store");
    OAuthService::new(resources.data(), Arc::clone(&resources.common.config))
        .disconnect_provider(
            user_id,
            oauth_providers::STRAVA,
            Some(tenant.as_uuid()),
            DisconnectReason::ProviderRevoked,
        )
        .await
        .unwrap();
    write.gate.notify_one();
    timeout(Duration::from_secs(10), write.landed.notified())
        .await
        .expect("the held copy never landed");
    // The step's own check follows its write; give it a few seconds.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !rows_under(&resources, user_id, tenant, oauth_providers::STRAVA)
        .await
        .is_empty()
        && Instant::now() < deadline
    {
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        rows_under(&resources, user_id, tenant, oauth_providers::STRAVA)
            .await
            .is_empty(),
        "bug: the grant's copy that landed after the grant's purge was left in the store"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, SESSION).await.len(),
        1,
        "the session's ride stays with the session"
    );
}

/// Drop the athlete's stored Strava grant token, as a credential gone dead
/// leaves the grant: nothing left to read with.
async fn drop_grant_token(resources: &Arc<ServerContext>, user_id: Uuid) {
    let sql = "DELETE FROM user_oauth_tokens WHERE user_id = $1 AND provider = 'strava'";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(sql)
                .bind(user_id.to_string())
                .execute(db.pool())
                .await
                .unwrap();
        }
        // The token table's user id is a UUID on Postgres.
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(sql)
                .bind(user_id)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

/// A walk whose grant's credential is dead ends incomplete at its first
/// step, without a retry, and the connection is flagged for a reconnect.
#[tokio::test]
async fn a_walk_whose_credential_is_dead_ends_incomplete() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    store_rides(&resources, user_id, tenant, SESSION, &[now - 5 * DAY]).await;
    open_window(&resources).await;
    drop_grant_token(&resources, user_id).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        ask_detail_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            since(now - 10 * DAY),
        )
        .await,
        DetailWalkAsked::Started { units_total: 1 },
        "the ride is owed"
    );
    // The ask's own run, and no sweep: a failure would leave the walk owed
    // for a retry.
    let deadline = Instant::now() + Duration::from_secs(10);
    let ended = loop {
        let row = walk_row(&resources, user_id, tenant).await.unwrap();
        if row.state != BackfillJobState::Owed {
            break row;
        }
        assert!(
            Instant::now() < deadline,
            "bug: a walk with a dead credential stays owed: {row:?}"
        );
        sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        ended.state,
        BackfillJobState::Incomplete,
        "bug: a walk with a dead credential did not end incomplete"
    );
    assert!(mock.detail_reads().is_empty(), "no ride was read");
    let flagged = resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await
        .unwrap()
        .into_iter()
        .find(|connection| connection.provider == oauth_providers::STRAVA)
        .map(|connection| connection.status);
    assert_eq!(
        flagged,
        Some(ConnectionStatus::NeedsReauth),
        "bug: the dead grant was not flagged for a reconnect"
    );
}

/// Each walk that stores efforts tells its end in the athlete's personal
/// thread through the real notifier: a later walk, asked later, posts again
/// rather than being taken for the first one's post.
#[tokio::test]
async fn a_second_walk_posts_its_own_end() {
    let (api_base, _mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let catalogue = strings();
    if catalogue
        .get_exact(KEY_DETAIL_WALK_COMPLETE, "en")
        .is_none()
    {
        // The line is authored in dravr-contremaitre; until the pinned
        // catalogue carries it, a stand-in names the count.
        catalogue.update(
            KEY_DETAIL_WALK_COMPLETE,
            "en",
            "{0}".to_owned(),
            "stand-in".to_owned(),
        );
    }
    let repos = Arc::clone(&resources.common.repos);
    let notifier: Arc<dyn BackfillNotifier> = Arc::new(ServerBackfillNotifier::with_resolver(
        Arc::clone(&repos),
        catalogue,
        Arc::new(FakeResolver::new(Arc::new(CapturingChannel::default()))),
        Arc::from("detail-walk-edges-link-secret"),
        "https://detail-walk-edges.example".to_owned(),
    ));
    let mut context = (*resources).clone();
    context.mcp.backfill_notifier = Some(notifier);
    let resources = Arc::new(context);
    let (user_id, tenant) = strava_athlete(&resources).await;
    repos.users.update_locale(user_id, "en").await.unwrap();
    let thread = repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "test",
            "test-model",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    let now = Utc::now().timestamp();
    store_rides(&resources, user_id, tenant, SESSION, &[now - 5 * DAY]).await;
    open_window(&resources).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let window = since(now - 10 * DAY);
    assert_eq!(
        ask_detail_walk(&runtime, user_id, tenant, oauth_providers::STRAVA, window).await,
        DetailWalkAsked::Started { units_total: 1 },
        "the first ride is owed"
    );
    walk_ended(&resources, user_id, tenant).await;

    // A second ride, then a second walk for it.
    store_rides(&resources, user_id, tenant, SESSION, &[now - 3 * DAY]).await;
    assert_eq!(
        ask_once_the_run_is_done(&runtime, user_id, tenant, since(now - 10 * DAY)).await,
        DetailWalkAsked::Started { units_total: 1 },
        "the second ride is owed"
    );
    walk_ended(&resources, user_id, tenant).await;

    let posts = repos
        .chat
        .get_messages(&thread, &user_id.to_string(), tenant)
        .await
        .unwrap()
        .into_iter()
        .filter(|message| message.role == "assistant")
        .count();
    assert_eq!(
        posts, 2,
        "bug: the second walk's end was taken for the first one's post"
    );
}
