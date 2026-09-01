// ABOUTME: Calendar writes end-to-end — MCP tools → authed provider → a fake Intervals.icu on loopback
// ABOUTME: prescribe / replace / withdraw one workout, and push a plan then reconcile its adjustments; asserts ids, ledger rows, and the bytes on the wire
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "provider-intervals-icu")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! End-to-end tests for every path that writes to an athlete's training
//! calendar.
//!
//! Intervals.icu is stood in for by a small fake on loopback that keeps one
//! athlete's events collection and answers the five calls the provider makes
//! (`GET /events`, `POST /events`, `PUT /events/{id}`, `PUT
//! /events/bulk-delete`, plus any refusal a test asks for), recording every
//! request it served. That is what lets a plan push be tested as a cycle —
//! push, adjust the plan, push again — and lets each assertion be one a stub
//! that only wrote rows could not pass: a provider-issued event id, a ledger
//! row carrying it, and the request bytes the calendar actually received.

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc, Weekday};
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::models::{
    CalendarEventSource, CalendarKey, ConnectionType, PrescribedWorkout, SportType, TenantId,
    UserOAuthToken, WorkoutStep,
};
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_memory::training_plans::{BlockPhase, GoalRace, PlanBlock, PlannedDay, RacePriority};
use pierre_providers::intervals_icu_provider::default_config;
use pierre_providers::ProviderRegistry;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use pierre_tool_runtime::task_cancellation::scoped_with_cancel_flag;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// The athlete id the stubbed account is linked under — it addresses the
/// athlete-scoped URL path, so the tests can assert on it.
const ATHLETE_ID: &str = "i123456";

/// One event in the fake calendar: the body the provider sent, the id the
/// fake assigned, and when it last changed.
#[derive(Clone, Debug)]
struct FakeEvent {
    id: i64,
    body: Value,
    updated: DateTime<Utc>,
}

/// How the fake answers.
#[derive(Clone, Copy)]
enum Mode {
    /// Keep an events collection and serve it.
    Live,
    /// Refuse every call with this status line.
    Refuse(&'static str),
}

/// A fake Intervals.icu on loopback: one athlete's events collection, plus a
/// record of every full request (head + body) it served.
struct Stub {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    events: Arc<Mutex<Vec<FakeEvent>>>,
    handle: JoinHandle<()>,
}

impl Stub {
    /// Every request this fake has served so far, in order.
    async fn requests(&self) -> Vec<String> {
        self.requests.lock().await.clone()
    }

    /// The single request the fake served. Fails loudly on zero or many, so a
    /// test can never mistake "the provider was never called" for a pass.
    async fn only_request(&self) -> String {
        let requests = self.requests().await;
        assert_eq!(
            requests.len(),
            1,
            "expected exactly one call to Intervals.icu, got {}",
            requests.len()
        );
        requests[0].clone()
    }

    /// Request lines (`METHOD /path`) served so far, without bodies.
    async fn request_lines(&self) -> Vec<String> {
        self.requests()
            .await
            .iter()
            .map(|r| {
                let line = r.lines().next().unwrap_or_default();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or_default();
                let target = parts.next().unwrap_or_default();
                format!("{method} {}", target.split('?').next().unwrap_or_default())
            })
            .collect()
    }

    /// The calendar as the fake holds it now.
    async fn events(&self) -> Vec<FakeEvent> {
        self.events.lock().await.clone()
    }

    /// The event carrying Dravr's `external_id`, if any.
    async fn event_by_key(&self, external_id: &str) -> Option<FakeEvent> {
        self.events()
            .await
            .into_iter()
            .find(|e| e.body["external_id"].as_str() == Some(external_id))
    }

    /// Pretend the athlete edited an event on Intervals.icu at `when`.
    async fn touch(&self, id: i64, when: DateTime<Utc>) {
        let mut events = self.events.lock().await;
        let event = events
            .iter_mut()
            .find(|e| e.id == id)
            .expect("touched event exists");
        event.updated = when;
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Read one complete HTTP request (head plus any `Content-Length` body) off
/// `socket`.
///
/// Reading only to the blank line would capture the head and drop the JSON the
/// provider posted — which is the half these tests are here to pin.
async fn read_request(socket: &mut TcpStream) -> String {
    let mut raw = Vec::new();
    let mut buf = [0_u8; 1024];
    let mut head_len = None;
    loop {
        let n = socket.read(&mut buf).await.expect("read request");
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
        if head_len.is_none() {
            if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                head_len = Some(pos + 4);
            }
        }
        if let Some(head_len) = head_len {
            let head = String::from_utf8_lossy(&raw[..head_len]).into_owned();
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if raw.len() >= head_len + content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&raw).into_owned()
}

/// Answer one request against the events collection.
fn respond(
    mode: Mode,
    next_id: &AtomicI64,
    events: &mut Vec<FakeEvent>,
    request: &str,
) -> (&'static str, String) {
    if let Mode::Refuse(status) = mode {
        return (status, "{}".to_owned());
    }
    let (head, body) = request.split_once("\r\n\r\n").unwrap_or((request, ""));
    let mut first = head.lines().next().unwrap_or_default().split_whitespace();
    let method = first.next().unwrap_or_default();
    let path = first
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    let collection = format!("/api/v1/athlete/{ATHLETE_ID}/events");
    let bulk_delete = format!("{collection}/bulk-delete");
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    match method {
        "GET" if path == collection => {
            let listed: Vec<Value> = events
                .iter()
                .map(|e| {
                    let mut v = e.body.clone();
                    v["id"] = json!(e.id);
                    v["updated"] = json!(e.updated.to_rfc3339());
                    v
                })
                .collect();
            ("HTTP/1.1 200 OK", serde_json::to_string(&listed).unwrap())
        }
        "POST" if path == collection => {
            let id = next_id.fetch_add(1, Ordering::SeqCst);
            events.push(FakeEvent {
                id,
                body: parsed,
                updated: Utc::now(),
            });
            ("HTTP/1.1 200 OK", json!({ "id": id }).to_string())
        }
        "PUT" if path == bulk_delete => {
            let doomed: HashSet<i64> = parsed
                .as_array()
                .map(|items| items.iter().filter_map(|d| d["id"].as_i64()).collect())
                .unwrap_or_default();
            let before = events.len();
            events.retain(|e| !doomed.contains(&e.id));
            (
                "HTTP/1.1 200 OK",
                json!({ "eventsDeleted": before - events.len() }).to_string(),
            )
        }
        "PUT" if path.starts_with(&format!("{collection}/")) => {
            let id: i64 = path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .parse()
                .unwrap_or(-1);
            match events.iter_mut().find(|e| e.id == id) {
                Some(event) => {
                    event.body = parsed;
                    event.updated = Utc::now();
                    ("HTTP/1.1 200 OK", json!({ "id": id }).to_string())
                }
                None => ("HTTP/1.1 404 Not Found", "{}".to_owned()),
            }
        }
        _ => ("HTTP/1.1 404 Not Found", "{}".to_owned()),
    }
}

/// Stand up the fake. `first_id` is the id the first created event gets, so a
/// test can assert on the id the provider hands back.
async fn stub_with(mode: Mode, first_id: i64) -> Stub {
    stub_inner(mode, first_id, None).await
}

/// A live fake that raises `flag` once it has served `writes` write requests
/// (POST/PUT) — how the cancellation tests flip the cooperative cancel flag
/// deterministically between entries of one push.
async fn stub_cancelling_after_writes(first_id: i64, flag: Arc<AtomicBool>, writes: usize) -> Stub {
    stub_inner(Mode::Live, first_id, Some((flag, writes))).await
}

async fn stub_inner(
    mode: Mode,
    first_id: i64,
    cancel_after_writes: Option<(Arc<AtomicBool>, usize)>,
) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    let calendar = Arc::clone(&events);
    let next_id = AtomicI64::new(first_id);
    let writes_seen = AtomicUsize::new(0);
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let request = read_request(&mut socket).await;
            recorded.lock().await.push(request.clone());
            if let Some((flag, after)) = &cancel_after_writes {
                if request.starts_with("POST ") || request.starts_with("PUT ") {
                    let served = writes_seen.fetch_add(1, Ordering::Relaxed) + 1;
                    if served >= *after {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
            }
            let (status_line, body) = {
                let mut events = calendar.lock().await;
                respond(mode, &next_id, &mut events, &request)
            };
            let response = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            socket.flush().await.expect("flush response");
        }
    });
    Stub {
        base_url: format!("http://{addr}"),
        requests,
        events,
        handle,
    }
}

/// A live fake whose first created event gets `first_id`.
async fn stub_live(first_id: i64) -> Stub {
    stub_with(Mode::Live, first_id).await
}

/// A fake that refuses everything with `status_line`.
async fn stub_refusing(status_line: &'static str) -> Stub {
    stub_with(Mode::Refuse(status_line), 1).await
}

/// An athlete with a connected provider, and the executor that serves them.
struct Fixture {
    executor: Arc<UniversalToolExecutor>,
    user_id: Uuid,
    tenant: TenantId,
}

impl Fixture {
    fn tenant_str(&self) -> String {
        self.tenant.to_string()
    }

