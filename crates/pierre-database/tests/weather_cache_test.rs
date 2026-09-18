// ABOUTME: Covers WeatherCacheRepository against whichever backend DATABASE_URL names
// ABOUTME: Round-trips a bucket, proves the upsert refreshes in place, and keeps the four-part key distinct
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `weather_cache` is one of the few tables reachable without a tenant or user
//! key, because a reading for a place and an hour is the same for everyone. Its
//! repository had no test of any kind, so nothing held its statements to their
//! contract — including the `CURRENT_TIMESTAMP` default the upsert writes,
//! which is the one construct that has to mean the same thing on both engines.
//!
//! These run on `SQLite` and on `PostgreSQL`: `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover both backends.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::WeatherCacheEntry;

fn entry(lat_centi: i32, lng_centi: i32, hour_unix: i64, temp: f32) -> WeatherCacheEntry {
    WeatherCacheEntry {
        lat_centi,
        lng_centi,
        hour_unix,
        provider: "openmeteo".to_owned(),
        temperature_celsius: temp,
        humidity_percentage: Some(61.5),
        wind_speed_kmh: Some(12.25),
        conditions: "partly cloudy".to_owned(),
    }
}

#[tokio::test]
async fn put_then_get_round_trips_every_field() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let written = entry(4551, -7362, 1_767_225_600, 7.5);

    repos.weather_cache.put(written.clone()).await.unwrap();
    let read = repos
        .weather_cache
        .get(4551, -7362, 1_767_225_600, "openmeteo")
        .await
        .unwrap()
        .expect("the bucket just written must read back");

    assert_eq!(read.lat_centi, 4551);
    assert_eq!(read.lng_centi, -7362);
    assert_eq!(read.hour_unix, 1_767_225_600);
    assert_eq!(read.provider, "openmeteo");
    assert_eq!(read.temperature_celsius, 7.5);
    assert_eq!(read.humidity_percentage, Some(61.5));
    assert_eq!(read.wind_speed_kmh, Some(12.25));
    assert_eq!(read.conditions, "partly cloudy");
}

#[tokio::test]
async fn put_refreshes_the_bucket_in_place() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    repos
        .weather_cache
        .put(entry(100, 200, 1_767_312_000, 3.0))
        .await
        .unwrap();
    let mut refreshed = entry(100, 200, 1_767_312_000, 21.0);
    refreshed.conditions = "clear".to_owned();
    refreshed.humidity_percentage = None;
    repos.weather_cache.put(refreshed).await.unwrap();

    let read = repos
        .weather_cache
        .get(100, 200, 1_767_312_000, "openmeteo")
        .await
        .unwrap()
        .expect("the refreshed bucket must still be there");

    // The conflict target is the four-part key, so the second write replaces
    // the reading rather than adding a row — and a field that went absent has
    // to come back absent, not stale.
    assert_eq!(read.temperature_celsius, 21.0);
    assert_eq!(read.conditions, "clear");
    assert_eq!(read.humidity_percentage, None);
    assert_eq!(read.wind_speed_kmh, Some(12.25));
}

#[tokio::test]
async fn each_part_of_the_key_addresses_a_different_bucket() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let base = entry(500, 600, 1_767_398_400, 10.0);
    repos.weather_cache.put(base.clone()).await.unwrap();

    let mut other_hour = base.clone();
    other_hour.hour_unix = 1_767_402_000;
    other_hour.temperature_celsius = 11.0;
    repos.weather_cache.put(other_hour).await.unwrap();

    let mut other_provider = base.clone();
    other_provider.provider = "openweathermap".to_owned();
    other_provider.temperature_celsius = 12.0;
    repos.weather_cache.put(other_provider).await.unwrap();

    assert_eq!(
        repos
            .weather_cache
            .get(500, 600, 1_767_398_400, "openmeteo")
            .await
            .unwrap()
            .unwrap()
            .temperature_celsius,
        10.0
    );
    assert_eq!(
        repos
            .weather_cache
            .get(500, 600, 1_767_402_000, "openmeteo")
            .await
            .unwrap()
            .unwrap()
            .temperature_celsius,
        11.0
    );
    assert_eq!(
        repos
            .weather_cache
            .get(500, 600, 1_767_398_400, "openweathermap")
            .await
            .unwrap()
            .unwrap()
            .temperature_celsius,
        12.0
    );
}

#[tokio::test]
async fn a_bucket_never_written_reads_as_absent() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    assert!(repos
        .weather_cache
        .get(1, 1, 1_767_484_800, "openmeteo")
        .await
        .unwrap()
        .is_none());
}
