// ABOUTME: A coach's reads of the rest of a coached athlete's training: profile, sources, analyses, goals, physiology
// ABOUTME: Each answers with the athlete's data in the thread and over athlete=; wellness and writes stay off the athlete

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1387 Phase 2. Phase 1 made eight training reads read the athlete
//! of a coach's thread, and `athlete=` over MCP and A2A; the rest of the
//! training class follows here: the athlete's provider profile and totals,
//! their connected sources and how fresh each is, the analyses of their
//! sessions, their goals, and their training physiology.
//!
//! A coach's read of them writes nothing on their account, refuses what would
//! read their wellness (the nutrition mode of `generate_recommendations`),
//! and shows none of their resting heart rate or body weight; the athlete's
//! own read still does.
//!
//! The suite reuses Phase 1's fixture: twelve uploaded runs of the athlete's,
//! one of the coach's, the coached group and the coach's thread.

use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use pierre_cache::{CacheKey, CacheResource};
use pierre_core::models::{
    Athlete, ConnectionType, DataSource, DeviceType, SportType, TimeSeriesData, UserOAuthToken,
    UserPhysiologicalProfile,
};
use pierre_database::repositories::{ActivityStreamsWrite, StreamOrigin};
use pierre_tool_runtime::protocols::ProtocolError;
use pierre_tool_runtime::read_subject::DataClass;
use secrecy::SecretString;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::coach_athlete_reads_test::{
    fixture, mentions, ok, person, refused_reason, Fixture, Person, ATHLETE_RUNS,
    ATHLETE_STREAM_RUN, COACH_RIDE, TRAINING_READS, UPLOAD,
};

/// The athlete's run whose samples carry a GPS track and altitude.
const ROUTE_RUN: &str = "athlete-run-2";

/// The athlete's FTP, which only a read of the athlete returns.
const ATHLETE_FTP: u32 = 248;
/// The coach's own FTP.
const COACH_FTP: u32 = 301;
/// The athlete's saved `VO2max`.
const ATHLETE_VO2MAX: f64 = 57.0;
/// The coach's own `VO2max`.
const COACH_VO2MAX: f64 = 44.0;
/// The athlete's body weight: wellness, never on a coach's read.
const ATHLETE_WEIGHT_KG: f64 = 61.5;
/// The athlete's resting heart rate: wellness, never on a coach's read.
const ATHLETE_RESTING_HR: u16 = 46;

/// A first name for each provider profile, read from the cache it lives in.
const ATHLETE_FIRST_NAME: &str = "Mira";
const COACH_FIRST_NAME: &str = "Cole";

/// A four-stage lactate test in watts.
fn lactate_stages() -> Value {
    json!({
        "unit": "watts",
        "stages": [
            { "intensity": 180, "lactate_mmol": 1.0 },
            { "intensity": 210, "lactate_mmol": 1.3 },
            { "intensity": 240, "lactate_mmol": 2.2 },
            { "intensity": 270, "lactate_mmol": 4.3 },
            { "intensity": 300, "lactate_mmol": 7.1 }
        ]
    })
}

/// `who`'s physiology: their own FTP and `VO2max`, the athlete's wellness
/// anchors with it.
fn physiology(who: Person, ftp: u32, vo2_max: f64, wellness: bool) -> UserPhysiologicalProfile {
    let mut profile = UserPhysiologicalProfile::new(who.id, SportType::Run);
    profile.ftp_watts = Some(ftp);
    profile.vo2_max = Some(vo2_max);
    profile.max_hr = Some(190);
    profile.age = Some(31);
    if wellness {
        profile.resting_hr = Some(ATHLETE_RESTING_HR);
        profile.weight = Some(ATHLETE_WEIGHT_KG);
    }
    profile
}

/// A provider profile cached for `who`, under the upload key their reads use.
fn cached_athlete(first_name: &str) -> Athlete {
    Athlete {
        created_at: None,
        id: "4242".to_owned(),
        username: first_name.to_lowercase(),
        firstname: Some(first_name.to_owned()),
        lastname: None,
        profile_picture: None,
        provider: UPLOAD.to_owned(),
        sex: None,
    }
}

