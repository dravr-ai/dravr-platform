// ABOUTME: PrescribedWorkoutRepository round trip, tenant isolation, ledger reads, status transitions, one-live-row-per-key
// ABOUTME: Validates the calendar ledger's semantics on the SQLite tier; PG mirrors via the same trait
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Storage-layer tests for the prescription audit trail.
//!
//! Split out of `workout_push_test.rs`, which was named for a push it never
//! performed — these are repository semantics over hand-built rows, and the
//! tool → provider → calendar path they were standing in for now lives in that
//! file for real.

use std::time::Duration;

use chrono::{NaiveDate, Utc};
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::models::{CalendarEventSource, PrescribedWorkout, SportType, TenantId};
use pierre_database::backends::factory::Database;
use pierre_database::DatabaseProvider;
use tokio::time::sleep;
use uuid::Uuid;

async fn make_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();
    #[cfg(feature = "postgresql")]
    let db = Database::new(
        "sqlite::memory:",
        encryption_key,
        &PostgresPoolConfig::default(),
    )
    .await
    .expect("create db");
    #[cfg(not(feature = "postgresql"))]
    let db = Database::new("sqlite::memory:", encryption_key)
        .await
        .expect("create db");
    db.migrate().await.expect("migrate");
    db
}

fn anchor_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()
}

/// A prescription as the tool writes one: the push landed, so the row carries
/// the provider's event id, its own key, and the `pushed` status.
fn make_prescribed(
    tenant_id: TenantId,
    user_id: Uuid,
    template_slug: &str,
    date: NaiveDate,
) -> PrescribedWorkout {
    let id = Uuid::new_v4();
    let now = Utc::now();
    PrescribedWorkout {
        id,
        tenant_id: tenant_id.as_uuid(),
        user_id,
        coach_id: Some("endurance-coach".to_owned()),
        template_slug: Some(template_slug.to_owned()),
        sport: SportType::Run,
        prescribed_for_date: date,
        provider: "intervals_icu".to_owned(),
        provider_event_id: Some("intervals-evt-1".to_owned()),
        external_id: Some(format!("dravr:rx:{id}")),
        source: CalendarEventSource::Prescription,
        plan_week_id: None,
        replaces_id: None,
        payload_hash: Some("hash-1".to_owned()),
        payload_json: r#"{"slug":"long_run_z2"}"#.to_owned(),
        status: "pushed".to_owned(),
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn upsert_and_list_round_trips() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let prescribed = make_prescribed(tenant_id, user_id, "long_run_z2", anchor_date());
    db.repositories()
        .prescribed_workouts
        .upsert_prescribed_workout(&prescribed)
        .await
        .expect("upsert");
    let rows = db
        .repositories()
        .prescribed_workouts
        .list_prescribed_workouts(tenant_id, user_id, 10)
        .await
        .expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].template_slug.as_deref(), Some("long_run_z2"));
    assert_eq!(rows[0].status, "pushed");
    assert_eq!(
        rows[0].provider_event_id.as_deref(),
        Some("intervals-evt-1")
    );
    assert_eq!(rows[0].coach_id.as_deref(), Some("endurance-coach"));
}

#[tokio::test]
async fn a_refused_push_round_trips_as_a_failed_row() {
    // The other terminal outcome: the provider refused, so there is no event id
    // and the status says so. Both must survive a round trip, because the audit
    // trail is what tells a coach whether the athlete actually got the workout.
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let mut prescribed = make_prescribed(tenant_id, user_id, "vo2_5x3", anchor_date());
    prescribed.provider_event_id = None;
    prescribed.status = "failed".to_owned();
    db.repositories()
        .prescribed_workouts
        .upsert_prescribed_workout(&prescribed)
        .await
        .expect("upsert");
    let rows = db
        .repositories()
        .prescribed_workouts
        .list_prescribed_workouts(tenant_id, user_id, 10)
        .await
        .expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "failed");
    assert!(rows[0].provider_event_id.is_none());
}

#[tokio::test]
async fn an_absent_event_id_reads_back_as_absent_not_as_an_empty_string() {
    // SQLite hands a NULL TEXT column back as an empty string rather than an
    // error, so the row mapper's old `try_get::<String>().ok()` turned every
    // absent value into `Some("")`. A caller then cannot tell a prescription
    // the provider never created from one whose event id is empty — and the
    // edit path this unblocks would address `/events/` with nothing after it.
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let mut prescribed = make_prescribed(tenant_id, user_id, "recovery_30min", anchor_date());
    prescribed.provider_event_id = None;
    prescribed.coach_id = None;
    db.repositories()
        .prescribed_workouts
        .upsert_prescribed_workout(&prescribed)
        .await
        .expect("upsert");
    let rows = db
        .repositories()
        .prescribed_workouts
        .list_prescribed_workouts(tenant_id, user_id, 10)
        .await
        .expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].provider_event_id, None,
        "a NULL event id must read back as None, never as Some(\"\")"
    );
    assert_eq!(
        rows[0].coach_id, None,
        "a NULL coach id must read back as None, never as Some(\"\")"
    );
}

