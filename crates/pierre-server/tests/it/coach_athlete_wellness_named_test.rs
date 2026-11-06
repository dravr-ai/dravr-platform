// ABOUTME: A coach's read of a coached athlete's stored wellness carries only what their grant names
// ABOUTME: Sleep, HRV, resting heart rate, weight, readiness and daily notes; the athlete's own read is whole

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1387 Phase 3, decision Q5 (Phil, 2026-11-06): the wellness consent
//! names the athlete's sleep, HRV, resting heart rate, weight, readiness and
//! daily notes, and a coach's read under it returns only those — in the
//! coach's thread and with `athlete=` over MCP alike. The athlete's sources
//! record more (blood pressure, glucose, body composition, the day's activity
//! totals, intraday heart rate...), which stays on the athlete's own read.
//!
//! The suite reuses the wellness suite's world (Phase 1's fixture with each
//! person's week of wellness) and files the athlete's unnamed health beside it.
//!
//! Each stored row is cut down before the sources merge, so a source that
//! recorded only unnamed metrics never stands for a coach's day: the merged
//! record carries the id, source and time of a source that recorded
//! something named.

use chrono::{Duration, Utc};
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::models::{
    DataSource, DeviceType, StoredDailyActivity, StoredHealthMetrics, StoredRecoveryMetrics,
    StoredSleepSession,
};
use pierre_database::repositories::WellnessWriteMode;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::coach_athlete_reads_test::{mentions, ok, refused_reason, Fixture};
use crate::coach_athlete_wellness_reads_test::{naming_mira, world, NIGHTS};

/// What the coach's read says the grant covers.
const GRANT_NAMES: &str = "Mira Subject shares their sleep, HRV, resting heart rate, weight, \
     readiness and daily notes with you, and nothing else of their health";

/// The weigh-in Garmin filed three days ago, beside Intervals.icu's blood
/// pressure and glucose for the same day.
const GARMIN_WEIGHT: f64 = 60.75;

