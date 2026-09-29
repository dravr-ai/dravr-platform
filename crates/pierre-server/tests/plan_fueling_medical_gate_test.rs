// ABOUTME: A medical/PAR-Q flag gates plan fuelling rates — the save refuses them, every read surface withholds them
// ABOUTME: get_training_plan, the plan card, the prompt block and the calendar note, each for a flagged and an unflagged athlete
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A plan day's `fueling` is carbohydrate, fluid and sodium per hour — amounts
//! to eat and drink. For an athlete with a medical flag on file those are the
//! clinician's to set, exactly as the nutrition tools' figures are. The save
//! refuses a payload carrying them, and a protocol stored before the flag was
//! raised reaches no surface: each one shows, in its place, that the amounts
//! are withheld and why. An athlete with no flag sees every rate unchanged.

mod common;

use std::sync::Arc;

use anyhow::Result;
use chrono::NaiveDate;
use pierre_chat_pipeline::stages::memory::{inject_training_plan, PlanPromptSources};
use pierre_contremaitre::TrainingCatalogueRegistry;
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::training_plans::PlannedDay;
use pierre_services::medical_flag::FiguresWithheld;
use pierre_services::parq;
use pierre_services::plan_calendar_push::desired_entries;
use pierre_services::plan_card::try_load_plan_card;
use pierre_services::plan_fueling::{FuelingDisclosure, FUELING_WITHHELD_CLAUSE};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use uuid::Uuid;

/// The day that carries the protocol.
const FUELLED_DATE: &str = "2026-07-15";

/// The athlete's civil date for the card and the prompt: inside the fuelled
/// day's week, so both render it.
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 14).expect("a valid date")
}

struct Athlete {
    user_id: Uuid,
    tenant: TenantId,
}

impl Athlete {
    fn tenant_str(&self) -> String {
        self.tenant.to_string()
    }
}

async fn resources() -> Result<Arc<ServerContext>> {
    common::init_server_config();
    common::init_test_http_clients();
    common::create_test_server_resources().await
}

fn executor(resources: &Arc<ServerContext>) -> UniversalToolExecutor {
    let shared: Arc<ServerContext> = Arc::clone(resources);
    UniversalToolExecutor::new(shared).with_scopes(OAuthScope::self_grant())
}

async fn athlete(resources: &ServerContext) -> Result<Athlete> {
    let email = format!("plan_fuel_{}@example.com", Uuid::new_v4());
    let (user_id, _user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter").await?;
    Ok(Athlete { user_id, tenant })
}

/// A PAR-Q "yes" on file — the flag the onboarding screen raises.
async fn raise_parq_flag(resources: &ServerContext, athlete: &Athlete) -> Result<()> {
    let raised = parq::persist_parq_flags(
        resources.common.repos.memory.as_ref(),
        athlete.tenant,
        &athlete.user_id.to_string(),
        &["heart_condition".to_owned()],
    )
    .await?;
    assert_eq!(raised, 1);
    Ok(())
}

fn request(tool: &str, athlete: &Athlete, params: Value) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: athlete.user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(athlete.tenant_str()),
    }
}

/// An outline and one week whose long ride carries a full protocol.
fn fuelled_plan() -> Value {
    json!({
        "agent_id": "endurance-coach",
        "outline": {
            "goal_race": {
                "name": "Big Red",
                "date": "2026-08-08",
                "discipline": "gravel",
                "priority": "A"
            },
            "strategy": "rebuild volume, then taper into the race",
            "phases": [
                {"kind": "build", "start": "2026-07-13", "weeks": 3, "intent": "volume back up"}
            ]
        },
        "weeks": [{
            "week_start": "2026-07-13",
            "focus": "volume back up",
            "days": [
                {"date": "2026-07-13", "sport": "rest", "workout": "off"},
                {"date": "2026-07-14", "sport": "gravel", "workout": "tempo 3x8min",
                 "duration_min": 60, "intensity": "Z3"},
                {"date": FUELLED_DATE, "sport": "mtb", "workout": "long endurance, eat from the first hour",
                 "duration_min": 150, "intensity": "Z2",
                 "fueling": {"carbs_g_per_h": 90, "fluid_ml_per_h": 700,
                             "sodium_mg_per_h": 600, "carb_source": "glucose:fructose 1:0.8"}}
            ]
        }]
    })
}

/// The same plan with the ride's fuelling described in words, as the refusal
/// asks.
fn plan_fuelled_in_words() -> Value {
    let mut plan = fuelled_plan();
    let ride = &mut plan["weeks"][0]["days"][2];
    ride.as_object_mut().expect("a day").remove("fueling");
    ride["workout"] = json!(
        "long endurance — fuel regularly from the first hour with the foods and drinks your \
         clinician has cleared"
    );
    plan
}

