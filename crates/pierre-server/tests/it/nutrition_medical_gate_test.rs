// ABOUTME: A medical/PAR-Q flag withholds prescriptive nutrition figures in code, on every transport
// ABOUTME: Flagged athletes get guidance plus figures_withheld; unflagged athletes get the figures unchanged
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The agent prompt already tells the model to give no amounts to an athlete
//! with a medical flag. A direct MCP client and a prompt-less agent call the
//! same tools, so these drive the tools themselves: a flagged athlete's answer
//! carries `figures_withheld` and no amount, an unflagged athlete's answer is
//! the tool's own shape.

use crate::common;

use pierre_core::transport::TransportPolicy;
use std::sync::Arc;

use anyhow::Result;
use chrono::{Duration, TimeZone, Utc};
use dravr_cageux::nutrition_calculator::{
    ExerciseEnergyExpenditure, Gender, HeartRateEnergyInputs,
};
use dravr_cageux::training_load::FormBand;
use dravr_cageux::zones::HrZones;
use pierre_core::models::{
    Activity, ActivityBuilder, PhysiologicalSex, SportType, TenantId, UserPhysiologicalProfile,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_services::parq;
use pierre_tool_runtime::implementations::analytics::{
    generate_nutrition_recommendations, load_nutrition_athlete, recovery_actions_for,
    NutritionAthlete,
};
use pierre_tool_runtime::implementations::nutrition_gate::{
    nutrition_figures_gate, FiguresWithheld,
};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use uuid::Uuid;

// ============================================================================
// Fixtures
// ============================================================================

struct Athlete {
    user_id: Uuid,
    tenant: TenantId,
}

async fn resources() -> Result<Arc<ServerContext>> {
    common::init_server_config();
    common::init_test_http_clients();
    common::create_test_server_resources().await
}

async fn athlete(resources: &ServerContext, email: &str) -> Result<Athlete> {
    // `professional` so the plan-gated tools (`get_nutrient_timing`) are
    // enabled for the tenant and the call reaches the tool.
    let (user_id, _user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await?;
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

fn executor(resources: Arc<ServerContext>) -> UniversalToolExecutor {
    UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant())
}

fn request(
    tool: &str,
    athlete: &Athlete,
    tenant: Option<TenantId>,
    params: Value,
) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: athlete.user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: tenant.map(|t| t.to_string()),
    }
}

async fn call(executor: &UniversalToolExecutor, request: UniversalRequest) -> Value {
    let tool = request.tool_name.clone();
    let response = executor
        .execute_tool(request)
        .await
        .unwrap_or_else(|e| panic!("{tool} dispatch failed: {e}"));
    assert!(response.success, "{tool} failed: {:?}", response.error);
    response
        .result
        .unwrap_or_else(|| panic!("{tool} returned no result"))
}

/// Whether `line` states an amount: a number followed by a unit the gate
/// withholds (kcal, g, mg, mL, L, and their per-kg / per-hour forms).
fn states_an_amount(line: &str) -> bool {
    let lower = line.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if !c.is_ascii_digit() {
            continue;
        }
        let rest: String = chars[i + 1..]
            .iter()
            .skip_while(|ch| ch.is_ascii_digit() || **ch == '.' || **ch == '-' || **ch == ' ')
            .collect();
        for unit in [
            "kcal", "g ", "g/", "g,", "g.", "mg", "ml", "l ", "l/", "l of",
        ] {
            if rest.starts_with(unit) {
                return true;
            }
        }
        if rest == "g" || rest == "l" {
            return true;
        }
    }
    false
}

