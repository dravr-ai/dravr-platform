// ABOUTME: Commitment sweep end to end — counts real cached activities, labels met/partial/missed, reports once
// ABOUTME: Pins the two guards that decide when NOT to speak: data freshness and the per-athlete cadence cap
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Drives `commitment_sweep::tick` against a real database and a real activity
//! cache. Every assertion is on content — a sweep that silently returned
//! nothing would fail each of these, which an `is_ok()` check would not.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_memory::commitments::{Commitment, CommitmentOutcome, CommitmentStatus};
use pierre_services::commitment_sweep::{tick, CommitmentReporter};
use uuid::Uuid;

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::{create_test_db, seed_user};

/// Captures what the sweep asked to deliver, and can refuse a route.
struct RecordingReporter {
    delivered: Mutex<Vec<Commitment>>,
    route: Option<String>,
}

impl RecordingReporter {
    fn open() -> Self {
        Self {
            delivered: Mutex::new(Vec::new()),
            route: Some("telegram".to_owned()),
        }
    }

    /// A reporter with no open delivery route — every channel window shut.
    fn closed() -> Self {
        Self {
            delivered: Mutex::new(Vec::new()),
            route: None,
        }
    }

    fn delivered(&self) -> Vec<Commitment> {
        self.delivered.lock().unwrap().clone()
    }
}

#[async_trait]
impl CommitmentReporter for RecordingReporter {
    async fn report(&self, commitment: &Commitment) -> Option<String> {
        if self.route.is_some() {
            self.delivered.lock().unwrap().push(commitment.clone());
        }
        self.route.clone()
    }
}

/// A commitment whose window ran from `days_ago` to one hour ago.
fn closed_window_commitment(
    tenant: TenantId,
    user: Uuid,
    sport: Option<&str>,
    target: u32,
    days_ago: i64,
) -> Commitment {
    let now = Utc::now();
    Commitment {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant.to_string(),
        user_id: user.to_string(),
        coach_id: Some("marathon-coach".to_owned()),
        conversation_id: None,
        statement: "three easy runs this week".to_owned(),
        sport: sport.map(str::to_owned),
        target_sessions: target,
        window_start: now - Duration::days(days_ago),
        window_end: now - Duration::hours(1),
        status: CommitmentStatus::Open,
        outcome: None,
        completed_sessions: None,
        swept_at: None,
        reported_at: None,
        created_at: now - Duration::days(days_ago),
        updated_at: now - Duration::days(days_ago),
    }
}

fn activity_at(id: &str, sport: SportType, start: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("session {id}"),
        sport,
        start,
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(10_000.0)
    .build()
}

/// Seed activities into the durable cache, which also stamps a fresh
/// `synced_at` — the signal the freshness guard reads.
async fn seed_activities(db: &Database, user: Uuid, tenant: TenantId, activities: &[Activity]) {
    db.repositories()
        .activity_cache
        .upsert_activities(user, &tenant, "strava", activities)
        .await
        .unwrap();
}

async fn fixture() -> (Database, Uuid, TenantId) {
    let db = create_test_db().await;
    let (user, tenant) = seed_user(&db).await;
    (db, user, tenant)
}

#[tokio::test]
async fn three_of_three_is_met_and_reaches_the_athlete() {
    let (db, user, tenant) = fixture().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = closed_window_commitment(tenant, user, Some("run"), 3, 7);
    repos.commitments.insert_commitment(&c).await.unwrap();

    let now = Utc::now();
    seed_activities(
        &db,
        user,
        tenant,
        &[
            activity_at("r1", SportType::Run, now - Duration::days(5)),
            activity_at("r2", SportType::Run, now - Duration::days(3)),
            activity_at("r3", SportType::Run, now - Duration::days(2)),
        ],
    )
    .await;

    let reporter = RecordingReporter::open();
    let outcome = tick(&repos, Some(&reporter), now, 50).await.unwrap();

    assert_eq!(outcome.scanned, 1);
    assert_eq!(outcome.labeled, 1, "the due commitment was counted");
    assert_eq!(outcome.reported, 1, "and the verdict went out");
    assert_eq!(outcome.errors, 0);

    let delivered = reporter.delivered();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].outcome, Some(CommitmentOutcome::Met));
    assert_eq!(delivered[0].completed_sessions, Some(3));
    assert_eq!(delivered[0].target_sessions, 3);
}

