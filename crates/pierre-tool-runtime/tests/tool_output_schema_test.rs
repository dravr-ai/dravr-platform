// ABOUTME: Asserts a tool's declared outputSchema actually validates the payload it returns
// ABOUTME: The schema is a promise to conforming MCP clients; this is what keeps it true
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! MCP requires a tool declaring `outputSchema` to answer with conforming
//! `structuredContent`. That makes the schema a promise about the payload, and
//! a promise nothing checks is one that quietly stops being true.
//!
//! Both halves come from the same Rust type — `answers_with::<T>` derives the
//! schema, `execute` serializes a `T` — so drift needs a deliberate effort.
//! These pin that the derivation is real: the declared schema is present, it
//! describes the fields the payload carries, and it rejects a payload missing
//! one.

use dravr_tronc::mcp::tool::McpTool;
use pierre_tool_runtime::implementations::goals::{
    AnalyzeGoalFeasibilityTool, SetGoalTool, SuggestGoalsTool, TrackProgressTool,
};
use pierre_tool_runtime::implementations::goals_output::{
    FeasibilityAnalysis, FeasibilityHistoricalContext, GoalFeasibilityResult, GoalSuggestionEntry,
    ProgressSummary, SetGoalResult, SuggestGoalsResult, TrackProgressResult,
};
use pierre_tool_runtime::implementations::memory::{
    CoachFollowupScheduleResult, CoachFollowupScheduleTool, CoachNoteAddResult, CoachNoteAddTool,
    RecallUserMemoryResult, RecallUserMemoryTool, RecalledFact, RememberFactResult,
    RememberFactTool,
};
use pierre_tool_runtime::implementations::nutrition::{
    AnalyzeMealNutritionResult, AnalyzeMealNutritionTool, CalculateDailyNutritionTool,
    DailyNutritionResult, FoodDetailsResult, FoodNutrientEntry, GetFoodDetailsTool,
    GetNutrientTimingTool, MealFoodEntry, NutrientTimingResult, PostWorkoutTiming,
    PreWorkoutTiming, ProteinDistribution, SearchFoodResult, SearchFoodTool,
};
use pierre_tool_runtime::implementations::playbooks::{
    ForgetPlaybookResult, ForgetPlaybookTool, InterventionEntry, ListCoachingPlaybooksResult,
    ListCoachingPlaybooksTool, PlaybookEntry, TriggerEntry,
};
use pierre_tool_runtime::implementations::verification::{VerifyClaimResult, VerifyClaimTool};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;

/// The schema a conforming client would validate `verify_claim` against.
fn declared_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(VerifyClaimResult))
        .expect("the derived schema serializes")
}

fn sample() -> VerifyClaimResult {
    VerifyClaimResult {
        verdict_id: "verdict-1".to_owned(),
        status: "supported".to_owned(),
        evidence_strength: "strong".to_owned(),
        layer_fired: "corpus".to_owned(),
        confidence: 0.82,
        explanation: "Two RCTs support the stated range.".to_owned(),
        evidence_refs: Some("doi:10.1000/example".to_owned()),
    }
}

#[test]
fn the_declared_schema_validates_the_payload_the_tool_returns() {
    let schema = declared_schema();
    let validator = jsonschema::validator_for(&schema).expect("the derived schema compiles");
    let payload = serde_json::to_value(sample()).expect("the payload serializes");

    assert!(
        validator.is_valid(&payload),
        "the tool's own payload must satisfy the schema it declares; schema:\n{schema:#}\npayload:\n{payload:#}"
    );
}

#[test]
fn the_schema_rejects_a_payload_missing_a_required_field() {
    // Without this the first test would pass against an empty schema, which is
    // exactly the failure mode a hand-written schema decays into.
    let validator = jsonschema::validator_for(&declared_schema()).expect("compiles");
    let missing_verdict_id = json!({
        "status": "supported",
        "evidence_strength": "strong",
        "layer_fired": "corpus",
        "confidence": 0.82,
        "explanation": "Two RCTs support the stated range.",
        "evidence_refs": null,
    });

    assert!(
        !validator.is_valid(&missing_verdict_id),
        "a schema that accepts a payload with no verdict_id is not describing anything"
    );
}

