// ABOUTME: The synthetic-activity seeder gives every outdoor activity a start near Montreal and an indoor one none
// ABOUTME: The dev fixture draws each seeded route from that start, so a missing point is a map that never renders

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::cast_possible_truncation)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use common::{create_test_server_resources, create_test_user_with_plan};
use pierre_database::backends::factory::Database;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_seeders::synthetic_activities::{run, SeedArgs};
use uuid::Uuid;

/// Sports a watch records indoors, on an erg or in a pool.
const INDOOR: [&str; 6] = [
    "virtual_ride",
    "swim",
    "weight_training",
    "yoga",
    "workout",
    "rowing",
];

/// Montreal's centre and how far from it a seeded start may fall, in degrees.
const MONTREAL: (f64, f64) = (45.5017, -73.5673);
const MAX_LATITUDE_OFFSET: f64 = 0.07;
const MAX_LONGITUDE_OFFSET: f64 = 0.1;

/// One seeded row: its sport, whether it has a distance, and its start.
type SeededRow = (String, Option<f64>, Option<f64>, Option<f64>);

async fn seeded_rows(resources: &Arc<ServerContext>, user_id: Uuid) -> Vec<SeededRow> {
    let user = user_id.to_string();
    match &*resources.agent.database {
        Database::SQLite(sqlite) => sqlx::query_as(
            "SELECT sport_type, distance_meters, start_latitude, start_longitude \
             FROM synthetic_activities WHERE user_id = $1",
        )
        .bind(&user)
        .fetch_all(sqlite.pool())
        .await
        .unwrap(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => sqlx::query_as(
            "SELECT sport_type, distance_meters, start_latitude, start_longitude \
             FROM synthetic_activities WHERE CAST(user_id AS TEXT) = $1",
        )
        .bind(&user)
        .fetch_all(pg.pool())
        .await
        .unwrap(),
    }
}

#[tokio::test]
async fn outdoor_activities_start_near_montreal_and_indoor_ones_nowhere() {
    let res = create_test_server_resources().await.unwrap();
    let (user_id, _, _) =
        create_test_user_with_plan(&res.agent.database, "seeded@example.com", "professional")
            .await
            .unwrap();

    run(
        SeedArgs {
            email: "seeded@example.com".to_owned(),
            count: 60,
            days: 30,
            reset: false,
            seed: Some(7),
            provider: "strava".to_owned(),
        },
        &res.common.repos,
    )
    .await
    .unwrap();

    let rows = seeded_rows(&res, user_id).await;
    assert_eq!(rows.len(), 60);

    let mut outdoor = 0;
    let mut indoor = 0;
    for (sport, distance, latitude, longitude) in &rows {
        if distance.is_none() || INDOOR.contains(&sport.as_str()) {
            indoor += 1;
            assert_eq!(
                (latitude, longitude),
                (&None, &None),
                "{sport} was recorded indoors"
            );
            continue;
        }
        outdoor += 1;
        let (latitude, longitude) = (latitude.unwrap(), longitude.unwrap());
        assert!(
            (latitude - MONTREAL.0).abs() <= MAX_LATITUDE_OFFSET,
            "{sport} starts at latitude {latitude}, not near Montreal"
        );
        assert!(
            (longitude - MONTREAL.1).abs() <= MAX_LONGITUDE_OFFSET,
            "{sport} starts at longitude {longitude}, not near Montreal"
        );
    }
    // Sixty weighted draws land on both kinds, so each branch above ran.
    assert!(
        outdoor > 0 && indoor > 0,
        "outdoor {outdoor}, indoor {indoor}"
    );

    // Starts are spread, not stacked: a map of the week is not one pin.
    let mut starts: Vec<(i64, i64)> = rows
        .iter()
        .filter_map(|(_, _, latitude, longitude)| {
            Some((
                (latitude.as_ref()? * 1e4) as i64,
                (longitude.as_ref()? * 1e4) as i64,
            ))
        })
        .collect();
    starts.sort_unstable();
    starts.dedup();
    assert!(
        starts.len() > 1,
        "every outdoor activity starts at the same point"
    );
}