/// The withheld arm: the machine-readable statement, guidance with no amount,
/// and not one of the tool's figure keys.
fn assert_withheld(result: &Value, figure_keys: &[&str], tool: &str) {
    let withheld = &result["figures_withheld"];
    assert_eq!(withheld["reason"], "medical_flag", "{tool}: {result:#}");
    assert_eq!(withheld["set_by"], "clinician", "{tool}: {result:#}");
    assert!(
        withheld["note"]
            .as_str()
            .is_some_and(|n| n.contains("clinician")),
        "{tool}: the note must say who sets the figures: {result:#}"
    );
    let guidance = result["guidance"].as_array().expect("guidance is a list");
    assert!(
        !guidance.is_empty(),
        "{tool}: qualitative guidance replaces the figures"
    );
    for line in guidance {
        let line = line.as_str().expect("guidance lines are text");
        assert!(
            !states_an_amount(line),
            "{tool}: guidance states an amount: {line}"
        );
    }
    for key in figure_keys {
        assert!(
            result.get(*key).is_none(),
            "{tool}: `{key}` must be withheld for a flagged athlete: {result:#}"
        );
    }
}

#[test]
fn the_amount_detector_sees_what_the_gate_withholds() {
    assert!(states_an_amount("about 21 g of protein"));
    assert!(states_an_amount("1.25-1.5 L of water"));
    assert!(states_an_amount("1-1.2 g/kg per hour"));
    assert!(states_an_amount("600 mg of sodium"));
    assert!(states_an_amount("2400 kcal"));
    assert!(!states_an_amount("1-3 hours beforehand"));
    assert!(!states_an_amount(
        "Spread protein across 3-4 meals through the day."
    ));
}

// ============================================================================
// The gate itself
// ============================================================================

#[tokio::test]
async fn the_gate_reads_the_medical_flag_under_the_calls_tenant() -> Result<()> {
    let resources = resources().await?;
    let repos = resources.common.repos.as_ref();
    let clear = athlete(&resources, "gate-clear@example.com").await?;
    let flagged = athlete(&resources, "gate-flagged@example.com").await?;
    raise_parq_flag(&resources, &flagged).await?;

    assert_eq!(
        nutrition_figures_gate(repos, Some(clear.tenant), clear.user_id).await?,
        None,
        "no flag, no gate"
    );
    assert_eq!(
        nutrition_figures_gate(repos, Some(flagged.tenant), flagged.user_id).await?,
        Some(FiguresWithheld::medical_flag())
    );
    // A tenant the flag was not raised under does not see it: the gate reads
    // the scope the dossier and the prompt read, never across tenants.
    assert_eq!(
        nutrition_figures_gate(repos, Some(clear.tenant), flagged.user_id).await?,
        None
    );
    // A call that names no tenant reads every tenant the athlete belongs to,
    // so an MCP client without a tenant claim cannot slip past the flag.
    assert_eq!(
        nutrition_figures_gate(repos, None, flagged.user_id).await?,
        Some(FiguresWithheld::medical_flag())
    );
    assert_eq!(
        nutrition_figures_gate(repos, None, clear.user_id).await?,
        None
    );
    Ok(())
}

#[tokio::test]
async fn a_tool_raised_or_stale_medical_flag_still_gates() -> Result<()> {
    let resources = resources().await?;
    let repos = resources.common.repos.as_ref();
    let athlete = athlete(&resources, "gate-stale@example.com").await?;

    // A coach tool's flag, whose freshness horizon has passed: the athlete has
    // not re-screened, so the answer on file still stands.
    repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: athlete.tenant,
            user_id: &athlete.user_id.to_string(),
            agent_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Medical,
            pillar: None,
            predicate_code: PredicateCode::Flagged,
            object: "type 1 diabetes",
            confidence: 0.9,
            source: FactSource::Coach,
            valid_until: Some(Utc::now() - Duration::days(30)),
            source_msg_id: None,
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await?;

    assert_eq!(
        nutrition_figures_gate(repos, Some(athlete.tenant), athlete.user_id).await?,
        Some(FiguresWithheld::medical_flag())
    );
    Ok(())
}

// ============================================================================
// calculate_daily_nutrition
// ============================================================================

fn daily_params() -> Value {
    json!({
        "weight_kg": 70.0,
        "height_cm": 175.0,
        "age": 35,
        "gender": "female",
        "activity_level": "very_active",
        "training_goal": "endurance_performance"
    })
}

const DAILY_FIGURES: &[&str] = &[
    "bmr",
    "tdee",
    "protein_g",
    "carbs_g",
    "fat_g",
    "protein_percent",
    "carbs_percent",
    "fat_percent",
];

