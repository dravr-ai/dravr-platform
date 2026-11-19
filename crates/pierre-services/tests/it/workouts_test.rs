// ABOUTME: Covers the one row-level merge of stored copies into workouts and the two gates a read applies first
// ABOUTME: Copies across providers merge into one workout keeping each row; the display gate and the model gate drop what they must

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `pierre_services::workouts` (carnet#1485): the merge Home, the gear
//! ledger, the places pass and the activity aggregator share, and the gates
//! that run before it.

use std::collections::BTreeSet;
use std::iter;

use chrono::{DateTime, TimeZone, Utc};
use pierre_core::ai_policy::{AiUse, ProviderTerms, SourcePolicy};
use pierre_core::constants::oauth_providers::garmin_recorded;
use pierre_core::models::{Activity, ActivityBuilder, PlaceRef, SportType};
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::CachedActivityRow;
use pierre_providers::ai_scope::{
    ai_read, filter_activities, serve_over, tallied, with_ai_consent,
};
use pierre_services::workouts::{
    filter_activity_rows, served_rows, sport_bucket, workouts, Workout,
};

/// Strava's terms keep its data on Dravr's own surfaces; Polar's hide the
/// heart rate from a model; Coros hands a model nothing.
struct Terms;

/// Polar's rule: everything but the heart rate.
const POLAR: SourcePolicy = SourcePolicy {
    direct: AiUse::Redact(&["average_heart_rate"]),
    by_source: &[],
    other_sources: AiUse::Redact(&["average_heart_rate"]),
};

impl ProviderTerms for Terms {
    fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
        match provider {
            "polar" => Some(&POLAR),
            "coros" => Some(&SourcePolicy::DENY_ALL),
            _ => None,
        }
    }

    fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
        (provider == "strava").then_some(TransportPolicy::FirstPartyOnly)
    }
}

fn start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 4, 13, 0, 0)
        .single()
        .unwrap_or_default()
}

/// One recording of the Sunday ride, as `provider` stored it under `id`.
fn ride(provider: &str, id: &str) -> Activity {
    ActivityBuilder::new(id, "Sunday ride", SportType::Ride, start(), 7_200, provider)
        .distance_meters(60_000.0)
        .average_heart_rate(142)
        .build()
}

fn row(activity: Activity) -> CachedActivityRow {
    CachedActivityRow {
        provider: activity.provider().to_owned(),
        activity,
        route: None,
        detail_read: false,
        segment_read: false,
        stored_at: None,
        place: None,
    }
}

fn place(id: &str) -> PlaceRef {
    PlaceRef {
        id: id.to_owned(),
        name: None,
        athlete_named: false,
    }
}

/// The ride as Strava and as Garmin stored it; Garmin's copy names the
/// device it was recorded on.
fn strava_and_garmin() -> Vec<CachedActivityRow> {
    let garmin = ActivityBuilder::new(
        "g-1",
        "Sunday ride",
        SportType::Ride,
        start(),
        7_200,
        "garmin",
    )
    .distance_meters(60_010.0)
    .device_name("Garmin Edge 840")
    .build();
    vec![row(ride("strava", "s-1")), row(garmin)]
}

/// The answer Home's `recordings()` walk gave: the merged session, the
/// copy whose route it draws (one of the rows), the canonical copy, then
/// the others.
fn garmin_by_every_recording(workout: &Workout<'_>) -> bool {
    iter::once(&workout.session)
        .chain(iter::once(&workout.canonical.activity))
        .chain(workout.copies.iter().map(|row| &row.activity))
        .any(|activity| garmin_recorded(activity.source(), activity.device_name()))
}

#[test]
fn a_strava_copy_and_its_garmin_twin_are_one_workout() {
    let rows = strava_and_garmin();
    let merged = workouts(&rows);
    assert_eq!(
        merged.len(),
        1,
        "bug: two copies of one ride are two workouts"
    );
    assert_eq!(
        merged[0].rows().count(),
        2,
        "bug: the merged workout does not keep both stored rows"
    );
}

#[test]
fn a_workout_takes_its_place_from_the_canonical_row_else_a_copy() {
    let mut rows = strava_and_garmin();
    for stored in &mut rows {
        stored.place = Some(place(&format!("place-{}", stored.activity.id())));
    }
    let merged = workouts(&rows);
    let canonical = merged[0].canonical.activity.id().to_owned();
    assert_eq!(
        merged[0].place().map(|p| p.id.clone()),
        Some(format!("place-{canonical}")),
        "bug: the canonical row's place is not the workout's"
    );

    let mut rows = strava_and_garmin();
    let other = rows
        .iter()
        .position(|stored| stored.activity.id() != canonical)
        .unwrap_or_default();
    rows[other].place = Some(place("from-a-copy"));
    let merged = workouts(&rows);
    assert_eq!(
        merged[0].place().map(|p| p.id.as_str()),
        Some("from-a-copy"),
        "bug: a place only a copy holds is lost"
    );
}