/// The athlete's health beyond what their grant names, from Intervals.icu:
/// yesterday's full body reading and recovery vitals, a day with only their
/// blood pressure, today's activity totals, and a night with its blood oxygen;
/// today, from Garmin, a blood pressure reading beside their weigh-in; and
/// three days ago a Garmin weigh-in, the day's primary, with Intervals.icu's
/// blood pressure and glucose merged into it.
async fn file_unnamed_health(f: &Fixture) {
    let repos = f.repos();
    let athlete = f.athlete;
    let data_source = |provider: &str| DataSource {
        id: String::new(),
        user_id: athlete.id.to_string(),
        provider: provider.to_owned(),
        device_model: None,
        software_version: None,
        source: None,
        device_type: DeviceType::Band,
        original_source_name: None,
    };
    let icu = repos
        .data_sources
        .upsert_data_source(&athlete.tenant, &data_source(INTERVALS_ICU))
        .await
        .unwrap();
    let garmin = repos
        .data_sources
        .upsert_data_source(&athlete.tenant, &data_source("garmin"))
        .await
        .unwrap();
    let now = Utc::now();
    let today = now.date_naive();
    let body = |source: &str, source_name: &str, days_ago: i64| StoredHealthMetrics {
        id: String::new(),
        user_id: athlete.id.to_string(),
        data_source_id: source.to_owned(),
        date: today - Duration::days(days_ago),
        weight_kg: None,
        body_fat_pct: None,
        muscle_mass_kg: None,
        bmi: None,
        bone_mass_kg: None,
        water_pct: None,
        systolic_bp: None,
        diastolic_bp: None,
        blood_glucose: None,
        vo2_max: None,
        source_name: source_name.to_owned(),
        recorded_at: now,
    };
    for day in [
        StoredHealthMetrics {
            weight_kg: Some(61.25),
            body_fat_pct: Some(17.4),
            muscle_mass_kg: Some(48.1),
            bmi: Some(21.3),
            bone_mass_kg: Some(2.6),
            water_pct: Some(58.2),
            systolic_bp: Some(118),
            diastolic_bp: Some(76),
            blood_glucose: Some(92.5),
            vo2_max: Some(54.5),
            ..body(&icu, INTERVALS_ICU, 1)
        },
        StoredHealthMetrics {
            systolic_bp: Some(121),
            diastolic_bp: Some(79),
            ..body(&icu, INTERVALS_ICU, 2)
        },
        StoredHealthMetrics {
            systolic_bp: Some(131),
            diastolic_bp: Some(84),
            body_fat_pct: Some(17.9),
            ..body(&garmin, "garmin", 0)
        },
        StoredHealthMetrics {
            weight_kg: Some(GARMIN_WEIGHT),
            ..body(&garmin, "garmin", 3)
        },
        StoredHealthMetrics {
            systolic_bp: Some(124),
            diastolic_bp: Some(81),
            blood_glucose: Some(95.5),
            ..body(&icu, INTERVALS_ICU, 3)
        },
    ] {
        repos
            .health_snapshots
            .upsert_health_snapshot(&athlete.tenant, &day, WellnessWriteMode::FullDay, now)
            .await
            .unwrap();
    }
    repos
        .daily_activity
        .upsert_daily_activity(
            &athlete.tenant,
            &StoredDailyActivity {
                id: String::new(),
                user_id: athlete.id.to_string(),
                data_source_id: icu.clone(),
                date: today,
                steps: Some(12_345),
                active_kcal: Some(640.5),
                total_kcal: Some(2_301.5),
                intensity_minutes: Some(55),
                source_name: INTERVALS_ICU.to_owned(),
                recorded_at: now,
            },
            WellnessWriteMode::FullDay,
            now,
        )
        .await
        .unwrap();
    repos
        .recovery
        .upsert_recovery_metrics(
            &athlete.tenant,
            &StoredRecoveryMetrics {
                id: format!("icu-day-{}", Uuid::new_v4()),
                user_id: athlete.id.to_string(),
                data_source_id: icu.clone(),
                date: today - Duration::days(1),
                recovery_score: None,
                readiness_score: Some(68),
                hrv_ms: None,
                hrv_rmssd: Some(64.5),
                resting_heart_rate: Some(47),
                stress_score: Some(22),
                body_battery: Some(81),
                spo2: Some(97.5),
                respiratory_rate: Some(14.5),
                skin_temp_deviation: Some(0.3),
                daily_strain: Some(12.5),
                athlete_note: Some("legs heavy after the long run".to_owned()),
                source_name: INTERVALS_ICU.to_owned(),
                recorded_at: now,
            },
            WellnessWriteMode::FullDay,
            now,
        )
        .await
        .unwrap();
    let woke = now - Duration::days(NIGHTS) - Duration::minutes(5);
    repos
        .sleep
        .upsert_sleep_session(
            &athlete.tenant,
            &StoredSleepSession {
                id: format!("icu-sleep-{}", Uuid::new_v4()),
                user_id: athlete.id.to_string(),
                data_source_id: icu,
                is_nap: false,
                start_datetime: woke - Duration::hours(7),
                end_datetime: woke,
                total_sleep_seconds: Some(25_100),
                deep_sleep_seconds: Some(5_400),
                light_sleep_seconds: None,
                rem_sleep_seconds: None,
                awake_seconds: None,
                sleep_efficiency: Some(93.5),
                avg_heart_rate: Some(52.5),
                min_heart_rate: Some(45),
                avg_hrv: Some(70.5),
                sleep_score: None,
                avg_spo2: Some(96.5),
                stages: Vec::new(),
                source_name: INTERVALS_ICU.to_owned(),
            },
            now,
        )
        .await
        .unwrap();
}

/// The rows of `payload[key]`.
fn rows<'payload>(payload: &'payload Value, key: &str) -> &'payload Vec<Value> {
    payload[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} in {payload}"))
}

/// The row of `payload[key]` whose `date` (or night) is `days_ago` days back.
fn row_on<'payload>(payload: &'payload Value, key: &str, days_ago: i64) -> &'payload Value {
    let date = (Utc::now().date_naive() - Duration::days(days_ago)).to_string();
    rows(payload, key)
        .iter()
        .find(|row| row["date"] == date.as_str())
        .unwrap_or_else(|| panic!("{key} on {date} in {payload}"))
}

