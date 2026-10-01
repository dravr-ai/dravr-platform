// ABOUTME: The activity cache keeps what a detail read found — splits and laps — beside the row every list sync rewrites
// ABOUTME: Survives list syncs, fills only what the list copy lacks, stays in its tenant and user, and goes with its row

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `ActivityCacheRepository::store_activity_detail` suite.
//!
//! A list read never carries splits or laps, and every list write-through
//! replaces the cached activity whole. Each case writes through the
//! repository's own writers and asserts the splits and laps read back by
//! value, on whichever backend the test factory opens.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{Activity, ActivityBuilder, Lap, Split, SportType, TenantId};
use pierre_database::backends::factory::Database;
use pierre_database::repositories::ActivityDetail;
use uuid::Uuid;

fn started() -> DateTime<Utc> {
    Utc::now() - Duration::days(1)
}

/// A run as a list read caches it: no splits, no laps.
fn listed_run(id: &str, name: &str) -> Activity {
    ActivityBuilder::new(id, name, SportType::Run, started(), 1_800, "strava")
        .distance_meters(5_000.0)
        .build()
}

fn split(index: u32, elapsed_time_seconds: u64) -> Split {
    Split {
        index,
        distance_meters: 1_000.0,
        elapsed_time_seconds,
        moving_time_seconds: Some(elapsed_time_seconds - 4),
        elevation_difference_meters: Some(2.5),
        average_speed_mps: Some(3.5),
        average_heart_rate: Some(150),
        pace_zone: None,
    }
}

fn lap() -> Lap {
    Lap {
        id: Some("lap-1".to_owned()),
        index: 1,
        distance_meters: 5_000.0,
        elapsed_time_seconds: 1_800,
        moving_time_seconds: Some(1_780),
        elevation_gain_meters: Some(35.0),
        average_speed_mps: Some(2.75),
        max_speed_mps: Some(4.25),
        average_heart_rate: Some(152),
        max_heart_rate: Some(171),
        average_cadence: Some(172),
        average_power: None,
    }
}

fn detail() -> ActivityDetail {
    ActivityDetail {
        splits: Some(vec![split(1, 262), split(2, 251)]),
        laps: Some(vec![lap()]),
    }
}

struct Fixture {
    database: Arc<Database>,
    user_id: Uuid,
    tenant: TenantId,
}

async fn fixture() -> Fixture {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    Fixture {
        database,
        user_id,
        tenant: TenantId::generate(),
    }
}

impl Fixture {
    async fn list_sync(&self, rows: &[Activity]) {
        self.database
            .repositories()
            .activity_cache
            .upsert_activities(self.user_id, &self.tenant, "strava", rows)
            .await
            .unwrap();
    }

    async fn store(&self, user_id: Uuid, tenant: TenantId, id: &str) -> bool {
        self.database
            .repositories()
            .activity_cache
            .store_activity_detail(user_id, &tenant, "strava", id, &detail(), None)
            .await
            .unwrap()
    }

    async fn read(&self, tenant: TenantId, id: &str) -> Option<Activity> {
        self.database
            .repositories()
            .activity_cache
            .get_cached_activity(self.user_id, &tenant, "strava", id)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_stored_detail_survives_every_later_list_sync_on_every_read() {
    let fx = fixture().await;
    fx.list_sync(&[listed_run("r1", "Morning run")]).await;
    assert!(fx.store(fx.user_id, fx.tenant, "r1").await);

    // The next list sync rewrites the row whole, and renames the run.
    fx.list_sync(&[listed_run("r1", "Tempo run")]).await;

    let one = fx.read(fx.tenant, "r1").await.unwrap();
    assert_eq!(one.name(), "Tempo run", "the list sync's copy is served");
    assert_eq!(one.splits(), Some(&vec![split(1, 262), split(2, 251)]));
    assert_eq!(one.laps(), Some(&vec![lap()]));

    let cache = &fx.database.repositories().activity_cache;
    let window = (started() - Duration::hours(1), Utc::now());
    let listed = cache
        .get_cached_activities(fx.user_id, &fx.tenant, None, window.0, window.1, 10)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].splits().map(Vec::len), Some(2));
    let rows = cache
        .get_cached_activity_rows(fx.user_id, &fx.tenant, window.0, window.1, 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].activity.laps(), Some(&vec![lap()]));
}

#[tokio::test]
async fn a_list_copy_that_carries_its_own_splits_keeps_them() {
    let fx = fixture().await;
    let own = vec![split(1, 300)];
    let detailed =
        ActivityBuilder::new("r2", "Detailed", SportType::Run, started(), 1_800, "strava")
            .splits(own.clone())
            .build();
    fx.list_sync(&[detailed]).await;
    assert!(fx.store(fx.user_id, fx.tenant, "r2").await);

    let read = fx.read(fx.tenant, "r2").await.unwrap();
    assert_eq!(read.splits(), Some(&own), "the copy's own splits stand");
    assert_eq!(
        read.laps(),
        Some(&vec![lap()]),
        "the laps it lacks come from the stored detail"
    );
}