/// An Intervals.icu-shaped token naming `provider`, the row a source's
/// freshness is read from.
fn token(who: Person, provider: &str) -> UserOAuthToken {
    let now = Utc::now();
    UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id: who.id,
        tenant_id: who.tenant.to_string(),
        provider: provider.to_owned(),
        access_token: SecretString::from(format!("{provider}-token")),
        refresh_token: None,
        token_type: "Bearer".to_owned(),
        expires_at: None,
        scope: None,
        provider_user_id: None,
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    }
}

/// `who`'s watch, the row `list_data_sources` reads.
fn device(who: Person, provider: &str, model: &str) -> DataSource {
    DataSource {
        id: String::new(),
        user_id: who.id.to_string(),
        provider: provider.to_owned(),
        device_model: Some(model.to_owned()),
        software_version: None,
        source: None,
        device_type: DeviceType::Watch,
        original_source_name: None,
    }
}

/// A climbing GPS track with altitude, enough for a route summary.
fn route_samples() -> TimeSeriesData {
    TimeSeriesData {
        timestamps: (0..12).collect(),
        heart_rate: None,
        power: None,
        cadence: None,
        speed: None,
        altitude: Some(
            (0_u16..12)
                .map(|i| Some(f32::from(i).mul_add(6.0, 40.0)))
                .collect(),
        ),
        temperature: None,
        gps_coordinates: Some(
            (0_u8..12)
                .map(|i| {
                    let step = f64::from(i);
                    Some((step.mul_add(0.001, 45.50), step.mul_add(0.001, -73.57)))
                })
                .collect(),
        ),
        distance: None,
    }
}

/// Phase 1's fixture, with what the rest of the training class reads:
/// provider profiles in the cache, a token and a watch each, each one's
/// physiology, a goal of the athlete's, a GPS track on one of their runs, and
/// an athlete's connection whose session is gone.
async fn world() -> (Fixture, String) {
    let f = fixture().await;
    let repos = f.repos();
    let cache = f.res.data().cache().clone();
    for (who, name) in [(f.athlete, ATHLETE_FIRST_NAME), (f.coach, COACH_FIRST_NAME)] {
        let key = CacheKey::new(
            who.tenant,
            who.id,
            UPLOAD.to_owned(),
            CacheResource::AthleteProfile,
        );
        cache
            .set(&key, &cached_athlete(name), StdDuration::from_secs(600))
            .await
            .unwrap();
    }
    for (who, provider, model) in [
        (f.athlete, "intervals_icu", "Forerunner 965"),
        (f.coach, "strava", "ELEMNT ROAM"),
    ] {
        repos
            .oauth_tokens
            .upsert_token(&token(who, provider))
            .await
            .unwrap();
        repos
            .data_sources
            .upsert_data_source(&who.tenant, &device(who, provider, model))
            .await
            .unwrap();
    }
    for (who, ftp, vo2, wellness) in [
        (f.athlete, ATHLETE_FTP, ATHLETE_VO2MAX, true),
        (f.coach, COACH_FTP, COACH_VO2MAX, false),
    ] {
        repos
            .user_physiological_profile
            .upsert_user_physiological_profile(
                who.tenant,
                who.id,
                &physiology(who, ftp, vo2, wellness),
            )
            .await
            .unwrap();
        // `predict_performance` is an enterprise tool, and a read of the
        // athlete honours both tenants' switches.
        repos
            .tenants
            .set_plan(who.tenant, "enterprise")
            .await
            .unwrap();
    }
    // The athlete's Garmin, whose session is gone: elected to answer a
    // whole window, it needs the athlete to reconnect it.
    repos
        .provider_connections
        .register_connection(
            f.athlete.id,
            f.athlete.tenant,
            "garmin",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    repos
        .activity_streams
        .upsert_activity_streams(&ActivityStreamsWrite {
            tenant_id: &f.athlete.tenant,
            user_id: f.athlete.id,
            provider: UPLOAD,
            activity_id: ROUTE_RUN,
            origin: StreamOrigin::Upload,
            source_ref: None,
            streams: &route_samples(),
            synced_at: Utc::now(),
        })
        .await
        .unwrap();
    let goal = repos
        .profiles
        .create_goal(
            f.athlete.id,
            json!({
                "goal_type": "distance",
                "target_value": 200.0,
                "timeframe": "month",
                "title": "200 km this month",
                "sport": "Run",
                "created_at": (Utc::now() - Duration::days(20)).to_rfc3339(),
            }),
        )
        .await
        .unwrap();
    (f, goal)
}

impl Fixture {
    /// What `tool` answers the coach with in their own direct conversation.
    async fn own(&self, tool: &str, params: Value) -> Value {
        ok(
            self.call(&self.own_executor(), self.coach, tool, params)
                .await,
            tool,
        )
    }

    /// What `tool` answers the athlete with in their own conversation.
    async fn athletes_own(&self, tool: &str, params: Value) -> Value {
        ok(
            self.call(&self.own_executor(), self.athlete, tool, params)
                .await,
            tool,
        )
    }
}

fn at<'payload>(payload: &'payload Value, pointer: &str) -> &'payload Value {
    payload
        .pointer(pointer)
        .unwrap_or_else(|| panic!("{pointer} in {payload}"))
}