#[tokio::test]
async fn calculate_daily_nutrition_answers_with_figures_without_a_flag() -> Result<()> {
    let resources = resources().await?;
    let clear = athlete(&resources, "daily-clear@example.com").await?;
    let executor = executor(resources);

    let result = call(
        &executor,
        request(
            "calculate_daily_nutrition",
            &clear,
            Some(clear.tenant),
            daily_params(),
        ),
    )
    .await;
    for key in DAILY_FIGURES {
        assert!(
            result[*key].as_f64().is_some_and(|v| v > 0.0),
            "{key}: {result:#}"
        );
    }
    assert!(result.get("figures_withheld").is_none(), "{result:#}");
    assert!(result.get("guidance").is_none(), "{result:#}");
    Ok(())
}

#[tokio::test]
async fn calculate_daily_nutrition_withholds_figures_for_a_flagged_athlete() -> Result<()> {
    let resources = resources().await?;
    let flagged = athlete(&resources, "daily-flagged@example.com").await?;
    raise_parq_flag(&resources, &flagged).await?;
    let executor = executor(resources);

    let with_tenant = call(
        &executor,
        request(
            "calculate_daily_nutrition",
            &flagged,
            Some(flagged.tenant),
            daily_params(),
        ),
    )
    .await;
    assert_withheld(&with_tenant, DAILY_FIGURES, "calculate_daily_nutrition");

    // The direct-client shape: no tenant on the call at all.
    let tenantless = call(
        &executor,
        request("calculate_daily_nutrition", &flagged, None, daily_params()),
    )
    .await;
    assert_withheld(
        &tenantless,
        DAILY_FIGURES,
        "calculate_daily_nutrition (no tenant)",
    );
    Ok(())
}

#[tokio::test]
async fn a_flagged_athlete_still_gets_input_errors() -> Result<()> {
    // The gate is not a way around validation: bad input is still refused.
    let resources = resources().await?;
    let flagged = athlete(&resources, "daily-invalid@example.com").await?;
    raise_parq_flag(&resources, &flagged).await?;
    let executor = executor(resources);

    let response = executor
        .execute_tool(request(
            "calculate_daily_nutrition",
            &flagged,
            Some(flagged.tenant),
            json!({ "weight_kg": 70.0 }),
        ))
        .await;
    let refused = match response {
        Ok(r) => !r.success,
        Err(_) => true,
    };
    assert!(
        refused,
        "missing parameters are refused for a flagged athlete too"
    );
    Ok(())
}

// ============================================================================
// get_nutrient_timing
// ============================================================================

fn timing_params() -> Value {
    json!({ "workout_intensity": "high", "weight_kg": 70.0, "daily_protein_g": 126.0 })
}

#[tokio::test]
async fn get_nutrient_timing_answers_with_figures_without_a_flag() -> Result<()> {
    let resources = resources().await?;
    let clear = athlete(&resources, "timing-clear@example.com").await?;
    let executor = executor(resources);

    let result = call(
        &executor,
        request(
            "get_nutrient_timing",
            &clear,
            Some(clear.tenant),
            timing_params(),
        ),
    )
    .await;
    assert!(result["pre_workout"]["carbs_g"]
        .as_f64()
        .is_some_and(|g| g > 0.0));
    assert!(result["post_workout"]["protein_g"]
        .as_f64()
        .is_some_and(|g| g > 0.0));
    assert!(result["daily_protein_distribution"]["protein_per_meal_g"]
        .as_f64()
        .is_some_and(|g| g > 0.0));
    assert!(result.get("figures_withheld").is_none(), "{result:#}");
    Ok(())
}

#[tokio::test]
async fn get_nutrient_timing_withholds_figures_for_a_flagged_athlete() -> Result<()> {
    let resources = resources().await?;
    let flagged = athlete(&resources, "timing-flagged@example.com").await?;
    raise_parq_flag(&resources, &flagged).await?;
    let executor = executor(resources);

    let result = call(
        &executor,
        request(
            "get_nutrient_timing",
            &flagged,
            Some(flagged.tenant),
            timing_params(),
        ),
    )
    .await;
    assert_withheld(
        &result,
        &["pre_workout", "post_workout", "daily_protein_distribution"],
        "get_nutrient_timing",
    );
    Ok(())
}