/// Whether every row of `payload[key]` leaves each of `unnamed` out, its
/// `filled` included.
fn leaves_out(payload: &Value, key: &str, unnamed: &[&str]) -> bool {
    rows(payload, key).iter().all(|row| {
        unnamed.iter().all(|field| row[*field].is_null())
            && row["filled"].as_array().is_none_or(|filled| {
                filled
                    .iter()
                    .all(|metric| !unnamed.iter().any(|field| metric["metric"] == *field))
            })
    })
}

const UNNAMED_BODY: [&str; 9] = [
    "body_fat_pct",
    "muscle_mass_kg",
    "bmi",
    "bone_mass_kg",
    "water_pct",
    "systolic_bp",
    "diastolic_bp",
    "blood_glucose",
    "vo2_max",
];

const UNNAMED_RECOVERY: [&str; 6] = [
    "stress_score",
    "body_battery",
    "spo2",
    "respiratory_rate",
    "skin_temp_deviation",
    "daily_strain",
];

#[tokio::test]
async fn a_coachs_health_snapshot_read_carries_what_the_consent_names_and_the_athletes_own_carries_all(
) {
    let f = world().await;
    file_unnamed_health(&f).await;
    f.share_wellness(true).await;

    let thread = ok(
        f.in_thread("get_health_snapshots", json!({})).await,
        "snapshots in the thread",
    );
    let over_mcp = ok(
        f.call(
            &f.mcp_executor(),
            f.coach,
            "get_health_snapshots",
            naming_mira(json!({})),
        )
        .await,
        "snapshots over MCP",
    );
    for (label, coachs) in [("thread", &thread), ("mcp", &over_mcp)] {
        assert!(
            leaves_out(coachs, "snapshots", &UNNAMED_BODY),
            "bug ({label}): a coach's read carries no body composition, blood pressure, glucose \
             or VO2max estimate: {coachs}"
        );
        assert!(
            rows(coachs, "snapshots")
                .iter()
                .all(|day| day["weight_kg"].is_number()),
            "bug ({label}): a day with nothing named, the blood-pressure-only day, is left out: \
             {coachs}"
        );
        assert_eq!(
            row_on(coachs, "snapshots", 1)["weight_kg"],
            61.25,
            "({label}) the weigh-in is named: {coachs}"
        );
        assert!(
            rows(coachs, "daily_activity").is_empty(),
            "bug ({label}): a coach's read carries none of the athlete's daily activity totals: \
             {coachs}"
        );
        assert!(
            mentions(&coachs["not_shared"], GRANT_NAMES)
                && mentions(&coachs["not_shared"], "blood pressure"),
            "bug ({label}): the read says what is shared and that nothing else is read: {coachs}"
        );
    }
    // Three days ago Garmin's weigh-in is the day's primary and
    // Intervals.icu filled its blood pressure and glucose in: on the coach's
    // read those fills go, and with them the source that gave nothing named.
    let merged_day = row_on(&thread, "snapshots", 3);
    assert_eq!(
        (
            &merged_day["weight_kg"],
            &merged_day["sources"],
            &merged_day["filled"]
        ),
        (&json!(GARMIN_WEIGHT), &json!(["garmin"]), &json!([])),
        "bug: a source that gave only unnamed metrics is not named, nor are its fills: \
         {merged_day}"
    );

    let own = f
        .own_read(f.athlete, "get_health_snapshots", json!({}))
        .await;
    let own_merged_day = row_on(&own, "snapshots", 3);
    assert_eq!(
        (
            &own_merged_day["weight_kg"],
            &own_merged_day["systolic_bp"],
            &own_merged_day["sources"]
        ),
        (
            &json!(GARMIN_WEIGHT),
            &json!(124),
            &json!(["garmin", INTERVALS_ICU])
        ),
        "the athlete's own read carries the day whole, both sources named: {own_merged_day}"
    );
    let yesterday = row_on(&own, "snapshots", 1);
    assert_eq!(
        (
            &yesterday["systolic_bp"],
            &yesterday["blood_glucose"],
            &yesterday["body_fat_pct"],
            &yesterday["vo2_max"],
        ),
        (&json!(118), &json!(92.5), &json!(17.4), &json!(54.5)),
        "the athlete's own read carries all of their health: {own}"
    );
    assert_eq!(row_on(&own, "snapshots", 2)["systolic_bp"], 121, "{own}");
    assert_eq!(rows(&own, "daily_activity")[0]["steps"], 12_345, "{own}");
    assert!(
        own.get("not_shared").is_none(),
        "the athlete's own read says nothing is left out: {own}"
    );
}