// ── The athlete's provider profile, totals and sources ───────────────────

#[tokio::test]
async fn the_athletes_provider_profile_and_totals_read_in_the_thread() {
    let (f, _) = world().await;
    let profile = ok(
        f.in_thread("get_athlete", json!({ "provider": UPLOAD }))
            .await,
        "get_athlete",
    );
    assert_eq!(
        at(&profile, "/athlete/firstname"),
        ATHLETE_FIRST_NAME,
        "bug: the thread reads the athlete's provider profile: {profile}"
    );
    let own = f.own("get_athlete", json!({ "provider": UPLOAD })).await;
    assert_eq!(
        at(&own, "/athlete/firstname"),
        COACH_FIRST_NAME,
        "the coach's own call reads their own profile: {own}"
    );

    let stats = ok(
        f.in_thread("get_stats", json!({ "provider": UPLOAD }))
            .await,
        "get_stats",
    );
    assert_eq!(
        at(&stats, "/stats/total_activities"),
        ATHLETE_RUNS,
        "bug: the totals are the athlete's twelve runs, never the coach's one: {stats}"
    );
    let own = f.own("get_stats", json!({ "provider": UPLOAD })).await;
    assert_eq!(
        at(&own, "/stats/total_activities"),
        1,
        "the coach's own totals count their one ride: {own}"
    );
}

#[tokio::test]
async fn the_athletes_sources_and_their_freshness_read_in_the_thread() {
    let (f, _) = world().await;
    let sources = ok(
        f.in_thread("list_data_sources", json!({})).await,
        "list_data_sources",
    );
    assert!(
        mentions(&sources, "Forerunner 965") && !mentions(&sources, "ELEMNT ROAM"),
        "bug: the thread lists the athlete's watch, never the coach's: {sources}"
    );
    let freshness = ok(
        f.in_thread("get_data_freshness", json!({})).await,
        "get_data_freshness",
    );
    let providers: Vec<&str> = at(&freshness, "/providers")
        .as_array()
        .expect("a providers array")
        .iter()
        .filter_map(|row| row["provider"].as_str())
        .collect();
    assert_eq!(
        providers,
        ["intervals_icu"],
        "bug: freshness describes the athlete's connections, not the coach's: {freshness}"
    );
    assert!(
        !mentions(&freshness, "connect_provider"),
        "nothing offers to connect a provider on the coach's account: {freshness}"
    );

    let own = f.own("get_data_freshness", json!({})).await;
    assert!(
        mentions(&own, "strava") && !mentions(&own, "intervals_icu"),
        "the coach's own call describes their own connections: {own}"
    );
}

// ── The analyses of the athlete's sessions ───────────────────────────────

/// An analysis of the sessions, the arguments it takes, and where its answer
/// counts the sessions it analysed.
struct Counted {
    tool: &'static str,
    params: Value,
    count: &'static str,
}

fn counted_analyses() -> [Counted; 5] {
    [
        Counted {
            tool: "analyze_performance_trends",
            params: json!({ "provider": UPLOAD, "metric": "distance", "timeframe": "month" }),
            count: "/activities_analyzed",
        },
        Counted {
            tool: "predict_performance",
            params: json!({ "provider": UPLOAD }),
            count: "/activities_analyzed",
        },
        Counted {
            tool: "suggest_goals",
            params: json!({ "provider": UPLOAD }),
            count: "/activities_analyzed",
        },
        Counted {
            tool: "analyze_goal_feasibility",
            params: json!({
                "provider": UPLOAD,
                "goal_type": "distance",
                "target_value": 150.0,
                "timeframe_days": 30
            }),
            count: "/historical_context/activities_analyzed",
        },
        Counted {
            tool: "generate_recommendations",
            params: json!({ "provider": UPLOAD }),
            count: "/training_summary/activities_analyzed",
        },
    ]
}