async fn save(executor: &UniversalToolExecutor, athlete: &Athlete, plan: Value) -> Result<()> {
    let saved = executor
        .execute_tool(request("save_training_plan", athlete, plan))
        .await?;
    assert!(saved.success, "the plan saves: {:?}", saved.error);
    Ok(())
}

async fn get_plan(executor: &UniversalToolExecutor, athlete: &Athlete) -> Result<Value> {
    let got = executor
        .execute_tool(request("get_training_plan", athlete, json!({})))
        .await?;
    assert!(got.success, "the plan reads: {:?}", got.error);
    Ok(got.result.expect("a result"))
}

/// The fuelled day as `get_training_plan` returns it.
fn fuelled_day(plan: &Value) -> &Value {
    plan["weeks"][0]["days"]
        .as_array()
        .expect("days")
        .iter()
        .find(|day| day["date"] == FUELLED_DATE)
        .expect("the fuelled day is returned")
}

/// The machine-readable statement every withholding surface carries.
fn assert_statement(statement: &Value) {
    assert_eq!(statement["reason"], "medical_flag", "{statement:#}");
    assert_eq!(statement["set_by"], "clinician", "{statement:#}");
    assert!(
        statement["note"]
            .as_str()
            .is_some_and(|note| note.contains("clinician")),
        "the note says who sets the amounts: {statement:#}"
    );
}

// ============================================================================
// save_training_plan
// ============================================================================

#[tokio::test]
async fn a_flagged_athletes_save_carrying_fuelling_is_refused_with_what_to_write_instead(
) -> Result<()> {
    let resources = resources().await?;
    let flagged = athlete(&resources).await?;
    raise_parq_flag(&resources, &flagged).await?;
    let executor = executor(&resources);

    let refused = executor
        .execute_tool(request("save_training_plan", &flagged, fuelled_plan()))
        .await;
    let message = match refused {
        Err(e) => e.to_string(),
        Ok(response) => {
            assert!(!response.success, "a fuelled save must be refused");
            response.error.unwrap_or_default()
        }
    };
    assert!(
        message.contains(&format!("fueling on {FUELLED_DATE} refused")),
        "the refusal names the day: {message}"
    );
    assert!(
        message.contains("clinician sets carbohydrate, fluid and sodium amounts"),
        "the refusal says why: {message}"
    );
    assert!(
        message.contains("describe fuelling in the day's `workout` in words"),
        "the refusal says what to write instead: {message}"
    );

    // Nothing landed — not even the outline the payload carried.
    let after = get_plan(&executor, &flagged).await?;
    assert!(
        after["plan"].is_null(),
        "a refused save writes nothing: {after:#}"
    );

    // The save the refusal asks for goes through.
    save(&executor, &flagged, plan_fuelled_in_words()).await?;
    let saved = get_plan(&executor, &flagged).await?;
    assert!(
        fuelled_day(&saved)["workout"]
            .as_str()
            .is_some_and(|w| w.contains("fuel regularly from the first hour")),
        "{saved:#}"
    );
    Ok(())
}

#[tokio::test]
async fn an_unflagged_athletes_fuelling_is_saved_and_read_back_whole() -> Result<()> {
    let resources = resources().await?;
    let clear = athlete(&resources).await?;
    let executor = executor(&resources);

    save(&executor, &clear, fuelled_plan()).await?;
    let plan = get_plan(&executor, &clear).await?;
    let fueling = &fuelled_day(&plan)["fueling"];
    assert_eq!(fueling["carbs_g_per_h"], 90.0, "{plan:#}");
    assert_eq!(fueling["fluid_ml_per_h"], 700.0, "{plan:#}");
    assert_eq!(fueling["sodium_mg_per_h"], 600.0, "{plan:#}");
    assert_eq!(fueling["carb_source"], "glucose:fructose 1:0.8", "{plan:#}");
    assert!(
        plan.get("fueling_withheld").is_none(),
        "no statement without a flag: {plan:#}"
    );
    Ok(())
}

// ============================================================================
// get_training_plan
// ============================================================================

