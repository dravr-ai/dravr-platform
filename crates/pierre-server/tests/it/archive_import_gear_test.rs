// ABOUTME: An imported Strava export's Activity Gear attributed in the athlete's gear ledger (carnet#1236)
// ABOUTME: Pairs matched by name or declared, shoe sports only, each workout once, a re-import changing nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Archive imports bring their gear (design B.4, step 3).
//!
//! Each test imports a synthetic Strava export through the athlete's own
//! `/api/imports` routes, steps run in this process, and reads the gear
//! ledger the sink wrote.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use pierre_core::constants::oauth_providers::{STRAVA, STRAVA_ARCHIVE};
use pierre_core::models::{
    ActivityBuilder, ActivityGearKey, AthleteGear, AttributionSource, GearAttribution,
    GearDeclaration, GearKind, SportType, DEFAULT_SHOE_WARN_DISTANCE_METERS,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_test_support::fit::FitWorkout;

use crate::archive_import_test::{import_export, imports_serving, seed_athlete, Athlete};
use crate::common;

/// Strava's `activities.csv` header row (English), as barrage pins it.
const CSV_HEADER: &str = "Activity ID,Activity Date,Activity Name,Activity Type,\
    Activity Description,Elapsed Time,Distance,Max Heart Rate,Relative Effort,Commute,\
    Activity Private Note,Activity Gear,Filename,Athlete Weight,Bike Weight,Elapsed Time,\
    Moving Time,Distance,Max Speed,Average Speed,Elevation Gain,Max Heart Rate,\
    Average Heart Rate,Calories";

/// One CSV row: id, start, name, type, elapsed seconds, filename and gear;
/// 5 km in the detail block.
fn csv_row(
    id: &str,
    start: DateTime<Utc>,
    kind: &str,
    elapsed: u32,
    file: &str,
    gear: &str,
) -> String {
    let date = start.format("%b %-d, %Y, %-I:%M:%S %p");
    format!(
        "{id},\"{date}\",Workout,{kind},,{elapsed},5.0,,,false,,\"{gear}\",{file},,,{elapsed}.0,\
         {elapsed}.0,5000.0,4.0,3.1,40.0,,,321"
    )
}

/// Twenty days ago at 07:00 UTC, whole seconds as the export writes them.
fn twenty_days_ago() -> DateTime<Utc> {
    (Utc::now() - Duration::days(20))
        .date_naive()
        .and_hms_opt(7, 0, 0)
        .expect("a valid time")
        .and_utc()
}

/// Two runs in one pair (its name in two cases), a ride on a bike, and a run
/// with no gear.
fn export_with_gear() -> Vec<u8> {
    let start = twenty_days_ago();
    let rows = [
        csv_row("9101", start, "Run", 1200, "", "Pegasus 40"),
        csv_row(
            "9102",
            start + Duration::days(1),
            "Run",
            1500,
            "",
            "pegasus 40",
        ),
        csv_row(
            "9103",
            start + Duration::days(2),
            "Ride",
            600,
            "activities/9103.fit",
            "Canyon Endurace",
        ),
        csv_row("9104", start + Duration::days(3), "Run", 900, "", ""),
    ];
    let mut csv = format!("{CSV_HEADER}\n");
    for row in &rows {
        csv.push_str(row);
        csv.push('\n');
    }
    zip_of(&[
        ("activities.csv", csv.as_bytes()),
        (
            "activities/9103.fit",
            &FitWorkout::ride(start + Duration::days(2), 600).encode(),
        ),
    ])
}

fn zip_of(members: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in members {
        writer.start_file(*name, options).expect("a member starts");
        writer.write_all(bytes).expect("a member is written");
    }
    writer.finish().expect("the archive closes").into_inner()
}

async fn ledger(resources: &Arc<ServerContext>, athlete: &Athlete) -> Vec<AthleteGear> {
    resources
        .common
        .repos
        .athlete_gear
        .list_gear(athlete.user_id)
        .await
        .expect("the ledger reads")
}

async fn attributions(resources: &Arc<ServerContext>, athlete: &Athlete) -> Vec<GearAttribution> {
    let start = twenty_days_ago();
    let mut held = resources
        .common
        .repos
        .athlete_gear
        .list_gear_attributions(
            athlete.user_id,
            start - Duration::days(2),
            start + Duration::days(10),
        )
        .await
        .expect("the attributions read");
    held.sort_by(|a, b| a.key.activity_id.cmp(&b.key.activity_id));
    held
}

async fn context_serving(export: Vec<u8>) -> Arc<ServerContext> {
    let (imports, _) = imports_serving(export);
    common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context")
}

#[tokio::test]
async fn an_imported_runs_gear_is_attributed_to_the_pair_of_that_name() {
    let resources = context_serving(export_with_gear()).await;
    let athlete = seed_athlete(&resources, "archive-gear").await;
    let finished = import_export(&resources, &athlete).await;
    assert_eq!(finished["status"], "completed", "{finished}");

    let pairs = ledger(&resources, &athlete).await;
    assert_eq!(
        pairs.len(),
        1,
        "one pair for both spellings of the shoe, none for the bike: {pairs:?}"
    );
    let pair = &pairs[0];
    assert_eq!(
        pair.declaration.name, "Pegasus 40",
        "the pair takes the first row's spelling"
    );
    assert_eq!(
        pair.declaration.kind,
        GearKind::Shoe,
        "imported gear on a run is a pair of shoes"
    );
    assert_eq!(
        pair.declaration.sport,
        SportType::Run,
        "the pair is worn for the sport of its first run"
    );
    assert_eq!(
        pair.added_at,
        twenty_days_ago(),
        "the pair is in use from the first run that names it"
    );
    assert_eq!(pair.activity_count, 2, "{pair:?}");
    assert!(
        (pair.attributed_distance_meters - 10_000.0).abs() < 1e-6,
        "both 5 km runs count: {pair:?}"
    );

    let held = attributions(&resources, &athlete).await;
    let keys: Vec<(&str, &str, AttributionSource)> = held
        .iter()
        .map(|a| {
            (
                a.key.provider.as_str(),
                a.key.activity_id.as_str(),
                a.source,
            )
        })
        .collect();
    assert_eq!(
        keys,
        [
            (STRAVA_ARCHIVE, "9101", AttributionSource::Declared),
            (STRAVA_ARCHIVE, "9102", AttributionSource::Declared),
        ],
        "the two shod runs, as the athlete's own choice; the ride and the bare run hold none"
    );
}

#[tokio::test]
async fn importing_the_same_export_again_adds_no_pair_and_no_attribution() {
    let resources = context_serving(export_with_gear()).await;
    let athlete = seed_athlete(&resources, "archive-gear-again").await;
    import_export(&resources, &athlete).await;
    let again = import_export(&resources, &athlete).await;
    assert_eq!(again["status"], "completed", "{again}");

    let pairs = ledger(&resources, &athlete).await;
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!(pairs[0].activity_count, 2, "{pairs:?}");
    assert_eq!(
        attributions(&resources, &athlete).await.len(),
        2,
        "a re-projected chunk records nothing twice"
    );
}

#[tokio::test]
async fn a_workout_the_athlete_already_attributed_keeps_its_pair() {
    let resources = context_serving(export_with_gear()).await;
    let athlete = seed_athlete(&resources, "archive-gear-kept").await;
    let repos = &resources.common.repos;
    let start = twenty_days_ago();

    // Run 9101 was synced from Strava before the import, and the athlete
    // put it on another pair.
    let kinvara = repos
        .athlete_gear
        .create_gear(
            athlete.user_id,
            &GearDeclaration {
                kind: GearKind::Shoe,
                name: "Kinvara 14".to_owned(),
                sport: SportType::Run,
                initial_distance_meters: 0.0,
                warn_distance_meters: DEFAULT_SHOE_WARN_DISTANCE_METERS,
                pinned: false,
            },
            start - Duration::days(30),
        )
        .await
        .expect("the pair is declared");
    let synced = ActivityBuilder::new("9101", "Workout", SportType::Run, start, 1200, STRAVA)
        .distance_meters(5000.0)
        .build();
    repos
        .activity_cache
        .upsert_activities_synced_at(
            athlete.user_id,
            &athlete.tenant,
            STRAVA,
            &[synced],
            Utc::now(),
        )
        .await
        .expect("the synced run is cached");
    let strava_copy = ActivityGearKey {
        provider: STRAVA.to_owned(),
        activity_id: "9101".to_owned(),
    };
    repos
        .athlete_gear
        .declare_gear_attribution(
            athlete.user_id,
            &GearAttribution {
                key: strava_copy.clone(),
                gear_id: Some(kinvara),
                source: AttributionSource::Declared,
                start_date: start,
                distance_meters: 5000.0,
            },
            &[],
        )
        .await
        .expect("the athlete's choice is recorded");

    import_export(&resources, &athlete).await;

    let held = attributions(&resources, &athlete).await;
    let on_9101: Vec<&GearAttribution> = held
        .iter()
        .filter(|a| a.key.activity_id == "9101")
        .collect();
    assert_eq!(
        on_9101.len(),
        1,
        "one workout, one attribution, whichever copy holds it: {held:?}"
    );
    assert_eq!(
        on_9101[0].key, strava_copy,
        "the attribution stays on the copy the athlete chose it on"
    );
    assert_eq!(
        on_9101[0].gear_id,
        Some(kinvara),
        "the athlete's choice stays"
    );

    let pegasus = ledger(&resources, &athlete)
        .await
        .into_iter()
        .find(|pair| pair.declaration.name.eq_ignore_ascii_case("Pegasus 40"))
        .expect("9102 still brings its pair, spelled as its own row spells it");
    assert_eq!(
        pegasus.activity_count, 1,
        "only the run the athlete had not attributed takes the imported pair"
    );
}