#[tokio::test]
async fn each_analysis_in_the_thread_analyses_the_athletes_sessions_and_none_of_the_coachs() {
    let (f, _) = world().await;
    for analysis in counted_analyses() {
        let thread = ok(
            f.in_thread(analysis.tool, analysis.params.clone()).await,
            analysis.tool,
        );
        // The coach has one session: more than one analysed can only be the
        // athlete's twelve, or as many of them as the tool reads.
        let analysed = at(&thread, analysis.count).as_i64().unwrap_or_default();
        assert!(
            (2..=ATHLETE_RUNS).contains(&analysed),
            "bug: {} analyses the athlete's runs in the thread, not the coach's one: {thread}",
            analysis.tool
        );
        let own = f.own(analysis.tool, analysis.params).await;
        assert!(
            own.pointer(analysis.count)
                .and_then(Value::as_i64)
                .is_none_or(|count| count <= 1),
            "{}: the coach's own call analyses their own one ride: {own}",
            analysis.tool
        );
    }

    let patterns = ok(
        f.in_thread("detect_patterns", json!({ "provider": UPLOAD }))
            .await,
        "detect_patterns",
    );
    assert!(
        patterns["preferred_training_days"].is_array(),
        "bug: twelve runs give the athlete's weekly schedule: {patterns}"
    );
    let own = f
        .own("detect_patterns", json!({ "provider": UPLOAD }))
        .await;
    assert_eq!(
        own["activities_analyzed"], 1,
        "the coach's one ride is too few for a pattern: {own}"
    );
}

#[tokio::test]
async fn the_athletes_session_reads_by_id_answer_in_the_thread_and_not_from_the_coachs_seat() {
    let (f, _) = world().await;
    let by_id = json!({ "provider": UPLOAD, "activity_id": ATHLETE_STREAM_RUN });

    let metrics = ok(
        f.in_thread("calculate_metrics", by_id.clone()).await,
        "calculate_metrics",
    );
    assert_eq!(
        at(&metrics, "/metrics_summary/distance_km"),
        10.0,
        "bug: the metrics are the athlete's 10 km run's: {metrics}"
    );
    let compared = ok(
        f.in_thread("compare_activities", by_id.clone()).await,
        "compare_activities",
    );
    assert_eq!(
        compared["activity_id"], ATHLETE_STREAM_RUN,
        "bug: the athlete's run is compared: {compared}"
    );
    assert!(
        !mentions(&compared, COACH_RIDE),
        "the coach's ride is never a comparison of the athlete's: {compared}"
    );
    let weather = ok(
        f.in_thread("analyze_weather_impact", by_id.clone()).await,
        "analyze_weather_impact",
    );
    assert!(
        mentions(&weather, &format!("{ATHLETE_STREAM_RUN} session")),
        "bug: the weather read is of the athlete's run: {weather}"
    );

    // From the coach's own seat the athlete's run is not theirs to find.
    for tool in [
        "calculate_metrics",
        "compare_activities",
        "analyze_weather_impact",
    ] {
        let own = f
            .try_call(&f.own_executor(), f.coach, tool, by_id.clone())
            .await;
        assert!(
            own.as_ref().map_or(true, |response| !response.success),
            "{tool}: the athlete's run is not on the coach's own account: {own:?}"
        );
    }
}