#[tokio::test]
async fn upsert_replays_with_same_id_to_update_provider_event_id() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let mut prescribed = make_prescribed(tenant_id, user_id, "threshold_4x8", anchor_date());
    prescribed.provider_event_id = None;
    prescribed.status = "failed".to_owned();
    db.repositories()
        .prescribed_workouts
        .upsert_prescribed_workout(&prescribed)
        .await
        .expect("upsert v1");
    prescribed.provider_event_id = Some("intervals-evt-99".to_owned());
    prescribed.status = "pushed".to_owned();
    db.repositories()
        .prescribed_workouts
        .upsert_prescribed_workout(&prescribed)
        .await
        .expect("upsert v2");
    let rows = db
        .repositories()
        .prescribed_workouts
        .list_prescribed_workouts(tenant_id, user_id, 10)
        .await
        .expect("list");
    assert_eq!(
        rows.len(),
        1,
        "same-id upsert must not create a duplicate row"
    );
    assert_eq!(
        rows[0].provider_event_id.as_deref(),
        Some("intervals-evt-99")
    );
    assert_eq!(rows[0].status, "pushed");
}

#[tokio::test]
async fn prescriptions_are_tenant_scoped() {
    let db = make_test_db().await;
    let tenant_a = TenantId::generate();
    let tenant_b = TenantId::generate();
    let user_id = Uuid::new_v4();
    let p_a = make_prescribed(tenant_a, user_id, "long_run_z2", anchor_date());
    let p_b = make_prescribed(tenant_b, user_id, "vo2_5x3", anchor_date());
    let repos = db.repositories();
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&p_a)
        .await
        .expect("upsert A");
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&p_b)
        .await
        .expect("upsert B");

    let rows_a = repos
        .prescribed_workouts
        .list_prescribed_workouts(tenant_a, user_id, 10)
        .await
        .expect("list A");
    let rows_b = repos
        .prescribed_workouts
        .list_prescribed_workouts(tenant_b, user_id, 10)
        .await
        .expect("list B");
    assert_eq!(rows_a.len(), 1);
    assert_eq!(rows_b.len(), 1);
    assert_eq!(rows_a[0].template_slug.as_deref(), Some("long_run_z2"));
    assert_eq!(rows_b[0].template_slug.as_deref(), Some("vo2_5x3"));

    let other_user = Uuid::new_v4();
    let rows_c = repos
        .prescribed_workouts
        .list_prescribed_workouts(tenant_a, other_user, 10)
        .await
        .expect("list C");
    assert!(rows_c.is_empty(), "other-user list must be empty");
}

#[tokio::test]
async fn list_orders_newest_first_and_respects_limit() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let repos = db.repositories();
    for i in 0..5 {
        let prescribed = make_prescribed(
            tenant_id,
            user_id,
            "recovery_30min",
            anchor_date() + chrono::Duration::days(i),
        );
        repos
            .prescribed_workouts
            .upsert_prescribed_workout(&prescribed)
            .await
            .expect("upsert");
        // Tiny delay to ensure distinct created_at values.
        sleep(Duration::from_millis(5)).await;
    }
    let rows = repos
        .prescribed_workouts
        .list_prescribed_workouts(tenant_id, user_id, 3)
        .await
        .expect("list");
    assert_eq!(rows.len(), 3, "limit must cap the returned rows");
    assert!(
        rows.windows(2).all(|w| w[0].created_at >= w[1].created_at),
        "rows must be newest-first"
    );
}

#[tokio::test]
async fn get_by_id_is_scoped_to_the_tenant_and_the_athlete() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let prescribed = make_prescribed(tenant_id, user_id, "long_run_z2", anchor_date());
    let repos = db.repositories();
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&prescribed)
        .await
        .expect("upsert");

    let found = repos
        .prescribed_workouts
        .get_prescribed_workout(tenant_id, user_id, prescribed.id)
        .await
        .expect("get")
        .expect("the athlete's own row is found");
    assert_eq!(
        found.external_id.as_deref(),
        Some(format!("dravr:rx:{}", prescribed.id).as_str())
    );
    assert_eq!(found.source, CalendarEventSource::Prescription);
    assert_eq!(found.payload_hash.as_deref(), Some("hash-1"));
    assert!(found.replaces_id.is_none());
    assert!(found.plan_week_id.is_none());

    // Another athlete of the same tenant, and the same athlete in another
    // tenant, both see nothing — a foreign row is indistinguishable from a
    // missing one.
    assert!(repos
        .prescribed_workouts
        .get_prescribed_workout(tenant_id, Uuid::new_v4(), prescribed.id)
        .await
        .expect("get other user")
        .is_none());
    assert!(repos
        .prescribed_workouts
        .get_prescribed_workout(TenantId::generate(), user_id, prescribed.id)
        .await
        .expect("get other tenant")
        .is_none());
}