#[tokio::test]
async fn two_of_three_is_partial_and_carries_both_numbers() {
    let (db, user, tenant) = fixture().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = closed_window_commitment(tenant, user, Some("run"), 3, 7);
    repos.commitments.insert_commitment(&c).await.unwrap();

    let now = Utc::now();
    seed_activities(
        &db,
        user,
        tenant,
        &[
            activity_at("r1", SportType::Run, now - Duration::days(5)),
            activity_at("r2", SportType::Run, now - Duration::days(2)),
        ],
    )
    .await;

    let reporter = RecordingReporter::open();
    let outcome = tick(&repos, Some(&reporter), now, 50).await.unwrap();
    assert_eq!(outcome.labeled, 1);
    assert_eq!(outcome.reported, 1);

    let delivered = reporter.delivered();
    assert_eq!(
        delivered[0].outcome,
        Some(CommitmentOutcome::Partial),
        "two of three is neither a success nor a failure"
    );
    assert_eq!(delivered[0].completed_sessions, Some(2));
    assert_eq!(delivered[0].target_sessions, 3);
}

#[tokio::test]
async fn the_wrong_sport_does_not_count() {
    let (db, user, tenant) = fixture().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = closed_window_commitment(tenant, user, Some("run"), 3, 7);
    repos.commitments.insert_commitment(&c).await.unwrap();

    // A ride inside the window warms the cache — so the data IS fresh and a
    // shortfall is believable — but it is not a run.
    let now = Utc::now();
    seed_activities(
        &db,
        user,
        tenant,
        &[activity_at("b1", SportType::Ride, now - Duration::days(2))],
    )
    .await;

    let reporter = RecordingReporter::open();
    let outcome = tick(&repos, Some(&reporter), now, 50).await.unwrap();
    assert_eq!(outcome.labeled, 1);
    assert_eq!(
        reporter.delivered()[0].outcome,
        Some(CommitmentOutcome::Missed)
    );
    assert_eq!(reporter.delivered()[0].completed_sessions, Some(0));
}

#[tokio::test]
async fn a_sport_agnostic_commitment_counts_anything() {
    let (db, user, tenant) = fixture().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = closed_window_commitment(tenant, user, None, 2, 7);
    repos.commitments.insert_commitment(&c).await.unwrap();

    let now = Utc::now();
    seed_activities(
        &db,
        user,
        tenant,
        &[
            activity_at("r1", SportType::Run, now - Duration::days(4)),
            activity_at("b1", SportType::Ride, now - Duration::days(2)),
        ],
    )
    .await;

    let reporter = RecordingReporter::open();
    tick(&repos, Some(&reporter), now, 50).await.unwrap();
    assert_eq!(
        reporter.delivered()[0].outcome,
        Some(CommitmentOutcome::Met)
    );
    assert_eq!(reporter.delivered()[0].completed_sessions, Some(2));
}

#[tokio::test]
async fn a_cold_cache_defers_instead_of_accusing() {
    let (db, user, tenant) = fixture().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = closed_window_commitment(tenant, user, Some("run"), 3, 7);
    repos.commitments.insert_commitment(&c).await.unwrap();

    // No activities seeded at all: the athlete may well have run three times
    // and simply not opened the app since. Nothing has proven a miss.
    let reporter = RecordingReporter::open();
    let outcome = tick(&repos, Some(&reporter), Utc::now(), 50).await.unwrap();

    assert_eq!(outcome.scanned, 1);
    assert_eq!(outcome.deferred, 1, "held for the data to catch up");
    assert_eq!(outcome.labeled, 0, "no verdict was invented");
    assert_eq!(outcome.reported, 0);
    assert!(
        reporter.delivered().is_empty(),
        "the athlete was told nothing"
    );

    // Still open, so a later sweep will try again.
    let due = repos
        .commitments
        .due_commitments(Utc::now().timestamp(), 50)
        .await
        .unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].status, CommitmentStatus::Open);
}