    /// Every ledger row recorded for this athlete, newest first.
    async fn ledger(&self) -> Vec<PrescribedWorkout> {
        self.executor
            .resources
            .repos()
            .prescribed_workouts
            .list_prescribed_workouts(self.tenant, self.user_id, 50)
            .await
            .expect("list ledger")
    }

    /// The rows whose entry is live on the calendar, in calendar order.
    async fn live(&self) -> Vec<PrescribedWorkout> {
        self.executor
            .resources
            .repos()
            .prescribed_workouts
            .list_live_calendar_events(self.tenant, self.user_id, INTERVALS_ICU, None)
            .await
            .expect("list live")
    }

    async fn execute(&self, tool_name: &str, params: Value) -> Result<UniversalResponse> {
        Ok(self
            .executor
            .execute_tool(UniversalRequest {
                tool_name: tool_name.to_owned(),
                parameters: params,
                user_id: self.user_id.to_string(),
                protocol: "test".to_owned(),
                tenant_id: Some(self.tenant_str()),
                progress_token: None,
                cancellation_token: None,
                progress_reporter: None,
            })
            .await?)
    }

    async fn prescribe(&self, params: Value) -> Result<UniversalResponse> {
        self.execute("prescribe_workout", params).await
    }

    /// Run a tool that must succeed and return its result payload.
    async fn ok(&self, tool_name: &str, params: Value) -> Result<Value> {
        let resp = self.execute(tool_name, params).await?;
        assert!(resp.success, "{tool_name} must succeed: {:?}", resp.error);
        Ok(resp.result.expect("result payload"))
    }

    /// Persist plan weeks the way `save_training_plan` does, with an outline
    /// on the first call so the athlete has an active plan to attach to.
    async fn save_weeks(&self, with_outline: bool, weeks: &[(NaiveDate, &str, Vec<PlannedDay>)]) {
        let goal = GoalRace {
            name: "Fall race".to_owned(),
            date: (weeks[0].0 + Duration::days(70))
                .format("%Y-%m-%d")
                .to_string(),
            discipline: "gravel".to_owned(),
            priority: RacePriority::A,
        };
        let blocks = vec![PlanBlock {
            phase: BlockPhase::Base,
            start: weeks[0].0.format("%Y-%m-%d").to_string(),
            weeks: 10,
            intent: "rebuild volume".to_owned(),
            target_hours: None,
        }];
        let outline = with_outline.then(|| PlanOutlineInput {
            goal_race: &goal,
            races: &[],
            strategy: "steady base, then sharpen",
            blocks: &blocks,
            source_conversation_id: None,
        });
        let starts: Vec<String> = weeks
            .iter()
            .map(|(start, _, _)| start.format("%Y-%m-%d").to_string())
            .collect();
        let inputs: Vec<PlanWeekInput<'_>> = weeks
            .iter()
            .zip(starts.iter())
            .map(|((_, focus, days), start)| PlanWeekInput {
                week_start: start,
                focus,
                days,
                adjustment_reason: "",
            })
            .collect();
        self.executor
            .resources
            .repos()
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &self.tenant_str(),
                user_id: &self.user_id.to_string(),
                coach_slug: None,
                goal_fact_id: None,
                outline,
                weeks: &inputs,
            })
            .await
            .expect("save plan bundle");
    }
}

/// Build an executor whose Intervals.icu provider points at `base_url`, with an
/// athlete linked to it exactly the way the link route links one: an API-key
/// token row carrying the athlete id, plus a `Manual` connection row.
///
/// `connect_intervals` off leaves the athlete on a Strava connection only —
/// enough to clear the dispatch chokepoint (which asks whether ANY data source
/// exists) so the tool's own "connect Intervals.icu" refusal is what gets
/// tested, rather than the pre-dispatch one.
async fn fixture(base_url: &str, connect_intervals: bool) -> Result<Fixture> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;

    let mut config = default_config();
    config.api_base_url = base_url.to_owned();
    let mut registry = ProviderRegistry::new();
    registry.set_default_config(INTERVALS_ICU, config);
    let mut context = (*resources).clone();
    context.fitness.provider_registry = Arc::new(registry);
    let executor = Arc::new(UniversalToolExecutor::new(Arc::new(context)));
    let resources = &executor.resources;

    let email = format!("prescribe_test_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(resources.database(), &email).await?;
    let tenants = resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .map(|t| t.id)
        .expect("user owns a tenant");

    let (provider, connection_type) = if connect_intervals {
        (INTERVALS_ICU, ConnectionType::Manual)
    } else {
        ("strava", ConnectionType::OAuth)
    };
    if connect_intervals {
        let now = Utc::now();
        resources
            .repos()
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id,
                tenant_id: tenant.to_string(),
                provider: INTERVALS_ICU.to_owned(),
                access_token: "test-api-key".to_owned(),
                refresh_token: None,
                token_type: "api_key".to_owned(),
                expires_at: None,
                scope: None,
                provider_user_id: Some(ATHLETE_ID.to_owned()),
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await?;
    }
    resources
        .repos()
        .provider_connections
        .register_connection(user_id, tenant, provider, &connection_type, None)
        .await?;

    Ok(Fixture {
        executor: Arc::clone(&executor),
        user_id,
        tenant,
    })
}

