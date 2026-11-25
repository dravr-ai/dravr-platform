// ABOUTME: save_workout_to_library end-to-end — MCP tool → the caller's own Intervals.icu library on a loopback stand-in
// ABOUTME: Pins the session it picks, the workout body on the wire (fuelling left out), and each refusal before any library request
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `save_workout_to_library` reads one session of the caller's plan and
//! keeps it in the caller's own Intervals.icu workout library.
//!
//! Intervals.icu is a loopback stand-in that serves the three library calls
//! (`GET /folders`, `POST /folders`, `POST /workouts`) and records the method,
//! path and body of each: the request bytes are what a tool that only
//! answered "saved" could not fake. The provider's own wire shape is pinned
//! by `intervals_icu_library_test` in pierre-providers; this suite pins what
//! the tool sends it, and what it refuses before sending anything.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::extract::State;
use axum::http::{Method, Uri};
use axum::routing::any;
use axum::{Json, Router};
use dravr_tronc::dates::{CivilDate, Elapsed, Timestamp, Weekday};
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{ConnectionType, FuelingProtocol, TenantId, WorkoutStep};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_memory::training_plans::{GoalRace, PlanPhase, PlannedDay, RacePriority};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

use super::workout_push_test::{connect_intervals_icu, serving_intervals_at};
use crate::common;

const TOOL: &str = "save_workout_to_library";

/// The athlete id `connect_intervals_icu` links the account under: the path
/// every library call of the caller's own goes to.
const LIBRARY_PATH: &str = "/api/v1/athlete/i123456";

/// Every request the stand-in served: method, path, and the JSON body.
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

/// Serve the library calls, recording each: no folder is listed, so a save
/// creates its folder (id 31) and then its workout (id 77).
async fn spawn_library(seen: Seen) -> String {
    async fn serve(
        State(seen): State<Seen>,
        method: Method,
        uri: Uri,
        body: Option<Json<Value>>,
    ) -> Json<Value> {
        let path = uri.path().to_owned();
        let answer = match (method.as_str(), path.strip_prefix(LIBRARY_PATH)) {
            ("POST", Some("/folders")) => json!({ "id": 31 }),
            ("POST", Some("/workouts")) => json!({ "id": 77 }),
            _ => json!([]),
        };
        seen.lock().unwrap().push((
            method.to_string(),
            path,
            body.map_or(Value::Null, |Json(body)| body),
        ));
        Json(answer)
    }
    let app = Router::new().fallback(any(serve)).with_state(seen);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{addr}")
}

/// An athlete whose plan the tests lay, and the executor serving them.
struct Fixture {
    executor: Arc<UniversalToolExecutor>,
    user_id: Uuid,
    tenant: TenantId,
    seen: Seen,
}

impl Fixture {
    /// `connect_intervals` off leaves the athlete on Strava only: a data
    /// source for the dispatch chokepoint, and no workout library.
    async fn new(connect_intervals: bool) -> Result<Self> {
        common::init_server_config();
        common::init_test_http_clients();
        let resources = common::create_test_server_resources().await?;
        let seen = Seen::default();
        let base_url = spawn_library(Arc::clone(&seen)).await;
        let executor = Arc::new(
            UniversalToolExecutor::new(serving_intervals_at(&resources, &base_url))
                .with_scopes(OAuthScope::self_grant()),
        );
        let resources = &executor.resources;
        let email = format!("library_test_{}@example.com", Uuid::new_v4());
        let (user_id, _user) =
            common::create_test_user_with_email(resources.database(), &email).await?;
        let tenant = resources
            .repos()
            .tenants
            .get_all()
            .await?
            .iter()
            .find(|t| t.owner_user_id == user_id)
            .map(|t| t.id)
            .expect("user owns a tenant");
        if connect_intervals {
            connect_intervals_icu(resources.as_ref(), user_id, tenant).await?;
        } else {
            resources
                .repos()
                .provider_connections
                .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
                .await?;
        }
        Ok(Self {
            executor: Arc::clone(&executor),
            user_id,
            tenant,
            seen,
        })
    }

    async fn save(&self, params: Value) -> Result<UniversalResponse> {
        Ok(self
            .executor
            .execute_tool(UniversalRequest {
                tool_name: TOOL.to_owned(),
                parameters: params,
                user_id: self.user_id.to_string(),
                protocol: "test".to_owned(),
                tenant_id: Some(self.tenant.to_string()),
            })
            .await?)
    }

    fn requests(&self) -> Vec<(String, String, Value)> {
        self.seen.lock().unwrap().clone()
    }

