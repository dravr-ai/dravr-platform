// ABOUTME: Integration tests for Terra provider (150+ wearables unified API)
// ABOUTME: Converters, webhook parsing into records, the store sink, reads from the activity cache, signatures

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Terra Provider Integration Tests
//!
//! - Model converters (Terra JSON to the activity and wellness records)
//! - Webhook parsing into records, and the sink that stores them
//! - The provider's reads, answered from the activity cache
//! - Webhook signature validation

#![cfg(feature = "provider-terra")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
#![allow(missing_docs)]

use std::sync::Arc;

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, TimeZone, Utc};
use pierre_core::models::{MealType, SportType, TenantId};
use pierre_core::pagination::PaginationParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::terra_ingest::{store_terra_records, TerraStored};
use pierre_providers::core::{FitnessProvider, OwnerStores};
use pierre_providers::spi::ProviderDescriptor;
use pierre_providers::terra::models::{
    TerraActivity, TerraCaloriesData, TerraDistanceData, TerraMetadata, TerraUser,
};
use pierre_providers::terra::webhook::{
    SignatureValidation, WebhookResult, WebhookSignatureValidator,
};
use pierre_providers::terra::{
    TerraApiClient, TerraApiConfig, TerraConverters, TerraDescriptor, TerraProvider, TerraRecords,
    TerraWebhookHandler,
};
use ring::hmac;
use secrecy::SecretString;
use uuid::Uuid;

use crate::common;

// ============================================================================
// Helper Functions
// ============================================================================

fn make_test_user() -> TerraUser {
    TerraUser {
        user_id: "test_user_123".to_owned(),
        provider: Some("GARMIN".to_owned()),
        last_webhook_update: None,
        reference_id: Some("ref_456".to_owned()),
    }
}

