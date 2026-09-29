// ABOUTME: The readiness rail end to end — an adjusted week is read against the flavour's ladder and the verdict logged
// ABOUTME: An athlete with no history sits at maintain, and maintain does not allow threshold work; nothing refuses

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Log-only, like the compliance rail beside it: the save must succeed
//! exactly as before, and what changes is one record per actionable week
//! carrying the level and the days it no longer allows.
//!
//! These tests seed no health data on purpose. A fresh athlete has no
//! chronic base and raises no alert, which is the ladder's own resting
//! state — P2, maintain — and the interesting thing about P2 is that
//! `polarized-classic` does not list `threshold` among the purposes it
//! allows. So the shipped catalogue, unaided, produces a real substitution.

use anyhow::Result;
use dravr_contremaitre::training;
use dravr_tronc::mcp::tool::McpTool;
use pierre_core::models::agents::AgentCategory;
use pierre_core::models::periodization::{Flavour, ReadinessLevel, WorkoutPurpose};
use pierre_core::models::{ArtefactKind, PackageArtefact, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_tool_runtime::implementations::training_plans::GetTrainingPlanTool;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex, PoisonError};
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use uuid::Uuid;

mod common;
mod helpers;

use helpers::agent_fixtures::publish_catalogue_agent_tagged;

#[derive(Clone, Debug)]
struct CapturedEvent {
    message: String,
    fields: HashMap<String, String>,
}

impl CapturedEvent {
    fn field(&self, name: &str) -> &str {
        self.fields.get(name).map_or("", String::as_str)
    }
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
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }
}

#[derive(Clone, Default)]
struct CaptureLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let message = visitor
            .fields
            .get("message")
            .cloned()
            .unwrap_or_else(|| event.metadata().name().to_owned());
        if let Ok(mut events) = self.events.lock() {
            events.push(CapturedEvent {
                message,
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

fn readiness_of<'a>(events: &'a [CapturedEvent], week_start: &str) -> &'a CapturedEvent {
    events
        .iter()
        .find(|e| e.message == "week readiness assessed" && e.field("week_start") == week_start)
        .unwrap_or_else(|| {
            panic!(
                "no readiness verdict for {week_start}; saw {:?}",
                events.iter().map(|e| &e.message).collect::<Vec<_>>()
            )
        })
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
    let email = format!("readiness_{}@example.com", Uuid::new_v4());
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
    }
}

/// A week far enough ahead that every day in it can still be changed.
fn week_start() -> String {
    (chrono::Utc::now().date_naive() + chrono::Duration::days(7))
        .format("%Y-%m-%d")
        .to_string()
}

fn day_in(week: &str, offset: i64) -> String {
    let start = chrono::NaiveDate::parse_from_str(week, "%Y-%m-%d").expect("a week start");
    (start + chrono::Duration::days(offset))
        .format("%Y-%m-%d")
        .to_string()
}

/// The catalogue's threshold template — purpose `threshold`.
fn threshold_day(date: &str) -> Value {
    json!({
        "date": date, "sport": "run", "workout": "4 x 8 min at threshold",
        "duration_min": 75, "intensity": "threshold",
        "template_slug": "threshold_4x8",
        "template_params": {"reps": 4, "work_seconds": 480, "rest_seconds": 120}
    })
}

fn easy_day(date: &str) -> Value {
    json!({"date": date, "sport": "run", "workout": "easy", "duration_min": 90, "intensity": "Z2"})
}

/// A day with no template at all — the ladder must report it, never swap it.
fn prose_day(date: &str) -> Value {
    json!({"date": date, "sport": "run", "workout": "run how you feel", "duration_min": 60, "intensity": "steady"})
}

fn plan_with(week: &str, days: &[Value]) -> Value {
    plan_under("polarized-classic", week, days)
}

/// A one-week build plan under `flavour`, saved by the catalogue's endurance
/// coach.
fn plan_under(flavour: &str, week: &str, days: &[Value]) -> Value {
    json!({
        "coach_id": "endurance-coach",
        "outline": {
            "goal_race": { "name": "Parkrun PB", "date": "2027-03-14", "discipline": "run_5k", "priority": "A" },
            "strategy": "polarised build, two hard days",
            "flavour": { "id": flavour, "selected_by": "coach", "override_reason": "house style" },
            "phases": [
                { "kind": "build", "start": week, "weeks": 6, "intent": "two hard days, the rest easy",
                  "target_hours": 8.0, "hard_sessions_max": 2 }
            ]
        },
        "weeks": [{ "week_start": week, "focus": "build week", "phase_index": 0, "days": days }]
    })
}