/// The reconnect the athlete's dead connection needs is theirs to make: a
/// coach's read says so in band, and never raises the re-auth error the chat
/// pipeline mints the turn's user — the coach — a login from.
#[tokio::test]
async fn a_coachs_read_of_an_athlete_whose_connection_died_never_offers_the_coach_a_reconnect() {
    let (f, _) = world().await;
    let snapshot = json!({ "window": 30 });
    let refused = f
        .in_thread("export_latest_snapshot", snapshot.clone())
        .await;
    assert_eq!(
        refused_reason(&refused),
        "athlete_reconnect_needed",
        "bug: the athlete's dead Garmin answers in band, not as the coach's re-auth"
    );
    let message = refused.error.clone().unwrap_or_default();
    assert!(
        message.contains("Mira Subject's training")
            && message.contains("Garmin connection needs to be reconnected")
            && message.contains("only Mira Subject can do"),
        "the refusal names whose connection it is and who reconnects it: {message}"
    );

    let over_mcp = f
        .call(
            &f.mcp_executor(),
            f.coach,
            "export_latest_snapshot",
            json!({ "window": 30, "athlete": "Mira" }),
        )
        .await;
    assert_eq!(
        refused_reason(&over_mcp),
        "athlete_reconnect_needed",
        "bug: an MCP coach is never handed a reconnect for the athlete's connection"
    );

    // The athlete's own read still raises the re-auth their reconnect flow
    // answers.
    let own = f
        .try_call(
            &f.own_executor(),
            f.athlete,
            "export_latest_snapshot",
            snapshot,
        )
        .await;
    assert!(
        matches!(own, Err(ProtocolError::ProviderAuthRequired { .. })),
        "the athlete's own read asks them to reconnect: {own:?}"
    );

    // A read that answers the dead connection in band names whose it is:
    // "your account" would read as the coach's own.
    let garmin = json!({ "provider": "garmin" });
    let in_band = f.in_thread("get_athlete", garmin.clone()).await;
    let message = in_band.error.clone().unwrap_or_default();
    assert!(
        !in_band.success
            && message.contains("only Mira Subject can do")
            && !message.contains("reconnect your"),
        "bug: the in-band answer names the athlete as the one to reconnect: {message}"
    );
    let own = f
        .call(&f.own_executor(), f.athlete, "get_athlete", garmin)
        .await;
    assert!(
        own.error
            .as_deref()
            .is_some_and(|error| error.contains("Please reconnect your")),
        "the athlete's own read asks them to reconnect: {:?}",
        own.error
    );
}

/// A window served without the athlete's dead connection keeps its note to
/// the model on a coach's read, but not the backend the reconnect mint keys
/// on.
#[tokio::test]
async fn a_coachs_window_read_carries_no_reconnect_offer_for_the_athletes_dead_connection() {
    let (f, _) = world().await;
    let own = f
        .athletes_own("get_activities", json!({ "limit": 50 }))
        .await;
    assert!(
        own["reconnect_required"]["provider_slug"].is_string(),
        "the athlete's own window offers them the reconnect: {own}"
    );

    let thread = ok(
        f.in_thread("get_activities", json!({ "limit": 50 })).await,
        "get_activities",
    );
    assert!(
        mentions(&thread, "athlete-run-"),
        "the window still reads the athlete's sessions: {thread}"
    );
    assert!(
        thread["reconnect_required"].is_object()
            && thread["reconnect_required"]["provider_slug"].is_null(),
        "bug: a coach's read keeps the note and drops the mint's backend: {thread}"
    );
}

#[tokio::test]
async fn the_athletes_goal_progress_reads_in_the_thread() {
    let (f, goal) = world().await;
    let progress = ok(
        f.in_thread(
            "track_progress",
            json!({ "provider": UPLOAD, "goal_id": goal }),
        )
        .await,
        "track_progress",
    );
    assert_eq!(progress["goal_id"], goal.as_str(), "{progress}");
    assert_eq!(
        at(&progress, "/summary/total_activities"),
        ATHLETE_RUNS,
        "bug: the athlete's runs since the goal count toward it: {progress}"
    );

    // The goal is the athlete's: the coach holds no such goal of their own.
    let own = f
        .call(
            &f.own_executor(),
            f.coach,
            "track_progress",
            json!({ "provider": UPLOAD, "goal_id": goal }),
        )
        .await;
    assert!(
        !own.success,
        "the athlete's goal is not on the coach's own account: {:?}",
        own.result
    );
}

// ── Training physiology, never wellness ──────────────────────────────────