#[tokio::test]
async fn live_calendar_events_are_the_pushed_rows_of_one_provider_from_a_date() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let repos = db.repositories();
    let d0 = anchor_date();
    let d1 = d0 + chrono::Duration::days(1);
    let d2 = d0 + chrono::Duration::days(2);
    let d3 = d0 + chrono::Duration::days(3);

    // Live on intervals.icu at d0 and d2; refused at d1; withdrawn at d3; and
    // a live row on another provider at d1 that must never leak across.
    let live_d0 = make_prescribed(tenant_id, user_id, "long_run_z2", d0);
    let live_d2 = make_prescribed(tenant_id, user_id, "vo2_5x3", d2);
    let mut refused = make_prescribed(tenant_id, user_id, "tempo_progression", d1);
    refused.status = "failed".to_owned();
    refused.provider_event_id = None;
    let mut withdrawn = make_prescribed(tenant_id, user_id, "recovery_30min", d3);
    withdrawn.status = "withdrawn".to_owned();
    let mut elsewhere = make_prescribed(tenant_id, user_id, "sweet_spot_2x20", d1);
    elsewhere.provider = "trainingpeaks".to_owned();
    for row in [&live_d0, &live_d2, &refused, &withdrawn, &elsewhere] {
        repos
            .prescribed_workouts
            .upsert_prescribed_workout(row)
            .await
            .expect("upsert");
    }

    let all_live = repos
        .prescribed_workouts
        .list_live_calendar_events(tenant_id, user_id, "intervals_icu", None)
        .await
        .expect("list live");
    assert_eq!(
        all_live.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![live_d0.id, live_d2.id],
        "only pushed rows of the provider, in calendar order"
    );

    let from_d1 = repos
        .prescribed_workouts
        .list_live_calendar_events(tenant_id, user_id, "intervals_icu", Some(d1))
        .await
        .expect("list live from d1");
    assert_eq!(
        from_d1.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![live_d2.id],
        "the window starts at `from`, inclusive"
    );
}

#[tokio::test]
async fn status_transitions_stamp_updated_at_and_reject_unknown_rows() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let repos = db.repositories();
    let prescribed = make_prescribed(tenant_id, user_id, "long_run_z2", anchor_date());
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&prescribed)
        .await
        .expect("upsert");
    sleep(Duration::from_millis(5)).await;

    repos
        .prescribed_workouts
        .set_prescribed_workout_status(tenant_id, prescribed.id, PrescribedWorkout::STATUS_REPLACED)
        .await
        .expect("replace");
    let row = repos
        .prescribed_workouts
        .get_prescribed_workout(tenant_id, user_id, prescribed.id)
        .await
        .expect("get")
        .expect("row");
    assert_eq!(row.status, "replaced");
    assert!(!row.is_live());
    assert!(
        row.updated_at > row.created_at,
        "a status change must move updated_at forward"
    );

    // Another tenant cannot move this row, and a made-up id is an error, not
    // a silent no-op.
    repos
        .prescribed_workouts
        .set_prescribed_workout_status(TenantId::generate(), prescribed.id, "withdrawn")
        .await
        .expect_err("a foreign tenant must not reach the row");
    repos
        .prescribed_workouts
        .set_prescribed_workout_status(tenant_id, Uuid::new_v4(), "withdrawn")
        .await
        .expect_err("an unknown id must be an error");
}

#[tokio::test]
async fn one_live_row_per_key_per_provider() {
    // The unique index that makes "one live calendar entry per Dravr key"
    // structural: a second pushed row under the same key is refused until the
    // first is superseded.
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let repos = db.repositories();
    let first = make_prescribed(tenant_id, user_id, "long_run_z2", anchor_date());
    let mut second = make_prescribed(tenant_id, user_id, "long_run_z2", anchor_date());
    second.external_id = first.external_id.clone();
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&first)
        .await
        .expect("first row");
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&second)
        .await
        .expect_err("a second live row under the same key must be refused");

    repos
        .prescribed_workouts
        .set_prescribed_workout_status(tenant_id, first.id, PrescribedWorkout::STATUS_REPLACED)
        .await
        .expect("supersede the first");
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&second)
        .await
        .expect("once the first is replaced the key is free again");

    // A failed attempt under a live key is fine: the index only guards live rows.
    let mut failed = make_prescribed(tenant_id, user_id, "long_run_z2", anchor_date());
    failed.external_id = first.external_id.clone();
    failed.status = "failed".to_owned();
    failed.provider_event_id = None;
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&failed)
        .await
        .expect("a failed row never collides");

    let live = repos
        .prescribed_workouts
        .list_live_calendar_events(tenant_id, user_id, "intervals_icu", None)
        .await
        .expect("list live");
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id, second.id);
}