async fn save(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    payload: Value,
) -> Result<()> {
    let saved = executor
        .execute_tool(request("save_training_plan", payload, user_id, tenant_id))
        .await?;
    assert!(
        saved.success,
        "the rail is advisory and must never fail a save: {:?}",
        saved.error
    );
    Ok(())
}

#[tokio::test]
async fn an_athlete_with_no_history_maintains_and_threshold_work_is_substituted() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let week = week_start();
    let tuesday = day_in(&week, 1);

    let (events, guard) = setup_capture();
    save(
        &executor,
        user_id,
        &tenant_id,
        plan_with(
            &week,
            &[
                threshold_day(&tuesday),
                easy_day(&day_in(&week, 3)),
                easy_day(&day_in(&week, 5)),
            ],
        ),
    )
    .await?;
    drop(guard);

    let captured = events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let verdict = readiness_of(&captured, &week);

    assert_eq!(
        verdict.field("readiness"),
        "p2",
        "no chronic base and no alert is the ladder's resting state, not a block"
    );
    assert_eq!(
        verdict.field("alerts"),
        "[]",
        "an athlete with no series raises nothing"
    );
    assert!(
        verdict.field("substitutions").contains(tuesday.as_str()),
        "maintain does not allow threshold work: {}",
        verdict.field("substitutions")
    );
    assert_eq!(verdict.field("clear"), "false");
    Ok(())
}

#[tokio::test]
async fn a_week_the_level_already_allows_reads_clear() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let week = week_start();

    let (events, guard) = setup_capture();
    save(
        &executor,
        user_id,
        &tenant_id,
        plan_with(
            &week,
            &[easy_day(&day_in(&week, 1)), easy_day(&day_in(&week, 3))],
        ),
    )
    .await?;
    drop(guard);

    let captured = events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let verdict = readiness_of(&captured, &week);
    assert_eq!(verdict.field("readiness"), "p2");
    assert_eq!(
        verdict.field("substitutions"),
        "[]",
        "nothing in an easy week is refused at maintain"
    );
    assert_eq!(verdict.field("hard_sessions"), "0");
    Ok(())
}

#[tokio::test]
async fn a_day_with_no_template_is_reported_unclassified_never_substituted() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let week = week_start();
    let prose = day_in(&week, 2);

    let (events, guard) = setup_capture();
    save(
        &executor,
        user_id,
        &tenant_id,
        plan_with(&week, &[prose_day(&prose), easy_day(&day_in(&week, 4))]),
    )
    .await?;
    drop(guard);

    let captured = events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let verdict = readiness_of(&captured, &week);
    assert_eq!(
        verdict.field("unclassified_days"),
        "2",
        "neither day resolves a template, so neither is guessed at"
    );
    assert_eq!(
        verdict.field("substitutions"),
        "[]",
        "swapping a session nobody could classify would be guessing at the week"
    );
    Ok(())
}

#[tokio::test]
async fn a_week_already_run_is_not_read_at_all() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    // A week that ended before today: every day in it is history, and
    // substituting a session the athlete already completed helps nobody.
    let past = (chrono::Utc::now().date_naive() - chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();

    let (events, guard) = setup_capture();
    save(
        &executor,
        user_id,
        &tenant_id,
        plan_with(&past, &[threshold_day(&day_in(&past, 1))]),
    )
    .await?;
    drop(guard);

    let captured = events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert!(
        !captured
            .iter()
            .any(|e| e.message == "week readiness assessed"),
        "a week with no changeable day left must not be read"
    );
    Ok(())
}