#[tokio::test]
async fn a_deferral_that_never_resolves_expires_rather_than_blocking() {
    let (db, user, tenant) = fixture().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let mut c = closed_window_commitment(tenant, user, Some("run"), 3, 30);
    // Eight days past its window with the data still cold — beyond the sweep's
    // staleness horizon, so it must stop head-of-line blocking newer rows.
    c.window_end = Utc::now() - Duration::days(8);
    repos.commitments.insert_commitment(&c).await.unwrap();

    let reporter = RecordingReporter::open();
    let outcome = tick(&repos, Some(&reporter), Utc::now(), 50).await.unwrap();

    assert_eq!(outcome.expired, 1);
    assert_eq!(outcome.labeled, 0);
    assert!(reporter.delivered().is_empty());
    assert!(repos
        .commitments
        .due_commitments(Utc::now().timestamp(), 50)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_closed_route_holds_the_verdict_for_the_next_tick() {
    let (db, user, tenant) = fixture().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = closed_window_commitment(tenant, user, Some("run"), 3, 7);
    repos.commitments.insert_commitment(&c).await.unwrap();
    let now = Utc::now();
    seed_activities(
        &db,
        user,
        tenant,
        &[activity_at("r1", SportType::Run, now - Duration::days(2))],
    )
    .await;

    // Every window shut — WhatsApp outside its 24h re-engagement window, say.
    let shut = RecordingReporter::closed();
    let first = tick(&repos, Some(&shut), now, 50).await.unwrap();
    assert_eq!(first.labeled, 1, "the verdict was still computed");
    assert_eq!(first.reported, 0);
    assert_eq!(first.held, 1, "and held rather than burned");

    // The athlete says something; the window reopens; the next tick delivers.
    let open = RecordingReporter::open();
    let second = tick(&repos, Some(&open), Utc::now(), 50).await.unwrap();
    assert_eq!(second.scanned, 0, "nothing new was due");
    assert_eq!(second.reported, 1);
    assert_eq!(
        open.delivered()[0].outcome,
        Some(CommitmentOutcome::Partial)
    );
}

#[tokio::test]
async fn the_cadence_cap_stops_two_verdicts_landing_at_once() {
    let (db, user, tenant) = fixture().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    // Two promises closing the same hour.
    let runs = closed_window_commitment(tenant, user, Some("run"), 2, 7);
    let swims = closed_window_commitment(tenant, user, Some("swim"), 2, 7);
    repos.commitments.insert_commitment(&runs).await.unwrap();
    repos.commitments.insert_commitment(&swims).await.unwrap();

    let now = Utc::now();
    seed_activities(
        &db,
        user,
        tenant,
        &[activity_at("r1", SportType::Run, now - Duration::days(2))],
    )
    .await;

    let reporter = RecordingReporter::open();
    let outcome = tick(&repos, Some(&reporter), now, 50).await.unwrap();

    assert_eq!(outcome.labeled, 2, "both were counted");
    assert_eq!(
        outcome.reported, 1,
        "but the athlete hears about one of them today"
    );
    assert_eq!(outcome.held, 1);
    assert_eq!(reporter.delivered().len(), 1);

    // The held one is still labeled and waiting, not lost.
    let pending = repos.commitments.unreported_commitments(50).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status, CommitmentStatus::Labeled);
}

#[tokio::test]
async fn a_sweep_with_no_reporter_still_counts_and_ages_out() {
    let (db, user, tenant) = fixture().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let c = closed_window_commitment(tenant, user, Some("run"), 2, 7);
    repos.commitments.insert_commitment(&c).await.unwrap();
    let now = Utc::now();
    seed_activities(
        &db,
        user,
        tenant,
        &[activity_at("r1", SportType::Run, now - Duration::days(2))],
    )
    .await;

    // A build with no messaging rail compiled in.
    let outcome = tick(&repos, None, now, 50).await.unwrap();
    assert_eq!(outcome.labeled, 1);
    assert_eq!(outcome.reported, 0);
    assert_eq!(outcome.held, 1);

    // Much later, with still nothing able to deliver, the verdict ages out
    // instead of queueing forever.
    let stale = tick(&repos, None, now + Duration::days(8), 50)
        .await
        .unwrap();
    assert_eq!(stale.expired, 1);
    assert!(repos
        .commitments
        .unreported_commitments(50)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_future_window_is_left_alone() {
    let (db, user, tenant) = fixture().await;
    let repos: Arc<RepositoryRegistry> = Arc::new(db.repositories());

    let mut c = closed_window_commitment(tenant, user, Some("run"), 3, 7);
    c.window_end = Utc::now() + Duration::days(3);
    repos.commitments.insert_commitment(&c).await.unwrap();

    let reporter = RecordingReporter::open();
    let outcome = tick(&repos, Some(&reporter), Utc::now(), 50).await.unwrap();

    assert_eq!(outcome.scanned, 0, "the week is not over yet");
    assert_eq!(outcome.labeled, 0);
    assert!(reporter.delivered().is_empty());
}