#[test]
fn an_absent_citation_is_still_valid() {
    // evidence_refs is Option: the layers below the corpus cite nothing, and a
    // schema that forbade that would reject the majority of real verdicts.
    let validator = jsonschema::validator_for(&declared_schema()).expect("compiles");
    let no_citations = VerifyClaimResult {
        evidence_refs: None,
        ..sample()
    };
    let payload = serde_json::to_value(no_citations).expect("serializes");

    assert!(
        validator.is_valid(&payload),
        "a verdict with no citations must still conform: {payload:#}"
    );
}

/// The loop the other tests leave open: they check the schema DERIVED from the
/// type, not the one the tool actually hands a client. A broken `answers_with`
/// — or a call site that forgot it — would leave every other assertion here
/// green while the tool declared nothing at all.
#[test]
fn the_tool_declares_the_schema_and_it_is_the_derived_one() {
    let declared = <VerifyClaimTool as McpTool<dyn ToolRuntime>>::definition(&VerifyClaimTool)
        .output_schema
        .expect("verify_claim must declare an outputSchema");

    assert_eq!(
        declared,
        declared_schema(),
        "the declared schema must be the one derived from VerifyClaimResult, not a hand-written copy"
    );

    let validator = jsonschema::validator_for(&declared).expect("the declared schema compiles");
    let payload = serde_json::to_value(sample()).expect("the payload serializes");
    assert!(
        validator.is_valid(&payload),
        "the schema the client receives must accept the payload the tool sends"
    );
}

/// The three things that must hold for any tool declaring a schema, in one
/// place so adding a tool costs one call rather than four copied tests.
///
/// Takes the DECLARED schema off the tool rather than deriving it here — that
/// is what catches a call site missing its `answers_with`.
fn assert_declares_and_accepts<T: serde::Serialize>(
    declared: Option<serde_json::Value>,
    derived: serde_json::Value,
    sample: &T,
    tool: &str,
) {
    let declared = declared.unwrap_or_else(|| panic!("{tool} must declare an outputSchema"));
    assert_eq!(
        declared, derived,
        "{tool}'s declared schema must be the one derived from its result type"
    );
    let validator =
        jsonschema::validator_for(&declared).unwrap_or_else(|e| panic!("{tool} schema: {e}"));
    let payload = serde_json::to_value(sample).expect("sample serializes");
    assert!(
        validator.is_valid(&payload),
        "{tool}: the declared schema rejected the payload the tool sends:\n{payload:#}"
    );
}

#[test]
fn list_coaching_playbooks_declares_a_schema_that_accepts_its_payload() {
    let sample = ListCoachingPlaybooksResult {
        playbooks: vec![PlaybookEntry {
            id: "pb-1".to_owned(),
            trigger: TriggerEntry {
                kind: "load_spike".to_owned(),
                sport: Some("run".to_owned()),
                magnitude: "high".to_owned(),
            },
            intervention: InterventionEntry {
                kind: "reduce_volume".to_owned(),
                magnitude: Some(20),
            },
            success_count: 18,
            failure_count: 2,
            neutral_count: 1,
            confidence: 0.71,
            last_outcome_at: Some("2026-09-01T10:00:00+00:00".to_owned()),
        }],
        count: 1,
    };

    assert_declares_and_accepts(
        <ListCoachingPlaybooksTool as McpTool<dyn ToolRuntime>>::definition(
            &ListCoachingPlaybooksTool,
        )
        .output_schema,
        serde_json::to_value(schemars::schema_for!(ListCoachingPlaybooksResult)).expect("derives"),
        &sample,
        "list_coaching_playbooks",
    );
}

#[test]
fn an_athlete_with_no_playbooks_still_conforms() {
    // The common first-run case, and the one an over-strict schema breaks.
    let empty = ListCoachingPlaybooksResult {
        playbooks: vec![],
        count: 0,
    };
    assert_declares_and_accepts(
        <ListCoachingPlaybooksTool as McpTool<dyn ToolRuntime>>::definition(
            &ListCoachingPlaybooksTool,
        )
        .output_schema,
        serde_json::to_value(schemars::schema_for!(ListCoachingPlaybooksResult)).expect("derives"),
        &empty,
        "list_coaching_playbooks (empty)",
    );
}

