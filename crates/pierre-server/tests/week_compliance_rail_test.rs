// ABOUTME: The compliance rail end to end — save_training_plan writes a week, training_plan.week_assessed reports its verdict
// ABOUTME: Three hard days under a two-hard-day phase reads off; a prose-only week reads seven unclassified and no TID guess; nothing refuses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The rail is Log-only: the save must succeed exactly as before, and what
//! changes is one event per saved week carrying the verdict. These tests
//! capture that event off the notify target and assert its words — the
//! same words the base-rate measurement counts.

use anyhow::Result;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use uuid::Uuid;

mod common;

#[derive(Clone, Debug)]
struct CapturedEvent {
    fields: HashMap<String, String>,
}

impl CapturedEvent {
    fn field(&self, name: &str) -> &str {
        self.fields.get(name).map_or("", String::as_str)
    }
}

#[derive(Clone, Default)]
struct CaptureLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Default)]
struct FieldVisitor {
    fields: HashMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != "notify" {
            return;
        }
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        if let Ok(mut events) = self.events.lock() {
            events.push(CapturedEvent {
                fields: visitor.fields,
            });
        }
    }
}

fn setup_capture() -> (Arc<Mutex<Vec<CapturedEvent>>>, DefaultGuard) {
    let capture = CaptureLayer::default();
    let events = Arc::clone(&capture.events);
    let guard = tracing_subscriber::registry().with(capture).set_default();
    (events, guard)
}

fn assessed<'a>(events: &'a [CapturedEvent], week_start: &str) -> &'a CapturedEvent {
    events
        .iter()
        .find(|e| {
            e.field("event") == "training_plan.week_assessed" && e.field("week_start") == week_start
        })
        .unwrap_or_else(|| panic!("no week_assessed for {week_start}: {events:?}"))
}

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("compliance_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?;
    Ok((user_id, tenant.id.to_string()))
}

fn request(tool: &str, params: Value, user_id: Uuid, tenant_id: &str) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant_id.to_owned()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

/// A day built from the catalogue's threshold template — a quality session.
fn threshold_day(date: &str) -> Value {
    json!({
        "date": date,
        "sport": "run",
        "workout": "4 x 8 min at threshold",
        "duration_min": 75,
        "intensity": "threshold",
        "template_slug": "threshold_4x8",
        "template_params": {"reps": 4, "work_seconds": 480, "rest_seconds": 120}
    })
}

/// An easy day with a label the grammar reads.
fn easy_day(date: &str) -> Value {
    json!({"date": date, "sport": "run", "workout": "easy", "duration_min": 90, "intensity": "Z2"})
}

/// A day written in prose: no template, no steps, no label the
/// grammar reads.
fn prose_day(date: &str) -> Value {
    json!({"date": date, "sport": "run", "workout": "run how you feel", "duration_min": 60, "intensity": "steady, keep it honest"})
}

/// A polarized plan with a build phase that states a time-in-zone target,
/// a cap of two hard sessions and eight hours, and one week of `days`.
fn plan_with(days: &[Value]) -> Value {
    json!({
        "agent_id": "endurance-coach",
        "outline": {
            "goal_race": { "name": "Parkrun PB", "date": "2026-11-14", "discipline": "run_5k", "priority": "A" },
            "strategy": "polarised build, two hard days",
            "flavour": { "id": "polarized-classic", "selected_by": "coach", "override_reason": "house style" },
            "phases": [
                {
                    "kind": "build", "start": "2026-09-14", "weeks": 6, "intent": "two hard days, the rest easy",
                    "target_hours": 8.0, "hard_sessions_max": 2,
                    "tid_target": {"z1": {"min": 0.75, "max": 0.85}, "z2": {"min": 0.0, "max": 0.05}, "z3": {"min": 0.15, "max": 0.20}}
                },
                {"kind": "taper", "start": "2026-10-26", "weeks": 3, "intent": "sharpen"}
            ]
        },
        "weeks": [
            { "week_start": "2026-09-14", "focus": "week one", "phase_index": 0, "days": days }
        ]
    })
}