#[test]
fn a_workout_reads_its_segments_from_any_copy() {
    let mut rows = strava_and_garmin();
    assert!(
        !workouts(&rows)[0].segments_read(),
        "bug: a workout no detail read reached reads its segments as read"
    );
    rows[1].segment_read = true;
    assert!(
        workouts(&rows)[0].segments_read(),
        "bug: a copy's segment read is not the workout's"
    );
}

#[test]
fn garmin_data_is_found_as_homes_recordings_found_it() {
    let rows = strava_and_garmin();
    let merged = workouts(&rows);
    assert!(
        merged[0].carries_garmin_data(),
        "bug: a workout with a Garmin-recorded copy carries no Garmin data"
    );
    assert_eq!(
        merged[0].carries_garmin_data(),
        garmin_by_every_recording(&merged[0]),
        "bug: the rewritten walk answers otherwise than Home's recordings walk"
    );
    let strava_only = vec![row(ride("strava", "s-2"))];
    let merged = workouts(&strava_only);
    assert!(
        !merged[0].carries_garmin_data(),
        "bug: a Strava-only workout carries Garmin data"
    );
}

#[tokio::test]
async fn the_display_gate_drops_a_first_party_copy_externally_only() {
    let rows = strava_and_garmin();
    let in_app = serve_over(Transport::MobileApp, async {
        served_rows(&Terms, rows.clone())
    })
    .await;
    assert_eq!(in_app.len(), 2, "bug: the athlete's own app loses a copy");

    let external = serve_over(Transport::McpHttp, async { served_rows(&Terms, rows) }).await;
    assert_eq!(
        external
            .iter()
            .map(|stored| stored.provider.as_str())
            .collect::<Vec<_>>(),
        vec!["garmin"],
        "bug: Strava's copy is served over an external transport"
    );
    assert_eq!(
        workouts(&external).len(),
        1,
        "bug: the external read lost the workout with its withheld copy"
    );
}

/// The ride as `provider` stored it, with a place and its segments read.
fn placed(provider: &str, id: &str) -> CachedActivityRow {
    CachedActivityRow {
        segment_read: true,
        place: Some(place("gai-luron")),
        ..row(ride(provider, id))
    }
}

#[tokio::test]
async fn the_model_gate_keeps_each_rows_own_fields_and_tallies_as_the_item_filter() {
    let rows = vec![
        placed("garmin", "kept"),
        placed("polar", "reduced"),
        placed("coros", "dropped"),
        placed("strava", "off-interface"),
    ];
    let activities: Vec<Activity> = rows.iter().map(|stored| stored.activity.clone()).collect();

    let gate = |rows: Vec<CachedActivityRow>| {
        serve_over(
            Transport::McpHttp,
            with_ai_consent(
                BTreeSet::new(),
                ai_read(tallied(async { filter_activity_rows(&Terms, rows) })),
            ),
        )
    };
    let ((kept, by_rows), _) = gate(rows).await;
    let ((filtered, by_items), _) = serve_over(
        Transport::McpHttp,
        with_ai_consent(
            BTreeSet::new(),
            ai_read(tallied(async { filter_activities(&Terms, activities) })),
        ),
    )
    .await;

    assert_eq!(
        kept.iter()
            .map(|stored| stored.activity.id())
            .collect::<Vec<_>>(),
        vec!["kept", "reduced"],
        "bug: the row gate keeps otherwise than the item filter"
    );
    let as_json = |activities: Vec<&Activity>| serde_json::to_value(activities).ok();
    assert_eq!(
        as_json(kept.iter().map(|stored| &stored.activity).collect()),
        as_json(filtered.iter().collect()),
        "bug: a row's activity is not the one the item filter serves"
    );
    let reduced = &kept[1];
    assert_eq!(
        reduced.activity.average_heart_rate(),
        None,
        "bug: a reduced activity keeps the field its terms withhold"
    );
    assert!(
        kept.iter()
            .all(|stored| stored.segment_read && stored.place.is_some()),
        "bug: the gate dropped a kept row's own place or segment read"
    );
    assert_eq!(
        by_rows, by_items,
        "bug: the row gate tallies otherwise than the item filter"
    );
    assert_eq!(
        (by_rows.dropped, by_rows.reduced, by_rows.off_interface),
        (1, 1, 1),
        "bug: the tally misses a withheld, reduced or off-interface row"
    );
}

#[test]
fn a_workout_is_bucketed_under_the_stores_sport_spelling() {
    assert_eq!(
        sport_bucket(&ride("strava", "s-3")),
        "ride",
        "bug: a ride is not bucketed under the store's own spelling"
    );
}