#[test]
fn forget_playbook_declares_a_schema_that_accepts_its_payload() {
    // `deleted` is a row COUNT, not a flag — 0 means "not yours or not there",
    // deliberately indistinguishable. The schema has to say number, not boolean.
    let sample = ForgetPlaybookResult {
        deleted: 0,
        playbook_id: "pb-missing".to_owned(),
    };
    assert_declares_and_accepts(
        <ForgetPlaybookTool as McpTool<dyn ToolRuntime>>::definition(&ForgetPlaybookTool)
            .output_schema,
        serde_json::to_value(schemars::schema_for!(ForgetPlaybookResult)).expect("derives"),
        &sample,
        "forget_playbook",
    );
}

#[test]
fn forget_playbook_schema_rejects_a_boolean_deleted() {
    // The wart typing exposed: the field reads like a flag and is a count. A
    // client that assumed boolean would have been wrong, and the schema now
    // says so out loud.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(ForgetPlaybookResult)).expect("derives"),
    )
    .expect("compiles");

    assert!(
        !validator.is_valid(&json!({"deleted": true, "playbook_id": "pb-1"})),
        "deleted is a count; a schema that accepts `true` is not describing it"
    );
}

#[test]
fn set_goal_declares_a_schema_that_accepts_its_payload() {
    let sample = SetGoalResult {
        goal_id: "goal-1".to_owned(),
        goal_type: "distance".to_owned(),
        target_value: 42.2,
        timeframe: "12 weeks".to_owned(),
        title: "First marathon".to_owned(),
        created_at: "2026-09-05T09:00:00+00:00".to_owned(),
        status: "created".to_owned(),
    };
    assert_declares_and_accepts(
        <SetGoalTool as McpTool<dyn ToolRuntime>>::definition(&SetGoalTool).output_schema,
        serde_json::to_value(schemars::schema_for!(SetGoalResult)).expect("derives"),
        &sample,
        "set_goal",
    );
}

#[test]
fn suggest_goals_declares_a_schema_that_accepts_its_payload() {
    let sample = SuggestGoalsResult {
        suggested_goals: vec![GoalSuggestionEntry {
            goal_type: "Distance".to_owned(),
            target_value: 21.1,
            difficulty: "Moderate".to_owned(),
            rationale: "Your last four weeks support a half.".to_owned(),
            estimated_timeline_days: 84,
            success_probability: 0.68,
        }],
        activities_analyzed: 37,
    };
    assert_declares_and_accepts(
        <SuggestGoalsTool as McpTool<dyn ToolRuntime>>::definition(&SuggestGoalsTool).output_schema,
        serde_json::to_value(schemars::schema_for!(SuggestGoalsResult)).expect("derives"),
        &sample,
        "suggest_goals",
    );
}

#[test]
fn track_progress_declares_a_schema_that_accepts_its_payload() {
    // projected_completion_days is None when the current rate supports no
    // projection, which is the common early-goal case.
    let sample = TrackProgressResult {
        goal_id: "goal-1".to_owned(),
        goal_type: "distance".to_owned(),
        current_value: 12.0,
        target_value: 42.2,
        unit: "km".to_owned(),
        progress_percentage: 28.4,
        on_track: true,
        days_remaining: 61,
        projected_completion_days: None,
        timeframe: "12 weeks".to_owned(),
        summary: ProgressSummary {
            total_activities: 9,
            total_distance_km: 78.5,
            total_duration_hours: 7.25,
        },
    };
    assert_declares_and_accepts(
        <TrackProgressTool as McpTool<dyn ToolRuntime>>::definition(&TrackProgressTool)
            .output_schema,
        serde_json::to_value(schemars::schema_for!(TrackProgressResult)).expect("derives"),
        &sample,
        "track_progress",
    );
}