#[tokio::test]
async fn three_hard_days_under_a_two_hard_day_phase_is_reported_and_still_saved() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let days = vec![
        threshold_day("2026-09-14"),
        easy_day("2026-09-15"),
        threshold_day("2026-09-16"),
        easy_day("2026-09-17"),
        threshold_day("2026-09-18"),
        json!({"date": "2026-09-19", "sport": "rest", "workout": "off"}),
    ];

    let (events, guard) = setup_capture();
    let saved = executor
        .execute_tool(request(
            "save_training_plan",
            plan_with(&days),
            user_id,
            &tenant_id,
        ))
        .await?;
    let captured = events.lock().expect("capture lock").clone();
    drop(guard);
    assert!(
        saved.success,
        "Log-only: the save must not refuse, got {:?}",
        saved.error
    );

    let week = assessed(&captured, "2026-09-14");
    assert_eq!(
        week.field("hard_sessions"),
        "off",
        "three threshold days under a cap of two: {week:?}"
    );
    assert_eq!(week.field("hard_days"), "3");
    assert_eq!(
        week.field("spacing"),
        "within",
        "the 14th, 16th and 18th are 48 h apart, the flavour's minimum"
    );
    assert_eq!(
        week.field("unclassified_days"),
        "0",
        "every day is a template or a parsed label"
    );
    assert_eq!(
        week.field("template_ranges"),
        "0",
        "reps 4, work 480, rest 120 sit inside threshold_4x8's ranges"
    );
    assert_eq!(
        week.field("volume"),
        "within",
        "3×75 + 2×90 = 405 min, 6.75 h against 8 h, is inside a fifth"
    );
    assert_eq!(
        week.field("tid"),
        "off",
        "three threshold sessions is not a polarized build week"
    );
    assert!(
        week.field("detail").contains("\"z2\""),
        "the detail carries the measured shares: {}",
        week.field("detail")
    );
    Ok(())
}

#[tokio::test]
async fn a_prose_only_week_reads_unclassified_and_guesses_no_time_in_zone() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let days: Vec<Value> = (14..=20)
        .map(|d| prose_day(&format!("2026-09-{d}")))
        .collect();

    let (events, guard) = setup_capture();
    let saved = executor
        .execute_tool(request(
            "save_training_plan",
            plan_with(&days),
            user_id,
            &tenant_id,
        ))
        .await?;
    let captured = events.lock().expect("capture lock").clone();
    drop(guard);
    assert!(saved.success, "{:?}", saved.error);

    let week = assessed(&captured, "2026-09-14");
    assert_eq!(week.field("unclassified_days"), "7");
    assert_eq!(
        week.field("tid"),
        "unmeasured",
        "nothing classifies, so nothing is guessed"
    );
    assert_eq!(
        week.field("hard_sessions"),
        "within",
        "zero hard days under a cap of two"
    );
    assert_eq!(week.field("spacing"), "unmeasured");
    assert_eq!(
        week.field("volume"),
        "within",
        "7 h against 8 h — durations are still durations"
    );
    Ok(())
}