// ============================================================================
// get_recipe_constraints
// ============================================================================

fn recipe_params() -> Value {
    json!({
        "tdee": 2600.0,
        "meal_timing": "post_training",
        "dietary_restrictions": ["gluten_free"],
        "max_prep_time_mins": 20
    })
}

#[tokio::test]
async fn get_recipe_constraints_answers_with_targets_without_a_flag() -> Result<()> {
    let resources = resources().await?;
    let clear = athlete(&resources, "recipe-clear@example.com").await?;
    let executor = executor(resources);

    let result = call(
        &executor,
        request(
            "get_recipe_constraints",
            &clear,
            Some(clear.tenant),
            recipe_params(),
        ),
    )
    .await;
    assert!(
        result["calories"].as_f64().is_some_and(|k| k > 0.0),
        "{result:#}"
    );
    assert!(
        result["protein_g"].as_f64().is_some_and(|g| g > 0.0),
        "{result:#}"
    );
    assert_eq!(result["tdee_based"], true);
    assert!(result.get("figures_withheld").is_none(), "{result:#}");
    Ok(())
}

#[tokio::test]
async fn get_recipe_constraints_withholds_targets_for_a_flagged_athlete() -> Result<()> {
    let resources = resources().await?;
    let flagged = athlete(&resources, "recipe-flagged@example.com").await?;
    raise_parq_flag(&resources, &flagged).await?;
    let executor = executor(resources);

    let result = call(
        &executor,
        request(
            "get_recipe_constraints",
            &flagged,
            Some(flagged.tenant),
            recipe_params(),
        ),
    )
    .await;
    assert_withheld(
        &result,
        &[
            "calories",
            "protein_g",
            "carbs_g",
            "fat_g",
            "prompt_hint",
            "tdee",
        ],
        "get_recipe_constraints",
    );
    // What is not a figure still reaches the recipe generator.
    let guidance = result["guidance"].to_string();
    assert!(guidance.contains("gluten_free"), "{guidance}");
    assert!(guidance.contains("20 minutes"), "{guidance}");
    assert!(guidance.contains("Post-training"), "{guidance}");
    Ok(())
}

// ============================================================================
// generate_recommendations (nutrition)
// ============================================================================

fn hard_long_ride() -> Activity {
    ActivityBuilder::new(
        "ride-1".to_owned(),
        "Long ride".to_owned(),
        SportType::Ride,
        Utc.with_ymd_and_hms(2026, 9, 28, 8, 0, 0).single().unwrap(),
        2 * 3600,
        "strava".to_owned(),
    )
    .average_heart_rate(165)
    .calories(1400)
    .distance_meters(60_000.0)
    .build()
}

/// The zones of the athlete riding it (resting 50, maximum 190, no `VO2max`
/// saved): 165 bpm sits in their zone 4, a hard session.
fn riders_zones() -> HrZones {
    HrZones::for_athlete(None, 50, 190).unwrap()
}

