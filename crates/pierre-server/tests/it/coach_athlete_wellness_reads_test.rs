// ABOUTME: A coach's reads of a coached athlete's wellness: sleep, recovery, health, heart rate, nutrition targets
// ABOUTME: Read in the coach's thread or with athlete= over MCP, under the athlete's coach_wellness_read only

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1387 Phase 3. Phases 1 and 2 let a coach's agent read a coached
//! athlete's training; their wellness — sleep, HRV, recovery and readiness,
//! resting heart rate, body weight — stayed out. It opens here under the
//! athlete's second grant, `coach_wellness_read`, in the coach's thread and
//! with `athlete=` over MCP or A2A alike: a coach's own AI client needs no
//! consent of its own (carnet#1424), and each provider's transport terms
//! still decide which rows leave Dravr.
//!
//! Without the grant every wellness read is refused, saying the athlete has
//! not shared it and where they would — never that it is missing — and a
//! training read keeps their resting heart rate and weight out, saying the
//! same. With it, both read the athlete, never the coach.
//!
//! The suite reuses Phase 1's fixture (the athlete's twelve uploaded runs,
//! the coach's ride, the coached group and the coach's thread) and files a
//! week of wellness for each of them, told apart by its numbers.

use chrono::{Duration, Utc};
use dravr_riviere::{DataPoint, SeriesType};
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::models::{
    DataSource, DeviceType, SportType, StoredHealthMetrics, StoredRecoveryMetrics,
    StoredSleepSession, UserPhysiologicalProfile,
};
use pierre_database::repositories::WellnessWriteMode;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_services::parq;
use pierre_tool_runtime::read_subject::DataClass;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::coach_athlete_reads_test::{
    fixture, mentions, ok, person, post_mcp, refused_reason, Fixture, Person, ATHLETE_STREAM_RUN,
    UPLOAD,
};
use crate::common;

/// One person's wellness, each number distinct from the other person's.
#[derive(Debug, Clone, Copy)]
struct Wellness {
    sleep_seconds: u32,
    hrv: f64,
    resting_hr: u16,
    weight_kg: f64,
    heart_rate: [f64; 2],
    hrv_series: [f64; 2],
}

const ATHLETE_WELLNESS: Wellness = Wellness {
    sleep_seconds: 25_200,
    hrv: 68.4,
    resting_hr: 46,
    weight_kg: 61.5,
    heart_rate: [52.0, 57.0],
    hrv_series: [61.25, 66.75],
};

const COACH_WELLNESS: Wellness = Wellness {
    sleep_seconds: 18_360,
    hrv: 31.7,
    resting_hr: 59,
    weight_kg: 83.2,
    heart_rate: [71.0, 77.0],
    hrv_series: [29.25, 33.75],
};

/// The athlete's FTP, which only a read of the athlete returns.
const ATHLETE_FTP: u32 = 248;

/// Nights of sleep filed for each person: enough for a trend.
pub(crate) const NIGHTS: i64 = 8;

/// Where the athlete shares their wellness, as every refusal says it.
const GRANT_WHERE: &str = "Group info → What your human coach can do";

/// What a refusal without the grant says about the athlete.
const NOT_SHARED: &str = "Mira Subject has not shared their wellness";

/// Every wellness read, with arguments that read the athlete.
fn wellness_reads() -> Vec<(&'static str, Value)> {
    let activities = json!({ "activity_provider": UPLOAD });
    let nutrition = json!({
        "weight_kg": 60.0, "height_cm": 168.0, "age": 31, "gender": "female",
        "activity_level": "very_active", "training_goal": "endurance_performance"
    });
    let timing =
        json!({ "workout_intensity": "high", "weight_kg": 60.0, "daily_protein_g": 110.0 });
    vec![
        ("get_sleep_sessions", json!({})),
        ("get_recovery_metrics", json!({})),
        ("get_health_snapshots", json!({})),
        ("get_heart_rate_series", json!({ "metric": "hrv_rmssd" })),
        ("analyze_sleep_quality", json!({})),
        ("calculate_recovery_score", activities.clone()),
        ("suggest_rest_day", activities.clone()),
        ("track_sleep_trends", json!({})),
        ("optimize_sleep_schedule", activities),
        ("calculate_daily_nutrition", nutrition),
        ("get_nutrient_timing", timing),
    ]
}

/// The wellness reads an MCP or A2A caller is offered: all but the
/// prescriptive nutrition tools, which run only on Dravr's own surfaces
/// (carnet#1565).
fn external_wellness_reads() -> Vec<(&'static str, Value)> {
    wellness_reads()
        .into_iter()
        .filter(|(tool, _)| !matches!(*tool, "calculate_daily_nutrition" | "get_nutrient_timing"))
        .collect()
}