#[tokio::test]
async fn get_training_plan_withholds_rates_stored_before_the_flag() -> Result<()> {
    let resources = resources().await?;
    let athlete = athlete(&resources).await?;
    let executor = executor(&resources);
    save(&executor, &athlete, fuelled_plan()).await?;
    raise_parq_flag(&resources, &athlete).await?;

    let plan = get_plan(&executor, &athlete).await?;
    assert!(
        fuelled_day(&plan).get("fueling").is_none(),
        "no stored rate reaches a flagged athlete's plan: {plan:#}"
    );
    let rendered = plan.to_string();
    for rate in [
        "carbs_g_per_h",
        "fluid_ml_per_h",
        "sodium_mg_per_h",
        "glucose:fructose",
    ] {
        assert!(!rendered.contains(rate), "{rate} leaked: {plan:#}");
    }
    let withheld = &plan["fueling_withheld"];
    assert_statement(&withheld["figures_withheld"]);
    assert_eq!(
        withheld["dates"],
        json!([FUELLED_DATE]),
        "the statement names the day whose rates were withheld: {plan:#}"
    );
    Ok(())
}

#[tokio::test]
async fn get_training_plan_states_the_withholding_even_before_a_plan_exists() -> Result<()> {
    let resources = resources().await?;
    let flagged = athlete(&resources).await?;
    let clear = athlete(&resources).await?;
    raise_parq_flag(&resources, &flagged).await?;
    let executor = executor(&resources);

    // The agent about to build this athlete's first plan learns up front that
    // its fuelling goes in words.
    let none_yet = get_plan(&executor, &flagged).await?;
    assert!(none_yet["plan"].is_null());
    assert_statement(&none_yet["fueling_withheld"]["figures_withheld"]);
    assert_eq!(none_yet["fueling_withheld"]["dates"], json!([]));

    let clear_none = get_plan(&executor, &clear).await?;
    assert!(
        clear_none.get("fueling_withheld").is_none(),
        "{clear_none:#}"
    );
    Ok(())
}

// ============================================================================
// The plan card
// ============================================================================

async fn card_day(resources: &ServerContext, athlete: &Athlete) -> Result<Value> {
    let card = try_load_plan_card(
        resources.common.repos.as_ref(),
        athlete.tenant,
        athlete.user_id,
        today(),
        &resources.mcp.messaging_strings_registry,
        "en",
    )
    .await?
    .expect("an active plan projects a card");
    let block = card.as_block("get_training_plan")?;
    let day = block["plan"]["weeks"]
        .as_array()
        .expect("weeks")
        .iter()
        .flat_map(|week| week["days"].as_array().expect("days").iter())
        .find(|day| day["date"] == FUELLED_DATE)
        .cloned()
        .expect("the card shows the fuelled day");
    Ok(day)
}

#[tokio::test]
async fn the_plan_card_shows_rates_unflagged_and_the_statement_flagged() -> Result<()> {
    let resources = resources().await?;
    let athlete = athlete(&resources).await?;
    let executor = executor(&resources);
    save(&executor, &athlete, fuelled_plan()).await?;

    let shown = card_day(&resources, &athlete).await?;
    assert_eq!(shown["fueling"]["carbs_g_per_h"], 90.0, "{shown:#}");
    assert!(shown.get("fueling_withheld").is_none(), "{shown:#}");

    raise_parq_flag(&resources, &athlete).await?;
    let withheld = card_day(&resources, &athlete).await?;
    assert!(withheld.get("fueling").is_none(), "{withheld:#}");
    assert_statement(&withheld["fueling_withheld"]);
    assert!(
        !withheld.to_string().contains("g_per_h"),
        "no rate on a flagged athlete's card: {withheld:#}"
    );
    Ok(())
}

#[test]
fn a_day_without_a_protocol_carries_neither_rates_nor_a_statement() {
    let withheld = FuelingDisclosure::Withheld(FiguresWithheld::medical_flag());
    let day: PlannedDay = serde_json::from_value(json!({
        "date": "2026-07-14", "sport": "gravel", "workout": "tempo", "intensity": "Z3"
    }))
    .expect("a day");
    assert_eq!(withheld.day_fueling(&day), (None, None));
    assert_eq!(FuelingDisclosure::Shown.day_fueling(&day), (None, None));
}

// ============================================================================
// The prompt's plan block
// ============================================================================

async fn prompt(resources: &ServerContext, athlete: &Athlete) -> String {
    inject_training_plan(
        PlanPromptSources {
            repos: resources.common.repos.as_ref(),
            catalogue: &TrainingCatalogueRegistry::new(),
        },
        &athlete.tenant_str(),
        &athlete.user_id.to_string(),
        Some("endurance-coach"),
        today(),
        false,
        "BASE PROMPT".to_owned(),
    )
    .await
}

fn ride_line(prompt: &str) -> &str {
    prompt
        .lines()
        .find(|line| line.starts_with(&format!("- {FUELLED_DATE}:")))
        .unwrap_or_else(|| panic!("the ride is rendered:\n{prompt}"))
}