#[test]
fn nutrition_recommendations_scale_recovery_protein_to_stored_weight() {
    let zones = riders_zones();
    assert_eq!(zones.classify(165).zone, 4);
    let answer = generate_nutrition_recommendations(
        &hard_long_ride(),
        Some(&zones),
        &NutritionAthlete {
            weight_kg: Some(70.0),
            ..NutritionAthlete::default()
        },
    );
    let targets = answer
        .macronutrient_targets
        .as_ref()
        .expect("a stored weight yields targets");
    // 0.3 g/kg after key sessions (Thomas, Erdman & Burke 2016).
    assert!((targets.protein_g - 21.0).abs() < f64::EPSILON);
    assert!((targets.protein_g_per_kg - 0.3).abs() < f64::EPSILON);
    // 1-1.2 g/kg/h speedy refuelling for the first 4 h.
    assert!((targets.refuel_carbohydrates_g_per_h_min - 70.0).abs() < f64::EPSILON);
    assert!((targets.refuel_carbohydrates_g_per_h_max - 84.0).abs() < f64::EPSILON);
    assert!((targets.body_weight_kg - 70.0).abs() < f64::EPSILON);

    assert!(
        answer.recommendations[0].contains("about 21 g of protein"),
        "{:?}",
        answer.recommendations
    );
    assert!(answer.recommendations[0].starts_with("Within about 2 hours"));
    assert!(answer.recommendations[1].contains("about 70-84 g per hour"));
    assert!(answer.recommendations[1].contains("less than 8 hours away"));
    assert!(answer.figures_withheld.is_none());

    // No part of the answer keeps the 30-minute urgency the position stands
    // do not support: the window is 0-2 h.
    let rendered = serde_json::to_string(&answer).unwrap();
    assert!(!rendered.contains("30 minutes"), "{rendered}");
    assert!(!rendered.contains("0-15 min"), "{rendered}");
    assert!(!rendered.contains("hydration_ml"), "{rendered}");
    assert_eq!(
        answer.meal_suggestions.len(),
        4,
        "a hard session adds the endurance option"
    );
    for meal in &answer.meal_suggestions {
        assert!(meal.timing.contains("2 hours"), "{}", meal.timing);
        assert!((20..=40).contains(&meal.protein_g), "{} g", meal.protein_g);
    }
}

#[test]
fn nutrition_recommendations_state_the_per_kg_dose_without_a_weight() {
    let zones = riders_zones();
    let answer = generate_nutrition_recommendations(
        &hard_long_ride(),
        Some(&zones),
        &NutritionAthlete::default(),
    );
    assert!(
        answer.macronutrient_targets.is_none(),
        "no weight on file, no gram targets invented"
    );
    assert!(answer.recommendations[0].contains("0.25-0.3 g of protein per kg"));
    assert!(answer.recommendations[0].contains("roughly 20-40 g"));
    assert!(answer.recommendations[1]
        .contains("1-1.2 g of carbohydrate per kg of body weight per hour"));
}

#[test]
fn nutrition_recommendations_withhold_figures_for_a_flagged_athlete() {
    let zones = riders_zones();
    let answer = generate_nutrition_recommendations(
        &hard_long_ride(),
        Some(&zones),
        &NutritionAthlete {
            weight_kg: Some(70.0),
            figures_withheld: Some(FiguresWithheld::medical_flag()),
            ..NutritionAthlete::default()
        },
    );
    assert_eq!(
        answer.figures_withheld,
        Some(FiguresWithheld::medical_flag())
    );
    assert!(answer.macronutrient_targets.is_none());
    assert!(
        answer.meal_suggestions.is_empty(),
        "every suggestion carries gram counts"
    );
    assert_ne!(answer.recommendations, [] as [String; 0]);
    for line in &answer.recommendations {
        assert!(
            !states_an_amount(line),
            "an amount reached a flagged athlete: {line}"
        );
    }
    assert!(answer
        .recommendations
        .iter()
        .any(|l| l.contains("clinician")));

    let value = serde_json::to_value(&answer).unwrap();
    assert_eq!(value["figures_withheld"]["reason"], "medical_flag");
    assert_eq!(value["figures_withheld"]["set_by"], "clinician");
    assert!(value.get("macronutrient_targets").is_none());
    // The session itself is still described.
    assert_eq!(value["activity_summary"]["calories"], 1400.0);
}