/// `who`'s physiology: training numbers with their wellness anchors.
fn physiology(who: Person, wellness: Wellness, ftp: u32) -> UserPhysiologicalProfile {
    let mut profile = UserPhysiologicalProfile::new(who.id, SportType::Run);
    profile.ftp_watts = Some(ftp);
    profile.vo2_max = Some(55.0);
    profile.max_hr = Some(190);
    profile.age = Some(31);
    profile.resting_hr = Some(wellness.resting_hr);
    profile.weight = Some(wellness.weight_kg);
    profile
}

/// A week of `who`'s wellness, synced from Intervals.icu: a night's sleep
/// each night, today's recovery and weigh-in, and an hour of heart rate.
async fn file_wellness(f: &Fixture, who: Person, wellness: Wellness) {
    let repos = f.repos();
    let source = repos
        .data_sources
        .upsert_data_source(
            &who.tenant,
            &DataSource {
                id: String::new(),
                user_id: who.id.to_string(),
                provider: INTERVALS_ICU.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Band,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    let now = Utc::now();
    for night in 0..NIGHTS {
        let woke = now - Duration::days(night) - Duration::minutes(5);
        repos
            .sleep
            .upsert_sleep_session(
                &who.tenant,
                &StoredSleepSession {
                    id: format!("icu-sleep-{}", Uuid::new_v4()),
                    user_id: who.id.to_string(),
                    data_source_id: source.clone(),
                    is_nap: false,
                    start_datetime: woke - Duration::seconds(i64::from(wellness.sleep_seconds)),
                    end_datetime: woke,
                    total_sleep_seconds: Some(wellness.sleep_seconds),
                    deep_sleep_seconds: None,
                    light_sleep_seconds: None,
                    rem_sleep_seconds: None,
                    awake_seconds: None,
                    sleep_efficiency: None,
                    avg_heart_rate: None,
                    min_heart_rate: None,
                    avg_hrv: Some(wellness.hrv),
                    sleep_score: None,
                    avg_spo2: None,
                    stages: Vec::new(),
                    source_name: INTERVALS_ICU.to_owned(),
                },
                now,
            )
            .await
            .unwrap();
    }
    let today = now.date_naive();
    repos
        .recovery
        .upsert_recovery_metrics(
            &who.tenant,
            &StoredRecoveryMetrics {
                id: format!("icu-day-{}", Uuid::new_v4()),
                user_id: who.id.to_string(),
                data_source_id: source.clone(),
                date: today,
                recovery_score: None,
                readiness_score: None,
                hrv_ms: Some(wellness.hrv),
                hrv_rmssd: Some(wellness.hrv),
                resting_heart_rate: Some(u32::from(wellness.resting_hr)),
                stress_score: None,
                body_battery: None,
                spo2: None,
                respiratory_rate: None,
                skin_temp_deviation: None,
                daily_strain: None,
                athlete_note: None,
                source_name: INTERVALS_ICU.to_owned(),
                recorded_at: now,
            },
            WellnessWriteMode::FullDay,
            now,
        )
        .await
        .unwrap();
    repos
        .health_snapshots
        .upsert_health_snapshot(
            &who.tenant,
            &StoredHealthMetrics {
                id: String::new(),
                user_id: who.id.to_string(),
                data_source_id: source.clone(),
                date: today,
                weight_kg: Some(wellness.weight_kg),
                body_fat_pct: None,
                muscle_mass_kg: None,
                bmi: None,
                bone_mass_kg: None,
                water_pct: None,
                systolic_bp: None,
                diastolic_bp: None,
                blood_glucose: None,
                vo2_max: None,
                source_name: INTERVALS_ICU.to_owned(),
                recorded_at: now,
            },
            WellnessWriteMode::FullDay,
            now,
        )
        .await
        .unwrap();
    let [first, second] = wellness.heart_rate;
    let [rmssd_first, rmssd_second] = wellness.hrv_series;
    repos
        .time_series_points
        .insert_batch(
            &source,
            SeriesType::HrvRmssd.id(),
            vec![
                DataPoint::new(now - Duration::hours(2), rmssd_first),
                DataPoint::new(now - Duration::hours(1), rmssd_second),
            ],
        )
        .await
        .unwrap();
    repos
        .time_series_points
        .insert_batch(
            &source,
            SeriesType::HeartRate.id(),
            vec![
                DataPoint::new(now - Duration::hours(2), first),
                DataPoint::new(now - Duration::hours(1), second),
            ],
        )
        .await
        .unwrap();
}

/// Phase 1's fixture, with each person's wellness and physiology filed.
pub(crate) async fn world() -> Fixture {
    let f = fixture().await;
    for (who, wellness, ftp) in [
        (f.athlete, ATHLETE_WELLNESS, ATHLETE_FTP),
        (f.coach, COACH_WELLNESS, 301),
    ] {
        file_wellness(&f, who, wellness).await;
        f.repos()
            .user_physiological_profile
            .upsert_user_physiological_profile(who.tenant, who.id, &physiology(who, wellness, ftp))
            .await
            .unwrap();
    }
    f
}

impl Fixture {
    /// The athlete grants (`true`) or revokes their wellness to the coach.
    pub(crate) async fn share_wellness(&self, shared: bool) {
        self.repos()
            .groups
            .set_coach_consents(
                &self.group.to_string(),
                self.athlete.id,
                None,
                Some(shared),
                Utc::now(),
            )
            .await
            .unwrap()
            .expect("the athlete is a live member");
    }

    /// What `tool` answers `who` with in their own conversation.
    pub(crate) async fn own_read(&self, who: Person, tool: &str, params: Value) -> Value {
        ok(
            self.call(&self.own_executor(), who, tool, params).await,
            tool,
        )
    }
}

fn at<'payload>(payload: &'payload Value, pointer: &str) -> &'payload Value {
    payload
        .pointer(pointer)
        .unwrap_or_else(|| panic!("{pointer} in {payload}"))
}