    /// Lay one week of plan starting `monday` holding `days`.
    async fn lay_week(&self, monday: CivilDate, days: &[PlannedDay]) {
        let goal = GoalRace {
            name: "Winter race".to_owned(),
            date: (monday + Elapsed::days(70)).to_string(),
            discipline: "road".to_owned(),
            priority: RacePriority::A,
        };
        let phases = vec![PlanPhase {
            kind: PhaseKind::Base,
            start: monday.to_string(),
            weeks: 10,
            intent: "build the engine".to_owned(),
            target_hours: None,
            purpose: String::new(),
            volume_share_of_peak: None,
            tid_target: None,
            hard_sessions_max: None,
            session_mix: BTreeMap::new(),
            flavour_override: None,
            loading_pattern: None,
            skeleton_id: None,
        }];
        let start = monday.to_string();
        let weeks = [PlanWeekInput {
            week_start: &start,
            focus: "",
            days,
            adjustment_reason: "",
            phase_index: None,
        }];
        self.executor
            .resources
            .repos()
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &self.tenant.to_string(),
                user_id: &self.user_id.to_string(),
                author: PlanAuthor::none(),
                goal_fact_id: None,
                replace_season: false,
                outline: Some(PlanOutlineInput {
                    goal_race: &goal,
                    races: Some(&[]),
                    strategy: "steady base",
                    phases: &phases,
                    source_conversation_id: None,
                    flavour: None,
                    season_start: None,
                    season_end: None,
                }),
                weeks: &weeks,
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .expect("save plan bundle");
    }
}

/// The Monday at least a week out, so every plan date is in the future.
fn next_monday() -> CivilDate {
    let mut day = Timestamp::now().utc_date() + Elapsed::days(7);
    while day.weekday() != Weekday::Mon {
        day += Elapsed::days(1);
    }
    day
}

fn day(date: CivilDate, sport: &str, workout: &str, minutes: u32) -> PlannedDay {
    PlannedDay {
        date: date.to_string(),
        sport: sport.to_owned(),
        workout: workout.to_owned(),
        duration_min: Some(minutes),
        intensity: "Z2".to_owned(),
        steps: Vec::new(),
        fueling: None,
        template_slug: None,
        template_params: None,
        template_source: None,
        template_preset: None,
        envelope: None,
        coach_original: None,
        coach_added: false,
        coach_call: None,
    }
}

/// The Zwift session from the coach's request: warm-up, 10 × 30/30, cool-down.
fn vo2_steps() -> Vec<WorkoutStep> {
    let step = |label: &str, seconds: u32, zone: &str, repeat: u32| WorkoutStep {
        label: label.to_owned(),
        duration_seconds: seconds,
        distance_meters: None,
        target_zone: zone.to_owned(),
        repeat,
        repeat_group: None,
        note: None,
    };
    vec![
        step("Warm-up", 900, "Z2", 1),
        step("Short interval", 30, "VO2max", 10),
        step("Recovery", 30, "Z1", 10),
        step("Cool-down", 600, "Z1", 1),
    ]
}

#[tokio::test]
async fn a_plan_session_lands_in_the_callers_library_without_its_fuelling() -> Result<()> {
    let fixture = Fixture::new(true).await?;
    let monday = next_monday();
    let wednesday = monday + Elapsed::days(2);
    let mut zwift = day(wednesday, "virtual_ride", "Zwift VO2 30/30", 60);
    zwift.steps = vo2_steps();
    zwift.fueling = Some(FuelingProtocol {
        carbs_g_per_h: 90.0,
        fluid_ml_per_h: 700.0,
        sodium_mg_per_h: Some(600.0),
        carb_source: None,
    });
    fixture.lay_week(monday, &[zwift]).await;

    let resp = fixture
        .save(json!({ "date": wednesday.to_string() }))
        .await?;
    assert!(resp.success, "the save lands: {:?}", resp.error);
    let result = resp.result.expect("result payload");
    assert_eq!(result["workout_id"], "77", "the provider's id: {result}");
    assert_eq!(result["folder"], "Dravr", "the default folder");
    assert_eq!(
        result["folder_created"], true,
        "bug: no folder was listed: {result}"
    );
    assert_eq!(
        result["already_saved"], false,
        "bug: nothing was there: {result}"
    );
    assert_eq!(result["provider"], "Intervals.icu", "bug: {result}");
    assert_eq!(result["date"], wednesday.to_string(), "bug: {result}");
    assert!(result.get("athlete").is_none(), "the caller's own plan");

    let seen = fixture.requests();
    let calls: Vec<(&str, &str)> = seen
        .iter()
        .map(|(method, path, _)| (method.as_str(), path.as_str()))
        .collect();
    assert_eq!(
        calls,
        [
            ("GET", "/api/v1/athlete/i123456/folders"),
            ("POST", "/api/v1/athlete/i123456/folders"),
            ("POST", "/api/v1/athlete/i123456/workouts"),
        ],
        "bug: list, create the folder, create the workout, at the caller's own path"
    );
    let workout = &seen[2].2;
    assert_eq!(
        workout["folder_id"], 31,
        "bug: into the created folder: {workout}"
    );
    assert_eq!(workout["type"], "VirtualRide", "bug: {workout}");
    assert_eq!(
        workout["moving_time"],
        900 + 10 * 30 + 10 * 30 + 600,
        "bug: the steps' total: {workout}"
    );
    let description = workout["description"].as_str().expect("a description");
    assert!(
        description.contains("10x"),
        "the 30/30 block is a repeat: {description}"
    );
    assert!(
        !description.contains("g/h") && !description.contains("ml/h"),
        "the athlete's fuelling rates stay out of a template: {description}"
    );
    Ok(())
}