#[tokio::test]
async fn the_estimates_echo_the_athletes_training_physiology_in_the_thread() {
    let (f, _) = world().await;
    let vo2 = ok(
        f.in_thread(
            "estimate_vo2max",
            json!({ "method": "cooper_test", "distance_meters": 2800 }),
        )
        .await,
        "estimate_vo2max",
    );
    assert_eq!(
        vo2["stored_vo2_max"], ATHLETE_VO2MAX,
        "bug: the stored VO2max compared is the athlete's: {vo2}"
    );
    let own = f
        .own(
            "estimate_vo2max",
            json!({ "method": "cooper_test", "distance_meters": 2800 }),
        )
        .await;
    assert_eq!(own["stored_vo2_max"], COACH_VO2MAX, "{own}");

    let lactate = ok(
        f.in_thread("estimate_lactate_thresholds", lactate_stages())
            .await,
        "estimate_lactate_thresholds",
    );
    assert_eq!(
        at(&lactate, "/stored_profile/ftp_watts"),
        ATHLETE_FTP,
        "bug: the stored FTP echoed is the athlete's: {lactate}"
    );
    let own = f.own("estimate_lactate_thresholds", lactate_stages()).await;
    assert_eq!(at(&own, "/stored_profile/ftp_watts"), COACH_FTP, "{own}");
}

#[tokio::test]
async fn a_coachs_read_never_defaults_to_the_athletes_body_weight() {
    let (f, _) = world().await;
    let rockport = json!({
        "method": "rockport_walk",
        "gender": "female",
        "time_seconds": 780,
        "heart_rate": 132
    });
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
            assert!(
                !response.success,
                "bug: no weight of the athlete's stands in on a coach's read: {:?}",
                response.result
            );
            response.error.unwrap_or_default()
        }
    };
    assert!(
        message.contains("body weight is their wellness")
            && message.contains("Mira Subject has not shared their wellness"),
        "bug: the coach is told why, not that the athlete has no weight: {message}"
    );
    assert!(
        !message.contains(&ATHLETE_WEIGHT_KG.to_string()),
        "the athlete's weight is never named: {message}"
    );

    // The athlete's own read defaults to their saved weight, and says so.
    let own = f.athletes_own("estimate_vo2max", rockport).await;
    assert!(
        own["defaults_from_profile"]
            .as_array()
            .is_some_and(|defaults| defaults.iter().any(|d| d == "weight_kg")),
        "the athlete's own estimate reads their saved weight: {own}"
    );

    // A weight the coach gives is used as given.
    let given = ok(
        f.in_thread(
            "estimate_vo2max",
            json!({
                "method": "rockport_walk",
                "gender": "female",
                "time_seconds": 780,
                "heart_rate": 132,
                "weight_kg": 60
            }),
        )
        .await,
        "estimate_vo2max with a weight",
    );
    assert!(
        given["vo2max_ml_kg_min"].as_f64().is_some_and(|v| v > 0.0),
        "{given}"
    );
}

#[tokio::test]
async fn the_nutrition_mode_reads_wellness_and_is_refused_on_a_coachs_read() {
    let (f, _) = world().await;
    let nutrition = json!({ "provider": UPLOAD, "recommendation_type": "nutrition" });
    let refused = f
        .in_thread("generate_recommendations", nutrition.clone())
        .await;
    assert_eq!(
        refused_reason(&refused),
        "wellness_grant_needed",
        "bug: the nutrition mode scales from the athlete's body weight"
    );
    let message = refused.error.clone().unwrap_or_default();
    assert!(
        message.contains("Ask for another `recommendation_type`"),
        "the refusal says how to read their training: {message}"
    );
    assert!(
        !mentions(
            refused.result.as_ref().unwrap(),
            &ATHLETE_WEIGHT_KG.to_string()
        ),
        "nothing of the athlete's wellness was read"
    );

    let over_mcp = f
        .call(
            &f.mcp_executor(),
            f.coach,
            "generate_recommendations",
            json!({ "provider": UPLOAD, "recommendation_type": "nutrition", "athlete": "Mira" }),
        )
        .await;
    assert_eq!(
        refused_reason(&over_mcp),
        "wellness_grant_needed",
        "bug: athlete= over MCP reads the athlete's wellness only under their grant, as the \
         thread does"
    );

    // Every other mode reads the athlete's training.
    for mode in [
        "training_plan",
        "recovery",
        "intensity",
        "goal_specific",
        "all",
    ] {
        let read = f
            .in_thread(
                "generate_recommendations",
                json!({ "provider": UPLOAD, "recommendation_type": mode }),
            )
            .await;
        ok(read, mode);
    }

    // The athlete's own nutrition read scales from their weight.
    let own = f.athletes_own("generate_recommendations", nutrition).await;
    assert_eq!(
        at(&own, "/macronutrient_targets/body_weight_kg"),
        ATHLETE_WEIGHT_KG,
        "the athlete's own read keeps their wellness: {own}"
    );
}