/// Whether `payload` carries any of `wellness`'s numbers.
fn carries(payload: &Value, wellness: Wellness) -> bool {
    [
        wellness.sleep_seconds.to_string(),
        wellness.hrv.to_string(),
        wellness.weight_kg.to_string(),
    ]
    .iter()
    .any(|number| mentions_number(payload, number))
}

/// Whether `number` appears in `payload` as a number of its own, not as part
/// of a longer one or of a timestamp. A plain substring search read the
/// coach's HRV `31.7` inside a range end of `03:24:31.708`, so the "never
/// reads the coach" check failed whenever the clock landed there.
fn mentions_number(payload: &Value, number: &str) -> bool {
    let text = payload.to_string();
    let bytes = text.as_bytes();
    text.match_indices(number).any(|(start, _)| {
        let end = start + number.len();
        // A digit or a dot before it continues a longer number; a colon after
        // a digit is a clock's (`03:24:31.708`), while JSON's `"hrv":31.7`
        // puts the colon after a quote.
        let continues_before = match start.checked_sub(1).map(|i| bytes[i]) {
            Some(byte) if byte.is_ascii_digit() || byte == b'.' => true,
            Some(b':') => start
                .checked_sub(2)
                .is_some_and(|i| bytes[i].is_ascii_digit()),
            _ => false,
        };
        // A sentence's full stop ends the number; only a digit, or a dot
        // followed by one, continues it.
        let continues_after = match bytes.get(end) {
            Some(byte) if byte.is_ascii_digit() => true,
            Some(b'.') => bytes.get(end + 1).is_some_and(u8::is_ascii_digit),
            _ => false,
        };
        !continues_before && !continues_after
    })
}

// ── The wellness reads, under the grant ──────────────────────────────────

#[tokio::test]
async fn each_stored_wellness_read_in_the_thread_reads_the_athletes_under_their_grant() {
    let f = world().await;
    f.share_wellness(true).await;

    let sleep = ok(f.in_thread("get_sleep_sessions", json!({})).await, "sleep");
    assert_eq!(
        sleep["count"], NIGHTS,
        "every night of the athlete's is listed: {sleep}"
    );
    assert!(
        mentions(&sleep, &ATHLETE_WELLNESS.sleep_seconds.to_string()),
        "bug: the nights read are the athlete's: {sleep}"
    );

    let recovery = ok(
        f.in_thread("get_recovery_metrics", json!({})).await,
        "recovery",
    );
    assert!(
        mentions(&recovery, &ATHLETE_WELLNESS.hrv.to_string()),
        "bug: the HRV read is the athlete's: {recovery}"
    );

    let health = ok(
        f.in_thread("get_health_snapshots", json!({})).await,
        "health",
    );
    assert!(
        mentions(&health, &ATHLETE_WELLNESS.weight_kg.to_string()),
        "bug: the weigh-in read is the athlete's: {health}"
    );

    let series = ok(
        f.in_thread("get_heart_rate_series", json!({ "metric": "hrv_rmssd" }))
            .await,
        "series",
    );
    assert_eq!(
        series["count"], 2,
        "the athlete's hour of HRV samples: {series}"
    );
    assert!(
        mentions(&series, "66.75") && !mentions(&series, "33.75"),
        "bug: the samples read are the athlete's: {series}"
    );

    for (tool, payload) in [
        ("get_sleep_sessions", &sleep),
        ("get_recovery_metrics", &recovery),
        ("get_health_snapshots", &health),
        ("get_heart_rate_series", &series),
    ] {
        assert!(
            !carries(payload, COACH_WELLNESS),
            "bug: {tool} in the athlete's thread never reads the coach: {payload}"
        );
    }
}

#[tokio::test]
async fn each_derived_wellness_read_in_the_thread_answers_as_the_athletes_own_and_not_the_coachs() {
    let f = world().await;
    f.share_wellness(true).await;
    for (tool, params) in wellness_reads().into_iter().skip(4) {
        if tool.starts_with("calculate_daily") || tool.starts_with("get_nutrient") {
            continue;
        }
        let thread = ok(f.in_thread(tool, params.clone()).await, tool);
        let athletes = f.own_read(f.athlete, tool, params.clone()).await;
        let coachs = f.own_read(f.coach, tool, params).await;
        assert_eq!(
            thread, athletes,
            "bug: {tool} in the thread answers exactly as the athlete's own read"
        );
        assert_ne!(
            thread, coachs,
            "bug: {tool} in the athlete's thread never answers from the coach's wellness"
        );
    }
}