#[test]
fn analyze_goal_feasibility_declares_a_schema_that_accepts_its_payload() {
    // The infeasible branch, because that is where the payload's adjusted_*
    // fields take their other values — a schema derived from the feasible case
    // alone would still cover it, and this proves it does.
    let sample = GoalFeasibilityResult {
        feasible: false,
        feasibility_score: 41.0,
        confidence_level: 0.6,
        risk_factors: vec!["Target requires 30% gain in 8 weeks".to_owned()],
        success_probability: 0.41,
        recommendations: vec!["Extend the timeframe to 16 weeks".to_owned()],
        adjusted_target: 34.0,
        adjusted_timeframe: 112,
        analysis: FeasibilityAnalysis {
            current_level: 28.0,
            target_value: 42.2,
            improvement_required_percent: 50.7,
            safe_improvement_capacity_percent: 21.4,
            timeframe_months: 2.6,
        },
        historical_context: FeasibilityHistoricalContext {
            activities_analyzed: 12,
            goal_type: "distance".to_owned(),
            data_quality: "limited".to_owned(),
        },
    };
    assert_declares_and_accepts(
        <AnalyzeGoalFeasibilityTool as McpTool<dyn ToolRuntime>>::definition(
            &AnalyzeGoalFeasibilityTool,
        )
        .output_schema,
        serde_json::to_value(schemars::schema_for!(GoalFeasibilityResult)).expect("derives"),
        &sample,
        "analyze_goal_feasibility",
    );
}

#[test]
fn coach_note_add_declares_a_schema_that_accepts_its_payload() {
    let sample = CoachNoteAddResult {
        note_id: "note-1".to_owned(),
        created_at: "2026-09-05T09:00:00+00:00".to_owned(),
    };
    assert_declares_and_accepts(
        <CoachNoteAddTool as McpTool<dyn ToolRuntime>>::definition(&CoachNoteAddTool).output_schema,
        serde_json::to_value(schemars::schema_for!(CoachNoteAddResult)).expect("derives"),
        &sample,
        "coach_note_add",
    );
}

#[test]
fn coach_followup_schedule_accepts_a_followup_with_no_due_date() {
    // due_at is None when the coach scheduled no date — the follow-up rides the
    // next conversation instead of a clock, and that is the common case.
    let sample = CoachFollowupScheduleResult {
        followup_id: "fu-1".to_owned(),
        status: "pending".to_owned(),
        due_at: None,
    };
    assert_declares_and_accepts(
        <CoachFollowupScheduleTool as McpTool<dyn ToolRuntime>>::definition(
            &CoachFollowupScheduleTool,
        )
        .output_schema,
        serde_json::to_value(schemars::schema_for!(CoachFollowupScheduleResult)).expect("derives"),
        &sample,
        "coach_followup_schedule",
    );
}

#[test]
fn remember_fact_declares_a_schema_that_accepts_its_payload() {
    let sample = RememberFactResult {
        fact_id: "fact-1".to_owned(),
        kind: "preference".to_owned(),
        confidence: 0.9,
    };
    assert_declares_and_accepts(
        <RememberFactTool as McpTool<dyn ToolRuntime>>::definition(&RememberFactTool).output_schema,
        serde_json::to_value(schemars::schema_for!(RememberFactResult)).expect("derives"),
        &sample,
        "remember_fact",
    );
}

#[test]
fn recall_user_memory_declares_a_schema_that_accepts_its_payload() {
    // source_msg_id is absent for facts that came from onboarding or a device
    // rather than a conversation, so the sample carries one of each.
    let sample = RecallUserMemoryResult {
        facts: vec![
            RecalledFact {
                id: "fact-1".to_owned(),
                kind: "injury".to_owned(),
                predicate_code: "has_injury".to_owned(),
                sentence: "Tu as une douleur au genou droit.".to_owned(),
                object: "genou droit".to_owned(),
                confidence: 0.88,
                source_msg_id: Some("msg-42".to_owned()),
                updated_at: "2026-09-04T18:30:00+00:00".to_owned(),
            },
            RecalledFact {
                id: "fact-2".to_owned(),
                kind: "equipment".to_owned(),
                predicate_code: "owns_equipment".to_owned(),
                sentence: "Tu as un capteur de puissance.".to_owned(),
                object: "capteur de puissance".to_owned(),
                confidence: 1.0,
                source_msg_id: None,
                updated_at: "2026-08-30T12:00:00+00:00".to_owned(),
            },
        ],
        count: 2,
    };
    assert_declares_and_accepts(
        <RecallUserMemoryTool as McpTool<dyn ToolRuntime>>::definition(&RecallUserMemoryTool)
            .output_schema,
        serde_json::to_value(schemars::schema_for!(RecallUserMemoryResult)).expect("derives"),
        &sample,
        "recall_user_memory",
    );
}