#[tokio::test]
async fn no_output_of_a_coachs_physiology_read_carries_resting_heart_rate_or_weight() {
    let (f, _) = world().await;
    let reads = [
        (
            "estimate_vo2max",
            json!({ "method": "cooper_test", "distance_meters": 2800 }),
        ),
        ("estimate_lactate_thresholds", lactate_stages()),
        (
            "calculate_metrics",
            json!({ "provider": UPLOAD, "activity_id": ATHLETE_STREAM_RUN }),
        ),
        ("get_athlete", json!({ "provider": UPLOAD })),
        ("generate_recommendations", json!({ "provider": UPLOAD })),
    ];
    for (tool, params) in reads {
        let payload = ok(f.in_thread(tool, params).await, tool);
        for wellness in ["resting_hr", "\"weight\"", "weight_kg", "body_weight"] {
            assert!(
                !payload.to_string().contains(wellness),
                "bug: {tool} on a coach's read carries {wellness}: {payload}"
            );
        }
    }
}

/// A heart-rate reading's fraction of reserve, beside the heart rate and the
/// maximum it was read against, solves back to the resting heart rate the
/// zones were cut from, so a coach's read carries no reading.
#[tokio::test]
async fn a_coachs_metrics_read_carries_no_heart_rate_reading_the_resting_rate_solves_from() {
    let (f, _) = world().await;
    let by_id = json!({ "provider": UPLOAD, "activity_id": ATHLETE_STREAM_RUN });

    // The athlete's own read: its reading gives back their resting heart
    // rate, (hr - f * max) / (1 - f).
    let own = f.athletes_own("calculate_metrics", by_id.clone()).await;
    let heart_rate = at(&own, "/heart_rate_reading/heart_rate").as_f64().unwrap();
    let fraction = at(&own, "/heart_rate_reading/reserve_fraction")
        .as_f64()
        .unwrap();
    let max = at(&own, "/max_hr_used").as_f64().unwrap();
    let solved = fraction.mul_add(-max, heart_rate) / (1.0 - fraction);
    assert!(
        (solved - f64::from(ATHLETE_RESTING_HR)).abs() < 0.5,
        "the athlete's own reading is cut from their resting heart rate: {own}"
    );

    let thread = ok(
        f.in_thread("calculate_metrics", by_id).await,
        "calculate_metrics",
    );
    assert!(
        thread["heart_rate_reading"].is_null(),
        "bug: a coach's read carries no reading the resting heart rate solves from: {thread}"
    );
    assert!(
        mentions(
            &thread["heart_rate_unavailable"],
            "Mira Subject has not shared their wellness"
        ),
        "the coach's model reads why there is none: {thread}"
    );
    assert_eq!(
        at(&thread, "/metrics_summary/average_heart_rate"),
        &json!(152),
        "the run's own heart rate is training, and stays: {thread}"
    );
}

// ── Nothing written on the athlete's account ─────────────────────────────

#[tokio::test]
async fn a_coachs_route_read_answers_the_summary_and_stores_nothing_for_the_athlete() {
    let (f, _) = world().await;
    let route = ok(
        f.in_thread("export_routes", json!({ "activity_id": ROUTE_RUN }))
            .await,
        "export_routes",
    );
    let hash = route["gpx_hash"]
        .as_str()
        .expect("the summary carries its track's hash")
        .to_owned();
    assert!(
        f.repos()
            .route_summaries
            .get_route_summary(f.athlete.tenant, f.athlete.id, ROUTE_RUN, &hash)
            .await
            .unwrap()
            .is_none(),
        "bug: a coach's read stores no summary on the athlete's account"
    );

    // The athlete's own read stores it for their next one.
    let own = f
        .athletes_own("export_routes", json!({ "activity_id": ROUTE_RUN }))
        .await;
    assert_eq!(own["gpx_hash"], hash.as_str(), "{own}");
    assert!(
        f.repos()
            .route_summaries
            .get_route_summary(f.athlete.tenant, f.athlete.id, ROUTE_RUN, &hash)
            .await
            .unwrap()
            .is_some(),
        "the athlete's own read stores the summary, as before"
    );
}