#[tokio::test]
async fn the_nutrition_targets_read_the_athletes_medical_flag_under_their_grant() {
    let f = world().await;
    parq::persist_parq_flags(
        f.repos().memory.as_ref(),
        f.athlete.tenant,
        &f.athlete.id.to_string(),
        &["heart_condition".to_owned()],
    )
    .await
    .unwrap();
    f.share_wellness(true).await;
    for (tool, params) in wellness_reads().into_iter().skip(9) {
        let thread = ok(f.in_thread(tool, params.clone()).await, tool);
        assert!(
            thread.get("figures_withheld").is_some(),
            "bug: {tool} for the flagged athlete withholds their amounts from the coach: {thread}"
        );
        let coachs = f.own_read(f.coach, tool, params).await;
        assert!(
            coachs.get("figures_withheld").is_none(),
            "the coach's own read is under the coach's own (clear) flag: {coachs}"
        );
    }
}

// ── No side effect on the athlete's account ──────────────────────────────

/// A night poor enough, after the athlete's dense block of runs, to score
/// their recovery under the threshold at which their own read notifies them.
#[cfg(feature = "client-notifications")]
fn low_recovery() -> Value {
    json!({
        "activity_provider": UPLOAD,
        "sleep_data": {
            "date": Utc::now().date_naive().to_string(),
            "duration_hours": 0.5,
            "deep_sleep_hours": 0.0,
            "rem_sleep_hours": 0.0,
            "light_sleep_hours": 0.5,
            "awake_hours": 4.0,
            "efficiency_percent": 10.0,
            "hrv_rmssd_ms": 18.0
        },
        "baseline_hrv": 70.0,
        "recent_hrv_values": [66.0, 71.0, 69.0]
    })
}

/// The recovery notifications stored for `who`.
#[cfg(feature = "client-notifications")]
async fn recovery_notices(f: &Fixture, who: Person) -> Vec<Value> {
    use pierre_notifications::TenantId as NotificationTenant;

    let (rows, _total, _unread) = f
        .res
        .common
        .notification_service
        .as_ref()
        .expect("the test server wires its notification service")
        .list_notifications(
            who.id,
            NotificationTenant(who.tenant.as_uuid()),
            10,
            0,
            Some("recovery"),
            false,
        )
        .await
        .unwrap();
    rows.into_iter()
        .map(|row| row.data.unwrap_or_else(|| json!({})))
        .collect()
}

#[cfg(feature = "client-notifications")]
#[tokio::test]
async fn a_coachs_recovery_read_sends_the_athlete_no_notification() {
    use tokio::time;

    let f = world().await;
    f.share_wellness(true).await;

    let read = ok(
        f.in_thread("calculate_recovery_score", low_recovery())
            .await,
        "the coach's read",
    );
    let score = at(&read, "/recovery_score/overall_score").as_f64().unwrap();
    assert!(
        score < 40.0,
        "the night scores under the low-recovery threshold, so the athlete's own read of it \
         notifies them: {read}"
    );

    // The athlete's own read of the same night notifies them. Once its row
    // is stored, the coach's read, dispatched before it, has had every chance
    // to store one too.
    f.own_read(f.athlete, "calculate_recovery_score", low_recovery())
        .await;
    for _ in 0..50 {
        if !recovery_notices(&f, f.athlete).await.is_empty() {
            break;
        }
        time::sleep(time::Duration::from_millis(100)).await;
    }
    time::sleep(time::Duration::from_millis(300)).await;
    let rows = recovery_notices(&f, f.athlete).await;
    assert_eq!(
        rows.len(),
        1,
        "bug: only the athlete's own read notifies them, never their coach's: {rows:?}"
    );
    assert!(
        rows[0].get("screen").is_none() && !mentions(&rows[0], &f.thread),
        "bug: the row is the athlete's own, opening no conversation — never the coach's \
         private thread: {rows:?}"
    );
}

// ── Without the grant ────────────────────────────────────────────────────

#[tokio::test]
async fn each_wellness_read_without_the_grant_says_the_athlete_has_not_shared_it_and_where_they_would(
) {
    let f = world().await;
    for (tool, params) in wellness_reads() {
        let refused = f.in_thread(tool, params).await;
        assert_eq!(
            refused_reason(&refused),
            "wellness_grant_needed",
            "bug: {tool} reads the athlete's wellness only under their grant"
        );
        let message = refused.error.clone().unwrap_or_default();
        assert!(
            message.contains(NOT_SHARED) && message.contains(GRANT_WHERE),
            "bug: {tool}'s refusal says the athlete has not shared it, and where they would: \
             {message}"
        );
        for missing in ["not saved", "no data", "no sleep", "not found"] {
            assert!(
                !message.contains(missing),
                "bug: {tool}'s refusal never reads as an absent value: {message}"
            );
        }
        let result = refused.result.unwrap();
        assert!(
            !carries(&result, ATHLETE_WELLNESS) && !carries(&result, COACH_WELLNESS),
            "bug: nothing was read before {tool} was refused: {result}"
        );
    }
}

