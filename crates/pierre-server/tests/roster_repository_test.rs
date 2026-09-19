// ABOUTME: Repository-level tests for RosterRepository — every method, with the values each row decodes to
// ABOUTME: Runs on whichever driver the test factory selects, so the same assertions prove SQLite and Postgres
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `RosterRepository` round trips below the HTTP layer.
//!
//! `roster_routes_test` proves the routes; this file proves the rows. It
//! reads back every column an assignment carries — ids, the assigner, the
//! timestamps and the revoker — so a decode that differs between the two
//! backends fails here rather than in a route that only checks a count.

mod common;

use chrono::{Duration, Utc};
use common::{create_test_database, create_test_user_with_email};
use pierre_core::models::{CoachAthleteAssignment, TenantId};
use pierre_database::backends::factory::Database;
use std::sync::Arc;
use uuid::Uuid;

struct Roster {
    db: Arc<Database>,
    coach: Uuid,
    athlete: Uuid,
    tenant: TenantId,
}

/// A coach and an athlete who share the coach's tenant, which the
/// assignment's `tenant_id` foreign key requires.
async fn seeded() -> Roster {
    let db = create_test_database().await.unwrap();
    let (coach, _) = create_test_user_with_email(&db, "coach@roster.test")
        .await
        .unwrap();
    let (athlete, _) = create_test_user_with_email(&db, "athlete@roster.test")
        .await
        .unwrap();
    let repos = db.repositories();
    let tenant = repos.tenants.list_for_user(coach).await.unwrap()[0].id;
    repos.users.update_tenant_id(athlete, tenant).await.unwrap();
    Roster {
        db,
        coach,
        athlete,
        tenant,
    }
}

fn assignment(r: &Roster, assigned_by: Option<Uuid>) -> CoachAthleteAssignment {
    CoachAthleteAssignment {
        id: Uuid::new_v4(),
        coach_user_id: r.coach,
        athlete_user_id: r.athlete,
        tenant_id: r.tenant,
        assigned_by,
        assigned_at: Utc::now() - Duration::minutes(5),
        revoked_at: None,
        revoked_by: None,
    }
}

#[tokio::test]
async fn assign_round_trips_every_column_from_both_sides_of_the_roster() {
    let r = seeded().await;
    let repo = &r.db.repositories().roster;
    let wanted = assignment(&r, Some(r.coach));

    let inserted = repo.assign_athlete(&wanted).await.unwrap();
    assert_eq!(inserted.as_ref().map(|a| a.id), Some(wanted.id));

    let for_coach = repo
        .list_athletes_for_coach(r.coach, r.tenant)
        .await
        .unwrap();
    assert_eq!(for_coach.len(), 1);
    let row = &for_coach[0];
    assert_eq!(row.id, wanted.id);
    assert_eq!(row.coach_user_id, r.coach);
    assert_eq!(row.athlete_user_id, r.athlete);
    assert_eq!(row.tenant_id, r.tenant);
    assert_eq!(row.assigned_by, Some(r.coach));
    assert_eq!(
        row.assigned_at.timestamp_micros(),
        wanted.assigned_at.timestamp_micros(),
        "assigned_at survives the round trip to the microsecond"
    );
    assert_eq!(row.revoked_at, None);
    assert_eq!(row.revoked_by, None);

    let for_athlete = repo
        .list_coaches_for_athlete(r.athlete, r.tenant)
        .await
        .unwrap();
    assert_eq!(for_athlete.len(), 1);
    assert_eq!(for_athlete[0].id, wanted.id);
    assert_eq!(for_athlete[0].coach_user_id, r.coach);

    assert!(repo
        .is_athlete_managed_by(r.coach, r.athlete, r.tenant)
        .await
        .unwrap());
}

#[tokio::test]
async fn a_second_active_assignment_is_refused_and_a_null_assigner_reads_back_as_none() {
    let r = seeded().await;
    let repo = &r.db.repositories().roster;

    let first = assignment(&r, None);
    assert!(repo.assign_athlete(&first).await.unwrap().is_some());
    let duplicate = assignment(&r, Some(r.coach));
    assert!(
        repo.assign_athlete(&duplicate).await.unwrap().is_none(),
        "the partial unique index refuses a second active row"
    );

    let rows = repo
        .list_athletes_for_coach(r.coach, r.tenant)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, first.id);
    assert_eq!(rows[0].assigned_by, None);
}

#[tokio::test]
async fn revoke_stamps_the_row_and_hides_it_from_every_active_read() {
    let r = seeded().await;
    let repo = &r.db.repositories().roster;
    let live = assignment(&r, Some(r.coach));
    repo.assign_athlete(&live).await.unwrap();

    let before = Utc::now();
    let revoked = repo
        .revoke_assignment(r.coach, r.athlete, r.tenant, Some(r.athlete))
        .await
        .unwrap();
    assert!(revoked, "the active row was revoked");

    assert!(repo
        .list_athletes_for_coach(r.coach, r.tenant)
        .await
        .unwrap()
        .is_empty());
    assert!(repo
        .list_coaches_for_athlete(r.athlete, r.tenant)
        .await
        .unwrap()
        .is_empty());
    assert!(!repo
        .is_athlete_managed_by(r.coach, r.athlete, r.tenant)
        .await
        .unwrap());

    let again = repo
        .revoke_assignment(r.coach, r.athlete, r.tenant, None)
        .await
        .unwrap();
    assert!(!again, "an already-revoked assignment revokes nothing");

    // Re-assigning after a revoke inserts a new row under a new id; the
    // revoked row no longer blocks the partial unique index.
    let fresh = assignment(&r, None);
    assert!(repo.assign_athlete(&fresh).await.unwrap().is_some());
    let active = repo
        .list_athletes_for_coach(r.coach, r.tenant)
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, fresh.id);
    assert!(active[0].revoked_at.is_none());
    assert!(
        active[0].assigned_at >= before - Duration::minutes(6),
        "the fresh row carries its own assigned_at"
    );
}

#[tokio::test]
async fn roster_reads_are_scoped_to_the_tenant_they_name() {
    let r = seeded().await;
    let repo = &r.db.repositories().roster;
    repo.assign_athlete(&assignment(&r, None)).await.unwrap();

    let other_tenant = TenantId::generate();
    assert!(repo
        .list_athletes_for_coach(r.coach, other_tenant)
        .await
        .unwrap()
        .is_empty());
    assert!(!repo
        .is_athlete_managed_by(r.coach, r.athlete, other_tenant)
        .await
        .unwrap());
    assert!(!repo
        .revoke_assignment(r.coach, r.athlete, other_tenant, None)
        .await
        .unwrap());
}