#[tokio::test]
async fn the_second_session_of_a_day_and_a_named_folder_are_honoured() -> Result<()> {
    let fixture = Fixture::new(true).await?;
    let monday = next_monday();
    let saturday = monday + Elapsed::days(5);
    fixture
        .lay_week(
            monday,
            &[
                day(saturday, "run", "Easy run", 40),
                day(saturday, "ride", "Long endurance ride", 120),
            ],
        )
        .await;

    let resp = fixture
        .save(json!({ "date": saturday.to_string(), "session": 2, "folder": "Endurance" }))
        .await?;
    assert!(resp.success, "the save lands: {:?}", resp.error);
    let seen = fixture.requests();
    assert_eq!(
        seen[1].2,
        json!({ "name": "Endurance", "type": "FOLDER" }),
        "bug: the folder the call named: {seen:?}"
    );
    let workout = &seen[2].2;
    assert_eq!(
        workout["type"], "Ride",
        "the day's second session: {workout}"
    );
    assert_eq!(workout["moving_time"], 7_200, "bug: {workout}");
    Ok(())
}

#[tokio::test]
async fn a_date_without_that_session_is_refused_before_any_library_request() -> Result<()> {
    let fixture = Fixture::new(true).await?;
    let monday = next_monday();
    let tuesday = monday + Elapsed::days(1);
    fixture
        .lay_week(monday, &[day(tuesday, "run", "Tempo run", 50)])
        .await;

    let empty = fixture
        .save(json!({ "date": (monday + Elapsed::days(3)).to_string() }))
        .await?;
    assert!(!empty.success, "bug: the date holds no session: {empty:?}");
    let refusal = empty.result.expect("an in-band refusal");
    assert_eq!(refusal["reason"], "no_session", "got: {refusal}");
    assert!(
        refusal["error"]
            .as_str()
            .is_some_and(|e| e.contains("no session")),
        "got: {refusal}"
    );

    let second = fixture
        .save(json!({ "date": tuesday.to_string(), "session": 2 }))
        .await?;
    assert!(
        !second.success,
        "bug: the date holds one session: {second:?}"
    );
    let refusal = second.result.expect("an in-band refusal");
    assert_eq!(refusal["reason"], "no_session", "got: {refusal}");
    assert!(
        refusal["error"]
            .as_str()
            .is_some_and(|e| e.contains("holds 1 session")),
        "got: {refusal}"
    );
    assert!(
        fixture
            .requests()
            .iter()
            .all(|(method, _, _)| method == "GET"),
        "nothing written: {:?}",
        fixture.requests()
    );
    Ok(())
}

#[tokio::test]
async fn a_caller_without_a_library_is_told_to_connect_one() -> Result<()> {
    let fixture = Fixture::new(false).await?;
    let monday = next_monday();
    fixture
        .lay_week(monday, &[day(monday, "run", "Easy run", 40)])
        .await;

    let resp = fixture.save(json!({ "date": monday.to_string() })).await?;
    assert!(!resp.success, "bug: no library to save to: {resp:?}");
    let refusal = resp.result.expect("an in-band refusal");
    assert_eq!(refusal["reason"], "no_workout_library", "got: {refusal}");
    assert!(
        refusal["error"]
            .as_str()
            .is_some_and(|e| e.contains("Intervals.icu")),
        "names the provider that keeps one: {refusal}"
    );
    assert!(
        fixture.requests().is_empty(),
        "bug: refused before any request: {:?}",
        fixture.requests()
    );
    Ok(())
}

#[tokio::test]
async fn a_malformed_call_is_refused_by_its_argument() -> Result<()> {
    let fixture = Fixture::new(true).await?;
    for (params, field) in [
        (json!({}), "date"),
        (json!({ "date": "25/11/2026" }), "date"),
        (json!({ "date": "2026-11-25", "session": 0 }), "session"),
        (
            json!({ "date": "2026-11-25", "folder": "x".repeat(101) }),
            "folder",
        ),
    ] {
        let error = fixture
            .save(params.clone())
            .await
            .expect_err("a malformed call is an invalid-parameters error");
        let rendered = format!("{error:?}");
        assert!(rendered.contains(field), "{params}: {rendered}");
    }
    assert!(
        fixture.requests().is_empty(),
        "bug: refused before any request: {:?}",
        fixture.requests()
    );
    Ok(())
}