#[tokio::test]
async fn the_rails_verdict_reaches_the_agent_when_it_asks_for_it() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let week = week_start();
    let tuesday = day_in(&week, 1);

    save(
        &executor,
        user_id,
        &tenant_id,
        plan_with(
            &week,
            &[threshold_day(&tuesday), easy_day(&day_in(&week, 3))],
        ),
    )
    .await?;

    // The hot path stays cheap: without the flag the block is absent, not
    // empty, so a turn asking "what am I doing this week" pays nothing.
    let plain = executor
        .execute_tool(request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach"}),
            user_id,
            &tenant_id,
        ))
        .await?;
    let plain = plain.result.expect("a plan");
    assert!(
        plain["state"].is_null(),
        "state must be absent unless asked for: {}",
        plain["state"]
    );

    // Asked for, the verdict both rails have been computing all along comes
    // back to the agent that has to act on it.
    let asked = executor
        .execute_tool(request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach", "include_state": true}),
            user_id,
            &tenant_id,
        ))
        .await?;
    let asked = asked.result.expect("a plan");
    let state = &asked["state"];
    assert!(!state.is_null(), "state was asked for: {asked}");

    assert_eq!(
        state["readiness"], "p2",
        "an athlete with no history maintains: {state}"
    );
    let subs = &state["readiness_weeks"][0]["substitutions"];
    assert_eq!(
        subs[0]["date"], tuesday,
        "the day maintain refuses is named: {state}"
    );
    assert_eq!(
        subs[0]["from"], "threshold",
        "and what it was asked to be: {state}"
    );
    assert_eq!(
        subs[0]["to"], "endurance",
        "replaced by the purpose this phase most wants that the level allows: {state}"
    );
    assert_eq!(
        subs[0]["reason"], "purpose_closed",
        "maintain closes threshold in this flavour, so the purpose is the reason: {state}"
    );
    assert!(
        subs[0].get("replacement").is_none(),
        "a closed purpose's `to` already names what to run: {state}"
    );

    // The compliance rail's verdict rides the same block.
    let compliance = &state["compliance_weeks"][0];
    assert_eq!(compliance["week_start"], week.as_str());
    for check in ["tid", "hard_sessions", "spacing", "volume", "week_loading"] {
        assert!(
            compliance[check].is_string(),
            "{check} must be reported: {compliance}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_plan_that_stops_covering_the_athlete_says_so() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // A phase that claims to be running today, with its only week long past:
    // the plan asserted it should be prescribing now, and it is not.
    let past = (chrono::Utc::now().date_naive() - chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();
    let mut payload = plan_with(&past, &[easy_day(&day_in(&past, 1))]);
    payload["outline"]["phases"][0]["start"] = json!(past);
    payload["outline"]["phases"][0]["weeks"] = json!(12);
    save(&executor, user_id, &tenant_id, payload).await?;

    let asked = executor
        .execute_tool(request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach", "include_state": true}),
            user_id,
            &tenant_id,
        ))
        .await?;
    let state = &asked.result.expect("a plan")["state"];
    let gaps: Vec<&str> = state["coverage_gaps"]
        .as_array()
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    assert!(
        gaps.contains(&"uncovered_today"),
        "the plan runs a phase today and no week covers today — that is the \
         signal a fortnight needs writing: {state}"
    );
    Ok(())
}

/// A run day that instantiates `slug`, whatever its purpose.
fn templated_day(date: &str, slug: &str) -> Value {
    json!({
        "date": date, "sport": "run", "workout": slug, "duration_min": 90,
        "intensity": "threshold", "template_slug": slug
    })
}

/// The plan's readiness state, as `get_training_plan` hands it to the agent.
async fn plan_state(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
) -> Result<Value> {
    let asked = executor
        .execute_tool(request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach", "include_state": true}),
            user_id,
            tenant_id,
        ))
        .await?;
    Ok(asked.result.expect("a plan")["state"].clone())
}

#[tokio::test]
async fn a_template_above_the_level_is_answered_with_an_easier_template_of_its_purpose(
) -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let week = week_start();
    let tuesday = day_in(&week, 1);

    // Maintain opens threshold under the Norwegian singles ladder, but the
    // double threshold day stacks two quality blocks and floors at build.
    // The purpose is open; only the template is above the level.
    let (events, guard) = setup_capture();
    save(
        &executor,
        user_id,
        &tenant_id,
        plan_under(
            "norwegian-singles-subthreshold",
            &week,
            &[
                templated_day(&tuesday, "double_threshold_day"),
                easy_day(&day_in(&week, 3)),
            ],
        ),
    )
    .await?;
    drop(guard);

    let captured = events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let logged = readiness_of(&captured, &week);
    assert!(
        logged.field("reasons").contains("template_above_level"),
        "the log names why the day moved: {}",
        logged.field("reasons")
    );

    let state = plan_state(&executor, user_id, &tenant_id).await?;
    assert_eq!(state["readiness"], "p2", "{state}");
    let subs = state["readiness_weeks"][0]["substitutions"]
        .as_array()
        .unwrap_or_else(|| panic!("the week names what moves: {state}"));
    assert_eq!(subs.len(), 1, "only the stacked day moves: {state}");
    let day = &subs[0];
    assert_eq!(day["date"], tuesday.as_str(), "{state}");
    assert_eq!(day["from"], "threshold", "{state}");
    assert_eq!(
        day["to"], "threshold",
        "the purpose is open, so the kernel keeps it: {state}"
    );
    assert_eq!(day["reason"], "template_above_level", "{state}");
    assert_eq!(
        day["replacement"],
        json!({
            "basis": "easier_template",
            "purpose": "threshold",
            "template": "threshold_4x8",
        }),
        "the bank's threshold session that maintain clears, written for the \
         day's own sport before one that only lists it as a variant: {state}"
    );
    Ok(())
}