#[tokio::test]
async fn a_revoked_grant_refuses_the_next_wellness_read() {
    let f = world().await;
    f.share_wellness(true).await;
    ok(
        f.in_thread("get_sleep_sessions", json!({})).await,
        "granted",
    );
    f.share_wellness(false).await;
    let refused = f.in_thread("get_sleep_sessions", json!({})).await;
    assert_eq!(
        refused_reason(&refused),
        "wellness_grant_needed",
        "bug: a revoke is enforced from the next read"
    );
}

// ── Over MCP and A2A ─────────────────────────────────────────────────────

/// `params` naming the athlete with `athlete=`.
pub(crate) fn naming_mira(mut params: Value) -> Value {
    params["athlete"] = json!("Mira");
    params
}

#[tokio::test]
async fn a_wellness_read_naming_an_athlete_over_mcp_reads_them_under_their_grant_alone() {
    let f = world().await;
    let stranger = person(&f.res, "stranger", "Sam Stranger").await;

    // Without the grant: refused, in the one wording, on this transport too.
    for (tool, params) in external_wellness_reads() {
        let refused = f
            .call(&f.mcp_executor(), f.coach, tool, naming_mira(params))
            .await;
        assert_eq!(
            refused_reason(&refused),
            "wellness_grant_needed",
            "bug: {tool} with athlete= over MCP reads the athlete's wellness only under their grant"
        );
        let message = refused.error.clone().unwrap_or_default();
        assert!(
            message.contains(NOT_SHARED) && message.contains(GRANT_WHERE),
            "bug: {tool}'s refusal says the athlete has not shared it, and where they would: \
             {message}"
        );
        assert!(
            !message.contains("own app only") && !message.contains("external client"),
            "bug: the grant decides on every transport; no refusal names one: {message}"
        );
        assert!(
            !carries(refused.result.as_ref().unwrap(), ATHLETE_WELLNESS),
            "bug: nothing was read before {tool} was refused"
        );
    }

    // Under the grant: the athlete's wellness, never the coach's.
    f.share_wellness(true).await;
    for (tool, params) in external_wellness_reads() {
        let read = ok(
            f.call(
                &f.mcp_executor(),
                f.coach,
                tool,
                naming_mira(params.clone()),
            )
            .await,
            tool,
        );
        assert!(
            !carries(&read, COACH_WELLNESS),
            "bug: {tool} with athlete= over MCP never reads the coach: {read}"
        );
        let refused = f
            .call(&f.mcp_executor(), stranger, tool, naming_mira(params))
            .await;
        assert_eq!(
            refused_reason(&refused),
            "no_match",
            "bug: {tool}: athlete= reaches only an athlete the caller coaches"
        );
    }
    for (tool, number) in [
        (
            "get_sleep_sessions",
            ATHLETE_WELLNESS.sleep_seconds.to_string(),
        ),
        ("get_recovery_metrics", ATHLETE_WELLNESS.hrv.to_string()),
        (
            "get_health_snapshots",
            ATHLETE_WELLNESS.weight_kg.to_string(),
        ),
    ] {
        let read = ok(
            f.call(&f.mcp_executor(), f.coach, tool, naming_mira(json!({})))
                .await,
            tool,
        );
        assert!(
            mentions(&read, &number),
            "bug: {tool} with athlete= over MCP reads the athlete's: {read}"
        );
    }
    // A derived read answers over MCP exactly as the athlete's own read there.
    for (tool, params) in external_wellness_reads().into_iter().skip(4) {
        let coachs = ok(
            f.call(
                &f.mcp_executor(),
                f.coach,
                tool,
                naming_mira(params.clone()),
            )
            .await,
            tool,
        );
        let athletes = ok(
            f.call(&f.mcp_executor(), f.athlete, tool, params).await,
            tool,
        );
        assert_eq!(
            coachs, athletes,
            "bug: {tool} with athlete= over MCP answers as the athlete's own read over MCP"
        );
    }

    // Over MCP a caller still reads their own wellness, with no athlete named.
    let own = ok(
        f.call(&f.mcp_executor(), f.coach, "get_sleep_sessions", json!({}))
            .await,
        "own sleep over MCP",
    );
    assert!(
        mentions(&own, &COACH_WELLNESS.sleep_seconds.to_string())
            && !carries(&own, ATHLETE_WELLNESS),
        "the coach's own read over MCP reads the coach: {own}"
    );
}