/// The structured session from the live conversation that surfaced carnet#100 —
/// a trail session no cornerstone slug can express.
fn trail_session() -> Value {
    json!({
        "name": "Trail technique — montées 55 min",
        "sport": "run",
        "intensity_distribution": "polarized",
        "structure": [
            {
                "label": "Trail continu",
                "duration_seconds": 3300,
                "target_zone": "Z2",
                "note": "montées/descentes continues, Z2 stable",
            }
        ],
    })
}

/// The Monday at least a week out, so every plan date is in the future on
/// whatever day the suite runs.
fn next_plan_monday() -> NaiveDate {
    let mut day = Utc::now().date_naive() + Duration::days(7);
    while day.weekday() != Weekday::Mon {
        day += Duration::days(1);
    }
    day
}

fn planned(
    date: NaiveDate,
    sport: &str,
    workout: &str,
    minutes: Option<u32>,
    intensity: &str,
) -> PlannedDay {
    PlannedDay {
        date: date.format("%Y-%m-%d").to_string(),
        sport: sport.to_owned(),
        workout: workout.to_owned(),
        duration_min: minutes,
        intensity: intensity.to_owned(),
        steps: Vec::new(),
        fueling: None,
    }
}

fn rest_day(date: NaiveDate) -> PlannedDay {
    planned(date, "rest", "", None, "")
}

/// The steps of a threshold session as a coach states them: 15 min warm-up,
/// 3 × (8 min on / 4 min off), 10 min cool-down — 61 minutes.
fn threshold_steps() -> Vec<WorkoutStep> {
    let step = |label: &str, seconds: u32, zone: &str, repeat: u32| WorkoutStep {
        label: label.to_owned(),
        duration_seconds: seconds,
        distance_meters: None,
        target_zone: zone.to_owned(),
        repeat,
        note: None,
    };
    vec![
        step("Warm-up", 900, "Z1", 1),
        step("Work", 480, "88-93% FTP", 3),
        step("Recovery", 240, "Z1", 3),
        step("Cool-down", 600, "Z1", 1),
    ]
}

fn iso(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

// ── Single prescriptions ───────────────────────────────────────────────────

#[tokio::test]
async fn prescribing_a_cornerstone_creates_the_calendar_event_and_records_its_id() -> Result<()> {
    let stub = stub_live(987_654).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;

    let resp = fixture
        .prescribe(json!({ "template_slug": "long_run_z2", "date": "2026-09-15" }))
        .await?;
    assert!(resp.success, "prescribe must succeed: {:?}", resp.error);
    let result = resp.result.expect("result payload");

    // The event id is the provider's, not ours — a tool that only wrote a row
    // has nothing to put here.
    assert_eq!(result["provider_event_id"].as_str(), Some("987654"));
    assert_eq!(result["status"].as_str(), Some("pushed"));
    assert_eq!(result["template_slug"].as_str(), Some("long_run_z2"));
    assert_eq!(result["scheduled_for"].as_str(), Some("2026-09-15"));
    assert_eq!(result["duration_minutes"].as_u64(), Some(90));
    let prescription_id = result["prescription_id"].as_str().expect("prescription id");

    let rows = fixture.ledger().await;
    assert_eq!(rows.len(), 1, "one prescription recorded");
    assert_eq!(rows[0].status, "pushed");
    assert_eq!(rows[0].provider_event_id.as_deref(), Some("987654"));
    assert_eq!(rows[0].provider, "intervals_icu");
    assert_eq!(rows[0].prescribed_for_date.to_string(), "2026-09-15");
    assert_eq!(rows[0].source, CalendarEventSource::Prescription);
    assert_eq!(
        rows[0].external_id.as_deref(),
        Some(format!("dravr:rx:{prescription_id}").as_str()),
        "the ledger and the calendar share Dravr's key"
    );
    assert!(rows[0].payload_hash.is_some());

    // And the athlete's actual calendar was addressed, on the date asked for,
    // keyed, timed, and with the steps in the DSL Intervals.icu parses.
    let request = stub.only_request().await;
    assert!(
        request.starts_with(&format!("POST /api/v1/athlete/{ATHLETE_ID}/events ")),
        "the push must POST to the athlete's events collection; got: {request}"
    );
    assert!(
        request.contains("2026-09-15T00:00:00"),
        "the event must carry the prescribed date; got: {request}"
    );
    assert!(
        request.contains("Long Run"),
        "the event must carry the workout name; got: {request}"
    );
    assert!(
        request.contains(&format!("\"external_id\":\"dravr:rx:{prescription_id}\"")),
        "the event must carry Dravr's key; got: {request}"
    );
    assert!(
        request.contains("\"moving_time\":5400"),
        "90 minutes as moving_time; got: {request}"
    );
    assert!(
        request.contains("Z2 Pace"),
        "a run's zone step must resolve against pace; got: {request}"
    );
    Ok(())
}

#[tokio::test]
async fn a_refused_push_records_a_failed_row_and_reports_the_failure() -> Result<()> {
    let stub = stub_refusing("HTTP/1.1 500 Internal Server Error").await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;

    let err = fixture
        .prescribe(json!({ "template_slug": "vo2_5x3", "date": "2026-09-16" }))
        .await
        .expect_err("a refused push must not report success");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("500") || rendered.to_lowercase().contains("intervals"),
        "the error must name the provider failure; got: {rendered}"
    );

    // The attempt is still on the record — and it is honest about the outcome.
    let rows = fixture.ledger().await;
    assert_eq!(rows.len(), 1, "a refused attempt is still recorded");
    assert_eq!(rows[0].status, "failed");
    assert!(
        rows[0].provider_event_id.is_none(),
        "a failed push has no event id to record"
    );
    assert_eq!(rows[0].template_slug.as_deref(), Some("vo2_5x3"));
    assert!(
        fixture.live().await.is_empty(),
        "a failed row is not a live entry"
    );
    Ok(())
}