#[tokio::test]
async fn a_weeks_only_adjustment_is_measured_with_the_previous_weeks_last_hard_day() -> Result<()> {
    // Week one ends on a hard Saturday; the adjustment adds week two with a
    // hard Monday. Within week two the spacing is fine; across the boundary
    // it is 48 h, exactly the minimum — so the rail must have read week one
    // to say "within" for the right reason, and a hard Sunday would flip it.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let week_one = vec![
        easy_day("2026-09-14"),
        easy_day("2026-09-16"),
        threshold_day("2026-09-19"),
    ];
    let saved = executor
        .execute_tool(request(
            "save_training_plan",
            plan_with(&week_one),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(saved.success, "{:?}", saved.error);

    let (events, guard) = setup_capture();
    let adjustment = executor
        .execute_tool(request(
            "save_training_plan",
            json!({
                "agent_id": "endurance-coach",
                "weeks": [{
                    "week_start": "2026-09-21", "focus": "week two", "phase_index": 0,
                    "adjustment_reason": "the second week, added after the first landed",
                    "days": [threshold_day("2026-09-21"), easy_day("2026-09-23"), threshold_day("2026-09-25")]
                }]
            }),
            user_id,
            &tenant_id,
        ))
        .await?;
    let captured = events.lock().expect("capture lock").clone();
    drop(guard);
    assert!(adjustment.success, "{:?}", adjustment.error);

    let reported: Vec<&str> = captured
        .iter()
        .filter(|e| e.field("event") == "training_plan.week_assessed")
        .map(|e| e.field("week_start"))
        .collect();
    assert_eq!(
        reported,
        vec!["2026-09-21"],
        "only the week this save wrote is reported"
    );
    let week = assessed(&captured, "2026-09-21");
    assert_eq!(
        week.field("spacing"),
        "within",
        "Saturday the 19th to Monday the 21st is 48 h: {week:?}"
    );
    assert_eq!(week.field("hard_sessions"), "within");
    Ok(())
}

/// An easy day of an exact length, for the volume arithmetic below.
fn easy_day_of(date: &str, minutes: u32) -> Value {
    json!({"date": date, "sport": "run", "workout": "easy", "duration_min": minutes, "intensity": "Z2"})
}

/// The build phase, now laid out from a catalogue skeleton and carrying a
/// loading pattern. `"3:1"` puts the recovery week at index 3; `run-5k-10k`
/// is where the size of its cut is authored.
fn plan_on_a_skeleton(days: &[Value], week_start: &str) -> Value {
    json!({
        "agent_id": "endurance-coach",
        "outline": {
            "goal_race": { "name": "Parkrun PB", "date": "2026-11-14", "discipline": "run_5k", "priority": "A" },
            "strategy": "polarised build, three up one down",
            "flavour": { "id": "polarized-classic", "selected_by": "coach", "override_reason": "house style" },
            "phases": [
                {
                    "kind": "build", "start": "2026-09-14", "weeks": 8, "intent": "three up, one down",
                    "target_hours": 8.0, "hard_sessions_max": 2,
                    "loading_pattern": "3:1", "skeleton_id": "run-5k-10k"
                }
            ]
        },
        "weeks": [
            { "week_start": week_start, "focus": "measured week", "phase_index": 0, "days": days }
        ]
    })
}

/// A 5.6-hour week — 4 x 84 min. Against the phase's 8 h target that is 30 %
/// down and reads `off`; against the recovery target it is exact.
fn five_point_six_hours(mondays: [&str; 4]) -> Vec<Value> {
    mondays.iter().map(|d| easy_day_of(d, 84)).collect()
}

/// The phase's loading pattern, the week's position in it, and the skeleton's
/// recovery cut all reach the kernel, and together they change the verdict.
///
/// Both saves are the same 5.6 hours against the same 8-hour phase target. The
/// only difference is the calendar. 2026-09-14 is the phase start — index 0, a
/// load week, measured against 8 h, so 5.6 h is 30 % down and reads `off`.
/// 2026-10-05 is three weeks later — index 3, which `"3:1"` makes the recovery
/// week, and `run-5k-10k` cuts a recovery week by 25-35 %, so the target
/// becomes 8 x (1 - 0.30) = 5.6 h and the same week is exactly on it.
///
/// The arithmetic is what makes this a real assertion: `VOLUME_TOLERANCE` is
/// 20 %, so a load week is within from 6.4 h and a recovery week from 4.48 h.
/// 5.6 h sits outside the first band and dead centre of the second. A build
/// passing `loading_pattern: None`, `week_index_in_phase: None`, or a
/// `recovery_week_cut` it never looked up — each of the three fields this
/// change wired — collapses the recovery target back to 8 h and returns `off`
/// twice.
#[tokio::test]
async fn a_recovery_week_is_measured_against_the_skeletons_cut() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let (events, guard) = setup_capture();
    executor
        .execute_tool(request(
            "save_training_plan",
            plan_on_a_skeleton(
                &five_point_six_hours(["2026-09-14", "2026-09-15", "2026-09-17", "2026-09-19"]),
                "2026-09-14",
            ),
            user_id,
            &tenant_id,
        ))
        .await?;
    executor
        .execute_tool(request(
            "save_training_plan",
            plan_on_a_skeleton(
                &five_point_six_hours(["2026-10-05", "2026-10-06", "2026-10-08", "2026-10-10"]),
                "2026-10-05",
            ),
            user_id,
            &tenant_id,
        ))
        .await?;
    drop(guard);

    let captured = events.lock().expect("capture mutex");
    let load_week = assessed(&captured, "2026-09-14").field("volume").to_owned();
    let recovery_week = assessed(&captured, "2026-10-05").field("volume").to_owned();

    assert_eq!(
        load_week, "off",
        "index 0 is a load week measured against the full 8 h, and 5.6 h is 30% under it"
    );
    assert_eq!(
        recovery_week, "within",
        "index 3 of a 3:1 phase is the recovery week; run-5k-10k cuts 25-35%, \
         so 5.6 h is on target — got {recovery_week}, load week read {load_week}"
    );
    Ok(())
}