/// The day Garmin, the day's higher-ranked source, recorded only unnamed
/// metrics and Intervals.icu something named.
const PROBE_DAY: i64 = 5;

/// The day both sources recorded only unnamed metrics.
const UNNAMED_ONLY_DAY: i64 = 6;

/// Intervals.icu's weigh-in on [`PROBE_DAY`].
const ICU_WEIGHT: f64 = 60.25;

/// Intervals.icu's resting heart rate on [`PROBE_DAY`].
const ICU_RESTING_HR: u32 = 49;

/// What [`file_probe_days`] filed on [`PROBE_DAY`], as the store holds it.
struct Probe {
    garmin_body: StoredHealthMetrics,
    icu_body: StoredHealthMetrics,
    garmin_morning: StoredRecoveryMetrics,
    icu_morning: StoredRecoveryMetrics,
}

/// On [`PROBE_DAY`], Garmin's blood pressure and body fat beside
/// Intervals.icu's weigh-in, and Garmin's stress and body battery beside
/// Intervals.icu's resting heart rate; on [`UNNAMED_ONLY_DAY`], Garmin's
/// blood pressure beside Intervals.icu's glucose, and Garmin's blood oxygen
/// beside Intervals.icu's stress. Garmin synced hours after Intervals.icu.
async fn file_probe_days(f: &Fixture) -> Probe {
    let repos = f.repos();
    let athlete = f.athlete;
    let data_source = |provider: &str| DataSource {
        id: String::new(),
        user_id: athlete.id.to_string(),
        provider: provider.to_owned(),
        device_model: None,
        software_version: None,
        source: None,
        device_type: DeviceType::Band,
        original_source_name: None,
    };
    let mut source_ids = Vec::new();
    for provider in ["garmin", INTERVALS_ICU] {
        let id = repos
            .data_sources
            .upsert_data_source(&athlete.tenant, &data_source(provider))
            .await
            .unwrap();
        source_ids.push((provider, id));
    }
    let now = Utc::now();
    let today = now.date_naive();
    let synced = |provider: &str| {
        if provider == "garmin" {
            now - Duration::hours(2)
        } else {
            now - Duration::hours(6)
        }
    };
    let body = |(provider, id): &(&str, String), days_ago: i64| StoredHealthMetrics {
        id: String::new(),
        user_id: athlete.id.to_string(),
        data_source_id: id.clone(),
        date: today - Duration::days(days_ago),
        weight_kg: None,
        body_fat_pct: None,
        muscle_mass_kg: None,
        bmi: None,
        bone_mass_kg: None,
        water_pct: None,
        systolic_bp: None,
        diastolic_bp: None,
        blood_glucose: None,
        vo2_max: None,
        source_name: (*provider).to_owned(),
        recorded_at: synced(provider),
    };
    let morning = |(provider, id): &(&str, String), days_ago: i64| StoredRecoveryMetrics {
        id: format!("{provider}-probe-{days_ago}"),
        user_id: athlete.id.to_string(),
        data_source_id: id.clone(),
        date: today - Duration::days(days_ago),
        recovery_score: None,
        readiness_score: None,
        hrv_ms: None,
        hrv_rmssd: None,
        resting_heart_rate: None,
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: (*provider).to_owned(),
        recorded_at: synced(provider),
    };
    let (garmin, icu) = (&source_ids[0], &source_ids[1]);
    for day in [
        StoredHealthMetrics {
            systolic_bp: Some(127),
            diastolic_bp: Some(83),
            body_fat_pct: Some(18.1),
            ..body(garmin, PROBE_DAY)
        },
        StoredHealthMetrics {
            weight_kg: Some(ICU_WEIGHT),
            ..body(icu, PROBE_DAY)
        },
        StoredHealthMetrics {
            systolic_bp: Some(119),
            diastolic_bp: Some(77),
            ..body(garmin, UNNAMED_ONLY_DAY)
        },
        StoredHealthMetrics {
            blood_glucose: Some(90.5),
            ..body(icu, UNNAMED_ONLY_DAY)
        },
    ] {
        let mode = WellnessWriteMode::FullDay;
        repos
            .health_snapshots
            .upsert_health_snapshot(&athlete.tenant, &day, mode, day.recorded_at)
            .await
            .unwrap();
    }
    for day in [
        StoredRecoveryMetrics {
            stress_score: Some(31),
            body_battery: Some(64),
            ..morning(garmin, PROBE_DAY)
        },
        StoredRecoveryMetrics {
            resting_heart_rate: Some(ICU_RESTING_HR),
            ..morning(icu, PROBE_DAY)
        },
        StoredRecoveryMetrics {
            spo2: Some(96.0),
            ..morning(garmin, UNNAMED_ONLY_DAY)
        },
        StoredRecoveryMetrics {
            stress_score: Some(27),
            ..morning(icu, UNNAMED_ONLY_DAY)
        },
    ] {
        let mode = WellnessWriteMode::FullDay;
        repos
            .recovery
            .upsert_recovery_metrics(&athlete.tenant, &day, mode, day.recorded_at)
            .await
            .unwrap();
    }

    let window = (now - Duration::days(30), now + Duration::days(1));
    let stored_body = repos
        .health_snapshots
        .get_health_snapshots(athlete.id, &athlete.tenant, window.0, window.1)
        .await
        .unwrap();
    let stored_mornings = repos
        .recovery
        .get_recovery_metrics(athlete.id, &athlete.tenant, window.0, window.1)
        .await
        .unwrap();
    let probe_date = today - Duration::days(PROBE_DAY);
    let body_of = |provider: &str| {
        stored_body
            .iter()
            .find(|day| day.source_name == provider && day.date == probe_date)
            .cloned()
            .unwrap_or_else(|| panic!("{provider}'s probe body day"))
    };
    let morning_of = |provider: &str| {
        stored_mornings
            .iter()
            .find(|day| day.source_name == provider && day.date == probe_date)
            .cloned()
            .unwrap_or_else(|| panic!("{provider}'s probe morning"))
    };
    let probe = Probe {
        garmin_body: body_of("garmin"),
        icu_body: body_of(INTERVALS_ICU),
        garmin_morning: morning_of("garmin"),
        icu_morning: morning_of(INTERVALS_ICU),
    };
    assert!(
        probe.garmin_body.recorded_at != probe.icu_body.recorded_at
            && probe.garmin_morning.recorded_at != probe.icu_morning.recorded_at,
        "the two sources' rows are told apart by when they were recorded"
    );
    probe
}