#[test]
fn the_state_parameter_tells_the_agent_why_a_day_moves_and_what_replaces_it() {
    // The agent reads what `include_state` returns from this description
    // alone, so it names both reasons the kernel gives and the bank's answer.
    let definition = GetTrainingPlanTool.definition();
    let described = definition.input_schema["properties"]["include_state"]["description"]
        .as_str()
        .expect("include_state is described");
    for term in [
        "purpose_closed",
        "template_above_level",
        "easier template",
        "falls back to",
    ] {
        assert!(described.contains(term), "{term}: {described}");
    }
}

/// The catalogue's polarized flavour under a house id, with a ladder that
/// opens downhill work at maintain — a purpose whose only template in the
/// bank floors at build.
fn house_downhill_flavour() -> String {
    let (_, text) = training::FLAVOURS
        .iter()
        .find(|(stem, _)| *stem == "polarized-classic")
        .expect("the pinned catalogue carries polarized-classic");
    let mut flavour = Flavour::from_yaml(text).expect("the pinned flavour parses");
    "house-downhill".clone_into(&mut flavour.id);
    for level in [ReadinessLevel::P2, ReadinessLevel::P3] {
        flavour
            .readiness_substitution
            .get_mut(&level)
            .expect("the ladder has every level")
            .purposes
            .push(WorkoutPurpose::Downhill);
    }
    serde_yaml::to_string(&flavour).expect("a flavour serialises")
}

#[tokio::test]
async fn a_template_above_the_level_with_no_easier_one_falls_back_to_the_phase_purpose(
) -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::parse_str(&tenant_id)?;
    let repos = executor.resources.repos();
    let agent_id = publish_catalogue_agent_tagged(
        repos,
        user_id,
        tenant,
        "House Downhill",
        "You coach the house way.",
        AgentCategory::Training,
        vec!["trail".to_owned()],
    )
    .await;
    let flavour = PackageArtefact::parse(ArtefactKind::Flavour, &house_downhill_flavour())
        .expect("the house flavour is a flavour the kernel accepts");
    repos
        .agent_artefacts
        .replace_agent_artefacts(&tenant_id, &agent_id.to_string(), &[flavour])
        .await?;

    let week = week_start();
    let tuesday = day_in(&week, 1);
    let mut payload = plan_under(
        "house-downhill",
        &week,
        &[
            templated_day(&tuesday, "downhill_repeats"),
            easy_day(&day_in(&week, 3)),
        ],
    );
    payload["agent_id"] = json!(agent_id.to_string());
    save(&executor, user_id, &tenant_id, payload).await?;

    let state = plan_state(&executor, user_id, &tenant_id).await?;
    assert_eq!(state["readiness"], "p2", "{state}");
    let day = &state["readiness_weeks"][0]["substitutions"][0];
    assert_eq!(day["date"], tuesday.as_str(), "{state}");
    assert_eq!(
        day["reason"], "template_above_level",
        "the house ladder opens downhill at maintain; the template floors at build: {state}"
    );
    assert_eq!(
        day["replacement"],
        json!({
            "basis": "purpose_fallback",
            "purpose": "endurance",
            "template": "endurance",
        }),
        "no downhill session in the bank clears maintain, so the day falls \
         back to the build phase's heaviest purpose the level opens, with the \
         bank's template of it: {state}"
    );
    Ok(())
}

#[tokio::test]
async fn the_template_bank_says_how_a_session_grows() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let listed = executor
        .execute_tool(request(
            "list_workout_templates",
            json!({"sport": "run"}),
            user_id,
            &tenant_id,
        ))
        .await?;
    let listed = listed.result.expect("templates");
    let rows = listed["templates"].as_array().expect("a bank of templates");
    let threshold = rows
        .iter()
        .find(|r| r["slug"] == "threshold_4x8")
        .unwrap_or_else(|| panic!("threshold_4x8 is in the shipped bank: {listed}"));

    // Week 2 of a fortnight is week 1 progressed by one lever. Which lever,
    // and how many, is authored per template — and was read by nothing, so
    // the agent invented the step.
    let order: Vec<&str> = threshold["progression"]["order"]
        .as_array()
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    assert_eq!(
        order,
        vec!["add_rep", "lengthen_rep", "shorten_rest"],
        "the levers reach the agent in the order the bank pulls them: {threshold}"
    );
    assert_eq!(
        threshold["progression"]["max_weekly_step"], 1,
        "and how many may move in one week: {threshold}"
    );
    Ok(())
}