/// `text`, an RFC 3339 time with its offset, as Terra sends one.
fn at(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

/// A server context and an athlete with a tenant.
async fn athlete() -> (Arc<ServerContext>, (TenantId, Uuid)) {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let email = format!("terra_{}@example.com", Uuid::new_v4());
    let (user_id, _) = common::create_test_user_with_email(&resources.agent.database, &email)
        .await
        .unwrap();
    let tenant = resources
        .common
        .repos
        .tenants
        .get_all()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .expect("the athlete owns a tenant")
        .id;
    let tenant = TenantId::parse_str(&tenant.to_string()).unwrap();
    (resources, (tenant, user_id))
}

/// A Terra provider bound to the athlete's stores, as the auth layer binds it.
fn bound_provider(resources: &ServerContext, (tenant, user_id): (TenantId, Uuid)) -> TerraProvider {
    let repos = &resources.common.repos;
    let provider = TerraProvider::new();
    provider.bind_owner_stores(OwnerStores {
        activity_cache: Arc::clone(&repos.activity_cache),
        activity_streams: Arc::clone(&repos.activity_streams),
        user_id,
        tenant_id: tenant,
    });
    provider
}

/// The records a webhook `payload` parses into.
fn records_of(payload: &serde_json::Value) -> (Option<String>, TerraRecords) {
    match TerraWebhookHandler::new().process(payload.to_string().as_bytes()) {
        WebhookResult::Success {
            relayed_from,
            records,
            ..
        } => (relayed_from, *records),
        other => panic!("bug: a data event must parse into records, got {other:?}"),
    }
}

/// A Terra activity event of `count` runs, one a day, the newest on
/// 2024-01-15, each starting at 08:00 in Montreal (UTC-5).
fn activity_event(count: u32) -> serde_json::Value {
    let data: Vec<serde_json::Value> = (0..count)
        .map(|i| {
            let day = 15 - i;
            serde_json::json!({
                "metadata": {
                    "summary_id": format!("act_{day:03}"),
                    "name": format!("Run {day}"),
                    "start_time": format!("2024-01-{day:02}T08:00:00-05:00"),
                    "end_time": format!("2024-01-{day:02}T09:00:00-05:00"),
                    "type": 1
                },
                "distance_data": { "distance_meters": 5000.0 },
                "calories_data": { "total_burned_calories": 400.0 }
            })
        })
        .collect();
    serde_json::json!({
        "type": "activity",
        "user": { "user_id": "test_user_123", "provider": "GARMIN" },
        "data": data
    })
}

// ============================================================================
// Converter Tests
// ============================================================================

#[test]
fn test_activity_conversion() {
    let terra_activity = TerraActivity {
        metadata: Some(TerraMetadata {
            start_time: Some(at("2024-01-15T08:00:00-05:00")),
            end_time: Some(at("2024-01-15T09:00:00-05:00")),
            activity_type: Some(1), // Running
            name: Some("Morning Run".to_owned()),
            summary_id: Some("act_123".to_owned()),
            city: Some("Montreal".to_owned()),
            country: Some("Canada".to_owned()),
            upload_type: None,
        }),
        distance_data: Some(TerraDistanceData {
            distance_meters: Some(5000.0),
            steps: Some(5500),
            elevation_gain_metres: Some(50.0),
            ..Default::default()
        }),
        calories_data: Some(TerraCaloriesData {
            total_burned_calories: Some(400.0),
            ..Default::default()
        }),
        ..Default::default()
    };

    let user = make_test_user();
    let activity = TerraConverters::activity_from_terra(&terra_activity, &user)
        .expect("a timed activity with a summary id converts");

    assert_eq!(activity.id(), "act_123");
    assert_eq!(activity.name(), "Morning Run");
    assert_eq!(activity.sport_type(), &SportType::Run);
    assert_eq!(activity.distance_meters(), Some(5000.0));
    assert_eq!(activity.elevation_gain(), Some(50.0));
    assert_eq!(activity.calories(), Some(400));
    assert_eq!(activity.steps(), Some(5500));
    assert_eq!(activity.city(), Some("Montreal"));
    assert_eq!(
        activity.duration_seconds(),
        3600,
        "bug: one hour, offset kept"
    );
    assert_eq!(
        activity.start_date(),
        Utc.with_ymd_and_hms(2024, 1, 15, 13, 0, 0).unwrap(),
        "bug: 08:00 at UTC-5 is 13:00 UTC"
    );
    assert_eq!(
        activity.provider(),
        "terra",
        "bug: a Terra copy is cached under the provider key it is read by"
    );
    assert_eq!(
        activity.source(),
        Some("garmin"),
        "bug: the copy names the service Terra relayed it from"
    );
}

#[test]
fn test_sport_type_mapping() {
    assert_eq!(TerraConverters::map_terra_activity_type(1), SportType::Run);
    assert_eq!(TerraConverters::map_terra_activity_type(5), SportType::Ride);
    assert_eq!(
        TerraConverters::map_terra_activity_type(10),
        SportType::Swim
    );
    assert_eq!(
        TerraConverters::map_terra_activity_type(14),
        SportType::Hike
    );
    assert_eq!(
        TerraConverters::map_terra_activity_type(30),
        SportType::StrengthTraining
    );
    assert_eq!(
        TerraConverters::map_terra_activity_type(999),
        SportType::Workout
    );
}

#[test]
fn test_meal_type_parsing() {
    assert_eq!(MealType::from_str_lossy("breakfast"), MealType::Breakfast);
    assert_eq!(MealType::from_str_lossy("LUNCH"), MealType::Lunch);
    assert_eq!(MealType::from_str_lossy("Dinner"), MealType::Dinner);
    assert_eq!(MealType::from_str_lossy("snack"), MealType::Snack);
    assert_eq!(MealType::from_str_lossy("unknown"), MealType::Other);
}

/// A night at UTC+10 ending at 07:00 there is that morning's, and a daily
/// summary is its own local date: Terra's offsets date the records, never
/// the UTC date and never the time they arrived.
#[test]
fn terra_records_take_their_local_dates() {
    let (relayed_from, sleep) = records_of(&serde_json::json!({
        "type": "sleep",
        "user": { "user_id": "terra_u", "provider": "OURA" },
        "data": [{
            "metadata": {
                "start_time": "2024-03-10T23:00:00+10:00",
                "end_time": "2024-03-11T07:00:00+10:00"
            },
            "sleep_durations_data": {
                "asleep_seconds": 25200.0,
                "deep_sleep_seconds": 5400.0,
                "rem_sleep_seconds": 6300.0,
                "light_sleep_seconds": 13500.0,
                "in_bed_seconds": 28800.0
            },
            "heart_rate_data": { "resting_hr_bpm": 48.0, "avg_hrv_rmssd": 72.5 },
            "readiness_data": { "readiness_score": 81.0 }
        }]
    }));
    assert_eq!(
        relayed_from.as_deref(),
        Some("OURA"),
        "bug: the relayed service is named"
    );
    let [night] = sleep.nights.as_slice() else {
        panic!("bug: one night, got {:?}", sleep.nights);
    };
    assert_eq!(night.total_sleep_seconds, Some(25_200), "bug: asleep time");
    assert_eq!(night.deep_sleep_seconds, Some(5_400), "bug: deep sleep");
    assert_eq!(
        night.sleep_efficiency,
        Some(87.5),
        "bug: 7 h asleep in 8 h in bed"
    );
    assert_eq!(
        night.start_datetime,
        Utc.with_ymd_and_hms(2024, 3, 10, 13, 0, 0).unwrap(),
        "bug: 23:00 at UTC+10 is 13:00 UTC"
    );
    let [recovery] = sleep.recovery.as_slice() else {
        panic!("bug: one recovery day, got {:?}", sleep.recovery);
    };
    assert_eq!(
        recovery.date,
        NaiveDate::from_ymd_opt(2024, 3, 11).unwrap(),
        "bug: the night's readings are the local morning's, not the UTC date (10th)"
    );
    assert_eq!(recovery.resting_heart_rate, Some(48), "bug: resting HR");
    assert_eq!(
        recovery.hrv_rmssd,
        Some(72.5),
        "bug: HRV is the rMSSD reading"
    );
    assert_eq!(recovery.readiness_score, Some(81), "bug: readiness");

    let (_, daily) = records_of(&serde_json::json!({
        "type": "daily",
        "user": { "user_id": "terra_u", "provider": "GARMIN" },
        "data": [{
            "metadata": {
                "start_time": "2024-03-11T00:00:00+10:00",
                "end_time": "2024-03-12T00:00:00+10:00"
            },
            "distance_data": { "steps": 11234 },
            "calories_data": { "total_burned_calories": 2650.0, "activity_burned_calories": 820.0 },
            "active_durations_data": {
                "moderate_intensity_seconds": 1800.0,
                "vigorous_intensity_seconds": 1200.0
            },
            "heart_rate_data": { "resting_hr_bpm": 50.0 },
            "stress_data": { "avg_stress_level": 31.0 }
        }]
    }));
    let [day] = daily.daily_activity.as_slice() else {
        panic!("bug: one day of totals, got {:?}", daily.daily_activity);
    };
    assert_eq!(
        day.date,
        NaiveDate::from_ymd_opt(2024, 3, 11).unwrap(),
        "bug: a day starting at local midnight is its own date, not the UTC date (10th)"
    );
    assert_eq!(day.steps, Some(11_234), "bug: steps");
    assert_eq!(day.active_kcal, Some(820.0), "bug: active energy");
    assert_eq!(day.total_kcal, Some(2_650.0), "bug: total energy");
    assert_eq!(
        day.intensity_minutes,
        Some(50),
        "bug: 30 moderate + 20 vigorous minutes"
    );
    let [day_recovery] = daily.recovery.as_slice() else {
        panic!(
            "bug: one recovery day from the daily summary, got {:?}",
            daily.recovery
        );
    };
    assert_eq!(day_recovery.stress_score, Some(31), "bug: stress");
    assert_eq!(day_recovery.resting_heart_rate, Some(50), "bug: resting HR");
}

/// A summary with no time is dropped, never dated by its arrival; one with
/// no reading files nothing; nutrition, which Dravr keeps no store for, is
/// answered as unhandled.
#[test]
fn terra_summaries_without_a_time_or_a_reading_file_nothing() {
    let (_, untimed) = records_of(&serde_json::json!({
        "type": "sleep",
        "user": { "user_id": "terra_u" },
        "data": [{ "sleep_durations_data": { "asleep_seconds": 25200.0 } }]
    }));
    assert!(
        untimed.is_empty(),
        "bug: an untimed night was dated: {untimed:?}"
    );

    let (_, empty) = records_of(&serde_json::json!({
        "type": "body",
        "user": { "user_id": "terra_u" },
        "data": [{ "metadata": { "start_time": "2024-03-11T07:00:00+10:00" } }]
    }));
    assert!(
        empty.is_empty(),
        "bug: a body summary with no measurement filed a day: {empty:?}"
    );

    let result = TerraWebhookHandler::new().process(
        serde_json::json!({
            "type": "nutrition",
            "user": { "user_id": "terra_u" },
            "data": [{ "metadata": { "start_time": "2024-03-11T07:00:00+10:00" } }]
        })
        .to_string()
        .as_bytes(),
    );
    assert!(
        matches!(&result, WebhookResult::Unhandled { event_type } if event_type == "nutrition"),
        "bug: nutrition has no store and is not parsed into records, got {result:?}"
    );
}

/// A Terra workout with no start time is never dated by its arrival, and
/// one with no summary id of Terra's is never given a fresh id that every
/// redelivery would store again: both file nothing.
#[test]
fn terra_activities_without_a_start_or_an_id_file_nothing() {
    for (what, metadata) in [
        (
            "no start time",
            serde_json::json!({ "summary_id": "act_1", "end_time": "2024-01-15T09:00:00-05:00" }),
        ),
        (
            "no summary id",
            serde_json::json!({
                "start_time": "2024-01-15T08:00:00-05:00",
                "end_time": "2024-01-15T09:00:00-05:00"
            }),
        ),
    ] {
        let (_, records) = records_of(&serde_json::json!({
            "type": "activity",
            "user": { "user_id": "terra_u", "provider": "GARMIN" },
            "data": [{ "metadata": metadata, "distance_data": { "distance_meters": 5000.0 } }]
        }));
        assert!(
            records.activities.is_empty(),
            "bug: an activity with {what} was filed: {records:?}"
        );
    }
}

// ============================================================================
// Store sink
// ============================================================================

/// What a Terra event carries lands where every ingress writes: the runs in
/// `cached_activities` under `terra`, which the bound provider reads back,
/// and the night, recovery and daily totals as rows the wellness store
/// filed under a data source naming the relayed service.
#[tokio::test]
async fn a_terra_event_lands_in_the_activity_cache_and_the_wellness_rows() {
    let (resources, owner) = athlete().await;
    let (tenant, user_id) = owner;
    let repos = &resources.common.repos;
    let received = Utc.with_ymd_and_hms(2024, 3, 12, 6, 0, 0).unwrap();

    let (relayed_from, runs) = records_of(&activity_event(2));
    let stored = store_terra_records(repos, owner, relayed_from.as_deref(), &runs, received)
        .await
        .unwrap();
    assert_eq!(
        stored,
        TerraStored {
            activities: 2,
            ..TerraStored::default()
        },
        "bug: two runs, no wellness"
    );

    let (relayed_from, mut wellness) = records_of(&serde_json::json!({
        "type": "sleep",
        "user": { "user_id": "terra_u", "provider": "GARMIN" },
        "data": [{
            "metadata": {
                "start_time": "2024-03-10T23:00:00-05:00",
                "end_time": "2024-03-11T07:00:00-05:00"
            },
            "sleep_durations_data": { "asleep_seconds": 25200.0 },
            "heart_rate_data": { "resting_hr_bpm": 48.0, "avg_hrv_rmssd": 72.5 }
        }]
    }));
    let (_, daily) = records_of(&serde_json::json!({
        "type": "daily",
        "user": { "user_id": "terra_u", "provider": "GARMIN" },
        "data": [{
            "metadata": { "start_time": "2024-03-11T00:00:00-05:00" },
            "distance_data": { "steps": 9000 },
            "stress_data": { "avg_stress_level": 31.0 }
        }]
    }));
    wellness.recovery.extend(daily.recovery);
    wellness.daily_activity.extend(daily.daily_activity);
    let stored = store_terra_records(repos, owner, relayed_from.as_deref(), &wellness, received)
        .await
        .unwrap();
    assert_eq!(
        stored,
        TerraStored {
            activities: 0,
            nights: 1,
            recovery: 2,
            body: 0,
            daily_activity: 1,
        },
        "bug: one night, two partial recovery summaries, one day of totals"
    );

    let cached = repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant,
            Some("terra"),
            DateTime::UNIX_EPOCH,
            Utc::now(),
            10,
        )
        .await
        .unwrap();
    assert_eq!(cached.len(), 2, "bug: both runs are cached under `terra`");

    let window = (
        Utc.with_ymd_and_hms(2024, 3, 9, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2024, 3, 13, 0, 0, 0).unwrap(),
    );
    let nights = repos
        .sleep
        .get_sleep_sessions(user_id, &tenant, window.0, window.1)
        .await
        .unwrap();
    let [night] = nights.as_slice() else {
        panic!("bug: one night stored, got {nights:?}");
    };
    assert_eq!(night.source_name, "terra", "bug: filed under the Terra key");
    let recovery = repos
        .recovery
        .get_recovery_metrics(user_id, &tenant, window.0, window.1)
        .await
        .unwrap();
    let [day] = recovery.as_slice() else {
        panic!("bug: the sleep and daily summaries share one recovery day, got {recovery:?}");
    };
    assert_eq!(
        day.date,
        NaiveDate::from_ymd_opt(2024, 3, 11).unwrap(),
        "bug: the local date"
    );
    assert_eq!(
        day.resting_heart_rate,
        Some(48),
        "bug: the sleep summary's resting HR stays"
    );
    assert_eq!(
        day.stress_score,
        Some(31),
        "bug: the daily summary's stress joins it"
    );
    assert_eq!(
        day.data_source_id, night.data_source_id,
        "bug: one data source for the connection's rows"
    );
    let sources = repos
        .data_sources
        .list_data_sources(user_id, &tenant)
        .await
        .unwrap();
    let source = sources
        .iter()
        .find(|ds| ds.id == day.data_source_id)
        .expect("the data source is recorded");
    assert_eq!(
        source.original_source_name.as_deref(),
        Some("garmin"),
        "bug: the data source names the service Terra relayed from"
    );
    let days = repos
        .daily_activity
        .get_daily_activity(user_id, &tenant, window.0, window.1)
        .await
        .unwrap();
    assert_eq!(
        days.iter().map(|d| d.steps).collect::<Vec<_>>(),
        [Some(9_000)],
        "bug: the day's totals are stored"
    );
}