#[test]
fn calculate_daily_nutrition_declares_a_schema_that_accepts_its_payload() {
    let sample = DailyNutritionResult {
        bmr: 1680.0,
        tdee: 2740.0,
        protein_g: 150.0,
        carbs_g: 340.0,
        fat_g: 85.0,
        protein_percent: 22.0,
        carbs_percent: 50.0,
        fat_percent: 28.0,
        goal: "Endurance".to_owned(),
    };
    assert_declares_and_accepts(
        <CalculateDailyNutritionTool as McpTool<dyn ToolRuntime>>::definition(
            &CalculateDailyNutritionTool,
        )
        .output_schema,
        serde_json::to_value(schemars::schema_for!(DailyNutritionResult)).expect("derives"),
        &sample,
        "calculate_daily_nutrition",
    );
}

#[test]
fn get_nutrient_timing_declares_a_schema_that_accepts_its_payload() {
    let sample = NutrientTimingResult {
        pre_workout: PreWorkoutTiming {
            timing_hours_before: 2.0,
            carbs_g: 60.0,
            recommendations: vec!["Porridge and a banana".to_owned()],
        },
        post_workout: PostWorkoutTiming {
            timing_hours_after: 0.5,
            protein_g: 30.0,
            carbs_g: 70.0,
            recommendations: vec!["Recovery shake".to_owned()],
        },
        daily_protein_distribution: ProteinDistribution {
            meals_per_day: 4,
            protein_per_meal_g: 37.5,
            strategy: "even".to_owned(),
        },
        intensity_source: "explicit".to_owned(),
    };
    assert_declares_and_accepts(
        <GetNutrientTimingTool as McpTool<dyn ToolRuntime>>::definition(&GetNutrientTimingTool)
            .output_schema,
        serde_json::to_value(schemars::schema_for!(NutrientTimingResult)).expect("derives"),
        &sample,
        "get_nutrient_timing",
    );
}

#[test]
fn search_food_declares_a_schema_that_accepts_the_vendor_shape() {
    // `foods` is USDA's own type, forwarded rather than projected, so this is
    // the test that notices if their shape stops matching what we publish.
    let sample = SearchFoodResult {
        foods: vec![],
        returned_count: 0,
        total_hits: 0,
        page_number: 1,
        page_size: 25,
        total_pages: 0,
        has_more: false,
    };
    assert_declares_and_accepts(
        <SearchFoodTool as McpTool<dyn ToolRuntime>>::definition(&SearchFoodTool).output_schema,
        serde_json::to_value(schemars::schema_for!(SearchFoodResult)).expect("derives"),
        &sample,
        "search_food (no matches)",
    );
}

#[test]
fn get_food_details_accepts_a_food_with_no_stated_serving() {
    // USDA states no serving size for plenty of foods; a schema that required
    // one would reject them.
    let sample = FoodDetailsResult {
        fdc_id: 173_944,
        description: "Oats, raw".to_owned(),
        data_type: "SR Legacy".to_owned(),
        nutrients: vec![FoodNutrientEntry {
            nutrient_id: 1003,
            name: "Protein".to_owned(),
            amount: 16.9,
            unit: "G".to_owned(),
        }],
        serving_size: None,
        serving_size_unit: None,
    };
    assert_declares_and_accepts(
        <GetFoodDetailsTool as McpTool<dyn ToolRuntime>>::definition(&GetFoodDetailsTool)
            .output_schema,
        serde_json::to_value(schemars::schema_for!(FoodDetailsResult)).expect("derives"),
        &sample,
        "get_food_details",
    );
}

#[test]
fn analyze_meal_nutrition_declares_a_schema_that_accepts_its_payload() {
    let sample = AnalyzeMealNutritionResult {
        total_calories: 620.0,
        total_protein_g: 34.0,
        total_carbs_g: 78.0,
        total_fat_g: 18.0,
        foods: vec![MealFoodEntry {
            fdc_id: 173_944,
            description: "Oats, raw".to_owned(),
            grams: 80.0,
        }],
    };
    assert_declares_and_accepts(
        <AnalyzeMealNutritionTool as McpTool<dyn ToolRuntime>>::definition(
            &AnalyzeMealNutritionTool,
        )
        .output_schema,
        serde_json::to_value(schemars::schema_for!(AnalyzeMealNutritionResult)).expect("derives"),
        &sample,
        "analyze_meal_nutrition",
    );
}