#[tokio::test]
async fn an_inline_session_is_stored_and_pushed_with_its_coaching_cue() -> Result<()> {
    let stub = stub_live(424_242).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;

    let resp = fixture
        .prescribe(json!({ "session": trail_session(), "date": "2026-09-17" }))
        .await?;
    assert!(resp.success, "session push must succeed: {:?}", resp.error);
    let result = resp.result.expect("result payload");
    assert_eq!(result["status"].as_str(), Some("pushed"));
    assert_eq!(result["provider_event_id"].as_str(), Some("424242"));
    // 3300s rounds up to 55 minutes, derived from the step — never supplied.
    assert_eq!(result["duration_minutes"].as_u64(), Some(55));

    // The session became a template this athlete can be prescribed again by slug.
    let slug = result["template_slug"]
        .as_str()
        .expect("template_slug")
        .to_owned();
    assert_eq!(slug, "trail_technique_montées_55_min");
    let stored = fixture
        .executor
        .resources
        .repos()
        .workout_templates
        .get_user_workout_template(fixture.tenant, fixture.user_id, &slug)
        .await?
        .expect("the inline session is persisted as a user template");
    assert_eq!(stored.duration_minutes, 55);
    assert_eq!(stored.structure.len(), 1);
    assert!(!stored.is_compiled_in);
    assert_eq!(stored.tenant_id, Some(fixture.tenant.as_uuid()));

    // The coach's cue reaches the athlete's calendar as prose, and the step
    // reaches it as structure: 3300 s is 55 m in the DSL.
    let request = stub.only_request().await;
    assert!(
        request.contains("Trail continu: montées/descentes continues"),
        "the step note must reach the calendar entry; got: {request}"
    );
    assert!(
        request.contains("- Trail continu 55m Z2 Pace"),
        "the step must go out in the DSL; got: {request}"
    );
    Ok(())
}

#[tokio::test]
async fn re_prescribing_the_same_session_reuses_one_template_row() -> Result<()> {
    let stub = stub_live(555).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;

    for date in ["2026-09-18", "2026-09-25"] {
        let resp = fixture
            .prescribe(json!({ "session": trail_session(), "date": date }))
            .await?;
        assert!(resp.success, "push must succeed: {:?}", resp.error);
    }

    // Two calendar entries (one per date), one library row — the slug is unique
    // per athlete, so a second mint would have violated that index outright.
    assert_eq!(stub.requests().await.len(), 2, "one push per prescription");
    assert_eq!(fixture.ledger().await.len(), 2);
    assert_eq!(stub.events().await.len(), 2, "two entries on the calendar");
    let templates = fixture
        .executor
        .resources
        .repos()
        .workout_templates
        .list_user_workout_templates(fixture.tenant, fixture.user_id)
        .await?;
    assert_eq!(
        templates.len(),
        1,
        "the same named session must converge on one stored template"
    );
    Ok(())
}

#[tokio::test]
async fn replacing_a_prescription_changes_the_same_calendar_event_in_place() -> Result<()> {
    let stub = stub_live(987_654).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;

    let first = fixture
        .ok(
            "prescribe_workout",
            json!({ "template_slug": "long_run_z2", "date": "2026-09-15" }),
        )
        .await?;
    let first_id = first["prescription_id"].as_str().expect("id").to_owned();

    // The coach changes their mind: same slot, different session.
    let second = fixture
        .ok(
            "prescribe_workout",
            json!({ "template_slug": "vo2_5x3", "date": "2026-09-15", "replaces": first_id }),
        )
        .await?;
    let second_id = second["prescription_id"].as_str().expect("id").to_owned();
    assert_ne!(second_id, first_id, "a replacement is a new prescription");
    assert_eq!(
        second["provider_event_id"].as_str(),
        Some("987654"),
        "the calendar entry keeps its provider id"
    );
    assert_eq!(
        second["replaced_prescription_id"].as_str(),
        Some(first_id.as_str())
    );
    assert_eq!(second["status"].as_str(), Some("pushed"));

    // On the wire: a PUT to the existing event, carrying the new key and name.
    let lines = stub.request_lines().await;
    assert_eq!(
        lines,
        vec![
            format!("POST /api/v1/athlete/{ATHLETE_ID}/events"),
            format!("PUT /api/v1/athlete/{ATHLETE_ID}/events/987654"),
        ]
    );
    let put = stub.requests().await[1].clone();
    assert!(
        put.contains(&format!("\"external_id\":\"dravr:rx:{second_id}\"")),
        "got: {put}"
    );
    let new_name = second["name"]
        .as_str()
        .expect("the reply names the session");
    assert!(
        put.contains(new_name),
        "the new session's name must reach the entry; got: {put}"
    );
    assert_eq!(
        stub.events().await.len(),
        1,
        "one entry on the calendar, not two"
    );

    // In the ledger: the first row is superseded, the second is live and points
    // back at it.
    let rows = fixture.ledger().await;
    assert_eq!(rows.len(), 2);
    let old = rows
        .iter()
        .find(|r| r.id.to_string() == first_id)
        .expect("first row");
    let new = rows
        .iter()
        .find(|r| r.id.to_string() == second_id)
        .expect("second row");
    assert_eq!(old.status, "replaced");
    assert_eq!(new.status, "pushed");
    assert_eq!(new.replaces_id, Some(old.id));
    assert_eq!(new.provider_event_id.as_deref(), Some("987654"));
    assert_eq!(fixture.live().await.len(), 1);

    // The superseded row can no longer be replaced or withdrawn.
    let err = fixture
        .execute(
            "prescribe_workout",
            json!({ "template_slug": "long_run_z2", "date": "2026-09-15", "replaces": first_id }),
        )
        .await
        .expect_err("a replaced prescription is not live");
    assert!(format!("{err:?}").contains("replaced"), "got: {err:?}");
    Ok(())
}