/// Whether no row of `payload[key]` is dated `days_ago` days back.
fn no_row_on(payload: &Value, key: &str, days_ago: i64) -> bool {
    let date = (Utc::now().date_naive() - Duration::days(days_ago)).to_string();
    rows(payload, key)
        .iter()
        .all(|row| row["date"] != date.as_str())
}

/// The fields of a merged row that say which stored row stands for its day.
fn standing(row: &Value) -> Value {
    json!({
        "source_name": row["source_name"],
        "id": row["id"],
        "data_source_id": row["data_source_id"],
        "recorded_at": row["recorded_at"],
        "sources": row["sources"],
        "filled": row["filled"],
    })
}

#[tokio::test]
async fn a_source_that_recorded_only_unnamed_metrics_never_stands_for_a_coachs_day() {
    let f = world().await;
    let probe = file_probe_days(&f).await;
    f.share_wellness(true).await;

    let icu_stands = |id: &str, data_source_id: &str, recorded_at: Value| {
        json!({
            "source_name": INTERVALS_ICU,
            "id": id,
            "data_source_id": data_source_id,
            "recorded_at": recorded_at,
            "sources": [INTERVALS_ICU],
            "filled": [],
        })
    };
    let body_stands = icu_stands(
        &probe.icu_body.id,
        &probe.icu_body.data_source_id,
        json!(probe.icu_body.recorded_at),
    );
    let morning_stands = icu_stands(
        &probe.icu_morning.id,
        &probe.icu_morning.data_source_id,
        json!(probe.icu_morning.recorded_at),
    );
    for (label, executor, params) in [
        ("thread", f.thread_executor(), json!({})),
        ("mcp", f.mcp_executor(), naming_mira(json!({}))),
    ] {
        let read = |tool: &'static str| f.call(&executor, f.coach, tool, params.clone());
        let snapshots = ok(read("get_health_snapshots").await, label);
        let day = row_on(&snapshots, "snapshots", PROBE_DAY);
        assert_eq!(
            day["weight_kg"], ICU_WEIGHT,
            "({label}) the weigh-in is named: {day}"
        );
        assert_eq!(
            standing(day),
            body_stands,
            "bug ({label}): the day stands on Intervals.icu's weigh-in, never on Garmin's row of \
             unnamed metrics: {day}"
        );
        assert!(
            !day.to_string().to_lowercase().contains("garmin"),
            "bug ({label}): the coach's day carries no trace of Garmin's unnamed reading: {day}"
        );
        assert!(
            no_row_on(&snapshots, "snapshots", UNNAMED_ONLY_DAY),
            "bug ({label}): a day with only unnamed metrics yields no row: {snapshots}"
        );

        let recovery = ok(read("get_recovery_metrics").await, label);
        let morning = row_on(&recovery, "metrics", PROBE_DAY);
        assert_eq!(
            morning["resting_heart_rate"], ICU_RESTING_HR,
            "({label}) the resting heart rate is named: {morning}"
        );
        assert_eq!(
            standing(morning),
            morning_stands,
            "bug ({label}): the morning stands on Intervals.icu's reading, never on Garmin's \
             stress and body battery: {morning}"
        );
        assert!(
            !morning.to_string().to_lowercase().contains("garmin"),
            "bug ({label}): the coach's morning carries no trace of Garmin's reading: {morning}"
        );
        assert!(
            no_row_on(&recovery, "metrics", UNNAMED_ONLY_DAY),
            "bug ({label}): a morning with only unnamed metrics yields no row: {recovery}"
        );
    }

    // The athlete's own read is unchanged: Garmin stands for both days, its
    // unnamed metrics kept, Intervals.icu's filled in, both sources named.
    let own = f
        .own_read(f.athlete, "get_health_snapshots", json!({}))
        .await;
    let own_day = row_on(&own, "snapshots", PROBE_DAY);
    assert_eq!(
        (
            &own_day["id"],
            &own_day["sources"],
            &own_day["systolic_bp"],
            &own_day["body_fat_pct"],
            &own_day["weight_kg"],
        ),
        (
            &json!(probe.garmin_body.id),
            &json!(["garmin", INTERVALS_ICU]),
            &json!(127),
            &json!(18.1),
            &json!(ICU_WEIGHT),
        ),
        "the athlete's own read carries the day whole: {own_day}"
    );
    let own_unnamed = row_on(&own, "snapshots", UNNAMED_ONLY_DAY);
    assert_eq!(
        (&own_unnamed["systolic_bp"], &own_unnamed["blood_glucose"]),
        (&json!(119), &json!(90.5)),
        "the athlete's own read keeps the day the grant names nothing of: {own_unnamed}"
    );
    let own_recovery = f
        .own_read(f.athlete, "get_recovery_metrics", json!({}))
        .await;
    let own_morning = row_on(&own_recovery, "metrics", PROBE_DAY);
    assert_eq!(
        (
            &own_morning["id"],
            &own_morning["sources"],
            &own_morning["stress_score"],
            &own_morning["resting_heart_rate"],
        ),
        (
            &json!(probe.garmin_morning.id),
            &json!(["garmin", INTERVALS_ICU]),
            &json!(31),
            &json!(ICU_RESTING_HR),
        ),
        "the athlete's own read carries the morning whole: {own_morning}"
    );
    assert_eq!(
        row_on(&own_recovery, "metrics", UNNAMED_ONLY_DAY)["spo2"],
        96.0,
        "the athlete's own read keeps the morning the grant names nothing of: {own_recovery}"
    );
}

