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

use crate::common;

use std::sync::Arc;

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{
    Activity, ActivityBuilder, ActivityTag, GearRef, Lap, Split, SportType, TagSource, TaggedBy,
    TenantId,
};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::{ActivityDetail, AmendedReads, CachedRowsQuery};
use uuid::Uuid;

fn started() -> Timestamp {
    Timestamp::now() - Elapsed::days(1)
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
        ..ActivityDetail::default()
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
    let window = (started() - Elapsed::hours(1), Timestamp::now());
    let listed = cache
        .get_cached_activities(fx.user_id, &fx.tenant, None, window.0, window.1, 10)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].splits().map(Vec::len), Some(2));
    let rows = cache
        .get_cached_activity_rows(
            fx.user_id,
            &fx.tenant,
            &CachedRowsQuery::window(window.0, window.1, 10),
        )
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
        ActivityDetail::default(),
        "bug: a read with no detail-only field yields an empty detail"
    );
    let with_laps = ActivityBuilder::new("r7", "Run", SportType::Run, started(), 1_800, "strava")
        .laps(vec![lap()])
        .build();
    assert_eq!(
        ActivityDetail::from_activity(&with_laps),
        ActivityDetail {
            laps: Some(vec![lap()]),
            ..ActivityDetail::default()
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
    let now = Timestamp::now();
    for (id, detail, recheck_at) in [
        ("r8", &found_nothing, Some(now + Elapsed::minutes(30))),
        ("r10", &found_nothing, Some(now - Elapsed::minutes(1))),
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
            &CachedRowsQuery::window(
                started() - Elapsed::hours(1),
                started() + Elapsed::hours(1),
                10,
            ),
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

/// A detail read once carries the athlete's description, device, gear and
/// private note, and every later read of the row is served them without
/// another provider read (carnet#1480).
#[tokio::test]
async fn a_stored_detail_serves_the_athletes_text_after_a_list_sync() {
    let fx = fixture().await;
    fx.list_sync(&[listed_run("r9", "Bûcheronner !")]).await;
    let detailed = ActivityBuilder::new(
        "r9",
        "Bûcheronner !",
        SportType::Run,
        started(),
        13_320,
        "strava",
    )
    .description("Mal aux bras !".to_owned())
    .device_name("Garmin fēnix 6S Pro")
    .gear(GearRef {
        id: Some("g7".to_owned()),
        name: "Speedgoat".to_owned(),
    })
    .private_note("Genou sensible")
    .build();
    let stored = fx
        .database
        .repositories()
        .activity_cache
        .store_activity_detail(
            fx.user_id,
            &fx.tenant,
            "strava",
            "r9",
            &ActivityDetail::from_activity(&detailed),
            None,
        )
        .await
        .unwrap();
    assert!(stored, "bug: the detail must be stored on the user's row");

    fx.list_sync(&[listed_run("r9", "Bûcheronner !")]).await;
    let row = fx.read(fx.tenant, "r9").await.unwrap();
    assert_eq!(
        row.description(),
        Some("Mal aux bras !"),
        "bug: description lost"
    );
    assert_eq!(
        row.device_name(),
        Some("Garmin fēnix 6S Pro"),
        "bug: device lost"
    );
    assert_eq!(
        row.gear().map(|g| g.name.as_str()),
        Some("Speedgoat"),
        "bug: gear lost"
    );
    assert_eq!(row.private_note(), Some("Genou sensible"), "bug: note lost");
}

/// The athlete's Strava tags ride in the list copy the cache stores whole:
/// they read back by value, source included, and the merged set unions them
/// with the workout code (carnet#1618).
#[tokio::test]
async fn the_athletes_tags_survive_the_cache_write_and_read() {
    let fx = fixture().await;
    let tagged = ActivityBuilder::new(
        "r12",
        "Easy spin",
        SportType::Run,
        started(),
        1_800,
        "strava",
    )
    .workout_type(1)
    .tags(vec![
        TaggedBy::new(ActivityTag::Recovery, TagSource::ProviderTag),
        TaggedBy::new(ActivityTag::Race, TagSource::ProviderTag),
    ])
    .build();
    fx.list_sync(&[tagged, listed_run("r13", "No tags read")])
        .await;
    let row = fx.read(fx.tenant, "r12").await.unwrap();
    assert_eq!(
        row.tags(),
        Some(
            &[
                TaggedBy::new(ActivityTag::Recovery, TagSource::ProviderTag),
                TaggedBy::new(ActivityTag::Race, TagSource::ProviderTag),
            ][..]
        ),
        "bug: tags lost in the cache round trip"
    );
    assert_eq!(
        row.merged_tags(),
        vec![ActivityTag::Recovery, ActivityTag::Race],
        "bug: code 1 and tag 1 are one race"
    );
    let bare = fx.read(fx.tenant, "r13").await.unwrap();
    assert!(bare.tags().is_none(), "bug: unread tags became Some");
}

/// Strava's tags come from the first training-list page alone: a deeper
/// page re-reading the activity carries none and must not erase them, while
/// a read that does carry tags replaces what was stored.
#[tokio::test]
async fn a_list_write_without_tags_keeps_the_stored_tags() {
    let fx = fixture().await;
    let with = |tags: Option<Vec<ActivityTag>>| {
        ActivityBuilder::new(
            "r14",
            "Easy spin",
            SportType::Run,
            started(),
            1_800,
            "strava",
        )
        .tags_opt(tags.map(|tags| {
            tags.into_iter()
                .map(|tag| TaggedBy::new(tag, TagSource::ProviderTag))
                .collect()
        }))
        .build()
    };
    fx.list_sync(&[with(Some(vec![ActivityTag::Recovery]))])
        .await;
    fx.list_sync(&[with(None)]).await;
    let kept = fx.read(fx.tenant, "r14").await.unwrap();
    assert_eq!(
        kept.tags(),
        Some(&[TaggedBy::new(ActivityTag::Recovery, TagSource::ProviderTag)][..]),
        "bug: a deeper page without tags erased the stored tags"
    );
    fx.list_sync(&[with(Some(vec![ActivityTag::Race]))]).await;
    let replaced = fx.read(fx.tenant, "r14").await.unwrap();
    assert_eq!(
        replaced.tags(),
        Some(&[TaggedBy::new(ActivityTag::Race, TagSource::ProviderTag)][..]),
        "bug: a fresh tag read did not replace the stored tags"
    );
}

fn run_with(id: &str, tags: Option<Vec<ActivityTag>>, gear: Option<&str>) -> Activity {
    ActivityBuilder::new(id, "Run", SportType::Run, started(), 1_800, "strava")
        .tags_opt(tags.map(|tags| {
            tags.into_iter()
                .map(|tag| TaggedBy::new(tag, TagSource::ProviderTag))
                .collect()
        }))
        .gear_opt(gear.map(|name| GearRef {
            id: Some(format!("g-{name}")),
            name: name.to_owned(),
        }))
        .build()
}

/// A page of rewrites keeps every row's stored tags and gear, however many
/// rows it holds: the stored copies are read in chunks, not one per row, and
/// a batch longer than one chunk loses none of them.
#[tokio::test]
async fn a_batch_of_rewrites_keeps_each_rows_tags_and_gear() {
    let fx = fixture().await;
    let ids: Vec<String> = (0..1_050).map(|i| format!("b{i}")).collect();
    let stored: Vec<Activity> = ids
        .iter()
        .map(|id| run_with(id, Some(vec![ActivityTag::Race]), Some("Shoes")))
        .collect();
    fx.list_sync(&stored).await;

    let bare: Vec<Activity> = ids.iter().map(|id| run_with(id, None, None)).collect();
    fx.list_sync(&bare).await;

    for id in [&ids[0], &ids[499], &ids[500], &ids[1_049]] {
        let kept = fx.read(fx.tenant, id).await.unwrap();
        assert_eq!(
            kept.tags(),
            Some(&[TaggedBy::new(ActivityTag::Race, TagSource::ProviderTag)][..]),
            "bug: a batched rewrite lost the tags of {id}"
        );
        assert_eq!(
            kept.gear().map(|g| g.name.as_str()),
            Some("Shoes"),
            "bug: a batched rewrite lost the gear of {id}"
        );
    }
}

/// Rows of one batch are judged each by its own stored copy: a row with
/// nothing stored stays bare, one stored with tags keeps them, and one that
/// brings its own tags replaces the stored ones.
#[tokio::test]
async fn a_mixed_batch_judges_each_row_by_its_own_stored_copy() {
    let fx = fixture().await;
    fx.list_sync(&[
        run_with("m1", Some(vec![ActivityTag::Recovery]), None),
        run_with("m3", Some(vec![ActivityTag::Recovery]), None),
    ])
    .await;
    fx.list_sync(&[
        run_with("m1", None, None),
        run_with("m2", None, None),
        run_with("m3", Some(vec![ActivityTag::Race]), None),
    ])
    .await;
    let m1 = fx.read(fx.tenant, "m1").await.unwrap();
    assert_eq!(
        m1.tags(),
        Some(&[TaggedBy::new(ActivityTag::Recovery, TagSource::ProviderTag)][..]),
        "bug: the stored tags of m1 were not kept"
    );
    assert!(
        fx.read(fx.tenant, "m2").await.unwrap().tags().is_none(),
        "bug: a row with nothing stored gained tags"
    );
    let m3 = fx.read(fx.tenant, "m3").await.unwrap();
    assert_eq!(
        m3.tags(),
        Some(&[TaggedBy::new(ActivityTag::Race, TagSource::ProviderTag)][..]),
        "bug: a row's own tags did not replace the stored ones"
    );
}

/// One batch carrying an id twice: the later copy keeps what the earlier one
/// wrote, as it did when each row was read back after the one before it.
#[tokio::test]
async fn a_later_copy_in_one_batch_keeps_what_the_earlier_one_wrote() {
    let fx = fixture().await;
    fx.list_sync(&[
        run_with("d1", Some(vec![ActivityTag::Race]), Some("Shoes")),
        run_with("d1", None, None),
    ])
    .await;
    let kept = fx.read(fx.tenant, "d1").await.unwrap();
    assert_eq!(
        kept.tags(),
        Some(&[TaggedBy::new(ActivityTag::Race, TagSource::ProviderTag)][..]),
        "bug: the later copy erased the tags the earlier copy of the batch wrote"
    );
    assert_eq!(
        kept.gear().map(|g| g.name.as_str()),
        Some("Shoes"),
        "bug: the later copy erased the earlier copy's gear"
    );
}

impl Fixture {
    async fn amend(&self, rows: &[Activity]) -> AmendedReads {
        self.database
            .repositories()
            .activity_cache
            .amend_listed_reads(self.user_id, &self.tenant, "strava", rows)
            .await
            .unwrap()
    }
}

/// A deeper list page fills the tags and gear its rows carry into the stored
/// rows, creates none, and moves nothing else of a row: neither its other
/// fields nor the sync stamp the list head's freshness reads.
#[tokio::test]
async fn an_amend_fills_tags_and_gear_and_nothing_else() {
    let fx = fixture().await;
    let cache = &fx.database.repositories().activity_cache;
    let synced_at = Timestamp::now() - Elapsed::hours(5);
    cache
        .upsert_activities_synced_at(
            fx.user_id,
            &fx.tenant,
            "strava",
            &[listed_run("a1", "Stored name"), listed_run("a2", "Other")],
            synced_at,
        )
        .await
        .unwrap();
    let before = cache
        .latest_activity_sync(fx.user_id, &fx.tenant, "strava")
        .await
        .unwrap();

    let page = [
        run_with("a1", Some(vec![ActivityTag::Race]), Some("Shoes")),
        run_with("absent", Some(vec![ActivityTag::Race]), None),
    ];
    let amended = fx.amend(&page).await;

    assert_eq!(amended.present, 1, "bug: an absent row was counted present");
    assert_eq!(
        amended.filled, 1,
        "bug: the row's tags and gear did not count as one fill"
    );
    let filled = fx.read(fx.tenant, "a1").await.unwrap();
    assert_eq!(
        filled.tags(),
        Some(&[TaggedBy::new(ActivityTag::Race, TagSource::ProviderTag)][..]),
        "bug: the listed tag was not filled in"
    );
    assert_eq!(
        filled.gear().map(|g| g.name.as_str()),
        Some("Shoes"),
        "bug: the listed gear was not filled in"
    );
    assert_eq!(filled.name(), "Stored name", "bug: an amend moved the name");
    assert!(
        fx.read(fx.tenant, "absent").await.is_none(),
        "bug: an amend created a row"
    );
    assert!(
        fx.read(fx.tenant, "a2").await.unwrap().tags().is_none(),
        "bug: a row off the page gained tags"
    );
    assert_eq!(
        cache
            .latest_activity_sync(fx.user_id, &fx.tenant, "strava")
            .await
            .unwrap(),
        before,
        "bug: an amend moved the list head's freshness"
    );
}

/// A page row that carries no tags never erases stored ones, one that
/// carries a read replaces them, and a second amend of the same page
/// changes nothing.
#[tokio::test]
async fn an_amend_never_replaces_a_stored_tag_read_with_none() {
    let fx = fixture().await;
    fx.list_sync(&[run_with("k1", Some(vec![ActivityTag::Recovery]), None)])
        .await;

    let bare = fx.amend(&[run_with("k1", None, None)]).await;
    assert_eq!(
        (bare.present, bare.filled),
        (1, 0),
        "bug: a page row without tags amended the stored tags"
    );
    assert_eq!(
        fx.read(fx.tenant, "k1").await.unwrap().tags(),
        Some(&[TaggedBy::new(ActivityTag::Recovery, TagSource::ProviderTag)][..]),
        "bug: a page row without tags erased the stored tags"
    );

    let fresh = [run_with("k1", Some(vec![ActivityTag::Race]), None)];
    assert_eq!(
        fx.amend(&fresh).await.filled,
        1,
        "bug: a fresh tag read did not amend the row"
    );
    assert_eq!(
        fx.read(fx.tenant, "k1").await.unwrap().tags(),
        Some(&[TaggedBy::new(ActivityTag::Race, TagSource::ProviderTag)][..]),
        "bug: a fresh tag read did not replace the stored one"
    );
    assert_eq!(
        fx.amend(&fresh).await.filled,
        0,
        "bug: an amend that changes nothing wrote the row"
    );
}

/// Gear fills a row that holds none and the id of one that holds a name
/// alone; a gear the row already names with its id stands.
#[tokio::test]
async fn an_amend_fills_the_gear_id_without_replacing_a_named_gear() {
    let fx = fixture().await;
    let named_only = ActivityBuilder::new("g1", "Run", SportType::Run, started(), 1_800, "strava")
        .gear_opt(Some(GearRef {
            id: None,
            name: "Old name".to_owned(),
        }))
        .build();
    let identified = ActivityBuilder::new("g2", "Run", SportType::Run, started(), 1_800, "strava")
        .gear_opt(Some(GearRef {
            id: Some("g-mine".to_owned()),
            name: "Mine".to_owned(),
        }))
        .build();
    fx.list_sync(&[named_only, identified]).await;

    let amended = fx
        .amend(&[
            run_with("g1", None, Some("New")),
            run_with("g2", None, Some("Other")),
        ])
        .await;

    assert_eq!(amended.filled, 1, "bug: the gear id fill did not count");
    let g1 = fx.read(fx.tenant, "g1").await.unwrap();
    let gear = g1.gear().unwrap();
    assert_eq!(
        gear.name, "Old name",
        "bug: the stored gear name was replaced"
    );
    assert_eq!(
        gear.id.as_deref(),
        Some("g-New"),
        "bug: the gear id was not filled"
    );
    let g2 = fx.read(fx.tenant, "g2").await.unwrap();
    assert_eq!(
        g2.gear().and_then(|g| g.id.as_deref()),
        Some("g-mine"),
        "bug: a named gear's own id was replaced"
    );
}

/// A tag walk's amend racing a batched list write of the same rows: the write
/// merges over the copies it read and lands only while the row still holds
/// them, so a row an amend filled is never left without its tags. An amend a
/// list write overtook is skipped by its own guard and counts nothing; every
/// row it did fill still holds its tags. The two run concurrently over many
/// rows, so their awaits interleave per row.
#[tokio::test]
async fn a_concurrent_amend_is_never_erased_by_a_batched_list_write() {
    let fx = fixture().await;
    let ids: Vec<String> = (0..120).map(|i| format!("r{i}")).collect();
    let stored: Vec<Activity> = ids.iter().map(|id| run_with(id, None, None)).collect();
    fx.list_sync(&stored).await;
    let amended: Vec<Activity> = ids
        .iter()
        .map(|id| run_with(id, Some(vec![ActivityTag::Race]), None))
        .collect();
    let rewrites: Vec<Activity> = ids.iter().map(|id| run_with(id, None, None)).collect();

    let cache = &fx.database.repositories().activity_cache;
    let (filled, rewritten) = tokio::join!(
        async {
            let mut filled = 0;
            for page in amended.chunks(10) {
                filled += fx.amend(page).await.filled;
            }
            filled
        },
        async {
            cache
                .upsert_activities(fx.user_id, &fx.tenant, "strava", &rewrites)
                .await
                .unwrap()
        }
    );
    assert_eq!(rewritten, 120, "bug: the batch lost a row");
    let mut tagged = 0_u64;
    for id in &ids {
        if fx.read(fx.tenant, id).await.unwrap().tags().is_some() {
            tagged += 1;
        }
    }
    assert!(
        tagged >= filled,
        "bug: {filled} rows were amended but {tagged} hold tags: a list write erased an amend"
    );
}

/// Before the insert of `lead`, the batch's first activity, gives `appears`
/// and `moves` the tagged copies stored under `tagged-<id>`: `appears` is
/// created, `moves` rewritten. It stands in for a tag walk's amend landing
/// between a batch's read of the stored copies and its later writes, as a
/// command of its own: the batch writes one statement per activity.
const SQLITE_AMEND_MID_BATCH: &str = "
    CREATE TRIGGER amend_mid_batch BEFORE INSERT ON cached_activities
    WHEN NEW.activity_id = 'lead'
    BEGIN
        INSERT INTO cached_activities
            (id, user_id, tenant_id, provider, activity_id, sport_type, start_date, synced_at, data_json)
        SELECT 'amend-appears', NEW.user_id, NEW.tenant_id, NEW.provider,
               'appears', NEW.sport_type, NEW.start_date, NEW.synced_at, t.data_json
        FROM cached_activities t WHERE t.activity_id = 'tagged-appears';
        UPDATE cached_activities
        SET data_json = (SELECT t.data_json FROM cached_activities t
                         WHERE t.activity_id = 'tagged-moves')
        WHERE activity_id = 'moves';
    END";

#[cfg(feature = "postgresql")]
const POSTGRES_AMEND_MID_BATCH: [&str; 2] = [
    "
    CREATE FUNCTION amend_mid_batch() RETURNS trigger LANGUAGE plpgsql AS $$
    BEGIN
        IF NEW.activity_id = 'lead' THEN
            INSERT INTO cached_activities
                (id, user_id, tenant_id, provider, activity_id, sport_type, start_date, synced_at, data_json)
            SELECT 'amend-appears', NEW.user_id, NEW.tenant_id, NEW.provider,
                   'appears', NEW.sport_type, NEW.start_date, NEW.synced_at, t.data_json
            FROM cached_activities t WHERE t.activity_id = 'tagged-appears';
            UPDATE cached_activities
            SET data_json = (SELECT t.data_json FROM cached_activities t
                             WHERE t.activity_id = 'tagged-moves')
            WHERE activity_id = 'moves';
        END IF;
        RETURN NEW;
    END $$",
    "CREATE TRIGGER amend_mid_batch BEFORE INSERT ON cached_activities
     FOR EACH ROW EXECUTE FUNCTION amend_mid_batch()",
];

async fn arm_amend_mid_batch(database: &Database) {
    match database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(SQLITE_AMEND_MID_BATCH)
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            for statement in POSTGRES_AMEND_MID_BATCH {
                sqlx::query(statement).execute(db.pool()).await.unwrap();
            }
        }
    }
}

/// An amend that lands between a batch's read and its write is merged over,
/// never erased, both when the read found the row and when it found none:
/// the row the amend created since is refused to the unguarded write and
/// read again (carnet#1622).
#[tokio::test]
async fn an_amend_landing_between_a_batchs_read_and_write_keeps_its_tags() {
    let fx = fixture().await;
    fx.list_sync(&[
        run_with("tagged-appears", Some(vec![ActivityTag::Race]), None),
        run_with("tagged-moves", Some(vec![ActivityTag::Race]), None),
        run_with("moves", None, None),
    ])
    .await;
    arm_amend_mid_batch(&fx.database).await;

    let written = fx
        .database
        .repositories()
        .activity_cache
        .upsert_activities(
            fx.user_id,
            &fx.tenant,
            "strava",
            &[
                run_with("lead", None, None),
                run_with("appears", None, None),
                run_with("moves", None, None),
            ],
        )
        .await
        .unwrap();

    assert_eq!(written, 3, "bug: the batch lost a row");
    for id in ["appears", "moves"] {
        let row = fx.read(fx.tenant, id).await.unwrap();
        assert_eq!(row.id(), id, "the batch's own copy is what stands");
        assert!(
            row.tags().is_some(),
            "bug: the amend that landed mid-batch on {id} was erased"
        );
    }
}