#[tokio::test]
async fn withdrawing_a_prescription_deletes_the_event_and_marks_the_row() -> Result<()> {
    let stub = stub_live(987_654).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;

    let first = fixture
        .ok(
            "prescribe_workout",
            json!({ "template_slug": "long_run_z2", "date": "2026-09-15" }),
        )
        .await?;
    let prescription_id = first["prescription_id"].as_str().expect("id").to_owned();

    let gone = fixture
        .ok(
            "withdraw_prescribed_workout",
            json!({ "prescription_id": prescription_id }),
        )
        .await?;
    assert_eq!(gone["status"].as_str(), Some("withdrawn"));
    assert_eq!(gone["provider_event_id"].as_str(), Some("987654"));

    // On the wire: a bulk delete by id — never by date range, never by key.
    let lines = stub.request_lines().await;
    assert_eq!(
        lines[1],
        format!("PUT /api/v1/athlete/{ATHLETE_ID}/events/bulk-delete")
    );
    let delete = stub.requests().await[1].clone();
    assert!(delete.contains(r#"[{"id":987654}]"#), "got: {delete}");
    assert!(
        stub.events().await.is_empty(),
        "the entry is gone from the calendar"
    );

    let rows = fixture.ledger().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "withdrawn");
    assert!(fixture.live().await.is_empty());

    // Withdrawing again is refused in words that say why.
    let err = fixture
        .execute(
            "withdraw_prescribed_workout",
            json!({ "prescription_id": prescription_id }),
        )
        .await
        .expect_err("a withdrawn prescription is not on the calendar");
    assert!(format!("{err:?}").contains("withdrawn"), "got: {err:?}");
    assert_eq!(
        stub.requests().await.len(),
        2,
        "no second delete reaches the provider"
    );
    Ok(())
}

#[tokio::test]
async fn a_slug_that_names_nothing_is_rejected_before_any_push() -> Result<()> {
    let stub = stub_live(1).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;

    let err = fixture
        .prescribe(json!({ "template_slug": "not_a_real_template", "date": "2026-09-19" }))
        .await
        .expect_err("unknown slug must be rejected");
    assert!(
        format!("{err:?}").contains("not_a_real_template"),
        "the rejection must name the slug: {err:?}"
    );
    assert!(
        stub.requests().await.is_empty(),
        "an unresolvable slug must never reach the provider"
    );
    assert!(fixture.ledger().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn the_workout_must_be_named_exactly_once() -> Result<()> {
    let stub = stub_live(1).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;

    let neither = fixture
        .prescribe(json!({ "date": "2026-09-20" }))
        .await
        .expect_err("a prescription with no workout must be rejected");
    assert!(
        format!("{neither:?}").contains("template_slug"),
        "the rejection must say what to send: {neither:?}"
    );

    let both = fixture
        .prescribe(json!({
            "template_slug": "long_run_z2",
            "session": trail_session(),
            "date": "2026-09-20",
        }))
        .await
        .expect_err("an ambiguous prescription must be rejected");
    assert!(format!("{both:?}").contains("not both"), "got: {both:?}");

    assert!(stub.requests().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_session_with_no_steps_is_rejected() -> Result<()> {
    let stub = stub_live(1).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;

    let err = fixture
        .prescribe(json!({
            "session": {
                "name": "Empty",
                "sport": "run",
                "intensity_distribution": "polarized",
                "structure": [],
            },
            "date": "2026-09-21",
        }))
        .await
        .expect_err("a session with no steps has no duration to prescribe");
    assert!(
        format!("{err:?}").contains("at least one step"),
        "got: {err:?}"
    );
    assert!(stub.requests().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn an_athlete_without_intervals_icu_is_told_to_connect_it() -> Result<()> {
    let stub = stub_live(1).await;
    // Strava-connected only: past the dispatch chokepoint, short of a calendar.
    let fixture = Box::pin(fixture(&stub.base_url, false)).await?;

    let resp = fixture
        .prescribe(json!({ "template_slug": "long_run_z2", "date": "2026-09-22" }))
        .await;
    let rendered = match resp {
        Ok(response) => {
            assert!(
                !response.success,
                "a calendar-less athlete must not succeed"
            );
            format!("{:?}", response.error)
        }
        Err(err) => format!("{err:?}"),
    };
    assert!(
        rendered.to_lowercase().contains("intervals"),
        "the refusal must name the account to connect; got: {rendered}"
    );
    assert!(
        stub.requests().await.is_empty(),
        "nothing may be pushed without a linked account"
    );
    assert!(fixture.ledger().await.is_empty());
    Ok(())
}

// ── Plan push ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn pushing_a_plan_puts_every_future_session_on_the_calendar_and_reconciles_adjustments(
) -> Result<()> {
    let stub = stub_live(1000).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;
    let user = fixture.user_id;
    let monday = next_plan_monday();
    let d = |offset: i64| monday + Duration::days(offset);

    // Two weeks: one full microcycle with a focus, one sparse week without.
    fixture
        .save_weeks(
            true,
            &[
                (
                    d(0),
                    "Volume back up",
                    vec![
                        planned(d(0), "vélo", "Endurance 60 min, low HR", Some(60), "Z2"),
                        rest_day(d(1)),
                        planned(
                            d(2),
                            "run",
                            "Intervals. Keep the recoveries easy",
                            Some(60),
                            "3x8min @ 88-93% FTP",
                        ),
                        planned(
                            d(3),
                            "muscu",
                            "- 20 min gainage\n- 20 min jambes",
                            Some(40),
                            "",
                        ),
                        rest_day(d(4)),
                        planned(d(5), "mtb", "Long ride", Some(150), "Z2"),
                        rest_day(d(6)),
                    ],
                ),
                (
                    d(7),
                    "",
                    vec![
                        planned(d(7), "vélo", "Tempo blocks", Some(75), "tempo"),
                        rest_day(d(8)),
                    ],
                ),
            ],
        )
        .await;

    // Before any push, the plan read shows an empty calendar that a push would fill.
    let before = fixture.ok("get_training_plan", json!({})).await?;
    assert_eq!(
        before["calendar"]["entries"].as_array().map(Vec::len),
        Some(0)
    );
    assert_eq!(before["calendar"]["pending"]["create"].as_u64(), Some(6));
    assert_eq!(before["calendar"]["stale"].as_bool(), Some(true));

    // ── First push: everything is created ──────────────────────────────
    let report = fixture.ok("push_training_plan", json!({})).await?;
    assert_eq!(
        report["created"].as_u64(),
        Some(6),
        "4 sessions + 1 week note + 1 session; got: {report}"
    );
    assert_eq!(report["updated"].as_u64(), Some(0));
    assert_eq!(report["unchanged"].as_u64(), Some(0));
    assert_eq!(report["removed"].as_u64(), Some(0));
    assert_eq!(report["failed"].as_array().map(Vec::len), Some(0));
    assert_eq!(report["provider"].as_str(), Some("intervals_icu"));

    let lines = stub.request_lines().await;
    assert_eq!(
        lines[0],
        format!("GET /api/v1/athlete/{ATHLETE_ID}/events"),
        "the window is read first"
    );
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("POST ")).count(),
        6,
        "one create per entry; got: {lines:?}"
    );
    assert_eq!(stub.events().await.len(), 6);

    // Each entry went out keyed, typed, timed, and — where the plan had an
    // in-grammar intensity — with one DSL step.
    let ride = stub
        .event_by_key(&CalendarKey::plan_day(user, d(0), 0))
        .await
        .expect("Monday ride on the calendar");
    assert_eq!(ride.body["type"].as_str(), Some("Ride"));
    assert_eq!(ride.body["moving_time"].as_u64(), Some(3600));
    assert_eq!(ride.body["category"].as_str(), Some("WORKOUT"));
    assert_eq!(
        ride.body["start_date_local"].as_str(),
        Some(format!("{}T00:00:00", iso(d(0))).as_str())
    );
    let ride_text = ride.body["description"].as_str().unwrap_or_default();
    assert!(
        ride_text.contains("Endurance 60 min, low HR"),
        "coach prose first; got: {ride_text}"
    );
    assert!(
        ride_text.contains("60m Z2"),
        "then the DSL step; got: {ride_text}"
    );

    // Interval structure the plan states only as prose goes out timed, with
    // no step — never as one wrong 60-minute block at 88-93 %.
    let intervals = stub
        .event_by_key(&CalendarKey::plan_day(user, d(2), 0))
        .await
        .expect("Wednesday intervals");
    let intervals_text = intervals.body["description"].as_str().unwrap_or_default();
    assert!(
        !intervals_text.contains("\n- "),
        "no DSL step for prose structure; got: {intervals_text}"
    );
    assert!(
        intervals_text.contains("3x8min @ 88-93% FTP"),
        "the coach's words survive; got: {intervals_text}"
    );
    assert_eq!(intervals.body["moving_time"].as_u64(), Some(3600));

    // Prose that looks like DSL is escaped, so Intervals.icu cannot turn the
    // coach's bullet list into two mystery steps.
    let strength = stub
        .event_by_key(&CalendarKey::plan_day(user, d(3), 0))
        .await
        .expect("Thursday strength");
    let strength_text = strength.body["description"].as_str().unwrap_or_default();
    assert!(
        strength_text.contains("– 20 min gainage"),
        "got: {strength_text}"
    );
    assert!(!strength_text.contains("- 20 min"), "got: {strength_text}");

    // The week's focus is a note pinned to the whole week; the second week
    // has none.
    let note = stub
        .event_by_key(&CalendarKey::plan_week_note(user, d(0)))
        .await
        .expect("week note");
    assert_eq!(note.body["category"].as_str(), Some("NOTE"));
    assert_eq!(note.body["for_week"].as_bool(), Some(true));
    assert_eq!(note.body["name"].as_str(), Some("Volume back up"));
    assert!(stub
        .event_by_key(&CalendarKey::plan_week_note(user, d(7)))
        .await
        .is_none());

    // The ledger mirrors the calendar: six live plan rows, each naming its
    // event and its week.
    let live = fixture.live().await;
    assert_eq!(live.len(), 6);
    assert!(live
        .iter()
        .all(|r| r.source.is_plan() && r.plan_week_id.is_some() && r.provider_event_id.is_some()));
    assert_eq!(
        live.iter()
            .filter(|r| r.source == CalendarEventSource::PlanWeekNote)
            .count(),
        1
    );

    let after = fixture.ok("get_training_plan", json!({})).await?;
    assert_eq!(
        after["calendar"]["entries"].as_array().map(Vec::len),
        Some(6)
    );
    assert_eq!(after["calendar"]["stale"].as_bool(), Some(false));

    // ── Second push, nothing changed: nothing is written ───────────────
    let again = fixture.ok("push_training_plan", json!({})).await?;
    assert_eq!(again["unchanged"].as_u64(), Some(6), "got: {again}");
    assert_eq!(again["created"].as_u64(), Some(0));
    let lines = stub.request_lines().await;
    assert_eq!(lines.len(), 8, "only one more GET; got: {lines:?}");
    assert_eq!(
        fixture.ledger().await.len(),
        6,
        "no new ledger rows for an unchanged push"
    );

    // ── The coach adjusts week one through the tool ────────────────────
    // Wednesday becomes rest, Saturday gets longer; the save reports that
    // the calendar is now behind, without touching it.
    let saved = fixture
        .ok(
            "save_training_plan",
            json!({
                "weeks": [{
                    "week_start": iso(d(0)),
                    "focus": "Volume back up",
                    "days": [
                        { "date": iso(d(0)), "sport": "vélo", "workout": "Endurance 60 min, low HR", "duration_min": 60, "intensity": "Z2" },
                        { "date": iso(d(1)), "sport": "rest", "workout": "Repos" },
                        { "date": iso(d(2)), "sport": "rest", "workout": "legs heavy, skip the intervals" },
                        { "date": iso(d(3)), "sport": "muscu", "workout": "- 20 min gainage\n- 20 min jambes", "duration_min": 40 },
                        { "date": iso(d(4)), "sport": "rest", "workout": "Repos" },
                        { "date": iso(d(5)), "sport": "mtb", "workout": "Long ride", "duration_min": 180, "intensity": "Z2" },
                        { "date": iso(d(6)), "sport": "rest", "workout": "Repos" }
                    ],
                    "adjustment_reason": "legs heavy after the race"
                }]
            }),
        )
        .await?;
    // Saturday changed and the week note changed (its adjustment reason is
    // part of it); Wednesday is gone; Monday, Thursday and next week's tempo
    // ride are as they were.
    assert_eq!(
        saved["calendar"]["stale"].as_bool(),
        Some(true),
        "got: {saved}"
    );
    assert_eq!(
        saved["calendar"]["pending"]["update"].as_u64(),
        Some(2),
        "got: {saved}"
    );
    assert_eq!(
        saved["calendar"]["pending"]["remove"].as_u64(),
        Some(1),
        "got: {saved}"
    );
    assert_eq!(
        saved["calendar"]["pending"]["unchanged"].as_u64(),
        Some(3),
        "got: {saved}"
    );
    assert_eq!(
        saved["calendar"]["pending"]["create"].as_u64(),
        Some(0),
        "got: {saved}"
    );
    assert_eq!(
        stub.request_lines().await.len(),
        8,
        "a save never writes to the calendar"
    );

    // Meanwhile the athlete edited next week's tempo ride on Intervals.icu,
    // and the coach also re-saved that week with a longer version.
    let tempo_key = CalendarKey::plan_day(user, d(7), 0);
    let tempo = stub.event_by_key(&tempo_key).await.expect("tempo ride");
    stub.touch(tempo.id, Utc::now() + Duration::hours(1)).await;
    fixture
        .save_weeks(
            false,
            &[(
                d(7),
                "",
                vec![
                    planned(d(7), "vélo", "Tempo blocks", Some(90), "tempo"),
                    rest_day(d(8)),
                ],
            )],
        )
        .await;

    // ── Third push: update, remove, and leave the edited one alone ─────
    let reconciled = fixture.ok("push_training_plan", json!({})).await?;
    assert_eq!(
        reconciled["updated"].as_u64(),
        Some(2),
        "Saturday and the week note; got: {reconciled}"
    );
    assert_eq!(
        reconciled["removed"].as_u64(),
        Some(1),
        "Wednesday; got: {reconciled}"
    );
    assert_eq!(
        reconciled["unchanged"].as_u64(),
        Some(2),
        "Monday and Thursday; got: {reconciled}"
    );
    assert_eq!(reconciled["created"].as_u64(), Some(0));
    let skipped = reconciled["skipped"].as_array().expect("skipped");
    assert_eq!(skipped.len(), 1, "the edited tempo ride; got: {reconciled}");
    assert_eq!(skipped[0]["external_id"].as_str(), Some(tempo_key.as_str()));
    assert_eq!(skipped[0]["reason"].as_str(), Some("edited_on_provider"));

    let lines = stub.request_lines().await;
    let new_lines = &lines[8..];
    assert_eq!(
        new_lines[0],
        format!("GET /api/v1/athlete/{ATHLETE_ID}/events")
    );
    assert!(
        new_lines
            .iter()
            .any(|l| l.starts_with("PUT ") && !l.ends_with("bulk-delete")),
        "got: {new_lines:?}"
    );
    assert!(
        new_lines.iter().any(|l| l.ends_with("bulk-delete")),
        "got: {new_lines:?}"
    );
    assert!(
        !new_lines.iter().any(|l| l.starts_with("POST ")),
        "nothing is created twice; got: {new_lines:?}"
    );

    assert_eq!(
        stub.events().await.len(),
        5,
        "Wednesday is gone from the calendar"
    );
    let saturday = stub
        .event_by_key(&CalendarKey::plan_day(user, d(5), 0))
        .await
        .expect("Saturday still on the calendar");
    assert_eq!(
        saturday.body["moving_time"].as_u64(),
        Some(10_800),
        "and it now carries 180 minutes"
    );
    let tempo_after = stub
        .event_by_key(&tempo_key)
        .await
        .expect("tempo ride left alone");
    assert_eq!(
        tempo_after.body["moving_time"].as_u64(),
        Some(4500),
        "the athlete's edit was not overwritten"
    );

    let live = fixture.live().await;
    assert_eq!(live.len(), 5);
    let saturday_row = live
        .iter()
        .find(|r| r.external_id.as_deref() == Some(CalendarKey::plan_day(user, d(5), 0).as_str()))
        .expect("Saturday's live row");
    assert!(
        saturday_row.replaces_id.is_some(),
        "the new row points at the one it superseded"
    );
    let ledger = fixture.ledger().await;
    let wednesday_key = CalendarKey::plan_day(user, d(2), 0);
    let wednesday_rows: Vec<&PrescribedWorkout> = ledger
        .iter()
        .filter(|r| r.external_id.as_deref() == Some(wednesday_key.as_str()))
        .collect();
    assert_eq!(wednesday_rows.len(), 1);
    assert_eq!(
        wednesday_rows[0].status, "withdrawn",
        "Wednesday's row is withdrawn"
    );

    // ── A ledger row lost while the calendar still has the event ───────
    // (a write that landed whose ledger write did not): the push adopts the
    // entry by its key instead of creating a twin.
    let monday_row = live
        .iter()
        .find(|r| r.external_id.as_deref() == Some(CalendarKey::plan_day(user, d(0), 0).as_str()))
        .expect("Monday's live row")
        .clone();
    fixture
        .executor
        .resources
        .repos()
        .prescribed_workouts
        .set_prescribed_workout_status(
            fixture.tenant,
            monday_row.id,
            PrescribedWorkout::STATUS_WITHDRAWN,
        )
        .await?;
    let adopted = fixture.ok("push_training_plan", json!({})).await?;
    assert_eq!(
        adopted["updated"].as_u64(),
        Some(1),
        "Monday adopted; got: {adopted}"
    );
    assert_eq!(
        adopted["created"].as_u64(),
        Some(0),
        "never a twin; got: {adopted}"
    );
    assert_eq!(stub.events().await.len(), 5);
    let monday_again = fixture
        .live()
        .await
        .into_iter()
        .find(|r| r.external_id.as_deref() == Some(CalendarKey::plan_day(user, d(0), 0).as_str()))
        .expect("Monday is live again");
    assert_eq!(
        monday_again.provider_event_id, monday_row.provider_event_id,
        "same provider event"
    );

    // ── Plan entries are not prescriptions ─────────────────────────────
    let err = fixture
        .execute(
            "withdraw_prescribed_workout",
            json!({ "prescription_id": monday_again.id.to_string() }),
        )
        .await
        .expect_err("a plan entry is withdrawn by adjusting the plan");
    assert!(format!("{err:?}").contains("training plan"), "got: {err:?}");
    Ok(())
}

#[tokio::test]
async fn a_structured_plan_day_reaches_the_calendar_as_repeat_blocks() -> Result<()> {
    // carnet#125: Phil's first real push landed four days with the right sport
    // and duration and a week header reading "Load 0" — interval structure
    // stated as prose reaches Intervals.icu as a timed entry it cannot build
    // a workout from. A day saved with steps goes out as the workout-builder
    // DSL, repeats grouped into one block, so the provider computes the load.
    let stub = stub_live(2000).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;
    let user = fixture.user_id;
    let monday = next_plan_monday();
    let d = |offset: i64| monday + Duration::days(offset);
    let workout = "Threshold 3x8. Keep the recoveries easy";
    let mut structured = planned(d(2), "vélo", workout, Some(61), "Z4");
    structured.steps = threshold_steps();
    fixture
        .save_weeks(
            true,
            &[(
                d(0),
                "",
                vec![
                    planned(d(0), "vélo", "Endurance", Some(60), "Z2"),
                    structured,
                ],
            )],
        )
        .await;

    let report = fixture.ok("push_training_plan", json!({})).await?;
    assert_eq!(report["created"].as_u64(), Some(2), "got: {report}");

    let key = CalendarKey::plan_day(user, d(2), 0);
    let event = stub
        .event_by_key(&key)
        .await
        .expect("the structured day is on the calendar");
    assert_eq!(
        event.body["moving_time"].as_u64(),
        Some(3660),
        "summed from the steps, not read from the day"
    );
    let text = event.body["description"].as_str().unwrap_or_default();
    assert!(text.starts_with(workout), "coach prose first; got: {text}");
    assert!(
        text.contains(
            "- Warm-up 15m Z1\n\n3x\n- Work 8m 88-93%\n- Recovery 4m Z1\n\n- Cool-down 10m Z1"
        ),
        "the workout-builder DSL with the repeats as one block; got: {text}"
    );

    // The coach re-saves the day without its steps: the calendar follows,
    // visibly — one update, a prose-only entry — rather than keeping a
    // structure the plan no longer states.
    fixture
        .save_weeks(
            false,
            &[(
                d(0),
                "",
                vec![
                    planned(d(0), "vélo", "Endurance", Some(60), "Z2"),
                    planned(d(2), "vélo", workout, Some(61), "Z4"),
                ],
            )],
        )
        .await;
    let again = fixture.ok("push_training_plan", json!({})).await?;
    assert_eq!(again["updated"].as_u64(), Some(1), "got: {again}");
    assert_eq!(again["unchanged"].as_u64(), Some(1), "got: {again}");
    let event = stub
        .event_by_key(&key)
        .await
        .expect("still on the calendar");
    let text = event.body["description"].as_str().unwrap_or_default();
    assert_eq!(event.body["moving_time"].as_u64(), Some(3660));
    assert!(
        !text.lines().any(|line| line == "3x"),
        "no repeat block once the steps are gone; got: {text}"
    );
    assert!(
        text.ends_with(&format!("\n\n- {} 61m Z4", SportType::Ride.display_name())),
        "back to the single intensity step, cued by the sport; got: {text}"
    );
    Ok(())
}

#[tokio::test]
async fn pushing_without_a_plan_is_refused_before_the_provider_is_called() -> Result<()> {
    let stub = stub_live(1).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;

    let err = fixture
        .execute("push_training_plan", json!({}))
        .await
        .expect_err("no plan, nothing to push");
    assert!(
        format!("{err:?}").contains("no active training plan"),
        "got: {err:?}"
    );
    assert!(stub.requests().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_refused_plan_push_records_every_failure_and_leaves_the_ledger_honest() -> Result<()> {
    let stub = stub_refusing("HTTP/1.1 503 Service Unavailable").await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;
    let monday = next_plan_monday();
    fixture
        .save_weeks(
            true,
            &[(
                monday,
                "",
                vec![
                    planned(monday, "run", "Easy", Some(30), "Z1"),
                    rest_day(monday + Duration::days(1)),
                ],
            )],
        )
        .await;

    // The window read itself is refused, so the push cannot even start — and
    // says so rather than pretending.
    let err = fixture
        .execute("push_training_plan", json!({}))
        .await
        .expect_err("a calendar that cannot be read cannot be reconciled");
    assert!(format!("{err:?}").contains("503"), "got: {err:?}");
    assert!(
        fixture.live().await.is_empty(),
        "nothing is recorded as live"
    );
    Ok(())
}

// ── Cooperative cancellation (MCP task handle) ─────────────────────────────

/// A cancel raised before the entry loop stops the push before ANY provider
/// write — and, critically, before the removal pass: a cut-short run's
/// `wanted` set is incomplete, so falling through to removal would bulk-delete
/// calendar entries the plan still wants. The follow-up push then reconciles
/// everything the cancelled run left, proving a partial run is always
/// recoverable.
#[tokio::test]
async fn a_cancelled_push_writes_nothing_and_never_reaches_the_removal_pass() -> Result<()> {
    let stub = stub_live(4000).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;
    let monday = next_plan_monday();
    let d = |offset: i64| monday + Duration::days(offset);

    // Two sessions (no week focus → no note entry), pushed to completion.
    fixture
        .save_weeks(
            true,
            &[(
                d(0),
                "",
                vec![
                    planned(d(0), "vélo", "Endurance", Some(60), "Z2"),
                    planned(d(2), "run", "Easy run", Some(40), ""),
                ],
            )],
        )
        .await;
    let report = fixture.ok("push_training_plan", json!({})).await?;
    assert_eq!(report["created"].as_u64(), Some(2), "got: {report}");

    // Adjust the plan: day 0 changes (a pending update), day 2 becomes rest
    // (its calendar entry becomes removable on the next push).
    fixture
        .save_weeks(
            false,
            &[(
                d(0),
                "",
                vec![
                    planned(d(0), "vélo", "Endurance, easier", Some(45), "Z1"),
                    rest_day(d(2)),
                ],
            )],
        )
        .await;

    // Cancelled push: the report says so, nothing lands, nothing is removed.
    let lines_before = stub.request_lines().await.len();
    let flag = Arc::new(AtomicBool::new(true));
    let report = scoped_with_cancel_flag(flag, fixture.ok("push_training_plan", json!({}))).await?;
    assert_eq!(report["cancelled"].as_bool(), Some(true), "got: {report}");
    assert_eq!(report["created"].as_u64(), Some(0));
    assert_eq!(report["updated"].as_u64(), Some(0));
    assert_eq!(report["removed"].as_u64(), Some(0));
    let lines = stub.request_lines().await;
    let new_lines = &lines[lines_before..];
    assert!(
        new_lines.len() == 1 && new_lines[0].starts_with("GET "),
        "a cancelled push reads the window and writes nothing; got: {new_lines:?}"
    );
    assert_eq!(fixture.live().await.len(), 2, "both ledger rows stay live");
    assert_eq!(stub.events().await.len(), 2, "both calendar entries stay");

    // The next (uncancelled) push converges: one update, one removal.
    let report = fixture.ok("push_training_plan", json!({})).await?;
    assert_eq!(report["cancelled"].as_bool(), Some(false), "got: {report}");
    assert_eq!(report["updated"].as_u64(), Some(1));
    assert_eq!(report["removed"].as_u64(), Some(1));
    assert_eq!(fixture.live().await.len(), 1);
    assert_eq!(stub.events().await.len(), 1);
    Ok(())
}

/// A cancel that lands while the push is mid-loop stops it at the next entry
/// boundary: the entry whose write already started finishes (cancellation is
/// cooperative, never mid-write), everything after it is left for the next
/// push, which completes the plan without duplicating what landed.
#[tokio::test]
async fn a_mid_push_cancel_stops_between_entries_and_the_next_push_finishes() -> Result<()> {
    let flag = Arc::new(AtomicBool::new(false));
    let stub = stub_cancelling_after_writes(5000, Arc::clone(&flag), 1).await;
    let fixture = Box::pin(fixture(&stub.base_url, true)).await?;
    let monday = next_plan_monday();
    let d = |offset: i64| monday + Duration::days(offset);

    // Three sessions; the stub raises the flag when the first create lands,
    // so the loop notices at the second entry's head.
    fixture
        .save_weeks(
            true,
            &[(
                d(0),
                "",
                vec![
                    planned(d(0), "vélo", "Endurance", Some(60), "Z2"),
                    planned(d(2), "run", "Intervals", Some(50), ""),
                    planned(d(4), "mtb", "Long ride", Some(120), "Z2"),
                ],
            )],
        )
        .await;

    let report = scoped_with_cancel_flag(
        Arc::clone(&flag),
        fixture.ok("push_training_plan", json!({})),
    )
    .await?;
    assert_eq!(report["cancelled"].as_bool(), Some(true), "got: {report}");
    assert_eq!(report["created"].as_u64(), Some(1), "got: {report}");
    let lines = stub.request_lines().await;
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("POST ")).count(),
        1,
        "exactly the one in-flight create landed; got: {lines:?}"
    );

    // The next push finishes the plan: the landed entry reads as unchanged,
    // the two never reached are created — no duplicates, no removals.
    let report = fixture.ok("push_training_plan", json!({})).await?;
    assert_eq!(report["cancelled"].as_bool(), Some(false), "got: {report}");
    assert_eq!(report["created"].as_u64(), Some(2), "got: {report}");
    assert_eq!(report["unchanged"].as_u64(), Some(1));
    assert_eq!(report["removed"].as_u64(), Some(0));
    assert_eq!(stub.events().await.len(), 3);
    Ok(())
}