// ── athlete= over MCP ────────────────────────────────────────────────────

/// Every read this phase scopes, with arguments that read the athlete.
fn phase_two_reads(goal: &str) -> Vec<(&'static str, Value)> {
    let by_id = json!({ "provider": UPLOAD, "activity_id": ATHLETE_STREAM_RUN });
    vec![
        ("get_athlete", json!({ "provider": UPLOAD })),
        ("get_stats", json!({ "provider": UPLOAD })),
        ("get_data_freshness", json!({})),
        ("list_data_sources", json!({})),
        ("export_routes", json!({ "activity_id": ROUTE_RUN })),
        ("calculate_metrics", by_id.clone()),
        (
            "analyze_performance_trends",
            json!({ "provider": UPLOAD, "metric": "distance" }),
        ),
        ("compare_activities", by_id.clone()),
        ("detect_patterns", json!({ "provider": UPLOAD })),
        ("predict_performance", json!({ "provider": UPLOAD })),
        ("analyze_weather_impact", by_id),
        ("suggest_goals", json!({ "provider": UPLOAD })),
        (
            "track_progress",
            json!({ "provider": UPLOAD, "goal_id": goal }),
        ),
        (
            "analyze_goal_feasibility",
            json!({ "provider": UPLOAD, "goal_type": "distance", "target_value": 150.0 }),
        ),
        (
            "estimate_vo2max",
            json!({ "method": "cooper_test", "distance_meters": 2800 }),
        ),
        ("estimate_lactate_thresholds", lactate_stages()),
        ("generate_recommendations", json!({ "provider": UPLOAD })),
    ]
}

fn named(mut params: Value, athlete: &str) -> Value {
    params["athlete"] = json!(athlete);
    params
}

#[tokio::test]
async fn athlete_over_mcp_reads_a_coached_athlete_with_every_phase_two_read_and_refuses_a_stranger()
{
    let (f, goal) = world().await;
    let stranger = person(&f.res, "stranger", "Sam Stranger").await;
    for (tool, params) in phase_two_reads(&goal) {
        let read = f
            .call(
                &f.mcp_executor(),
                f.coach,
                tool,
                named(params.clone(), "Mira"),
            )
            .await;
        let payload = ok(read, tool);
        assert!(
            !mentions(&payload, COACH_FIRST_NAME) && !mentions(&payload, COACH_RIDE),
            "bug: {tool} with athlete= never reads the coach: {payload}"
        );

        let refused = f
            .call(&f.mcp_executor(), stranger, tool, named(params, "Mira"))
            .await;
        assert_eq!(
            refused_reason(&refused),
            "no_match",
            "bug: {tool}: athlete= reaches only an athlete the caller coaches"
        );
    }
    // The snapshot reads through the athlete's dead Garmin, so the coach's
    // read of it answers the reconnect (tested above); a stranger is refused
    // before anything is read.
    let refused = f
        .call(
            &f.mcp_executor(),
            stranger,
            "export_latest_snapshot",
            json!({ "window": 30, "athlete": "Mira" }),
        )
        .await;
    assert_eq!(
        refused_reason(&refused),
        "no_match",
        "bug: export_latest_snapshot: athlete= reaches only an athlete the caller coaches"
    );
}

#[tokio::test]
async fn every_training_read_is_offered_in_the_thread_and_named_to_its_model() {
    let (f, _) = world().await;
    let readable = f
        .res
        .mcp
        .tool_registry
        .chat_callable_of_class(DataClass::Training);
    for tool in TRAINING_READS {
        assert!(
            readable.iter().any(|name| name == tool),
            "bug: {tool} reads the thread's athlete, so the thread's prompt names it: {readable:?}"
        );
    }
    for wellness in [
        "get_sleep_sessions",
        "get_recovery_metrics",
        "get_health_snapshots",
    ] {
        assert!(
            !readable.iter().any(|name| name == wellness),
            "bug: the athlete's wellness ({wellness}) is not a training read"
        );
    }
}