#[tokio::test]
async fn a_coachs_recovery_and_sleep_reads_carry_only_what_the_consent_names() {
    let f = world().await;
    file_unnamed_health(&f).await;
    f.share_wellness(true).await;

    let recovery = ok(
        f.in_thread("get_recovery_metrics", json!({})).await,
        "recovery",
    );
    assert!(
        leaves_out(&recovery, "metrics", &UNNAMED_RECOVERY),
        "bug: no stress, energy, blood oxygen, breathing, skin temperature or strain: {recovery}"
    );
    let day = row_on(&recovery, "metrics", 1);
    assert_eq!(
        (
            &day["readiness_score"],
            &day["hrv_rmssd"],
            &day["resting_heart_rate"]
        ),
        (&json!(68), &json!(64.5), &json!(47)),
        "readiness, HRV and resting heart rate are named: {day}"
    );
    assert!(
        mentions(&day["athlete_note"], "legs heavy"),
        "the daily note is named: {day}"
    );
    assert!(mentions(&recovery["not_shared"], GRANT_NAMES), "{recovery}");

    let sleep = ok(f.in_thread("get_sleep_sessions", json!({})).await, "sleep");
    assert!(
        rows(&sleep, "sessions")
            .iter()
            .all(|night| night["avg_spo2"].is_null() && night["avg_heart_rate"].is_null()),
        "bug: no night carries its blood oxygen or average heart rate: {sleep}"
    );
    let night = rows(&sleep, "sessions")
        .iter()
        .find(|night| night["total_sleep_seconds"] == 25_100)
        .unwrap_or_else(|| panic!("the night with its vitals: {sleep}"));
    assert_eq!(
        (
            &night["min_heart_rate"],
            &night["avg_hrv"],
            &night["sleep_efficiency"]
        ),
        (&json!(45), &json!(70.5), &json!(93.5)),
        "the night's sleep, HRV and lowest heart rate are named: {night}"
    );

    let own_recovery = f
        .own_read(f.athlete, "get_recovery_metrics", json!({}))
        .await;
    let own_day = row_on(&own_recovery, "metrics", 1);
    assert_eq!(
        (
            &own_day["stress_score"],
            &own_day["spo2"],
            &own_day["daily_strain"]
        ),
        (&json!(22), &json!(97.5), &json!(12.5)),
        "the athlete's own read carries every reading: {own_day}"
    );
    let own_sleep = f.own_read(f.athlete, "get_sleep_sessions", json!({})).await;
    assert!(
        rows(&own_sleep, "sessions")
            .iter()
            .any(|night| night["avg_spo2"] == 96.5 && night["avg_heart_rate"] == 52.5),
        "{own_sleep}"
    );
}