#[test]
fn the_session_intensity_is_read_on_the_athletes_zones_and_absent_without_them() {
    let athlete = NutritionAthlete::default();
    let zones = riders_zones();
    let with_zones = generate_nutrition_recommendations(&hard_long_ride(), Some(&zones), &athlete);
    assert!(
        with_zones
            .key_insights
            .iter()
            .any(|line| line.contains("Workout intensity: high")),
        "{:?}",
        with_zones.key_insights
    );
    assert_eq!(with_zones.meal_suggestions.len(), 4);

    // The same ride for an athlete with no saved heart rates: 165 bpm is
    // graded by nobody's zones, and no published rule grades a session by its
    // length, so the intensity is absent and says why.
    let without = generate_nutrition_recommendations(&hard_long_ride(), None, &athlete);
    assert!(
        without.key_insights.iter().any(|line| line
            == "Workout intensity not graded: no saved resting and maximum heart rate to read the session's heart rate against"),
        "{:?}",
        without.key_insights
    );
    assert!(
        !without
            .key_insights
            .iter()
            .any(|line| line.starts_with("Workout intensity:")),
        "{:?}",
        without.key_insights
    );
    assert_eq!(without.meal_suggestions.len(), 3);

    // Zones but no heart rate on the session: absent, with that reason.
    let no_strap = ActivityBuilder::new(
        "ride-3".to_owned(),
        "Ride without a strap".to_owned(),
        SportType::Ride,
        Utc.with_ymd_and_hms(2026, 9, 30, 8, 0, 0).single().unwrap(),
        7200,
        "strava".to_owned(),
    )
    .calories(1400)
    .build();
    let unstrapped = generate_nutrition_recommendations(&no_strap, Some(&zones), &athlete);
    assert!(
        unstrapped
            .key_insights
            .iter()
            .any(|line| line == "Workout intensity not graded: the session recorded no heart rate"),
        "{:?}",
        unstrapped.key_insights
    );

    // A zone-2 heart rate on the same athlete's zones is a low session.
    let easy_bpm = zones.zones()[1].upper_bpm;
    let easy = ActivityBuilder::new(
        "ride-2".to_owned(),
        "Easy spin".to_owned(),
        SportType::Ride,
        Utc.with_ymd_and_hms(2026, 9, 29, 8, 0, 0).single().unwrap(),
        3600,
        "strava".to_owned(),
    )
    .average_heart_rate(u32::from(easy_bpm))
    .calories(500)
    .build();
    let low = generate_nutrition_recommendations(&easy, Some(&zones), &athlete);
    assert!(
        low.key_insights
            .iter()
            .any(|line| line.contains("Workout intensity: low")),
        "{:?}",
        low.key_insights
    );
}