#[tokio::test]
async fn the_prompt_block_renders_rates_unflagged_and_withholds_them_flagged() -> Result<()> {
    let resources = resources().await?;
    let athlete = athlete(&resources).await?;
    let executor = executor(&resources);
    save(&executor, &athlete, fuelled_plan()).await?;

    let shown = prompt(&resources, &athlete).await;
    assert!(
        ride_line(&shown).contains("· fuel: 90 g/h carbs · 700 ml/h fluid"),
        "{shown}"
    );
    assert!(
        !shown.contains("Fuelling amounts: a medical/PAR-Q flag"),
        "{shown}"
    );

    raise_parq_flag(&resources, &athlete).await?;
    let withheld = prompt(&resources, &athlete).await;
    let line = ride_line(&withheld);
    assert!(
        line.contains(&format!("· fuel: {FUELING_WITHHELD_CLAUSE}")),
        "the clause names why: {line}"
    );
    assert!(
        !withheld.contains("g/h"),
        "no rate in the prompt:\n{withheld}"
    );
    assert!(
        withheld.contains("Fuelling amounts: a medical/PAR-Q flag is on file"),
        "the block tells the agent up front, so its next save is not refused:\n{withheld}"
    );
    Ok(())
}

// ============================================================================
// The calendar push
// ============================================================================

#[tokio::test]
async fn the_calendar_note_carries_rates_unflagged_and_the_clause_flagged() -> Result<()> {
    let resources = resources().await?;
    let athlete = athlete(&resources).await?;
    let executor = executor(&resources);
    save(&executor, &athlete, fuelled_plan()).await?;
    let repos = resources.common.repos.as_ref();
    let plan = repos
        .training_plans
        .get_active_plan(&athlete.tenant_str(), &athlete.user_id.to_string())
        .await?
        .expect("the plan is stored");
    let weeks = repos
        .training_plans
        .list_plan_weeks(
            &athlete.tenant_str(),
            &athlete.user_id.to_string(),
            &plan.id,
            false,
        )
        .await?;
    let from = NaiveDate::from_ymd_opt(2026, 7, 13).expect("a valid date");
    let ride_note = |disclosure: &FuelingDisclosure| {
        desired_entries(athlete.user_id, &weeks, from, disclosure)
            .into_iter()
            .find(|entry| entry.session.date.format("%Y-%m-%d").to_string() == FUELLED_DATE)
            .expect("the ride is a calendar entry")
            .session
            .notes
    };

    let shown = FuelingDisclosure::for_athlete(repos, athlete.tenant, athlete.user_id).await?;
    assert_eq!(shown, FuelingDisclosure::Shown);
    let note = ride_note(&shown);
    assert!(note.contains("90 g/h carbs · 700 ml/h fluid"), "{note}");

    raise_parq_flag(&resources, &athlete).await?;
    let withheld = FuelingDisclosure::for_athlete(repos, athlete.tenant, athlete.user_id).await?;
    assert_eq!(
        withheld,
        FuelingDisclosure::Withheld(FiguresWithheld::medical_flag())
    );
    let note = ride_note(&withheld);
    assert!(note.contains(FUELING_WITHHELD_CLAUSE), "{note}");
    assert!(!note.contains("g/h"), "no rate on the calendar: {note}");
    assert!(
        note.starts_with("long endurance, eat from the first hour"),
        "the coach's prose stays: {note}"
    );
    Ok(())
}

// ============================================================================
// A clean re-screen gives the rates back
// ============================================================================

#[tokio::test]
async fn a_clean_rescreen_restores_the_rates_on_read_and_on_save() -> Result<()> {
    let resources = resources().await?;
    let athlete = athlete(&resources).await?;
    let executor = executor(&resources);
    save(&executor, &athlete, fuelled_plan()).await?;
    raise_parq_flag(&resources, &athlete).await?;
    assert!(get_plan(&executor, &athlete)
        .await?
        .get("fueling_withheld")
        .is_some());

    let retired = parq::retire_parq_flags(
        resources.common.repos.memory.as_ref(),
        athlete.tenant,
        &athlete.user_id.to_string(),
        &["heart_condition".to_owned()],
    )
    .await?;
    assert_eq!(retired, 1);

    let plan = get_plan(&executor, &athlete).await?;
    assert!(plan.get("fueling_withheld").is_none(), "{plan:#}");
    assert_eq!(fuelled_day(&plan)["fueling"]["carbs_g_per_h"], 90.0);
    save(&executor, &athlete, fuelled_plan()).await?;
    Ok(())
}