#[tokio::test]
async fn a_coachs_heart_rate_series_read_is_refused_and_their_hrv_series_reads() {
    let f = world().await;
    f.share_wellness(true).await;
    for params in [json!({}), json!({ "metric": "heart_rate" })] {
        for (label, refused) in [
            (
                "thread",
                f.in_thread("get_heart_rate_series", params.clone()).await,
            ),
            (
                "mcp",
                f.call(
                    &f.mcp_executor(),
                    f.coach,
                    "get_heart_rate_series",
                    naming_mira(params.clone()),
                )
                .await,
            ),
        ] {
            assert_eq!(
                refused_reason(&refused),
                "wellness_not_named",
                "bug ({label}): the grant does not name the athlete's intraday heart rate"
            );
            let message = refused.error.clone().unwrap_or_default();
            assert!(
                message.contains(GRANT_NAMES) && message.contains("'hrv_rmssd'"),
                "bug ({label}): the refusal says what is shared and what to ask instead: \
                 {message}"
            );
            assert!(
                !mentions(refused.result.as_ref().unwrap(), "57.0"),
                "bug ({label}): nothing was read before the refusal"
            );
        }
    }
    let hrv = ok(
        f.in_thread("get_heart_rate_series", json!({ "metric": "hrv_rmssd" }))
            .await,
        "hrv series",
    );
    assert!(mentions(&hrv, "66.75"), "the athlete's HRV series: {hrv}");

    let own = f
        .own_read(f.athlete, "get_heart_rate_series", json!({}))
        .await;
    assert!(
        mentions(&own, "57.0"),
        "the athlete's own read carries their heart rate: {own}"
    );
}