/// The day's carbohydrate comes from cageux's reading of the 2016 joint ACSM
/// position table (Thomas, Erdman & Burke): duration and heart-rate intensity
/// pick the row. No line keys a rule to the session's length alone.
#[test]
fn the_days_carbohydrate_target_follows_the_position_table() {
    let zones = riders_zones();
    let weighed = NutritionAthlete {
        weight_kg: Some(70.0),
        ..NutritionAthlete::default()
    };
    let carbohydrate_line = |recommendations: &[String]| {
        recommendations
            .iter()
            .find(|line| {
                line.starts_with("Across the day") || line.starts_with("Daily carbohydrate")
            })
            .cloned()
            .unwrap_or_else(|| panic!("no daily carbohydrate line: {recommendations:?}"))
    };

    // Two hours in zone 4: the endurance row, 6-10 g/kg, 420-700 g at 70 kg.
    let hard = generate_nutrition_recommendations(&hard_long_ride(), Some(&zones), &weighed);
    let line = carbohydrate_line(&hard.recommendations);
    assert!(line.contains("an endurance training day"), "{line}");
    assert!(line.contains("6-10 g of carbohydrate per kg"), "{line}");
    assert!(line.contains("about 420-700 g"), "{line}");

    // No weight on file: the per-kg target only.
    let unweighed = generate_nutrition_recommendations(
        &hard_long_ride(),
        Some(&zones),
        &NutritionAthlete::default(),
    );
    let line = carbohydrate_line(&unweighed.recommendations);
    assert!(line.contains("6-10 g of carbohydrate per kg"), "{line}");
    assert!(!line.contains("about"), "{line}");

    // A zone-2 hour is the table's low intensity: 3-5 g/kg.
    let easy = ActivityBuilder::new(
        "ride-2".to_owned(),
        "Easy spin".to_owned(),
        SportType::Ride,
        Utc.with_ymd_and_hms(2026, 9, 29, 8, 0, 0).single().unwrap(),
        3600,
        "strava".to_owned(),
    )
    .average_heart_rate(u32::from(zones.zones()[1].upper_bpm))
    .build();
    let line = carbohydrate_line(
        &generate_nutrition_recommendations(&easy, Some(&zones), &weighed).recommendations,
    );
    assert!(line.contains("a light training day"), "{line}");
    assert!(line.contains("3-5 g of carbohydrate per kg"), "{line}");

    // A zone-3 hour is the table's "~1 h/d" moderate row: 5-7 g/kg.
    let steady = ActivityBuilder::new(
        "ride-4".to_owned(),
        "Tempo hour".to_owned(),
        SportType::Ride,
        Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).single().unwrap(),
        3600,
        "strava".to_owned(),
    )
    .average_heart_rate(u32::from(zones.zones()[2].upper_bpm))
    .build();
    let line = carbohydrate_line(
        &generate_nutrition_recommendations(&steady, Some(&zones), &weighed).recommendations,
    );
    assert!(line.contains("a moderate training day"), "{line}");
    assert!(line.contains("5-7 g of carbohydrate per kg"), "{line}");
    assert!(line.contains("about 350-490 g"), "{line}");

    // Four hours in zone 4 is the "> 4-5 h/d" row: 8-12 g/kg.
    let epic = ActivityBuilder::new(
        "ride-5".to_owned(),
        "Epic ride".to_owned(),
        SportType::Ride,
        Utc.with_ymd_and_hms(2026, 10, 2, 8, 0, 0).single().unwrap(),
        4 * 3600,
        "strava".to_owned(),
    )
    .average_heart_rate(165)
    .build();
    let line = carbohydrate_line(
        &generate_nutrition_recommendations(&epic, Some(&zones), &weighed).recommendations,
    );
    assert!(line.contains("a very high training day"), "{line}");
    assert!(line.contains("8-12 g of carbohydrate per kg"), "{line}");
    assert!(line.contains("about 560-840 g"), "{line}");

    // Ungraded intensity: no row, and the reason.
    let line = carbohydrate_line(
        &generate_nutrition_recommendations(&hard_long_ride(), None, &weighed).recommendations,
    );
    assert_eq!(
        line,
        "Daily carbohydrate target not set: no saved resting and maximum heart rate to read the session's heart rate against"
    );

    // The two unsourced duration rules are gone.
    let rendered = serde_json::to_string(&hard).unwrap();
    assert!(!rendered.contains("Extended duration"), "{rendered}");
    assert!(!rendered.contains("complete meal"), "{rendered}");
}

/// A ride whose device recorded heart rate but no calories.
fn ride_without_calories() -> Activity {
    ActivityBuilder::new(
        "ride-3".to_owned(),
        "Steady ride".to_owned(),
        SportType::Ride,
        Utc.with_ymd_and_hms(2026, 9, 30, 8, 0, 0).single().unwrap(),
        3600,
        "strava".to_owned(),
    )
    .average_heart_rate(150)
    .distance_meters(30_000.0)
    .build()
}

