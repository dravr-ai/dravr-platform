// ABOUTME: A coach's morning brief in the routines repository: created once, changed by its owner, its runs and their items
// ABOUTME: Pins that only a new hour or state unschedules it, and that a listed item counts as acted on once, in its tenant

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{
    BriefSettings, LeftOut, Routine, RoutineKind, RoutineScope, RoutineState, RunOutcome, Schedule,
    TenantId, User, Weekdays, MORNING_BRIEF_HOUR,
};
use pierre_database::repositories::{ListedItem, NewRoutineRun, NextFiring, RoutineChange};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("brief-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Brief Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

fn every_morning() -> Schedule {
    Schedule::try_new(Weekdays::EVERY_DAY, MORNING_BRIEF_HOUR).unwrap()
}

/// The coach's brief, created in `tenant`.
async fn brief(repos: &RepositoryRegistry, tenant: TenantId, coach: Uuid) -> Routine {
    repos
        .routines
        .ensure_builtin_routine(
            tenant,
            coach,
            RoutineKind::MorningBrief,
            RoutineScope::Roster,
            every_morning(),
            Timestamp::now(),
        )
        .await
        .unwrap();
    repos
        .routines
        .builtin_routine(tenant, coach, RoutineKind::MorningBrief)
        .await
        .unwrap()
        .expect("bug: the brief just created reads back as none")
}

#[tokio::test]
async fn a_brief_is_created_once_and_keeps_what_its_owner_set() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let coach = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let created = brief(repos, tenant, coach).await;
    assert_eq!(
        created.state,
        RoutineState::Active,
        "bug: a coach's brief starts on"
    );
    assert_eq!(
        created.scope,
        RoutineScope::Roster,
        "bug: a brief speaks about the roster"
    );
    assert_eq!(
        created.schedule(),
        Some(every_morning()),
        "bug: a brief is created on the schedule it was given"
    );

    let paused = RoutineChange {
        state: RoutineState::Paused,
        hour: MORNING_BRIEF_HOUR,
        brief: BriefSettings::default(),
    };
    repos
        .routines
        .update_routine(tenant, created.id, &paused, Timestamp::now())
        .await
        .unwrap();
    let again = repos
        .routines
        .ensure_builtin_routine(
            tenant,
            coach,
            RoutineKind::MorningBrief,
            RoutineScope::Roster,
            every_morning(),
            Timestamp::now(),
        )
        .await
        .unwrap();
    assert!(!again, "bug: a second brief was created for the same coach");
    let kept = repos
        .routines
        .routine(tenant, created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        kept.state,
        RoutineState::Paused,
        "bug: the reconcile switched a paused brief back on"
    );
}

#[tokio::test]
async fn a_new_hour_or_state_unschedules_the_brief_and_new_settings_alone_do_not() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let coach = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let created = brief(repos, tenant, coach).await;
    // On the hour: the column keeps milliseconds, and real slots are hours.
    let slot = Timestamp::parse_from_rfc3339("2026-11-25T12:00:00Z").unwrap();
    let schedule = |expected| async move {
        repos
            .routines
            .schedule_routine(
                tenant,
                created.id,
                expected,
                NextFiring {
                    slot: Some(slot),
                    fire: Some(slot),
                    deferrals: 0,
                },
                Timestamp::now(),
            )
            .await
            .unwrap()
    };
    assert!(
        schedule(None).await,
        "bug: the new brief could not be scheduled"
    );

    let muted = RoutineChange {
        state: RoutineState::Active,
        hour: MORNING_BRIEF_HOUR,
        brief: BriefSettings {
            reasons_off: vec!["plan_ended".to_owned()],
            muted: vec![Uuid::new_v4()],
        },
    };
    repos
        .routines
        .update_routine(tenant, created.id, &muted, Timestamp::now())
        .await
        .unwrap();
    let read = repos
        .routines
        .routine(tenant, created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read.next_fire,
        Some(slot),
        "bug: a settings change dropped the pending firing"
    );
    assert_eq!(
        read.brief, muted.brief,
        "bug: the settings did not read back"
    );

    let moved = RoutineChange { hour: 9, ..muted };
    repos
        .routines
        .update_routine(tenant, created.id, &moved, Timestamp::now())
        .await
        .unwrap();
    let read = repos
        .routines
        .routine(tenant, created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read.next_fire, None,
        "bug: a new hour kept the old schedule's firing"
    );
    assert_eq!(
        read.schedule().map(Schedule::hour),
        Some(9),
        "bug: the new hour did not read back"
    );
}

#[tokio::test]
async fn a_listed_item_counts_as_acted_on_once_within_the_window_and_in_its_tenant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let coach = fresh_user(repos).await;
    let athlete = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let created = brief(repos, tenant, coach).await;
    let ran_at = Timestamp::parse_from_rfc3339("2026-11-25T12:00:00Z").unwrap();
    let items = [ListedItem {
        athlete_user_id: athlete,
        reason: "nothing_received".to_owned(),
    }];
    repos
        .routines
        .record_run(&NewRoutineRun {
            routine: &created,
            slot: ran_at,
            ran_at,
            outcome: RunOutcome::Posted,
            items: &items,
            left_out: LeftOut {
                not_sharing: 2,
                unreadable: 1,
            },
        })
        .await
        .unwrap();
    let last = repos
        .routines
        .last_run(tenant, created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (last.outcome, last.items, last.left_out),
        (
            RunOutcome::Posted,
            1,
            LeftOut {
                not_sharing: 2,
                unreadable: 1
            }
        ),
        "bug: the run did not read back as recorded"
    );

    let since = ran_at - Elapsed::hours(1);
    let later = ran_at + Elapsed::hours(2);
    let elsewhere = repos
        .routines
        .mark_listed_acted(
            TenantId::generate(),
            coach,
            athlete,
            since,
            "treated",
            later,
        )
        .await
        .unwrap();
    assert!(
        elsewhere.is_empty(),
        "bug: another tenant's action reached the item"
    );
    let too_late = repos
        .routines
        .mark_listed_acted(
            tenant,
            coach,
            athlete,
            ran_at + Elapsed::hours(1),
            "treated",
            later,
        )
        .await
        .unwrap();
    assert!(
        too_late.is_empty(),
        "bug: an item listed before the window counted"
    );

    let first = repos
        .routines
        .mark_listed_acted(tenant, coach, athlete, since, "treated", later)
        .await
        .unwrap();
    assert_eq!(
        first,
        ["nothing_received"],
        "bug: the listed item did not count"
    );
    let second = repos
        .routines
        .mark_listed_acted(tenant, coach, athlete, since, "opened", later)
        .await
        .unwrap();
    assert!(second.is_empty(), "bug: one item counted twice");
}