/// A WHOOP night's sleep, which WHOOP's terms keep on Dravr's own surfaces:
/// told apart from the Intervals.icu nights by its length.
const WHOOP_NIGHT_SECONDS: u32 = 27_720;

/// Three of the athlete's nights from WHOOP, before the Intervals.icu week so
/// no night merges with one, with their consent to AI use of WHOOP data.
async fn file_whoop_nights(f: &Fixture) {
    let repos = f.repos();
    let athlete = f.athlete;
    let source = repos
        .data_sources
        .upsert_data_source(
            &athlete.tenant,
            &DataSource {
                id: String::new(),
                user_id: athlete.id.to_string(),
                provider: "whoop".to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Band,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    let now = Utc::now();
    for night in NIGHTS + 2..NIGHTS + 5 {
        let woke = now - Duration::days(night);
        repos
            .sleep
            .upsert_sleep_session(
                &athlete.tenant,
                &StoredSleepSession {
                    id: format!("whoop-sleep-{}", Uuid::new_v4()),
                    user_id: athlete.id.to_string(),
                    data_source_id: source.clone(),
                    is_nap: false,
                    start_datetime: woke - Duration::seconds(i64::from(WHOOP_NIGHT_SECONDS)),
                    end_datetime: woke,
                    total_sleep_seconds: Some(WHOOP_NIGHT_SECONDS),
                    deep_sleep_seconds: None,
                    light_sleep_seconds: None,
                    rem_sleep_seconds: None,
                    awake_seconds: None,
                    sleep_efficiency: None,
                    avg_heart_rate: None,
                    min_heart_rate: None,
                    avg_hrv: None,
                    sleep_score: None,
                    avg_spo2: None,
                    stages: Vec::new(),
                    source_name: "whoop".to_owned(),
                },
                now,
            )
            .await
            .unwrap();
    }
    repos
        .users
        .record_provider_terms(
            athlete.id,
            "whoop",
            provider_terms_version("whoop").expect("WHOOP carries a notice"),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_wellness_read_over_mcp_serves_only_the_rows_each_providers_terms_let_leave_dravr() {
    let f = world().await;
    file_whoop_nights(&f).await;
    f.share_wellness(true).await;
    let whoop = WHOOP_NIGHT_SECONDS.to_string();
    let icu = ATHLETE_WELLNESS.sleep_seconds.to_string();

    // In the coach's thread, Dravr's own surface, both providers' nights.
    let thread = ok(
        f.in_thread("get_sleep_sessions", json!({})).await,
        "sleep in the thread",
    );
    assert!(
        mentions(&thread, &whoop) && mentions(&thread, &icu),
        "the coach's thread reads every night the athlete lets AI read: {thread}"
    );

    // Over MCP, Intervals.icu's nights (any transport) and none of WHOOP's.
    let over_mcp = ok(
        f.call(
            &f.mcp_executor(),
            f.coach,
            "get_sleep_sessions",
            naming_mira(json!({})),
        )
        .await,
        "sleep over MCP",
    );
    assert!(
        mentions(&over_mcp, &icu),
        "bug: Intervals.icu's terms let its nights leave Dravr: {over_mcp}"
    );
    assert!(
        !mentions(&over_mcp, &whoop),
        "bug: WHOOP's terms keep its nights on Dravr's own surfaces, for a coach's client too: \
         {over_mcp}"
    );
    assert_eq!(
        over_mcp["count"], NIGHTS,
        "only the Intervals.icu week is served: {over_mcp}"
    );
}

#[tokio::test]
async fn every_wellness_read_is_listed_with_athlete_over_mcp_and_offered_in_the_thread_without_it()
{
    let f = world().await;
    let registry = &f.res.mcp.tool_registry;
    let takes_athlete = |schemas: &[pierre_mcp_schema::ToolSchema], name: &str| {
        schemas
            .iter()
            .find(|schema| schema.name == name)
            .unwrap_or_else(|| panic!("{name} is listed"))
            .input_schema
            .properties
            .as_ref()
            .is_some_and(|properties| properties.contains_key("athlete"))
    };
    let listed = registry.user_visible_schemas();
    let chat = registry.chat_callable_schemas();
    for (tool, _) in wellness_reads() {
        assert!(
            takes_athlete(&listed, tool),
            "bug: {tool} is offered athlete= over MCP and A2A (carnet#1424)"
        );
        assert!(
            !takes_athlete(&chat, tool),
            "bug: chat never offers athlete= on {tool}: the thread's athlete is implicit"
        );
    }

    // tools/list offers athlete= to a coach by the roster tool's rule: coach
    // access and an active group (carnet#1426, carnet#1511).
    f.res
        .common
        .repos
        .users
        .set_manages_roster(f.coach.id, true)
        .await
        .unwrap();
    // The MCP endpoint lists it on every one.
    let app = ProviderToolRouter::build_http_app(&f.res);
    let grant = format!(
        "Bearer {}",
        common::delegated_test_token_for_tenant(
            &f.res,
            f.coach.id,
            Some(f.coach.tenant.to_string())
        )
    );
    let tools_list = post_mcp(&app, &grant, "tools/list", json!({})).await;
    let tools = tools_list["result"]["tools"]
        .as_array()
        .expect("a tool list");
    let over_http: Vec<&str> = tools
        .iter()
        .filter(|tool| tool["inputSchema"]["properties"].get("athlete").is_some())
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for (tool, _) in external_wellness_reads() {
        assert!(
            over_http.contains(&tool),
            "bug: tools/list shows athlete on {tool}: {over_http:?}"
        );
    }

    let mut offered = registry.chat_callable_of_class(DataClass::Wellness);
    offered.sort_unstable();
    let mut expected: Vec<&str> = wellness_reads().into_iter().map(|(tool, _)| tool).collect();
    expected.sort_unstable();
    assert_eq!(
        offered, expected,
        "bug: every wellness read is offered in the thread, and the prompt names it"
    );
}

// ── The athlete's physiology, by the grant ───────────────────────────────

#[tokio::test]
async fn the_athletes_resting_heart_rate_and_weight_reach_the_coach_only_under_the_grant() {
    let f = world().await;
    let rockport = json!({
        "method": "rockport_walk", "gender": "female", "time_seconds": 780, "heart_rate": 132
    });

    // Without the grant: no weight stands in, and the coach is told why.
    let refused = f
        .try_call(
            &f.thread_executor(),
            f.coach,
            "estimate_vo2max",
            rockport.clone(),
        )
        .await;
    let message = match refused {
        Err(error) => error.to_string(),
        Ok(response) => {
            assert!(!response.success, "{:?}", response.result);
            response.error.unwrap_or_default()
        }
    };
    assert!(
        message.contains(NOT_SHARED) && message.contains(GRANT_WHERE),
        "bug: the missing weight says the athlete has not shared it: {message}"
    );
    let nutrition = json!({ "provider": UPLOAD, "recommendation_type": "nutrition" });
    let refused = f
        .in_thread("generate_recommendations", nutrition.clone())
        .await;
    assert_eq!(refused_reason(&refused), "wellness_grant_needed");

    // Under it, the athlete's own numbers, never the coach's.
    f.share_wellness(true).await;
    let vo2 = ok(
        f.in_thread("estimate_vo2max", rockport).await,
        "estimate_vo2max",
    );
    assert!(
        vo2["defaults_from_profile"]
            .as_array()
            .is_some_and(|defaults| defaults.iter().any(|d| d == "weight_kg")),
        "bug: under the grant the athlete's saved weight stands in: {vo2}"
    );
    let advice = ok(
        f.in_thread("generate_recommendations", nutrition).await,
        "nutrition",
    );
    assert_eq!(
        at(&advice, "/macronutrient_targets/body_weight_kg"),
        ATHLETE_WELLNESS.weight_kg,
        "bug: the nutrition mode scales from the athlete's weight: {advice}"
    );
    for (tool, params) in [
        ("analyze_training_load", json!({ "provider": UPLOAD })),
        ("calculate_fitness_score", json!({ "provider": UPLOAD })),
    ] {
        let mut with_sleep = params;
        with_sleep["sleep_provider"] = json!(INTERVALS_ICU);
        ok(f.in_thread(tool, with_sleep).await, tool);
    }
}

/// The heart-rate zones are training (Phil, 2026-11-06): the athlete's saved
/// resting heart rate cuts them on a coach's read with or without the grant,
/// and only its value, echoed among the inputs, waits for the grant.
#[tokio::test]
async fn the_athletes_heart_rate_zones_reach_the_coach_without_the_grant_and_their_resting_rate_only_with_it(
) {
    let f = world().await;
    let zones = || json!({});
    let own = f
        .own_read(f.athlete, "calculate_personalized_zones", zones())
        .await;
    let own_zones = at(&own, "/personalized_zones/heart_rate_zones");
    assert!(own_zones.is_object(), "the athlete's own zones: {own}");

    let without = ok(
        f.in_thread("calculate_personalized_zones", zones()).await,
        "zones without the grant",
    );
    assert_eq!(
        at(&without, "/personalized_zones/heart_rate_zones"),
        own_zones,
        "bug: the athlete's heart-rate zones are training, cut from their saved resting heart \
         rate as on their own read: {without}"
    );
    assert!(
        without["user_profile"]["resting_hr"].is_null(),
        "bug: the resting heart rate's value is wellness, never echoed without the grant: \
         {without}"
    );
    assert_eq!(
        at(&without, "/input_sources/resting_hr"),
        "profile",
        "the zones say where the number that cut them came from: {without}"
    );
    assert!(
        without["unavailable"].get("heart_rate_zones").is_none(),
        "{without}"
    );
    assert_eq!(
        at(&without, "/personalized_zones/ftp"),
        ATHLETE_FTP,
        "the athlete's power zones are training, and stay: {without}"
    );
    assert!(
        without["personalized_zones"]["power_zones"].is_object(),
        "{without}"
    );

    f.share_wellness(true).await;
    let with = ok(
        f.in_thread("calculate_personalized_zones", zones()).await,
        "zones with the grant",
    );
    assert_eq!(
        with["user_profile"]["resting_hr"], ATHLETE_WELLNESS.resting_hr,
        "bug: under the grant the athlete's resting heart rate is echoed: {with}"
    );
    assert_eq!(
        at(&with, "/personalized_zones/heart_rate_zones"),
        own_zones,
        "{with}"
    );

    // Over MCP the zones are training too, and the grant echoes the resting
    // heart rate there as in the thread.
    let over_mcp = ok(
        f.call(
            &f.mcp_executor(),
            f.coach,
            "calculate_personalized_zones",
            json!({ "athlete": "Mira" }),
        )
        .await,
        "zones over MCP",
    );
    assert_eq!(
        at(&over_mcp, "/personalized_zones/heart_rate_zones"),
        own_zones,
        "bug: athlete= over MCP reads the athlete's heart-rate zones: {over_mcp}"
    );
    assert_eq!(
        over_mcp["user_profile"]["resting_hr"], ATHLETE_WELLNESS.resting_hr,
        "bug: a coach's own AI client reads under the same grant: {over_mcp}"
    );
}

#[tokio::test]
async fn the_metrics_heart_rate_reading_reaches_the_coach_only_under_the_grant() {
    let f = world().await;
    let by_id = json!({ "provider": UPLOAD, "activity_id": ATHLETE_STREAM_RUN });

    let without = ok(
        f.in_thread("calculate_metrics", by_id.clone()).await,
        "metrics without the grant",
    );
    assert!(
        without["heart_rate_reading"].is_null(),
        "bug: no reading the resting heart rate solves from: {without}"
    );
    let note = without["heart_rate_unavailable"].as_str().unwrap();
    assert!(
        note.contains(NOT_SHARED) && note.contains(GRANT_WHERE),
        "bug: the note says the athlete has not shared it, and where: {note}"
    );

    f.share_wellness(true).await;
    let with = ok(
        f.in_thread("calculate_metrics", by_id).await,
        "metrics with the grant",
    );
    let heart_rate = at(&with, "/heart_rate_reading/heart_rate")
        .as_f64()
        .unwrap();
    let fraction = at(&with, "/heart_rate_reading/reserve_fraction")
        .as_f64()
        .unwrap();
    let max = at(&with, "/max_hr_used").as_f64().unwrap();
    let resting = fraction.mul_add(-max, heart_rate) / (1.0 - fraction);
    assert!(
        (resting - f64::from(ATHLETE_WELLNESS.resting_hr)).abs() < 0.5,
        "bug: under the grant the reading is cut from the athlete's resting heart rate: {with}"
    );
}

#[tokio::test]
async fn the_athletes_configuration_shows_their_wellness_overrides_only_under_the_grant() {
    let f = world().await;
    // A document saved before measurements were refused in it.
    f.repos()
        .profiles
        .save_configuration(
            &f.athlete.id.to_string(),
            &json!({
                "active_profile": "default",
                "session_overrides": { "resting_hr": 47, "weight_kg": 61.0, "zone_scaling": 1.1 }
            })
            .to_string(),
        )
        .await
        .unwrap();

    let without = ok(
        f.in_thread("get_user_configuration", json!({})).await,
        "configuration without the grant",
    );
    let overrides = at(&without, "/configuration/session_overrides");
    assert_eq!(
        overrides,
        &json!({ "zone_scaling": 1.1 }),
        "bug: the athlete's resting heart rate and weight stay out: {without}"
    );
    assert!(
        at(&without, "/configuration/effective_parameters")
            .get("weight_kg")
            .is_none(),
        "{without}"
    );

    f.share_wellness(true).await;
    let with = ok(
        f.in_thread("get_user_configuration", json!({})).await,
        "configuration with the grant",
    );
    assert_eq!(
        at(&with, "/configuration/session_overrides/resting_hr"),
        47,
        "under the grant the athlete's whole configuration reads: {with}"
    );
}

#[test]
fn a_coach_number_is_found_as_a_number_never_inside_a_timestamp() {
    let range = json!({ "end": "2026-11-13T03:24:31.708009814+00:00" });
    assert!(
        !mentions_number(&range, "31.7"),
        "bug: a timestamp's seconds are not the coach's HRV"
    );
    let longer = json!({ "value": 31.75 });
    assert!(!mentions_number(&longer, "31.7"), "bug: 31.75 is not 31.7");
    let prose = json!({ "text": "Your HRV was 31.7." });
    assert!(
        mentions_number(&prose, "31.7"),
        "bug: a sentence's full stop ends the number"
    );
    let field = json!({ "hrv": 31.7 });
    assert!(
        mentions_number(&field, "31.7"),
        "bug: a JSON number is found"
    );
}