#[test]
fn without_device_calories_the_session_is_estimated_by_keytel_or_not_at_all() {
    let zones = riders_zones();
    let complete = NutritionAthlete {
        weight_kg: Some(70.0),
        age_years: Some(35),
        vo2_max: Some(50.0),
        sex: Some(Gender::Male),
        ..NutritionAthlete::default()
    };
    let estimated =
        generate_nutrition_recommendations(&ride_without_calories(), Some(&zones), &complete);
    let expected = ExerciseEnergyExpenditure::from_heart_rate(&HeartRateEnergyInputs {
        heart_rate_bpm: 150.0,
        body_mass_kg: 70.0,
        age_years: 35.0,
        gender: Gender::Male,
        vo2_max: Some(50.0),
    })
    .unwrap()
    .kcal_over(3600);
    let summary = estimated.activity_summary.as_ref().unwrap();
    assert_eq!(summary.calories, Some(expected.round()));
    assert_eq!(
        summary.calories_source.as_deref(),
        Some("keytel_with_vo2max")
    );
    assert!(summary.calories_unavailable.is_none());
    assert!(estimated.key_insights[0].contains(&format!("{expected:.0} calories")));

    // Without a VO2max the fitness-free equation is used.
    let no_vo2 = NutritionAthlete {
        vo2_max: None,
        ..complete.clone()
    };
    let fitness_free =
        generate_nutrition_recommendations(&ride_without_calories(), Some(&zones), &no_vo2);
    assert_eq!(
        fitness_free
            .activity_summary
            .as_ref()
            .unwrap()
            .calories_source
            .as_deref(),
        Some("keytel_without_vo2max")
    );

    // Missing the athlete's sex (none saved), there is no figure and the
    // reply says why — never a per-minute constant.
    let unknown_sex = NutritionAthlete {
        sex: None,
        ..complete.clone()
    };
    let absent =
        generate_nutrition_recommendations(&ride_without_calories(), Some(&zones), &unknown_sex);
    let summary = absent.activity_summary.as_ref().unwrap();
    assert_eq!(summary.calories, None);
    assert!(summary.calories_source.is_none());
    assert!(
        summary
            .calories_unavailable
            .as_deref()
            .is_some_and(|reason| reason.contains("sex")),
        "{summary:?}"
    );
    assert!(absent.key_insights[0].starts_with("Calories not estimated:"));

    // An age outside Keytel's 18-45 sample is refused, not extrapolated.
    let older = NutritionAthlete {
        age_years: Some(60),
        ..complete
    };
    let refused =
        generate_nutrition_recommendations(&ride_without_calories(), Some(&zones), &older);
    assert_eq!(refused.activity_summary.as_ref().unwrap().calories, None);

    // A device figure always wins.
    let device = generate_nutrition_recommendations(&hard_long_ride(), Some(&zones), &unknown_sex);
    let summary = device.activity_summary.as_ref().unwrap();
    assert_eq!(summary.calories, Some(1400.0));
    assert_eq!(summary.calories_source.as_deref(), Some("device"));
}

#[tokio::test]
async fn the_nutrition_mode_reads_the_flag_and_the_stored_weight() -> Result<()> {
    let resources = resources().await?;
    let repos = resources.common.repos.as_ref();
    let clear = athlete(&resources, "reco-clear@example.com").await?;
    let flagged = athlete(&resources, "reco-flagged@example.com").await?;
    raise_parq_flag(&resources, &flagged).await?;

    // Nothing stored yet: no weight, no flag.
    assert_eq!(
        load_nutrition_athlete(repos, Some(clear.tenant), clear.user_id).await?,
        NutritionAthlete::default()
    );

    let mut profile = UserPhysiologicalProfile::new(clear.user_id, SportType::Ride);
    profile.weight = Some(68.0);
    profile.age = Some(34);
    profile.vo2_max = Some(52.0);
    profile.sex = Some(PhysiologicalSex::Female);
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(clear.tenant, clear.user_id, &profile)
        .await?;
    assert_eq!(
        load_nutrition_athlete(repos, None, clear.user_id).await?,
        NutritionAthlete {
            weight_kg: Some(68.0),
            figures_withheld: None,
            age_years: Some(34),
            vo2_max: Some(52.0),
            // The saved sex selects Keytel's equation.
            sex: Some(Gender::Female),
        }
    );

    let gated = load_nutrition_athlete(repos, Some(flagged.tenant), flagged.user_id).await?;
    assert_eq!(
        gated.figures_withheld,
        Some(FiguresWithheld::medical_flag())
    );
    assert_eq!(gated.weight_kg, None, "a withheld answer needs no weight");
    Ok(())
}

// ============================================================================
// generate_recommendations (recovery)
// ============================================================================

#[test]
fn recovery_actions_advise_fluid_by_thirst_not_a_daily_volume() {
    for band in [
        FormBand::Fresh,
        FormBand::Detraining,
        FormBand::InsufficientHistory,
        FormBand::DeepFatigue,
        FormBand::HeavyBlock,
        FormBand::Productive,
        FormBand::Balanced,
    ] {
        for action in recovery_actions_for(band) {
            assert!(!states_an_amount(action), "{band:?}: {action}");
        }
    }
    assert!(recovery_actions_for(FormBand::Fresh).contains(&"Drink to thirst through the day"));
}