#[tokio::test]
async fn a_detail_is_stored_only_on_the_callers_own_row_in_its_tenant() {
    let fx = fixture().await;
    fx.list_sync(&[listed_run("r3", "Easy run")]).await;
    let other_tenant = TenantId::generate();
    let (other_user, _) = common::create_test_user_with_email(
        &fx.database,
        &format!("detail-other-{}@example.com", Uuid::new_v4()),
    )
    .await
    .unwrap();

    assert!(!fx.store(fx.user_id, other_tenant, "r3").await);
    assert!(!fx.store(other_user, fx.tenant, "r3").await);
    assert!(!fx.store(fx.user_id, fx.tenant, "unknown").await);

    let read = fx.read(fx.tenant, "r3").await.unwrap();
    assert_eq!(read.splits(), None, "no write reached the owner's row");
    assert_eq!(read.laps(), None);
    assert!(fx.read(other_tenant, "r3").await.is_none());
}

#[tokio::test]
async fn a_deleted_or_purged_activity_takes_its_detail_with_it() {
    let fx = fixture().await;
    fx.list_sync(&[listed_run("r4", "Run"), listed_run("r5", "Run")])
        .await;
    assert!(fx.store(fx.user_id, fx.tenant, "r4").await);
    assert!(fx.store(fx.user_id, fx.tenant, "r5").await);
    let cache = &fx.database.repositories().activity_cache;

    assert_eq!(
        cache
            .delete_cached_activity(fx.user_id, &fx.tenant, "strava", "r4")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        cache
            .delete_provider_activities(fx.user_id, &fx.tenant, "strava")
            .await
            .unwrap(),
        1
    );

    // The provider lists both again: nothing a detail read found returns.
    fx.list_sync(&[listed_run("r4", "Run"), listed_run("r5", "Run")])
        .await;
    for id in ["r4", "r5"] {
        let read = fx.read(fx.tenant, id).await.unwrap();
        assert_eq!(read.splits(), None, "{id}");
        assert_eq!(read.laps(), None, "{id}");
    }
}

#[test]
fn a_read_with_neither_splits_nor_laps_yields_a_detail_with_neither() {
    assert_eq!(
        ActivityDetail::from_activity(&listed_run("r6", "Run")),
        ActivityDetail {
            splits: None,
            laps: None,
        }
    );
    let with_laps = ActivityBuilder::new("r7", "Run", SportType::Run, started(), 1_800, "strava")
        .laps(vec![lap()])
        .build();
    assert_eq!(
        ActivityDetail::from_activity(&with_laps),
        ActivityDetail {
            splits: None,
            laps: Some(vec![lap()]),
        }
    );
}

/// A row records whether a stored detail read still answers it. One that
/// carried splits or laps stands; one that found neither answers only until
/// its recheck instant, since a provider serves its activity without them
/// when the request carrying them failed, and past it the row reads as unread
/// so the activity view asks again.
#[tokio::test]
async fn a_detail_read_that_found_nothing_answers_its_row_only_until_its_recheck() {
    let fx = fixture().await;
    let runs = [
        listed_run("r8", "Run"),
        listed_run("r9", "Run"),
        listed_run("r10", "Run"),
        listed_run("r11", "Run"),
    ];
    fx.list_sync(&runs).await;
    let found_nothing = ActivityDetail::from_activity(&listed_run("r8", "Run"));
    assert!(found_nothing.is_empty());
    assert!(!detail().is_empty());
    let cache = &fx.database.repositories().activity_cache;
    let now = Utc::now();
    for (id, detail, recheck_at) in [
        ("r8", &found_nothing, Some(now + Duration::minutes(30))),
        ("r10", &found_nothing, Some(now - Duration::minutes(1))),
        ("r11", &detail(), None),
    ] {
        assert!(cache
            .store_activity_detail(fx.user_id, &fx.tenant, "strava", id, detail, recheck_at)
            .await
            .unwrap());
    }
    // A list sync after the reads leaves every mark in place.
    fx.list_sync(&runs).await;

    let rows = cache
        .get_cached_activity_rows(
            fx.user_id,
            &fx.tenant,
            started() - Duration::hours(1),
            started() + Duration::hours(1),
            10,
        )
        .await
        .unwrap();
    let mut read: Vec<(&str, bool)> = rows
        .iter()
        .map(|row| (row.activity.id(), row.detail_read))
        .collect();
    read.sort_unstable();
    assert_eq!(
        read,
        vec![("r10", false), ("r11", true), ("r8", true), ("r9", false)],
        "an empty read answers until its recheck, one with splits stands, none stored is unread"
    );
    let r8 = fx.read(fx.tenant, "r8").await.unwrap();
    assert_eq!(r8.splits(), None, "a read that found none fills none");
    assert_eq!(r8.laps(), None);
    let r11 = fx.read(fx.tenant, "r11").await.unwrap();
    assert_eq!(r11.laps(), Some(&vec![lap()]));
}
