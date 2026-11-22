// ABOUTME: A forecast with no location reads the athlete's top training place, rounded to 0.01° before it leaves
// ABOUTME: Pins the place chosen, its served name, the coordinates the vendor receives and the daylight read there

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1486, § Daylight and the default place.
//!
//! "What's the weather for my ride Saturday at 9?" names no place: the
//! forecast reads the athlete's most frequent training place. Its centroid is
//! derived from provider data and sits near where the athlete trains, so the
//! weather vendor receives it rounded to 0.01° (a ~1 km cell), never the point
//! itself; a location the caller gives is sent as given.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::Mutex;

use async_trait::async_trait;
use dravr_meteo::{
    GeocodeError, Geocoded, Geocoder, PlaceQuery, WeatherError, WeatherProvider, WeatherQuery,
    WeatherSample,
};
use dravr_tronc::dates::Timestamp;
use pierre_core::models::TenantId;
use pierre_database::repositories::{AthletePlace, PlaceRun};
use pierre_database::RepositoryRegistry;
use pierre_fitness_compute::daylight::{daylight, solar_date, Daylight};
use pierre_fitness_compute::places::PLACES_VERSION;
use pierre_tool_runtime::implementations::weather_forecast::{
    default_place, forecast, forecast_at_place,
};
use serde_json::json;
use uuid::Uuid;

use crate::messaging_fixtures::seed_user;
use dravr_tronc::dates::Zone;

/// A weather vendor that records the coordinates it was asked for.
struct RecordingProvider {
    asked: Mutex<Vec<WeatherQuery>>,
}

#[async_trait]
impl WeatherProvider for RecordingProvider {
    async fn weather_at(&self, query: WeatherQuery) -> Result<WeatherSample, WeatherError> {
        self.asked.lock().unwrap().push(query);
        Ok(WeatherSample {
            temperature_celsius: 9.0,
            humidity_percentage: Some(70.0),
            wind_speed_kmh: Some(10.0),
            conditions: "overcast".to_owned(),
        })
    }

    fn name(&self) -> &'static str {
        "recording"
    }
}

/// No place resolves by name here: the cases give coordinates or none.
struct NoGeocoder;

#[async_trait]
impl Geocoder for NoGeocoder {
    async fn resolve(&self, query: &PlaceQuery) -> Result<Geocoded, GeocodeError> {
        Err(GeocodeError::NotFound {
            place: query.name.clone(),
        })
    }

    fn name(&self) -> &'static str {
        "none"
    }
}

/// An unnamed place of `count` workouts centred at (`lat`, `lon`).
fn place(lat: f64, lon: f64, count: u32) -> AthletePlace {
    let now = Timestamp::now();
    AthletePlace {
        id: Uuid::new_v4().to_string(),
        centroid_lat: lat,
        centroid_lon: lon,
        radius_m: 300.0,
        activity_count: count,
        name: None,
        name_source: None,
        version: PLACES_VERSION,
        computed_at: now,
        created_at: now,
    }
}

async fn give_places(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user: Uuid,
    places: &[AthletePlace],
) {
    repos
        .athlete_places
        .replace_clustering(
            &tenant,
            user,
            places,
            &[],
            &PlaceRun {
                rows_seen: 0,
                newest_synced_at: None,
                version: PLACES_VERSION,
                computed_at: Timestamp::now(),
                home_declared_at_seen: None,
                newest_track_at_seen: None,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn no_location_reads_the_top_place_rounded_and_its_daylight_there() {
    let resources = common::create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (user, tenant) = seed_user(&resources.agent.database).await;
    let top = place(45.507_49, -73.572_51, 42);
    give_places(repos, tenant, user, &[place(45.3, -73.9, 3), top.clone()]).await;
    // The athlete names it, as name_place does: a clustering keeps the
    // athlete's name but never writes one.
    repos
        .athlete_places
        .set_name(&tenant, user, &top.id, Some("Mont-Royal loop"))
        .await
        .unwrap()
        .expect("bug: the athlete's place is there to name");

    let chosen = default_place(repos, tenant, user)
        .await
        .unwrap()
        .expect("bug: an athlete with places has a default one");
    assert_eq!(
        chosen.place.id, top.id,
        "bug: the most frequent place is the default"
    );
    assert_eq!(
        chosen.place.name.as_deref(),
        Some("Mont-Royal loop"),
        "bug: an athlete-named place is served by its name"
    );

    let provider = RecordingProvider {
        asked: Mutex::new(Vec::new()),
    };
    let args = json!({ "timestamp": "2026-11-21T14:00:00Z" });
    let answer = forecast_at_place(&provider, &args, &Zone::utc(), chosen)
        .await
        .unwrap();
    assert!(
        !answer.is_error,
        "bug: the forecast must answer: {:#}",
        answer.content
    );
    let asked = provider.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "bug: one forecast lookup");
    assert_eq!(
        (asked[0].latitude.to_bits(), asked[0].longitude.to_bits()),
        (45.51_f64.to_bits(), (-73.57_f64).to_bits()),
        "bug: the vendor must receive the centroid rounded to 0.01°, never the point"
    );
    assert_eq!(
        answer.content["default_place"]["id"],
        json!(top.id),
        "bug: the answer names the place used"
    );
    assert_eq!(
        answer.content["latitude"],
        json!(45.51),
        "bug: the answer echoes the rounded latitude"
    );

    let Daylight::Day { sunset, .. } =
        daylight(45.51, -73.57, solar_date(asked[0].timestamp, -73.57))
    else {
        panic!("bug: Montréal has a sunset in November");
    };
    assert_eq!(
        answer.content["daylight"]["sunset"],
        json!(sunset.to_rfc3339()),
        "bug: the daylight must be computed at the rounded pair the vendor was asked for"
    );
}

#[tokio::test]
async fn coordinates_the_caller_gives_are_sent_as_given() {
    let provider = RecordingProvider {
        asked: Mutex::new(Vec::new()),
    };
    let args = json!({
        "latitude": 45.507_49,
        "longitude": -73.572_51,
        "timestamp": "2026-11-21T14:00:00Z",
    });
    let answer = forecast(&provider, &NoGeocoder, &args, &Zone::utc())
        .await
        .unwrap();
    assert!(
        !answer.is_error,
        "bug: the forecast must answer: {:#}",
        answer.content
    );
    let asked = provider.asked.lock().unwrap().clone();
    assert_eq!(
        (asked[0].latitude.to_bits(), asked[0].longitude.to_bits()),
        (45.507_49_f64.to_bits(), (-73.572_51_f64).to_bits()),
        "bug: a location the caller gave is sent unrounded, as today"
    );
    assert!(
        answer.content.get("default_place").is_none(),
        "bug: a given location uses no default place"
    );
}

#[tokio::test]
async fn an_athlete_with_no_places_has_no_default_and_is_refused() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user, tenant) = seed_user(&resources.agent.database).await;
    assert!(
        default_place(&resources.common.repos, tenant, user)
            .await
            .unwrap()
            .is_none(),
        "bug: no places means no default"
    );
    let provider = RecordingProvider {
        asked: Mutex::new(Vec::new()),
    };
    let refused = forecast(
        &provider,
        &NoGeocoder,
        &json!({ "timestamp": "2026-11-21T14:00:00Z" }),
        &Zone::utc(),
    )
    .await
    .unwrap();
    assert!(
        refused.is_error,
        "bug: no location and no default must be refused"
    );
    assert!(
        provider.asked.lock().unwrap().is_empty(),
        "bug: a refusal asks the vendor nothing"
    );
}