/// Each service Terra relays from keeps its own data source: a WHOOP night
/// stays WHOOP's after a Garmin event for the same athlete, rather than
/// taking the last relay's name.
#[tokio::test]
async fn each_relayed_service_keeps_its_own_data_source() {
    let (resources, owner) = athlete().await;
    let (tenant, user_id) = owner;
    let repos = &resources.common.repos;
    let received = Utc.with_ymd_and_hms(2024, 3, 12, 6, 0, 0).unwrap();
    let night_from = |provider: &str, day: u32| {
        serde_json::json!({
            "type": "sleep",
            "user": { "user_id": "terra_u", "provider": provider },
            "data": [{
                "metadata": {
                    "start_time": format!("2024-03-{:02}T23:00:00-05:00", day - 1),
                    "end_time": format!("2024-03-{day:02}T07:00:00-05:00")
                },
                "sleep_durations_data": { "asleep_seconds": 25200.0 }
            }]
        })
    };
    for (provider, day) in [("WHOOP", 10), ("GARMIN", 11)] {
        let (relayed_from, records) = records_of(&night_from(provider, day));
        store_terra_records(repos, owner, relayed_from.as_deref(), &records, received)
            .await
            .unwrap();
    }

    let nights = repos
        .sleep
        .get_sleep_sessions(
            user_id,
            &tenant,
            Utc.with_ymd_and_hms(2024, 3, 8, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 3, 13, 0, 0, 0).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(nights.len(), 2, "bug: two nights stored, got {nights:?}");
    let sources = repos
        .data_sources
        .list_data_sources(user_id, &tenant)
        .await
        .unwrap();
    let relay_of = |night_end_day: u32| {
        let night = nights
            .iter()
            .find(|n| n.end_datetime.day() == night_end_day)
            .expect("the night is stored");
        sources
            .iter()
            .find(|ds| ds.id == night.data_source_id)
            .and_then(|ds| ds.original_source_name.clone())
    };
    assert_eq!(
        relay_of(10).as_deref(),
        Some("whoop"),
        "bug: the WHOOP night took the later relay's name"
    );
    assert_eq!(
        relay_of(11).as_deref(),
        Some("garmin"),
        "bug: the Garmin night names the service it was relayed from"
    );
}

// ============================================================================
// Provider Tests
// ============================================================================

#[tokio::test]
async fn test_provider_name() {
    let provider = TerraProvider::new();
    assert_eq!(provider.name(), "terra");
}

#[tokio::test]
async fn test_authentication_flow() {
    let provider = TerraProvider::new();

    assert!(!provider.is_authenticated().await);

    provider.set_terra_user_id("test_user_123").await;
    assert!(provider.is_authenticated().await);
}

#[tokio::test]
async fn test_descriptor() {
    let descriptor = TerraDescriptor;
    assert_eq!(descriptor.name(), "terra");
    assert_eq!(descriptor.display_name(), "Terra");
    // Sleep and recovery are not advertised (registre#513).
    assert!(!descriptor.capabilities().supports_sleep());
    assert!(!descriptor.capabilities().supports_recovery());
    assert!(!descriptor.capabilities().supports_health());
    assert!(descriptor.capabilities().supports_activities());
    assert!(
        descriptor.capabilities().is_push_only(),
        "bug: Terra pushes; its reads answer from the stored rows"
    );
}

/// A read before the connection's stores are bound is refused: Terra has
/// no read to fall back on, and nothing is held in memory.
#[tokio::test]
async fn an_unbound_terra_provider_refuses_to_read() {
    let provider = TerraProvider::new();
    provider.set_terra_user_id("test_user").await;
    assert!(
        provider.get_activities(None, None).await.is_err(),
        "bug: an unbound provider answered a read"
    );
}

#[tokio::test]
async fn test_get_activities_empty() {
    let (resources, owner) = athlete().await;
    let provider = bound_provider(&resources, owner);

    let activities = provider.get_activities(None, None).await.unwrap();
    assert!(activities.is_empty(), "bug: nothing was pushed");
}

/// The provider's list, single, cursor and stats reads all answer from the
/// rows a Terra event stored.
#[tokio::test]
async fn the_terra_provider_reads_what_terra_pushed() {
    let (resources, owner) = athlete().await;
    let (_, runs) = records_of(&activity_event(5));
    store_terra_records(
        &resources.common.repos,
        owner,
        Some("GARMIN"),
        &runs,
        Utc::now(),
    )
    .await
    .unwrap();
    let provider = bound_provider(&resources, owner);

    let listed = provider.get_activities(Some(10), None).await.unwrap();
    assert_eq!(
        listed.iter().map(|a| a.id().to_owned()).collect::<Vec<_>>(),
        ["act_015", "act_014", "act_013", "act_012", "act_011"],
        "bug: newest first"
    );
    assert_eq!(
        provider.get_activity("act_013").await.unwrap().name(),
        "Run 13",
        "bug: one activity by id"
    );
    assert!(
        provider.get_activity("act_999").await.is_err(),
        "bug: an activity never pushed is not found"
    );

    let first = provider
        .get_activities_cursor(&PaginationParams::forward(None, 2))
        .await
        .unwrap();
    assert_eq!(first.items.len(), 2, "bug: a page of two");
    assert!(first.has_more, "bug: three more");
    let second = provider
        .get_activities_cursor(&PaginationParams::forward(first.next_cursor, 2))
        .await
        .unwrap();
    assert_eq!(
        second
            .items
            .iter()
            .map(|a| a.id().to_owned())
            .collect::<Vec<_>>(),
        ["act_013", "act_012"],
        "bug: the next page follows the first"
    );

    let stats = provider.get_stats().await.unwrap();
    assert_eq!(stats.total_activities, 5, "bug: five runs");
    assert!(
        (stats.total_distance - 25_000.0).abs() < f64::EPSILON,
        "bug: 5 x 5 km, got {}",
        stats.total_distance
    );
    assert_eq!(stats.total_duration, 5 * 3600, "bug: five hours");
}

// ============================================================================
// Webhook Tests
// ============================================================================

#[test]
fn test_webhook_handler_auth_event() {
    let handler = TerraWebhookHandler::new();

    let payload = serde_json::json!({
        "type": "auth",
        "status": "success",
        "user": {
            "user_id": "test_user_123",
            "provider": "GARMIN",
            "reference_id": "my_user_456"
        }
    });

    let result = handler.process(payload.to_string().as_bytes());

    assert!(
        matches!(
            &result,
            WebhookResult::AuthEvent {
                event_type,
                status,
                user_id,
                reference_id,
            } if event_type == "auth"
                && status == "success"
                && *user_id == Some("test_user_123".to_owned())
                && *reference_id == Some("my_user_456".to_owned())
        ),
        "Expected AuthEvent with correct values, got: {result:?}"
    );
}

#[test]
fn test_webhook_handler_activity_event() {
    let result = TerraWebhookHandler::new().process(activity_event(1).to_string().as_bytes());

    let WebhookResult::Success {
        event_type,
        user_id,
        relayed_from,
        records,
    } = result
    else {
        panic!("bug: an activity event parses, got {result:?}");
    };
    assert_eq!(event_type, "activity", "bug: the event type");
    assert_eq!(user_id, "test_user_123", "bug: the Terra user");
    assert_eq!(
        relayed_from.as_deref(),
        Some("GARMIN"),
        "bug: the relayed service"
    );
    assert_eq!(records.len(), 1, "bug: one record");
    assert_eq!(records.activities[0].name(), "Run 15", "bug: the run");
}

#[test]
fn test_webhook_handler_unhandled_event() {
    let handler = TerraWebhookHandler::new();

    let payload = serde_json::json!({
        "type": "unknown_event",
        "user": {
            "user_id": "test_user_123"
        }
    });

    let result = handler.process(payload.to_string().as_bytes());

    assert!(
        matches!(
            &result,
            WebhookResult::Unhandled { event_type } if event_type == "unknown_event"
        ),
        "Expected Unhandled result with unknown_event, got: {result:?}"
    );
}

/// Signing secret shared by the signature validation tests
const WEBHOOK_SECRET: &str = "whsec_test_secret";

/// Raw JSON body shared by the signature validation tests
const WEBHOOK_BODY: &[u8] = br#"{"type":"activity","user":{"user_id":"terra_user_1"}}"#;

/// Timestamp the signature validation tests sign with
const WEBHOOK_TIMESTAMP: &str = "1723808700";

/// Hex HMAC-SHA256 of `message` keyed by `secret`
fn hmac_sha256_hex(secret: &str, message: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    hex::encode(hmac::sign(&key, message).as_ref())
}

/// Terra's `v1` signature: HMAC-SHA256 of `"{timestamp}.{body}"`
fn terra_v1_signature(secret: &str, timestamp: &str, body: &[u8]) -> String {
    let mut signed_payload = format!("{timestamp}.").into_bytes();
    signed_payload.extend_from_slice(body);
    hmac_sha256_hex(secret, &signed_payload)
}

#[test]
fn test_signature_validation_valid() {
    let signature = terra_v1_signature(WEBHOOK_SECRET, WEBHOOK_TIMESTAMP, WEBHOOK_BODY);
    let validator = WebhookSignatureValidator::new(WEBHOOK_SECRET.to_owned());
    let header = format!("t={WEBHOOK_TIMESTAMP},v1={signature}");

    assert_eq!(
        validator.validate(Some(&header), WEBHOOK_BODY),
        SignatureValidation::Valid
    );
}

/// Anyone can sign under the empty key, so a validator holding no secret
/// accepts nothing — not even a signature correctly computed with it.
#[test]
fn test_signature_validation_rejects_everything_under_an_empty_secret() {
    let signature = terra_v1_signature("", WEBHOOK_TIMESTAMP, WEBHOOK_BODY);
    let validator = WebhookSignatureValidator::new(String::new());
    let header = format!("t={WEBHOOK_TIMESTAMP},v1={signature}");

    assert_eq!(
        validator.validate(Some(&header), WEBHOOK_BODY),
        SignatureValidation::Invalid
    );
}

#[test]
fn test_signature_validation_known_vector() {
    // Computed independently of ring with:
    // printf '%s' '1723808700.{"type":"activity","user":{"user_id":"terra_user_1"}}' \
    //   | openssl dgst -sha256 -hmac 'whsec_test_secret'
    let header = "t=1723808700,v1=954bc40fa3da266bcd04eccf25cb512d7ac6cc9f6eec3d9fcb2cef6bdb6aa6a0";
    let validator = WebhookSignatureValidator::new(WEBHOOK_SECRET.to_owned());

    assert_eq!(
        validator.validate(Some(header), WEBHOOK_BODY),
        SignatureValidation::Valid
    );
}

#[test]
fn test_signature_validation_rejects_body_only_signature() {
    let body_only = hmac_sha256_hex(WEBHOOK_SECRET, WEBHOOK_BODY);
    // The same body-only HMAC computed by openssl, pinning what is being rejected
    assert_eq!(
        body_only,
        "fe5a9fccd59883bfd5394e4d2d8f7b35980783203829f3217c6f55ef98f4153e"
    );
    let validator = WebhookSignatureValidator::new(WEBHOOK_SECRET.to_owned());
    let header = format!("t={WEBHOOK_TIMESTAMP},v1={body_only}");

    assert_eq!(
        validator.validate(Some(&header), WEBHOOK_BODY),
        SignatureValidation::Invalid
    );
}

#[test]
fn test_signature_validation_rejects_signature_over_other_timestamp() {
    let signature = terra_v1_signature(WEBHOOK_SECRET, "1723808699", WEBHOOK_BODY);
    let validator = WebhookSignatureValidator::new(WEBHOOK_SECRET.to_owned());
    let header = format!("t={WEBHOOK_TIMESTAMP},v1={signature}");

    assert_eq!(
        validator.validate(Some(&header), WEBHOOK_BODY),
        SignatureValidation::Invalid
    );
}

#[test]
fn test_signature_validation_accepts_second_matching_v1() {
    let stale = terra_v1_signature("whsec_rotated_out", WEBHOOK_TIMESTAMP, WEBHOOK_BODY);
    let current = terra_v1_signature(WEBHOOK_SECRET, WEBHOOK_TIMESTAMP, WEBHOOK_BODY);
    assert_ne!(stale, current);
    let validator = WebhookSignatureValidator::new(WEBHOOK_SECRET.to_owned());
    let header = format!("t={WEBHOOK_TIMESTAMP},v1={stale},v1={current}");

    assert_eq!(
        validator.validate(Some(&header), WEBHOOK_BODY),
        SignatureValidation::Valid
    );
}

#[test]
fn test_signature_validation_ignores_non_v1_schemes() {
    // A correct signature under `v0` must not stand in for a wrong `v1`
    let signature = terra_v1_signature(WEBHOOK_SECRET, WEBHOOK_TIMESTAMP, WEBHOOK_BODY);
    let wrong = terra_v1_signature(WEBHOOK_SECRET, WEBHOOK_TIMESTAMP, b"{}");
    let validator = WebhookSignatureValidator::new(WEBHOOK_SECRET.to_owned());
    let header = format!("t={WEBHOOK_TIMESTAMP},v1={wrong},v0={signature}");

    assert_eq!(
        validator.validate(Some(&header), WEBHOOK_BODY),
        SignatureValidation::Invalid
    );
}

#[test]
fn test_signature_validation_trims_whitespace_between_elements() {
    let signature = terra_v1_signature(WEBHOOK_SECRET, WEBHOOK_TIMESTAMP, WEBHOOK_BODY);
    let validator = WebhookSignatureValidator::new(WEBHOOK_SECRET.to_owned());
    let header = format!("t={WEBHOOK_TIMESTAMP}, v1={signature}");

    assert_eq!(
        validator.validate(Some(&header), WEBHOOK_BODY),
        SignatureValidation::Valid
    );
}

#[test]
fn test_signature_validation_missing_timestamp_is_invalid() {
    let signature = terra_v1_signature(WEBHOOK_SECRET, "", WEBHOOK_BODY);
    let validator = WebhookSignatureValidator::new(WEBHOOK_SECRET.to_owned());

    assert_eq!(
        validator.validate(Some(&format!("v1={signature}")), WEBHOOK_BODY),
        SignatureValidation::Invalid
    );
    assert_eq!(
        validator.validate(Some(&format!("t=,v1={signature}")), WEBHOOK_BODY),
        SignatureValidation::Invalid
    );
}

#[test]
fn test_signature_validation_missing_v1_is_invalid() {
    let signature = terra_v1_signature(WEBHOOK_SECRET, WEBHOOK_TIMESTAMP, WEBHOOK_BODY);
    let validator = WebhookSignatureValidator::new(WEBHOOK_SECRET.to_owned());

    assert_eq!(
        validator.validate(Some(&format!("t={WEBHOOK_TIMESTAMP}")), WEBHOOK_BODY),
        SignatureValidation::Invalid
    );
    assert_eq!(
        validator.validate(
            Some(&format!("t={WEBHOOK_TIMESTAMP},v0={signature}")),
            WEBHOOK_BODY
        ),
        SignatureValidation::Invalid
    );
}

#[test]
fn test_signature_validation_missing() {
    let validator = WebhookSignatureValidator::new("test_secret".to_owned());
    let result = validator.validate(None, b"test body");
    assert_eq!(result, SignatureValidation::Missing);
}

#[test]
fn test_signature_validation_invalid_format() {
    let validator = WebhookSignatureValidator::new("test_secret".to_owned());
    let result = validator.validate(Some("invalid_header"), b"test body");
    assert_eq!(result, SignatureValidation::Invalid);
}

#[test]
fn test_signature_validation_wrong_signature() {
    let validator = WebhookSignatureValidator::new("test_secret".to_owned());
    let header = "t=1234567890,v1=wrong_signature";
    let result = validator.validate(Some(header), b"test body");
    assert_eq!(result, SignatureValidation::Invalid);
}

// ============================================================================
// API Client Tests
// ============================================================================

#[test]
fn test_api_config_default() {
    use std::time::Duration as StdDuration;

    let config = TerraApiConfig::default();
    assert_eq!(config.base_url, "https://api.tryterra.co/v2");
    assert_eq!(config.timeout, StdDuration::from_secs(30));
}

#[test]
fn test_api_client_creation() {
    use std::mem::size_of_val;

    let config = TerraApiConfig {
        api_key: SecretString::from("test_key"),
        dev_id: "test_dev".to_owned(),
        ..Default::default()
    };
    // Debug names the dev id and prints a redaction in place of the API key.
    let printed = format!("{config:?}");
    assert!(!printed.contains("test_key"), "{printed}");
    assert!(
        printed.contains("api_key: SecretBox<str>([REDACTED])"),
        "{printed}"
    );
    assert!(printed.contains("test_dev"), "{printed}");
    // Verify client creation succeeds
    let client = TerraApiClient::new(config);
    // Assert client has non-zero size (struct was instantiated)
    assert!(size_of_val(&client) > 0);
}
