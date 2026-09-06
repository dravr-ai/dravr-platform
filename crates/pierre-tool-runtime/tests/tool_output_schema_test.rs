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
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::SportType;
use pierre_services::plan_calendar_push::PushReport;
use pierre_tool_runtime::conversions::Formatted;
use pierre_tool_runtime::implementations::admin::{
    AdminAssignCoachTool, AdminCreateSystemCoachTool, AdminDeleteSystemCoachTool,
    AdminGetSystemCoachTool, AdminListCoachAssignmentsTool, AdminListSystemCoachesTool,
    AdminUnassignCoachTool, AdminUpdateSystemCoachTool,
};
use pierre_tool_runtime::implementations::admin_output::{
    AdminAssignCoachResult, AdminCreateSystemCoachResult, AdminDeleteSystemCoachResult,
    AdminGetSystemCoachResult, AdminListCoachAssignmentsResult, AdminListSystemCoachesResult,
    AdminUnassignCoachResult, AdminUpdateSystemCoachResult, CoachAssignmentEntry, SystemCoachEntry,
};
use pierre_tool_runtime::implementations::analytics::output::{
    ActivityMetricsResult, BestPerformance, CompareActivitiesResult, DayFrequency,
    HardEasyPatternResult, InsufficientPatternData, IntensityDistribution, MetricComparison,
    MetricsInputSummary, NoRacePrediction, OvertrainingResult, PatternsResult,
    PerformanceTrendsResult, PersonalRecordComparison, RacePrediction, RacePredictionDetail,
    RacePredictionResult, TrendStatistics, VolumeProgressionResult, WeeklySchedulePatternResult,
};
use pierre_tool_runtime::implementations::analytics::{
    AnalyzePerformanceTrendsTool, CalculateMetricsTool, CompareActivitiesTool, DetectPatternsTool,
    PredictPerformanceTool,
};
use pierre_tool_runtime::implementations::athlete_stats::{
    GetAthleteResult, GetAthleteTool, GetStatsResult, GetStatsTool,
};
use pierre_tool_runtime::implementations::coaches::{
    ActivateCoachTool, CreateCoachTool, DeactivateCoachTool, DeleteCoachTool, GetActiveCoachTool,
    GetCoachTool, HideCoachTool, ListCoachesTool, ListHiddenCoachesTool, SearchCoachesTool,
    ShowCoachTool, ToggleCoachFavoriteTool, UpdateCoachTool,
};
use pierre_tool_runtime::implementations::coaches_output::{
    ActivateCoachResult, ActiveCoachDetail, CoachListEntry, CoachSearchEntry, CreateCoachResult,
    DeactivateCoachResult, DeleteCoachResult, GetActiveCoachResult, GetCoachResult,
    HiddenCoachEntry, HideCoachResult, ListCoachesResult, ListHiddenCoachesResult,
    SearchCoachesResult, ShowCoachResult, ToggleCoachFavoriteResult, UpdateCoachResult,
};
use pierre_tool_runtime::implementations::commitments::{
    CommitmentCancelResult, CommitmentCancelTool, CommitmentCreateResult, CommitmentCreateTool,
};
use pierre_tool_runtime::implementations::connection::{
    ConnectProviderResult, ConnectProviderTool, ConnectionStatusResult, DisconnectProviderResult,
    DisconnectProviderTool, GetConnectionStatusTool, ProviderConnectionStatus,
};
use pierre_tool_runtime::implementations::fitness_config::{
    DeleteFitnessConfigResult, DeleteFitnessConfigTool, GetFitnessConfigResult,
    GetFitnessConfigTool, ListFitnessConfigsResult, ListFitnessConfigsTool, SetFitnessConfigResult,
    SetFitnessConfigTool,
};
use pierre_tool_runtime::implementations::goals::{
    AnalyzeGoalFeasibilityTool, SetGoalTool, SuggestGoalsTool, TrackProgressTool,
};
use pierre_tool_runtime::implementations::goals_output::{
    FeasibilityAnalysis, FeasibilityHistoricalContext, GoalFeasibilityResult, GoalSuggestionEntry,
    ProgressSummary, SetGoalResult, SuggestGoalsResult, TrackProgressResult,
};
use pierre_tool_runtime::implementations::groups::{
    GetGroupMemberActivitiesTool, GroupMemberActivitiesResult, GroupMemberActivity,
};
use pierre_tool_runtime::implementations::memory::{
    CoachFollowupScheduleResult, CoachFollowupScheduleTool, CoachNoteAddResult, CoachNoteAddTool,
    RecallUserMemoryResult, RecallUserMemoryTool, RecalledFact, RememberFactResult,
    RememberFactTool,
};
use pierre_tool_runtime::implementations::mobility::{
    GetStretchingExerciseTool, GetYogaPoseTool, ListStretchingExercisesResult,
    ListStretchingExercisesTool, ListYogaPosesResult, ListYogaPosesTool, SequencePose,
    StretchingExerciseDetail, StretchingExerciseSummary, SuggestStretchesForActivityTool,
    SuggestStretchesResult, SuggestYogaSequenceResult, SuggestYogaSequenceTool, SuggestedStretch,
    YogaPoseDetail, YogaPoseSummary,
};
use pierre_tool_runtime::implementations::nutrition::{
    AnalyzeMealNutritionResult, AnalyzeMealNutritionTool, CalculateDailyNutritionTool,
    DailyNutritionResult, FoodDetailsResult, FoodNutrientEntry, GetFoodDetailsTool,
    GetNutrientTimingTool, MealFoodEntry, NutrientTimingResult, PostWorkoutTiming,
    PreWorkoutTiming, ProteinDistribution, SearchFoodResult, SearchFoodTool,
};
use pierre_tool_runtime::implementations::physiology::{
    EstimateVo2maxResult, EstimateVo2maxTool, PhysiologyProfile, SetPhysiologyResult,
    SetPhysiologyTool,
};
use pierre_tool_runtime::implementations::playbooks::{
    ForgetPlaybookResult, ForgetPlaybookTool, InterventionEntry, ListCoachingPlaybooksResult,
    ListCoachingPlaybooksTool, PlaybookEntry, TriggerEntry,
};
use pierre_tool_runtime::implementations::recipes::{
    DeleteRecipeResult, DeleteRecipeTool, GetRecipeConstraintsTool, GetRecipeTool,
    ListRecipesResult, ListRecipesTool, RecipeConstraintsResult, RecipeDetail, RecipeSearchMatch,
    RecipeSummary, SaveRecipeResult, SaveRecipeTool, SearchRecipesResult, SearchRecipesTool,
    ServingNutrition, ValidateRecipeResult, ValidateRecipeTool, ValidatedIngredient,
};
use pierre_tool_runtime::implementations::routes::{
    DiscoverRoutesResult, DiscoverRoutesTool, DiscoveredRouteEntry, RouteSearchCenter,
};
use pierre_tool_runtime::implementations::store::{
    BrowseCoachStoreResult, BrowseCoachStoreTool, InstallCoachFromStoreResult,
    InstallCoachFromStoreTool, SearchCoachStoreResult, SearchCoachStoreTool, StoreCoachEntry,
};
use pierre_tool_runtime::implementations::stored_data::{
    DataSourcesResult, DateRange, GetHealthSnapshotsTool, GetRecoveryMetricsTool,
    GetSleepSessionsTool, HealthSnapshotsResult, ListDataSourcesTool, RecoveryMetricsResult,
    SleepSessionsResult,
};
use pierre_tool_runtime::implementations::sync::{
    AllProvidersRefresh, DataFreshnessResult, GetDataFreshnessTool, RefreshProviderDataResult,
    RefreshProviderDataTool, SingleProviderRefresh,
};
use pierre_tool_runtime::implementations::training_plan_push::PushTrainingPlanTool;
use pierre_tool_runtime::implementations::verification::{VerifyClaimResult, VerifyClaimTool};
use pierre_tool_runtime::implementations::weather_forecast::{
    GetWeatherForecastTool, WeatherForecastResult,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use std::collections::BTreeMap;

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
    derived: &serde_json::Value,
    sample: &T,
    tool: &str,
) {
    let declared = declared.unwrap_or_else(|| panic!("{tool} must declare an outputSchema"));
    assert_eq!(
        &declared, derived,
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
        &serde_json::to_value(schemars::schema_for!(ListCoachingPlaybooksResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(ListCoachingPlaybooksResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(ForgetPlaybookResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(SetGoalResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(SuggestGoalsResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(TrackProgressResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(GoalFeasibilityResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(CoachNoteAddResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(CoachFollowupScheduleResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(RememberFactResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(RecallUserMemoryResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(DailyNutritionResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(NutrientTimingResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(SearchFoodResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(FoodDetailsResult)).expect("derives"),
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
        &serde_json::to_value(schemars::schema_for!(AnalyzeMealNutritionResult)).expect("derives"),
        &sample,
        "analyze_meal_nutrition",
    );
}

/// A stretch summary with every optional absent — the shape a sparse row takes.
fn stretch_summary() -> StretchingExerciseSummary {
    StretchingExerciseSummary {
        id: "st-1".to_owned(),
        name: "Standing hamstring".to_owned(),
        description: "Hinge at the hip with a long spine.".to_owned(),
        category: "static".to_owned(),
        difficulty: "beginner".to_owned(),
        primary_muscles: vec!["hamstrings".to_owned()],
        secondary_muscles: vec![],
        duration_seconds: 30,
        sets: 2,
    }
}

#[test]
fn list_stretching_exercises_declares_a_schema_that_accepts_its_payload() {
    let sample = ListStretchingExercisesResult {
        exercises: vec![stretch_summary()],
        count: 1,
        timestamp: "2026-09-05T10:00:00+00:00".to_owned(),
    };
    assert_declares_and_accepts(
        <ListStretchingExercisesTool as McpTool<dyn ToolRuntime>>::definition(
            &ListStretchingExercisesTool,
        )
        .output_schema,
        &serde_json::to_value(schemars::schema_for!(ListStretchingExercisesResult))
            .expect("derives"),
        &sample,
        "list_stretching_exercises",
    );
}

#[test]
fn get_stretching_exercise_accepts_a_held_stretch_with_no_repetitions() {
    // repetitions is None for a stretch that is held rather than repeated,
    // which is most of them.
    let sample = StretchingExerciseDetail {
        id: "st-1".to_owned(),
        name: "Standing hamstring".to_owned(),
        description: "Hinge at the hip with a long spine.".to_owned(),
        category: "static".to_owned(),
        difficulty: "beginner".to_owned(),
        primary_muscles: vec!["hamstrings".to_owned()],
        secondary_muscles: vec!["calves".to_owned()],
        duration_seconds: 30,
        repetitions: None,
        sets: 2,
        recommended_for_activities: vec!["run".to_owned()],
        contraindications: vec!["acute hamstring strain".to_owned()],
        instructions: vec!["Stand tall".to_owned(), "Hinge forward".to_owned()],
        cues: vec!["Keep the spine long".to_owned()],
        image_url: None,
        video_url: None,
    };
    assert_declares_and_accepts(
        <GetStretchingExerciseTool as McpTool<dyn ToolRuntime>>::definition(
            &GetStretchingExerciseTool,
        )
        .output_schema,
        &serde_json::to_value(schemars::schema_for!(StretchingExerciseDetail)).expect("derives"),
        &sample,
        "get_stretching_exercise",
    );
}

#[test]
fn suggest_stretches_for_activity_declares_a_schema_that_accepts_its_payload() {
    let sample = SuggestStretchesResult {
        activity_type: "run".to_owned(),
        exercises: vec![SuggestedStretch {
            id: "st-1".to_owned(),
            name: "Standing hamstring".to_owned(),
            category: "static".to_owned(),
            difficulty: "beginner".to_owned(),
            duration_seconds: 30,
            sets: 2,
            primary_muscles: vec!["hamstrings".to_owned()],
            instructions: vec!["Hinge forward".to_owned()],
        }],
        count: 1,
        total_duration_seconds: 60,
        suggested_at: "2026-09-05T10:00:00+00:00".to_owned(),
    };
    assert_declares_and_accepts(
        <SuggestStretchesForActivityTool as McpTool<dyn ToolRuntime>>::definition(
            &SuggestStretchesForActivityTool,
        )
        .output_schema,
        &serde_json::to_value(schemars::schema_for!(SuggestStretchesResult)).expect("derives"),
        &sample,
        "suggest_stretches_for_activity",
    );
}

#[test]
fn list_yoga_poses_accepts_a_pose_with_no_sanskrit_name() {
    let sample = ListYogaPosesResult {
        poses: vec![YogaPoseSummary {
            id: "yp-1".to_owned(),
            english_name: "Legs up the wall".to_owned(),
            sanskrit_name: None,
            description: "Restorative inversion.".to_owned(),
            category: "restorative".to_owned(),
            difficulty: "beginner".to_owned(),
            pose_type: "inversion".to_owned(),
            primary_muscles: vec!["hamstrings".to_owned()],
            hold_duration_seconds: 300,
        }],
        count: 1,
        timestamp: "2026-09-05T10:00:00+00:00".to_owned(),
    };
    assert_declares_and_accepts(
        <ListYogaPosesTool as McpTool<dyn ToolRuntime>>::definition(&ListYogaPosesTool)
            .output_schema,
        &serde_json::to_value(schemars::schema_for!(ListYogaPosesResult)).expect("derives"),
        &sample,
        "list_yoga_poses",
    );
}

#[test]
fn get_yoga_pose_declares_a_schema_that_accepts_its_payload() {
    let sample = YogaPoseDetail {
        id: "yp-1".to_owned(),
        english_name: "Legs up the wall".to_owned(),
        sanskrit_name: Some("Viparita Karani".to_owned()),
        description: "Restorative inversion.".to_owned(),
        benefits: vec!["Calms the nervous system".to_owned()],
        category: "restorative".to_owned(),
        difficulty: "beginner".to_owned(),
        pose_type: "inversion".to_owned(),
        primary_muscles: vec!["hamstrings".to_owned()],
        secondary_muscles: vec![],
        chakras: vec![],
        hold_duration_seconds: 300,
        breath_guidance: Some("Slow nasal breathing".to_owned()),
        recommended_for_activities: vec!["run".to_owned()],
        recommended_for_recovery: vec!["post_race".to_owned()],
        contraindications: vec!["glaucoma".to_owned()],
        instructions: vec!["Sit side-on to the wall".to_owned()],
        modifications: vec!["Bolster under the hips".to_owned()],
        progressions: vec![],
        cues: vec!["Let the legs be heavy".to_owned()],
        warmup_poses: vec![],
        followup_poses: vec!["yp-2".to_owned()],
        image_url: None,
        video_url: None,
    };
    assert_declares_and_accepts(
        <GetYogaPoseTool as McpTool<dyn ToolRuntime>>::definition(&GetYogaPoseTool).output_schema,
        &serde_json::to_value(schemars::schema_for!(YogaPoseDetail)).expect("derives"),
        &sample,
        "get_yoga_pose",
    );
}

#[test]
fn suggest_yoga_sequence_declares_a_schema_that_accepts_its_payload() {
    // total_duration_seconds lands at or UNDER the target: poses are added
    // while they fit, so an empty sequence is reachable when nothing does.
    let sample = SuggestYogaSequenceResult {
        purpose: "post_cardio".to_owned(),
        sequence: vec![SequencePose {
            order: 1,
            id: "yp-1".to_owned(),
            english_name: "Legs up the wall".to_owned(),
            sanskrit_name: None,
            category: "restorative".to_owned(),
            difficulty: "beginner".to_owned(),
            hold_duration_seconds: 300,
            breath_guidance: None,
            primary_muscles: vec!["hamstrings".to_owned()],
            instructions: vec!["Sit side-on to the wall".to_owned()],
        }],
        pose_count: 1,
        total_duration_seconds: 300,
        target_duration_minutes: 15,
        guidance: "Take your time with each pose.".to_owned(),
        suggested_at: "2026-09-05T10:00:00+00:00".to_owned(),
    };
    assert_declares_and_accepts(
        <SuggestYogaSequenceTool as McpTool<dyn ToolRuntime>>::definition(&SuggestYogaSequenceTool)
            .output_schema,
        &serde_json::to_value(schemars::schema_for!(SuggestYogaSequenceResult)).expect("derives"),
        &sample,
        "suggest_yoga_sequence",
    );
}

#[test]
fn connect_provider_declares_a_schema_that_accepts_its_payload() {
    let sample = ConnectProviderResult {
        provider: "strava".to_owned(),
        authorization_url: "https://www.strava.com/oauth/authorize?...".to_owned(),
        state: "3f9c1a".to_owned(),
        instructions: "To connect your strava account: ...".to_owned(),
        expires_in_minutes: 10,
        status: "pending_authorization".to_owned(),
    };
    assert_declares_and_accepts(
        <ConnectProviderTool as McpTool<dyn ToolRuntime>>::definition(&ConnectProviderTool)
            .output_schema,
        &serde_json::to_value(schemars::schema_for!(ConnectProviderResult)).expect("derives"),
        &sample,
        "connect_provider",
    );
}

#[test]
fn get_connection_status_declares_one_schema_that_accepts_all_three_shapes() {
    // The tool answers a different shape depending on what was asked, so the
    // schema is an anyOf and every shape it sends has to validate against it.
    let derived =
        &serde_json::to_value(schemars::schema_for!(ConnectionStatusResult)).expect("derives");
    let declared =
        <GetConnectionStatusTool as McpTool<dyn ToolRuntime>>::definition(&GetConnectionStatusTool)
            .output_schema
            .expect("get_connection_status must declare an outputSchema");
    assert_eq!(
        &declared, derived,
        "declared schema must be the derived one"
    );

    let validator = jsonschema::validator_for(&declared).expect("compiles");
    for (label, shape) in [
        (
            "single",
            ConnectionStatusResult::Single {
                provider: "strava".to_owned(),
                status: "connected".to_owned(),
                connected: true,
                needs_reauth: false,
                backend: "native".to_owned(),
            },
        ),
        (
            "unknown",
            ConnectionStatusResult::Unknown {
                provider: "peloton".to_owned(),
                status: "disconnected".to_owned(),
                connected: false,
                backend: "none".to_owned(),
                note: "Unknown provider. Use 'strava' or 'garmin' instead.".to_owned(),
            },
        ),
        (
            "all",
            ConnectionStatusResult::All {
                providers: BTreeMap::from([(
                    "strava".to_owned(),
                    ProviderConnectionStatus {
                        connected: true,
                        status: "connected".to_owned(),
                        needs_reauth: false,
                        backend: "native".to_owned(),
                    },
                )]),
            },
        ),
    ] {
        let payload = serde_json::to_value(&shape).expect("serializes");
        assert!(
            validator.is_valid(&payload),
            "the {label} arm must satisfy the declared schema:\n{payload:#}"
        );
    }
}

#[test]
fn the_connection_status_schema_still_rejects_a_shape_the_tool_never_sends() {
    // Without this the anyOf could be vacuous — three arms that between them
    // accept anything would pass the test above and describe nothing.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(ConnectionStatusResult)).expect("derives"),
    )
    .expect("compiles");

    assert!(
        !validator.is_valid(&json!({"provider": "strava"})),
        "a bare provider name is not one of the three shapes this tool sends"
    );
}

#[test]
fn disconnect_provider_declares_a_schema_that_accepts_its_payload() {
    let sample = DisconnectProviderResult {
        provider: "strava".to_owned(),
        status: "disconnected".to_owned(),
        message: "Successfully disconnected from strava".to_owned(),
    };
    assert_declares_and_accepts(
        <DisconnectProviderTool as McpTool<dyn ToolRuntime>>::definition(&DisconnectProviderTool)
            .output_schema,
        &serde_json::to_value(schemars::schema_for!(DisconnectProviderResult)).expect("derives"),
        &sample,
        "disconnect_provider",
    );
}

/// The stored-data tools declare Formatted<T>, because their payload shape
/// depends on the caller's `format` argument rather than on the data. These
/// assert the JSON arm — the one an athlete's client actually receives.
#[test]
fn get_sleep_sessions_declares_a_schema_that_accepts_an_empty_window() {
    // Empty is the ordinary case for a window with no sleep recorded, and the
    // one an over-strict schema would reject.
    let sample = Formatted::Json(SleepSessionsResult {
        count: 0,
        sessions: vec![],
        range: DateRange {
            start: "2026-08-06T00:00:00+00:00".to_owned(),
            end: "2026-09-05T00:00:00+00:00".to_owned(),
        },
    });
    assert_declares_and_accepts(
        <GetSleepSessionsTool as McpTool<dyn ToolRuntime>>::definition(&GetSleepSessionsTool)
            .output_schema,
        &serde_json::to_value(schemars::schema_for!(Formatted<SleepSessionsResult>))
            .expect("derives"),
        &sample,
        "get_sleep_sessions",
    );
}

#[test]
fn get_recovery_metrics_declares_a_schema_that_accepts_its_payload() {
    let sample = Formatted::Json(RecoveryMetricsResult {
        count: 0,
        metrics: vec![],
        range: DateRange {
            start: "2026-08-06T00:00:00+00:00".to_owned(),
            end: "2026-09-05T00:00:00+00:00".to_owned(),
        },
    });
    assert_declares_and_accepts(
        <GetRecoveryMetricsTool as McpTool<dyn ToolRuntime>>::definition(&GetRecoveryMetricsTool)
            .output_schema,
        &serde_json::to_value(schemars::schema_for!(Formatted<RecoveryMetricsResult>))
            .expect("derives"),
        &sample,
        "get_recovery_metrics",
    );
}

#[test]
fn get_health_snapshots_declares_a_schema_that_accepts_its_payload() {
    let sample = Formatted::Json(HealthSnapshotsResult {
        count: 0,
        snapshots: vec![],
        range: DateRange {
            start: "2026-08-06T00:00:00+00:00".to_owned(),
            end: "2026-09-05T00:00:00+00:00".to_owned(),
        },
    });
    assert_declares_and_accepts(
        <GetHealthSnapshotsTool as McpTool<dyn ToolRuntime>>::definition(&GetHealthSnapshotsTool)
            .output_schema,
        &serde_json::to_value(schemars::schema_for!(Formatted<HealthSnapshotsResult>))
            .expect("derives"),
        &sample,
        "get_health_snapshots",
    );
}

#[test]
fn list_data_sources_declares_a_schema_that_accepts_its_payload() {
    let sample = Formatted::Json(DataSourcesResult {
        count: 0,
        sources: vec![],
    });
    assert_declares_and_accepts(
        <ListDataSourcesTool as McpTool<dyn ToolRuntime>>::definition(&ListDataSourcesTool)
            .output_schema,
        &serde_json::to_value(schemars::schema_for!(Formatted<DataSourcesResult>))
            .expect("derives"),
        &sample,
        "list_data_sources",
    );
}

/// The TOON arm is a different shape entirely, and it is the arm nobody would
/// have hand-written into a schema for a sleep tool.
#[test]
fn the_stored_data_schema_also_accepts_the_toon_envelope() {
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(Formatted<SleepSessionsResult>))
            .expect("derives"),
    )
    .expect("compiles");

    let toon: Formatted<SleepSessionsResult> = Formatted::Toon {
        toon: "count:0\nsessions:[]".to_owned(),
        format: "toon".to_owned(),
    };
    let payload = serde_json::to_value(&toon).expect("serializes");
    assert!(
        validator.is_valid(&payload),
        "format=toon is a real reply from this tool and must satisfy its schema:\n{payload:#}"
    );
}

/// The schema has to be attached to the tool it describes, and NOTHING in the
/// type system enforces that: `answers_with::<T>` accepts any T, and
/// `ok_typed`'s label is a free string. A result type wired to the wrong tool
/// compiles, passes its own conformance test, and lies to every client.
///
/// This caught a real mistake. The three recipe payloads were first labelled
/// `search_recipes` / `get_recipe` / `analyze_recipe_nutrition`, read off the
/// order they appear in `inner.rs`. Two were wrong, and
/// `analyze_recipe_nutrition` is not a tool at all.
#[test]
fn each_recipe_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "list_recipes",
            <ListRecipesTool as McpTool<dyn ToolRuntime>>::definition(&ListRecipesTool),
            serde_json::to_value(schemars::schema_for!(Formatted<ListRecipesResult>))
                .expect("derives"),
        ),
        (
            "get_recipe",
            <GetRecipeTool as McpTool<dyn ToolRuntime>>::definition(&GetRecipeTool),
            serde_json::to_value(schemars::schema_for!(Formatted<RecipeDetail>)).expect("derives"),
        ),
        (
            "search_recipes",
            <SearchRecipesTool as McpTool<dyn ToolRuntime>>::definition(&SearchRecipesTool),
            serde_json::to_value(schemars::schema_for!(Formatted<SearchRecipesResult>))
                .expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn list_recipes_accepts_a_recipe_with_no_nutrition_yet() {
    // has_nutrition false and calories_per_serving absent is the ordinary
    // state of a freshly saved recipe.
    let sample = Formatted::Json(ListRecipesResult {
        recipes: vec![RecipeSummary {
            id: "3f9c1a2e-0000-4000-8000-000000000001".to_owned(),
            name: "Overnight oats".to_owned(),
            servings: 1,
            meal_timing: "breakfast".to_owned(),
            total_time_mins: Some(5),
            tags: vec!["quick".to_owned()],
            has_nutrition: false,
            calories_per_serving: None,
            updated_at: "2026-09-05T09:00:00+00:00".to_owned(),
        }],
        count: 1,
        offset: 0,
        limit: 20,
        has_more: false,
    });
    assert_declares_and_accepts(
        <ListRecipesTool as McpTool<dyn ToolRuntime>>::definition(&ListRecipesTool).output_schema,
        &serde_json::to_value(schemars::schema_for!(Formatted<ListRecipesResult>))
            .expect("derives"),
        &sample,
        "list_recipes",
    );
}

#[test]
fn search_recipes_declares_a_schema_that_accepts_no_matches() {
    let sample: Formatted<SearchRecipesResult> = Formatted::Json(SearchRecipesResult {
        query: "tempeh".to_owned(),
        results: vec![],
        count: 0,
        offset: 0,
        limit: 20,
        has_more: false,
    });
    assert_declares_and_accepts(
        <SearchRecipesTool as McpTool<dyn ToolRuntime>>::definition(&SearchRecipesTool)
            .output_schema,
        &serde_json::to_value(schemars::schema_for!(Formatted<SearchRecipesResult>))
            .expect("derives"),
        &sample,
        "search_recipes (no matches)",
    );
    // And the populated arm, since RecipeSearchMatch appears nowhere else.
    let populated = Formatted::Json(SearchRecipesResult {
        query: "oats".to_owned(),
        results: vec![RecipeSearchMatch {
            id: "3f9c1a2e-0000-4000-8000-000000000001".to_owned(),
            name: "Overnight oats".to_owned(),
            description: None,
            servings: 1,
            meal_timing: "breakfast".to_owned(),
            tags: vec![],
            calories_per_serving: Some(410.0),
        }],
        count: 1,
        offset: 0,
        limit: 20,
        has_more: false,
    });
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(Formatted<SearchRecipesResult>))
            .expect("derives"),
    )
    .expect("compiles");
    assert!(
        validator.is_valid(&serde_json::to_value(&populated).expect("serializes")),
        "a populated search result must satisfy the schema too"
    );
}

// ============================================================================
// coaches
// ============================================================================

/// Every coach tool, paired with the type its `execute` actually serializes.
///
/// Five of the thirteen honour a `format` argument and so answer through the
/// `Formatted` envelope; the other eight always send their own shape. Getting
/// that wrong is exactly the drift this table exists to catch — a client told
/// to expect the envelope's `anyOf` and handed a bare object has been lied
/// to.
#[test]
fn each_coach_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "list_coaches",
            <ListCoachesTool as McpTool<dyn ToolRuntime>>::definition(&ListCoachesTool),
            serde_json::to_value(schemars::schema_for!(Formatted<ListCoachesResult>))
                .expect("derives"),
        ),
        (
            "create_coach",
            <CreateCoachTool as McpTool<dyn ToolRuntime>>::definition(&CreateCoachTool),
            serde_json::to_value(schemars::schema_for!(CreateCoachResult)).expect("derives"),
        ),
        (
            "get_coach",
            <GetCoachTool as McpTool<dyn ToolRuntime>>::definition(&GetCoachTool),
            serde_json::to_value(schemars::schema_for!(Formatted<GetCoachResult>))
                .expect("derives"),
        ),
        (
            "update_coach",
            <UpdateCoachTool as McpTool<dyn ToolRuntime>>::definition(&UpdateCoachTool),
            serde_json::to_value(schemars::schema_for!(UpdateCoachResult)).expect("derives"),
        ),
        (
            "delete_coach",
            <DeleteCoachTool as McpTool<dyn ToolRuntime>>::definition(&DeleteCoachTool),
            serde_json::to_value(schemars::schema_for!(DeleteCoachResult)).expect("derives"),
        ),
        (
            "toggle_coach_favorite",
            <ToggleCoachFavoriteTool as McpTool<dyn ToolRuntime>>::definition(
                &ToggleCoachFavoriteTool,
            ),
            serde_json::to_value(schemars::schema_for!(ToggleCoachFavoriteResult))
                .expect("derives"),
        ),
        (
            "search_coaches",
            <SearchCoachesTool as McpTool<dyn ToolRuntime>>::definition(&SearchCoachesTool),
            serde_json::to_value(schemars::schema_for!(Formatted<SearchCoachesResult>))
                .expect("derives"),
        ),
        (
            "activate_coach",
            <ActivateCoachTool as McpTool<dyn ToolRuntime>>::definition(&ActivateCoachTool),
            serde_json::to_value(schemars::schema_for!(ActivateCoachResult)).expect("derives"),
        ),
        (
            "deactivate_coach",
            <DeactivateCoachTool as McpTool<dyn ToolRuntime>>::definition(&DeactivateCoachTool),
            serde_json::to_value(schemars::schema_for!(DeactivateCoachResult)).expect("derives"),
        ),
        (
            "get_active_coach",
            <GetActiveCoachTool as McpTool<dyn ToolRuntime>>::definition(&GetActiveCoachTool),
            serde_json::to_value(schemars::schema_for!(Formatted<GetActiveCoachResult>))
                .expect("derives"),
        ),
        (
            "hide_coach",
            <HideCoachTool as McpTool<dyn ToolRuntime>>::definition(&HideCoachTool),
            serde_json::to_value(schemars::schema_for!(HideCoachResult)).expect("derives"),
        ),
        (
            "show_coach",
            <ShowCoachTool as McpTool<dyn ToolRuntime>>::definition(&ShowCoachTool),
            serde_json::to_value(schemars::schema_for!(ShowCoachResult)).expect("derives"),
        ),
        (
            "list_hidden_coaches",
            <ListHiddenCoachesTool as McpTool<dyn ToolRuntime>>::definition(&ListHiddenCoachesTool),
            serde_json::to_value(schemars::schema_for!(Formatted<ListHiddenCoachesResult>))
                .expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn list_coaches_declares_a_schema_that_accepts_its_payload() {
    let sample = Formatted::Json(ListCoachesResult {
        coaches: vec![CoachListEntry {
            id: "6bd0b0f4-0000-4000-8000-000000000001".to_owned(),
            title: "Threshold Builder".to_owned(),
            description: Some("Six weeks of tempo work".to_owned()),
            category: "training".to_owned(),
            tags: vec!["run".to_owned(), "threshold".to_owned()],
            token_count: 812,
            is_favorite: true,
            is_system: false,
            is_assigned: true,
            use_count: 14,
            last_used_at: Some("2026-09-04T18:22:00+00:00".to_owned()),
            updated_at: "2026-09-01T09:00:00+00:00".to_owned(),
        }],
        count: 1,
        total: 7,
        offset: 0,
        limit: 50,
        has_more: false,
    });

    assert_declares_and_accepts(
        <ListCoachesTool as McpTool<dyn ToolRuntime>>::definition(&ListCoachesTool).output_schema,
        &serde_json::to_value(schemars::schema_for!(Formatted<ListCoachesResult>))
            .expect("derives"),
        &sample,
        "list_coaches",
    );
}

#[test]
fn a_coach_with_no_description_and_no_use_yet_still_validates() {
    // A freshly created coach: no description was given, it has never been
    // used, so the two optional fields are absent rather than zeroed.
    let sample = Formatted::Json(ListCoachesResult {
        coaches: vec![CoachListEntry {
            id: "6bd0b0f4-0000-4000-8000-000000000002".to_owned(),
            title: "Untitled".to_owned(),
            description: None,
            category: "custom".to_owned(),
            tags: vec![],
            token_count: 0,
            is_favorite: false,
            is_system: false,
            is_assigned: false,
            use_count: 0,
            last_used_at: None,
            updated_at: "2026-09-05T00:00:00+00:00".to_owned(),
        }],
        count: 1,
        total: 1,
        offset: 0,
        limit: 50,
        has_more: false,
    });
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(Formatted<ListCoachesResult>))
            .expect("derives"),
    )
    .expect("compiles");

    assert!(
        validator.is_valid(&serde_json::to_value(&sample).expect("serializes")),
        "an absent description or last-use must not fail the schema"
    );
}

#[test]
fn get_active_coach_declares_one_schema_that_covers_both_of_its_answers() {
    // The tool sends the same key set whether or not a coach is active. If the
    // schema only ever described the active answer, every idle reply would be
    // a protocol violation — and idle is the common case.
    let derived = serde_json::to_value(schemars::schema_for!(Formatted<GetActiveCoachResult>))
        .expect("derives");
    let validator = jsonschema::validator_for(&derived).expect("compiles");

    let active = Formatted::Json(GetActiveCoachResult {
        active: true,
        coach: Some(ActiveCoachDetail {
            id: "6bd0b0f4-0000-4000-8000-000000000003".to_owned(),
            title: "Base Phase".to_owned(),
            description: None,
            system_prompt: "You coach aerobic base building.".to_owned(),
            category: "training".to_owned(),
            tags: vec!["base".to_owned()],
            token_count: 640,
        }),
    });
    let idle = Formatted::Json(GetActiveCoachResult {
        active: false,
        coach: None,
    });

    for (label, payload) in [("active", &active), ("idle", &idle)] {
        let value = serde_json::to_value(payload).expect("serializes");
        assert!(
            validator.is_valid(&value),
            "get_active_coach's {label} answer must satisfy the schema it declares:\n{value:#}"
        );
    }
}

#[test]
fn get_coach_does_not_promise_usage_fields_it_cannot_fill() {
    // It used to send is_favorite false, use_count 0 and last_used_at null
    // unconditionally — the single-coach read does not join the usage table,
    // so those were constants dressed as data. Declaring an outputSchema would
    // have made them a promise. They are gone; list_coaches is where usage
    // signals actually come from.
    let derived = serde_json::to_value(schemars::schema_for!(GetCoachResult)).expect("derives");
    let properties = derived["properties"]
        .as_object()
        .expect("the result type is an object schema");

    for absent in ["is_favorite", "use_count", "last_used_at"] {
        assert!(
            !properties.contains_key(absent),
            "get_coach must not declare {absent}: it has no value to put there"
        );
    }
    assert!(
        properties.contains_key("system_prompt"),
        "get_coach exists to return the full coach, prompt included"
    );
}

#[test]
fn the_coach_schemas_reject_payloads_missing_a_required_field() {
    // Without this the conformance tests above would pass just as happily
    // against a schema that describes nothing.
    let search = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(SearchCoachesResult)).expect("derives"),
    )
    .expect("compiles");
    assert!(
        !search.is_valid(&json!({
            "query": "tempo",
            "results": [{ "title": "Threshold Builder", "category": "training",
                          "tags": [], "token_count": 12 }],
            "returned_count": 1,
            "offset": 0,
            "limit": 20,
            "has_more": false,
        })),
        "a search hit with no id is not something a client can act on"
    );

    let delete = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(DeleteCoachResult)).expect("derives"),
    )
    .expect("compiles");
    assert!(
        !delete.is_valid(&json!({ "deleted": true })),
        "delete_coach must say WHICH coach it deleted"
    );

    let envelope = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(Formatted<ListHiddenCoachesResult>))
            .expect("derives"),
    )
    .expect("compiles");
    assert!(
        !envelope.is_valid(&json!({ "count": 0 })),
        "the Formatted envelope must not accept an answer missing the coaches it counts"
    );
}

#[test]
fn the_narrow_coach_projections_accept_their_payloads() {
    for (tool, derived, payload) in [
        (
            "search_coaches",
            serde_json::to_value(schemars::schema_for!(Formatted<SearchCoachesResult>))
                .expect("derives"),
            serde_json::to_value(Formatted::Json(SearchCoachesResult {
                query: "tempo".to_owned(),
                results: vec![CoachSearchEntry {
                    id: "6bd0b0f4-0000-4000-8000-000000000004".to_owned(),
                    title: "Threshold Builder".to_owned(),
                    description: None,
                    category: "training".to_owned(),
                    tags: vec!["tempo".to_owned()],
                    token_count: 812,
                }],
                returned_count: 1,
                offset: 0,
                limit: 20,
                has_more: false,
            }))
            .expect("serializes"),
        ),
        (
            "list_hidden_coaches",
            serde_json::to_value(schemars::schema_for!(Formatted<ListHiddenCoachesResult>))
                .expect("derives"),
            serde_json::to_value(Formatted::Json(ListHiddenCoachesResult {
                coaches: vec![HiddenCoachEntry {
                    id: "6bd0b0f4-0000-4000-8000-000000000005".to_owned(),
                    title: "Nutrition Basics".to_owned(),
                    description: Some("Shipped with the platform".to_owned()),
                    category: "nutrition".to_owned(),
                    is_system: true,
                }],
                count: 1,
            }))
            .expect("serializes"),
        ),
        (
            "activate_coach",
            serde_json::to_value(schemars::schema_for!(ActivateCoachResult)).expect("derives"),
            serde_json::to_value(ActivateCoachResult {
                id: "6bd0b0f4-0000-4000-8000-000000000006".to_owned(),
                title: "Base Phase".to_owned(),
                description: None,
                system_prompt: "You coach aerobic base building.".to_owned(),
                category: "training".to_owned(),
                is_active: true,
                token_count: 640,
            })
            .expect("serializes"),
        ),
        (
            "deactivate_coach",
            serde_json::to_value(schemars::schema_for!(DeactivateCoachResult)).expect("derives"),
            serde_json::to_value(DeactivateCoachResult { deactivated: false }).expect("serializes"),
        ),
        (
            "toggle_coach_favorite",
            serde_json::to_value(schemars::schema_for!(ToggleCoachFavoriteResult))
                .expect("derives"),
            serde_json::to_value(ToggleCoachFavoriteResult {
                coach_id: "6bd0b0f4-0000-4000-8000-000000000007".to_owned(),
                is_favorite: true,
            })
            .expect("serializes"),
        ),
        (
            "hide_coach",
            serde_json::to_value(schemars::schema_for!(HideCoachResult)).expect("derives"),
            serde_json::to_value(HideCoachResult {
                coach_id: "6bd0b0f4-0000-4000-8000-000000000008".to_owned(),
                is_hidden: true,
            })
            .expect("serializes"),
        ),
        (
            "show_coach",
            serde_json::to_value(schemars::schema_for!(ShowCoachResult)).expect("derives"),
            serde_json::to_value(ShowCoachResult {
                coach_id: "6bd0b0f4-0000-4000-8000-000000000009".to_owned(),
                is_hidden: false,
                removed_preference: true,
            })
            .expect("serializes"),
        ),
        (
            "create_coach",
            serde_json::to_value(schemars::schema_for!(CreateCoachResult)).expect("derives"),
            serde_json::to_value(CreateCoachResult {
                id: "6bd0b0f4-0000-4000-8000-00000000000a".to_owned(),
                title: "Recovery Week".to_owned(),
                description: Some("Deload guidance".to_owned()),
                category: "recovery".to_owned(),
                tags: vec![],
                token_count: 210,
                created_at: "2026-09-05T12:00:00+00:00".to_owned(),
            })
            .expect("serializes"),
        ),
        (
            "update_coach",
            serde_json::to_value(schemars::schema_for!(UpdateCoachResult)).expect("derives"),
            serde_json::to_value(UpdateCoachResult {
                id: "6bd0b0f4-0000-4000-8000-00000000000b".to_owned(),
                title: "Recovery Week".to_owned(),
                description: None,
                system_prompt: "You guide deload weeks.".to_owned(),
                category: "recovery".to_owned(),
                tags: vec!["deload".to_owned()],
                token_count: 232,
                updated_at: "2026-09-05T13:00:00+00:00".to_owned(),
            })
            .expect("serializes"),
        ),
    ] {
        let validator =
            jsonschema::validator_for(&derived).unwrap_or_else(|e| panic!("{tool} schema: {e}"));
        assert!(
            validator.is_valid(&payload),
            "{tool}: the declared schema rejected the payload the tool sends:\n{payload:#}"
        );
    }
}

// ============================================================================
// analytics
// ============================================================================

#[test]
fn each_analytics_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "analyze_performance_trends",
            <AnalyzePerformanceTrendsTool as McpTool<dyn ToolRuntime>>::definition(
                &AnalyzePerformanceTrendsTool,
            ),
            serde_json::to_value(schemars::schema_for!(Formatted<PerformanceTrendsResult>))
                .expect("derives"),
        ),
        (
            "detect_patterns",
            <DetectPatternsTool as McpTool<dyn ToolRuntime>>::definition(&DetectPatternsTool),
            serde_json::to_value(schemars::schema_for!(Formatted<PatternsResult>))
                .expect("derives"),
        ),
        (
            "calculate_metrics",
            <CalculateMetricsTool as McpTool<dyn ToolRuntime>>::definition(&CalculateMetricsTool),
            serde_json::to_value(schemars::schema_for!(Formatted<ActivityMetricsResult>))
                .expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn analyze_performance_trends_declares_a_schema_that_accepts_both_of_its_answers() {
    // Six of this tool's seven returns are degenerate — no activities, an
    // unknown metric, too few points to regress — and carry no statistics.
    // A schema that only described the successful answer would make every one
    // of them a protocol violation.
    let derived = serde_json::to_value(schemars::schema_for!(Formatted<PerformanceTrendsResult>))
        .expect("derives");
    let validator = jsonschema::validator_for(&derived).expect("compiles");

    let regressed = Formatted::Json(PerformanceTrendsResult {
        metric: "pace".to_owned(),
        timeframe: "month".to_owned(),
        trend: "improving".to_owned(),
        activities_analyzed: 18,
        statistics: Some(TrendStatistics {
            slope: -0.004,
            r_squared: 0.62,
            confidence: 0.62,
            correlation: -0.79,
            standard_error: 0.001,
            p_value: Some(0.002),
            moving_average_7day: 5.31,
            start_value: Some(5.48),
            end_value: Some(5.19),
            percent_change: Some(-5.29),
        }),
        insights: vec!["Analyzed 18 data points over 29 days".to_owned()],
    });
    let no_data = Formatted::Json(PerformanceTrendsResult {
        metric: "power".to_owned(),
        timeframe: "week".to_owned(),
        trend: "no_data".to_owned(),
        activities_analyzed: 0,
        statistics: None,
        insights: vec!["No activities found for analysis".to_owned()],
    });

    for (label, payload) in [("regressed", &regressed), ("no_data", &no_data)] {
        let value = serde_json::to_value(payload).expect("serializes");
        assert!(
            validator.is_valid(&value),
            "the {label} answer must satisfy the schema the tool declares:\n{value:#}"
        );
    }

    // The degenerate answer omits the key rather than sending it null: a
    // client that sees `statistics` at all can trust there is a regression.
    let value = serde_json::to_value(&no_data).expect("serializes");
    assert!(
        value.get("statistics").is_none(),
        "an absent regression must be an absent key, not a null one"
    );
}

/// Each `detect_patterns` shape must match exactly ONE arm of the derived
/// schema.
///
/// This is the property that makes an untagged enum usable by a client: it
/// branches on which arm accepted the answer. It is not free — it holds only
/// because every variant requires a field no other variant has — and the
/// schema does NOT assert it, because schemars emits `anyOf` for an untagged
/// enum, which is satisfied by matching one arm OR several. So checking the
/// whole schema proves nothing here; the arms are pulled out of `anyOf` and
/// checked one at a time, and an overlap fails this test rather than quietly
/// making the contract unusable.
#[test]
fn every_detect_patterns_shape_matches_exactly_one_arm() {
    let schema = serde_json::to_value(schemars::schema_for!(PatternsResult)).expect("derives");
    let defs = schema.get("$defs").cloned().unwrap_or_else(|| json!({}));
    let arms: Vec<serde_json::Value> = schema["anyOf"]
        .as_array()
        .expect("an untagged enum derives a list of arms")
        .iter()
        .map(|arm| {
            // Each arm is a $ref into $defs; give it the definitions back so
            // it can be compiled on its own.
            let mut arm = arm.clone();
            arm.as_object_mut()
                .expect("each arm is an object")
                .insert("$defs".to_owned(), defs.clone());
            arm
        })
        .collect();
    assert_eq!(arms.len(), 5, "detect_patterns answers with five shapes");
    let arm_validators: Vec<_> = arms
        .iter()
        .map(|a| jsonschema::validator_for(a).expect("each arm compiles"))
        .collect();

    let shapes = [
        (
            "insufficient",
            PatternsResult::Insufficient(InsufficientPatternData {
                pattern_type: "overtraining".to_owned(),
                activities_analyzed: 2,
                patterns_detected: vec![],
                insights: vec!["Need at least 3 activities for pattern detection".to_owned()],
                confidence: "insufficient_data".to_owned(),
            }),
        ),
        (
            "weekly_schedule",
            PatternsResult::WeeklySchedule(Box::new(WeeklySchedulePatternResult {
                pattern_type: "weekly_schedule".to_owned(),
                preferred_training_days: vec![DayFrequency {
                    day: "Tuesday".to_owned(),
                    frequency: 9,
                }],
                patterns_detected: vec!["Consistent weekly schedule detected".to_owned()],
                insights: vec!["Consistent weekly schedule detected".to_owned()],
                consistency_score: 44.0,
                avg_activities_per_week: 3.4,
                confidence: "high".to_owned(),
            })),
        ),
        (
            "training_blocks",
            PatternsResult::HardEasy(Box::new(HardEasyPatternResult {
                pattern_type: "training_blocks".to_owned(),
                pattern_detected: true,
                intensity_distribution: IntensityDistribution {
                    hard_percentage: 22.0,
                    easy_percentage: 78.0,
                },
                adequate_recovery: true,
                patterns_detected: vec!["Hard days follow easy days".to_owned()],
                insights: vec!["Hard days follow easy days".to_owned()],
                confidence: "medium".to_owned(),
            })),
        ),
        (
            "progression",
            PatternsResult::VolumeProgression(Box::new(VolumeProgressionResult {
                pattern_type: "progression".to_owned(),
                trend: "increasing".to_owned(),
                weekly_volumes: vec![32.0, 36.5, 41.0],
                week_numbers: vec![34, 35, 36],
                volume_spikes_detected: false,
                spike_weeks: vec![],
                patterns_detected: vec!["Volume is increasing".to_owned()],
                insights: vec!["Volume is increasing".to_owned()],
                confidence: "medium".to_owned(),
            })),
        ),
        (
            "overtraining",
            PatternsResult::Overtraining(Box::new(OvertrainingResult {
                pattern_type: "overtraining".to_owned(),
                risk_level: "moderate".to_owned(),
                warning_signs: vec!["Heart rate drift detected: 4.2% increase".to_owned()],
                insights: vec!["Heart rate drift detected: 4.2% increase".to_owned()],
                hr_drift_detected: true,
                performance_decline: false,
                insufficient_recovery: false,
                confidence: "medium".to_owned(),
                recommendations: vec!["Monitor recovery closely".to_owned()],
            })),
        ),
    ];

    for (label, shape) in shapes {
        let value = serde_json::to_value(shape).expect("serializes");
        let matched: Vec<usize> = arm_validators
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_valid(&value))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            matched.len(),
            1,
            "detect_patterns' {label} answer matched arms {matched:?}; it must match exactly one, \
             or a client cannot tell which shape it was handed:\n{value:#}"
        );
    }
}

#[test]
fn calculate_metrics_declares_a_schema_that_accepts_its_payload() {
    // An activity with no heart rate is ordinary — a bike computer without a
    // strap — and the summary reports it absent rather than zero.
    let sample = Formatted::Json(ActivityMetricsResult {
        pace: 5.24,
        speed: 11.45,
        intensity_score: 0.0,
        efficiency_score: 68.2,
        max_hr_used: 187.0,
        max_hr_source: "estimated from age".to_owned(),
        metrics_summary: MetricsInputSummary {
            distance_km: 12.4,
            duration_minutes: 65,
            elevation_meters: 143.0,
            average_heart_rate: None,
        },
    });

    assert_declares_and_accepts(
        <CalculateMetricsTool as McpTool<dyn ToolRuntime>>::definition(&CalculateMetricsTool)
            .output_schema,
        &serde_json::to_value(schemars::schema_for!(Formatted<ActivityMetricsResult>))
            .expect("derives"),
        &sample,
        "calculate_metrics",
    );
}

#[test]
fn the_analytics_schemas_reject_payloads_missing_a_required_field() {
    let trends = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(PerformanceTrendsResult)).expect("derives"),
    )
    .expect("compiles");
    assert!(
        !trends.is_valid(&json!({
            "metric": "pace",
            "timeframe": "month",
            "activities_analyzed": 18,
            "insights": [],
        })),
        "a trend answer with no trend is not something a client can read"
    );

    let metrics = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(ActivityMetricsResult)).expect("derives"),
    )
    .expect("compiles");
    assert!(
        !metrics.is_valid(&json!({
            "pace": 5.24,
            "speed": 11.45,
            "intensity_score": 0.0,
            "efficiency_score": 68.2,
            "max_hr_used": 187.0,
            "max_hr_source": "estimated from age",
        })),
        "calculate_metrics must report the inputs its numbers came from"
    );
}

#[test]
fn predict_performance_declares_a_schema_that_accepts_both_of_its_answers() {
    let derived = serde_json::to_value(schemars::schema_for!(Formatted<RacePredictionResult>))
        .expect("derives");
    assert_eq!(
        <PredictPerformanceTool as McpTool<dyn ToolRuntime>>::definition(&PredictPerformanceTool)
            .output_schema
            .expect("predict_performance must declare an outputSchema"),
        derived,
        "predict_performance declares a schema derived from a DIFFERENT result type"
    );

    let validator = jsonschema::validator_for(&derived).expect("compiles");
    let predicted = Formatted::Json(RacePredictionResult::Predicted(Box::new(
        RacePredictionDetail {
            target_sport: "run".to_owned(),
            vdot: 48.0,
            best_performance: BestPerformance {
                distance_meters: 10_000.0,
                time_seconds: 2_580.0,
                pace_min_km: "4:18".to_owned(),
                date: "2026-08-30T09:12:00+00:00".to_owned(),
            },
            predictions: vec![RacePrediction {
                distance: "Half Marathon".to_owned(),
                distance_meters: 21_097.5,
                predicted_time_seconds: 5_712.0,
                predicted_time_formatted: "1:35:12".to_owned(),
                predicted_pace_min_km: "4:31".to_owned(),
            }],
            confidence: "high".to_owned(),
            activities_analyzed: 34,
            notes: vec!["Based on VDOT methodology by Jack Daniels".to_owned()],
        },
    )));
    // Two ways to reach the empty answer: nothing to predict from, and a
    // computation that failed. Only the second carries an error.
    let no_history = Formatted::Json(RacePredictionResult::Unavailable(NoRacePrediction {
        target_sport: "run".to_owned(),
        message: "No running activities found for prediction".to_owned(),
        error: None,
        predictions: vec![],
    }));
    let failed = Formatted::Json(RacePredictionResult::Unavailable(NoRacePrediction {
        target_sport: "run".to_owned(),
        message: "Unable to calculate race predictions from available data".to_owned(),
        error: Some("Failed to generate predictions: no valid efforts".to_owned()),
        predictions: vec![],
    }));

    for (label, payload) in [
        ("predicted", &predicted),
        ("no_history", &no_history),
        ("failed", &failed),
    ] {
        let value = serde_json::to_value(payload).expect("serializes");
        assert!(
            validator.is_valid(&value),
            "predict_performance's {label} answer must satisfy its declared schema:\n{value:#}"
        );
        assert!(
            value["predictions"].is_array(),
            "{label}: predictions must always be an array — it used to be an empty OBJECT on the \
             no-history paths, so a client reading .length got undefined:\n{value:#}"
        );
    }
}

#[test]
fn the_two_race_prediction_shapes_match_exactly_one_arm_each() {
    // Same property the pattern shapes need, and the reason NoRacePrediction
    // carries an optional `error` instead of being split in two: a variant
    // whose required keys are a SUBSET of another's can never be told apart.
    let schema =
        serde_json::to_value(schemars::schema_for!(RacePredictionResult)).expect("derives");
    let defs = schema.get("$defs").cloned().unwrap_or_else(|| json!({}));
    let arm_validators: Vec<_> = schema["anyOf"]
        .as_array()
        .expect("an untagged enum derives a list of arms")
        .iter()
        .map(|arm| {
            let mut arm = arm.clone();
            arm.as_object_mut()
                .expect("each arm is an object")
                .insert("$defs".to_owned(), defs.clone());
            jsonschema::validator_for(&arm).expect("each arm compiles")
        })
        .collect();
    assert_eq!(
        arm_validators.len(),
        2,
        "predict_performance answers two shapes"
    );

    for (label, value) in [
        (
            "unavailable",
            serde_json::to_value(RacePredictionResult::Unavailable(NoRacePrediction {
                target_sport: "run".to_owned(),
                message: "No running activities found for prediction".to_owned(),
                error: None,
                predictions: vec![],
            }))
            .expect("serializes"),
        ),
        (
            "predicted",
            serde_json::to_value(RacePredictionResult::Predicted(Box::new(
                RacePredictionDetail {
                    target_sport: "run".to_owned(),
                    vdot: 48.0,
                    best_performance: BestPerformance {
                        distance_meters: 10_000.0,
                        time_seconds: 2_580.0,
                        pace_min_km: "4:18".to_owned(),
                        date: "2026-08-30T09:12:00+00:00".to_owned(),
                    },
                    predictions: vec![],
                    confidence: "medium".to_owned(),
                    activities_analyzed: 9,
                    notes: vec![],
                },
            )))
            .expect("serializes"),
        ),
    ] {
        let matched: Vec<usize> = arm_validators
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_valid(&value))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            matched.len(),
            1,
            "the {label} answer matched arms {matched:?}; it must match exactly one:\n{value:#}"
        );
    }
}

// ============================================================================
// admin
// ============================================================================

#[test]
fn each_admin_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "admin_list_system_coaches",
            <AdminListSystemCoachesTool as McpTool<dyn ToolRuntime>>::definition(
                &AdminListSystemCoachesTool,
            ),
            serde_json::to_value(schemars::schema_for!(
                Formatted<AdminListSystemCoachesResult>
            ))
            .expect("derives"),
        ),
        (
            "admin_create_system_coach",
            <AdminCreateSystemCoachTool as McpTool<dyn ToolRuntime>>::definition(
                &AdminCreateSystemCoachTool,
            ),
            serde_json::to_value(schemars::schema_for!(AdminCreateSystemCoachResult))
                .expect("derives"),
        ),
        (
            "admin_get_system_coach",
            <AdminGetSystemCoachTool as McpTool<dyn ToolRuntime>>::definition(
                &AdminGetSystemCoachTool,
            ),
            serde_json::to_value(schemars::schema_for!(Formatted<AdminGetSystemCoachResult>))
                .expect("derives"),
        ),
        (
            "admin_update_system_coach",
            <AdminUpdateSystemCoachTool as McpTool<dyn ToolRuntime>>::definition(
                &AdminUpdateSystemCoachTool,
            ),
            serde_json::to_value(schemars::schema_for!(AdminUpdateSystemCoachResult))
                .expect("derives"),
        ),
        (
            "admin_delete_system_coach",
            <AdminDeleteSystemCoachTool as McpTool<dyn ToolRuntime>>::definition(
                &AdminDeleteSystemCoachTool,
            ),
            serde_json::to_value(schemars::schema_for!(AdminDeleteSystemCoachResult))
                .expect("derives"),
        ),
        (
            "admin_assign_coach",
            <AdminAssignCoachTool as McpTool<dyn ToolRuntime>>::definition(&AdminAssignCoachTool),
            serde_json::to_value(schemars::schema_for!(AdminAssignCoachResult)).expect("derives"),
        ),
        (
            "admin_unassign_coach",
            <AdminUnassignCoachTool as McpTool<dyn ToolRuntime>>::definition(
                &AdminUnassignCoachTool,
            ),
            serde_json::to_value(schemars::schema_for!(AdminUnassignCoachResult)).expect("derives"),
        ),
        (
            "admin_list_coach_assignments",
            <AdminListCoachAssignmentsTool as McpTool<dyn ToolRuntime>>::definition(
                &AdminListCoachAssignmentsTool,
            ),
            serde_json::to_value(schemars::schema_for!(AdminListCoachAssignmentsResult))
                .expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn every_admin_projection_declares_the_visibility_an_operator_decides() {
    // The operator-facing twins differ from the athlete-facing coach tools in
    // exactly this: who a system coach is visible to is the operator's call,
    // so every admin projection reports it. An admin schema that dropped it
    // would be describing the athlete's view by mistake.
    for (name, schema) in [
        (
            "SystemCoachEntry",
            serde_json::to_value(schemars::schema_for!(SystemCoachEntry)).expect("derives"),
        ),
        (
            "AdminCreateSystemCoachResult",
            serde_json::to_value(schemars::schema_for!(AdminCreateSystemCoachResult))
                .expect("derives"),
        ),
        (
            "AdminGetSystemCoachResult",
            serde_json::to_value(schemars::schema_for!(AdminGetSystemCoachResult))
                .expect("derives"),
        ),
        (
            "AdminUpdateSystemCoachResult",
            serde_json::to_value(schemars::schema_for!(AdminUpdateSystemCoachResult))
                .expect("derives"),
        ),
    ] {
        assert!(
            schema["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("{name} is an object schema"))
                .contains_key("visibility"),
            "{name} must declare visibility: it is what makes this the operator's view"
        );
    }
}

#[test]
fn admin_list_system_coaches_declares_a_schema_that_accepts_its_payload() {
    let sample = Formatted::Json(AdminListSystemCoachesResult {
        coaches: vec![SystemCoachEntry {
            id: "b21f0f4e-0000-4000-8000-000000000001".to_owned(),
            title: "Nutrition Basics".to_owned(),
            description: Some("Shipped with the platform".to_owned()),
            category: "nutrition".to_owned(),
            tags: vec!["nutrition".to_owned()],
            token_count: 540,
            visibility: "tenant".to_owned(),
            created_at: "2026-07-01T08:00:00+00:00".to_owned(),
            updated_at: "2026-08-14T11:30:00+00:00".to_owned(),
        }],
        count: 1,
        total: 12,
        offset: 0,
    });

    assert_declares_and_accepts(
        <AdminListSystemCoachesTool as McpTool<dyn ToolRuntime>>::definition(
            &AdminListSystemCoachesTool,
        )
        .output_schema,
        &serde_json::to_value(schemars::schema_for!(
            Formatted<AdminListSystemCoachesResult>
        ))
        .expect("derives"),
        &sample,
        "admin_list_system_coaches",
    );
}

#[test]
fn an_assignment_row_validates_without_an_email_or_an_assigner() {
    // Both are Option on the wire and the compiler caught it: a deleted
    // account leaves the assignment row behind with no email to join to, and
    // rows predating the assigned_by column have no operator to name. A
    // schema demanding either would reject a listing the tool really sends.
    let sample = AdminListCoachAssignmentsResult {
        coach_id: "b21f0f4e-0000-4000-8000-000000000002".to_owned(),
        assignments: vec![
            CoachAssignmentEntry {
                user_id: "u-1".to_owned(),
                user_email: Some("alice@acme.test".to_owned()),
                assigned_at: "2026-08-01T09:00:00+00:00".to_owned(),
                assigned_by: Some("admin-1".to_owned()),
            },
            CoachAssignmentEntry {
                user_id: "u-2".to_owned(),
                user_email: None,
                assigned_at: "2026-05-02T09:00:00+00:00".to_owned(),
                assigned_by: None,
            },
        ],
        count: 2,
        total: 240,
        truncated: true,
    };
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(AdminListCoachAssignmentsResult))
            .expect("derives"),
    )
    .expect("compiles");
    let value = serde_json::to_value(&sample).expect("serializes");

    assert!(
        validator.is_valid(&value),
        "an assignment with no email and no assigner must still validate:\n{value:#}"
    );
    assert!(
        value["truncated"].as_bool() == Some(true)
            && value["total"].as_u64() > value["count"].as_u64(),
        "a truncated listing must say so, or an operator reads a short list as a complete one"
    );
}

#[test]
fn the_admin_schemas_reject_payloads_missing_a_required_field() {
    let assign = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(AdminAssignCoachResult)).expect("derives"),
    )
    .expect("compiles");
    assert!(
        !assign.is_valid(&json!({
            "assigned": true,
            "coach_id": "b21f0f4e-0000-4000-8000-000000000003",
            "coach_title": "Nutrition Basics",
            "user_id": "u-1",
        })),
        "an assignment reply with no assigned_by is not an audit record"
    );

    let unassign = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(AdminUnassignCoachResult)).expect("derives"),
    )
    .expect("compiles");
    assert!(
        !unassign.is_valid(&json!({ "unassigned": true })),
        "admin_unassign_coach must say which coach and which athlete"
    );
}

// ============================================================================
// coach store
// ============================================================================

fn a_store_coach() -> StoreCoachEntry {
    StoreCoachEntry {
        id: "9c3a77b1-0000-4000-8000-000000000001".to_owned(),
        title: "Marathon Build".to_owned(),
        description: Some("Sixteen weeks to a first marathon".to_owned()),
        category: "training".to_owned(),
        tags: vec!["run".to_owned(), "marathon".to_owned()],
        sample_prompts: vec!["What should this week look like?".to_owned()],
        install_count: 412,
        published_at: Some("2026-06-11T14:00:00+00:00".to_owned()),
    }
}

#[test]
fn each_store_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "browse_coach_store",
            <BrowseCoachStoreTool as McpTool<dyn ToolRuntime>>::definition(&BrowseCoachStoreTool),
            serde_json::to_value(schemars::schema_for!(Formatted<BrowseCoachStoreResult>))
                .expect("derives"),
        ),
        (
            "search_coach_store",
            <SearchCoachStoreTool as McpTool<dyn ToolRuntime>>::definition(&SearchCoachStoreTool),
            serde_json::to_value(schemars::schema_for!(Formatted<SearchCoachStoreResult>))
                .expect("derives"),
        ),
        (
            "install_coach_from_store",
            <InstallCoachFromStoreTool as McpTool<dyn ToolRuntime>>::definition(
                &InstallCoachFromStoreTool,
            ),
            serde_json::to_value(schemars::schema_for!(
                Formatted<InstallCoachFromStoreResult>
            ))
            .expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn no_store_schema_promises_a_system_prompt() {
    // Browse and search return many coaches and the prompt is by far the
    // largest field on a store row; install echoes the same compact shape so
    // a client renders one card either way. A schema that declared the prompt
    // would be promising the tools send something they deliberately withhold.
    let entry = serde_json::to_value(schemars::schema_for!(StoreCoachEntry)).expect("derives");
    assert!(
        !entry["properties"]
            .as_object()
            .expect("object schema")
            .contains_key("system_prompt"),
        "the store projection must not declare system_prompt"
    );
    assert!(
        serde_json::to_value(a_store_coach())
            .expect("serializes")
            .get("system_prompt")
            .is_none(),
        "and the payload must not carry it either"
    );
}

#[test]
fn the_store_schemas_accept_their_payloads() {
    for (tool, derived, payload) in [
        (
            "browse_coach_store",
            serde_json::to_value(schemars::schema_for!(Formatted<BrowseCoachStoreResult>))
                .expect("derives"),
            serde_json::to_value(Formatted::Json(BrowseCoachStoreResult {
                coaches: vec![a_store_coach()],
                count: 1,
                has_more: true,
                next_cursor: Some("eyJvIjoyMH0".to_owned()),
            }))
            .expect("serializes"),
        ),
        (
            "browse_coach_store (last page)",
            serde_json::to_value(schemars::schema_for!(Formatted<BrowseCoachStoreResult>))
                .expect("derives"),
            // The last page has no cursor to hand back, and an unpublished
            // coach has no publication date. Both are ordinary.
            serde_json::to_value(Formatted::Json(BrowseCoachStoreResult {
                coaches: vec![StoreCoachEntry {
                    published_at: None,
                    description: None,
                    ..a_store_coach()
                }],
                count: 1,
                has_more: false,
                next_cursor: None,
            }))
            .expect("serializes"),
        ),
        (
            "search_coach_store",
            serde_json::to_value(schemars::schema_for!(Formatted<SearchCoachStoreResult>))
                .expect("derives"),
            serde_json::to_value(Formatted::Json(SearchCoachStoreResult {
                query: "marathon".to_owned(),
                count: 1,
                coaches: vec![a_store_coach()],
            }))
            .expect("serializes"),
        ),
        (
            "install_coach_from_store",
            serde_json::to_value(schemars::schema_for!(
                Formatted<InstallCoachFromStoreResult>
            ))
            .expect("derives"),
            serde_json::to_value(Formatted::Json(InstallCoachFromStoreResult {
                installed: true,
                coach: a_store_coach(),
                message: "'Marathon Build' is now in your coach library.".to_owned(),
            }))
            .expect("serializes"),
        ),
    ] {
        let validator =
            jsonschema::validator_for(&derived).unwrap_or_else(|e| panic!("{tool} schema: {e}"));
        assert!(
            validator.is_valid(&payload),
            "{tool}: the declared schema rejected the payload the tool sends:\n{payload:#}"
        );
    }
}

// ============================================================================
// sync
// ============================================================================

#[test]
fn each_sync_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "refresh_provider_data",
            <RefreshProviderDataTool as McpTool<dyn ToolRuntime>>::definition(
                &RefreshProviderDataTool,
            ),
            serde_json::to_value(schemars::schema_for!(RefreshProviderDataResult))
                .expect("derives"),
        ),
        (
            "get_data_freshness",
            <GetDataFreshnessTool as McpTool<dyn ToolRuntime>>::definition(&GetDataFreshnessTool),
            serde_json::to_value(schemars::schema_for!(DataFreshnessResult)).expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn both_refresh_shapes_match_exactly_one_arm() {
    // Asked for one provider the tool reports an outcome; asked for all of
    // them it reports what it started. `status` and `provider` are what keep
    // those apart — drop either and a client is guessing.
    let schema =
        serde_json::to_value(schemars::schema_for!(RefreshProviderDataResult)).expect("derives");
    let defs = schema.get("$defs").cloned().unwrap_or_else(|| json!({}));
    let arms: Vec<_> = schema["anyOf"]
        .as_array()
        .expect("an untagged enum derives a list of arms")
        .iter()
        .map(|arm| {
            let mut arm = arm.clone();
            arm.as_object_mut()
                .expect("each arm is an object")
                .insert("$defs".to_owned(), defs.clone());
            jsonschema::validator_for(&arm).expect("each arm compiles")
        })
        .collect();
    assert_eq!(arms.len(), 2, "refresh_provider_data answers two shapes");

    for (label, value) in [
        (
            "single",
            serde_json::to_value(RefreshProviderDataResult::Single(SingleProviderRefresh {
                provider: "strava".to_owned(),
                success: true,
                message: "Synced 42 activities".to_owned(),
                records_synced: 42,
            }))
            .expect("serializes"),
        ),
        (
            "all",
            serde_json::to_value(RefreshProviderDataResult::All(AllProvidersRefresh {
                status: "refresh_triggered".to_owned(),
                refreshing: vec!["garmin".to_owned()],
                already_fresh: vec!["strava".to_owned()],
                details: vec![],
            }))
            .expect("serializes"),
        ),
    ] {
        let matched: Vec<usize> = arms
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_valid(&value))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            matched.len(),
            1,
            "the {label} refresh answer matched arms {matched:?}; it must match exactly one:\n{value:#}"
        );
    }
}

#[test]
fn a_failed_sync_is_a_reported_outcome_not_a_rejected_payload() {
    // A provider being down is news the athlete can act on, so the tool
    // answers success:false rather than erroring. The schema has to accept
    // that, or the honest answer becomes a protocol violation.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(RefreshProviderDataResult)).expect("derives"),
    )
    .expect("compiles");
    let failed = serde_json::to_value(RefreshProviderDataResult::Single(SingleProviderRefresh {
        provider: "whoop".to_owned(),
        success: false,
        message: "Provider returned 503".to_owned(),
        records_synced: 0,
    }))
    .expect("serializes");

    assert!(
        validator.is_valid(&failed),
        "a reported sync failure must satisfy the schema:\n{failed:#}"
    );
}

// ============================================================================
// commitments
// ============================================================================

#[test]
fn each_commitment_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "commitment_create",
            <CommitmentCreateTool as McpTool<dyn ToolRuntime>>::definition(&CommitmentCreateTool),
            serde_json::to_value(schemars::schema_for!(CommitmentCreateResult)).expect("derives"),
        ),
        (
            "commitment_cancel",
            <CommitmentCancelTool as McpTool<dyn ToolRuntime>>::definition(&CommitmentCancelTool),
            serde_json::to_value(schemars::schema_for!(CommitmentCancelResult)).expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn a_duplicate_commitment_is_a_valid_answer_not_an_error() {
    // A second identical commitment is dropped rather than stacked, so the
    // coach can say "already noted" instead of promising a second check. The
    // schema has to accept recorded:false, or the honest answer to a repeat
    // becomes a protocol violation.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(CommitmentCreateResult)).expect("derives"),
    )
    .expect("compiles");

    for (label, sample) in [
        (
            "first",
            CommitmentCreateResult {
                commitment_id: "c1f0b2d3-0000-4000-8000-000000000001".to_owned(),
                recorded: true,
                sessions: 3,
                sport: Some("run".to_owned()),
                window_end: "2026-09-13T00:00:00+00:00".to_owned(),
                duplicate_of_open_commitment: false,
            },
        ),
        (
            "duplicate",
            CommitmentCreateResult {
                commitment_id: "c1f0b2d3-0000-4000-8000-000000000001".to_owned(),
                recorded: false,
                sessions: 3,
                // No sport named: a commitment can be to sessions in general.
                sport: None,
                window_end: "2026-09-13T00:00:00+00:00".to_owned(),
                duplicate_of_open_commitment: true,
            },
        ),
    ] {
        let value = serde_json::to_value(&sample).expect("serializes");
        assert!(
            validator.is_valid(&value),
            "the {label} commitment answer must satisfy the declared schema:\n{value:#}"
        );
        assert_eq!(
            value["recorded"].as_bool(),
            value["duplicate_of_open_commitment"].as_bool().map(|d| !d),
            "the two flags answer different questions but must stay consistent"
        );
    }
}

#[test]
fn cancelling_nothing_is_a_valid_answer_too() {
    // False means nothing open matched. The athlete is not committed either
    // way, so it is a success, and the schema must accept it.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(CommitmentCancelResult)).expect("derives"),
    )
    .expect("compiles");
    let value = serde_json::to_value(CommitmentCancelResult {
        commitment_id: "c1f0b2d3-0000-4000-8000-000000000002".to_owned(),
        cancelled: false,
    })
    .expect("serializes");

    assert!(
        validator.is_valid(&value),
        "must accept a no-op cancel:\n{value:#}"
    );
    assert!(
        !validator.is_valid(&json!({ "cancelled": false })),
        "but it must still say WHICH commitment was asked about"
    );
}

// ============================================================================
// fitness_config
// ============================================================================

#[test]
fn each_fitness_config_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "get_fitness_config",
            <GetFitnessConfigTool as McpTool<dyn ToolRuntime>>::definition(&GetFitnessConfigTool),
            serde_json::to_value(schemars::schema_for!(GetFitnessConfigResult)).expect("derives"),
        ),
        (
            "set_fitness_config",
            <SetFitnessConfigTool as McpTool<dyn ToolRuntime>>::definition(&SetFitnessConfigTool),
            serde_json::to_value(schemars::schema_for!(SetFitnessConfigResult)).expect("derives"),
        ),
        (
            "list_fitness_configs",
            <ListFitnessConfigsTool as McpTool<dyn ToolRuntime>>::definition(
                &ListFitnessConfigsTool,
            ),
            serde_json::to_value(schemars::schema_for!(ListFitnessConfigsResult)).expect("derives"),
        ),
        (
            "delete_fitness_config",
            <DeleteFitnessConfigTool as McpTool<dyn ToolRuntime>>::definition(
                &DeleteFitnessConfigTool,
            ),
            serde_json::to_value(schemars::schema_for!(DeleteFitnessConfigResult))
                .expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn a_missing_fitness_config_answers_with_a_null_config_not_an_error() {
    // A configuration nobody has saved is a fact about the tenant, not a
    // fault, so the tool reports it. `config: null` has to be describable or
    // the ordinary answer to a first-time read is a protocol violation.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(GetFitnessConfigResult)).expect("derives"),
    )
    .expect("compiles");
    let value = serde_json::to_value(GetFitnessConfigResult {
        configuration_name: "default".to_owned(),
        config: None,
        source: "not_found".to_owned(),
        message: Some("No configuration found with name 'default'".to_owned()),
        retrieved_at: "2026-09-06T17:00:00+00:00".to_owned(),
    })
    .expect("serializes");

    assert!(
        validator.is_valid(&value),
        "a not-found configuration must satisfy the schema:\n{value:#}"
    );
    assert!(
        value["config"].is_null(),
        "and it must send config as an explicit null, not omit it — a client \
         distinguishes 'no configuration' from 'field missing'"
    );
    assert_eq!(value["source"], "not_found");
}

#[test]
fn deleting_nothing_omits_the_delete_timestamp() {
    // success:false means there was nothing to delete, which leaves the
    // tenant in the state the caller wanted. deleted_at is what separates a
    // delete that happened from one that had nothing to do.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(DeleteFitnessConfigResult)).expect("derives"),
    )
    .expect("compiles");

    let removed = serde_json::to_value(DeleteFitnessConfigResult {
        success: true,
        configuration_name: "old".to_owned(),
        user_level: true,
        message: "Configuration 'old' deleted successfully".to_owned(),
        deleted_at: Some("2026-09-06T17:05:00+00:00".to_owned()),
    })
    .expect("serializes");
    let nothing = serde_json::to_value(DeleteFitnessConfigResult {
        success: false,
        configuration_name: "never-existed".to_owned(),
        user_level: true,
        message: "Configuration 'never-existed' not found".to_owned(),
        deleted_at: None,
    })
    .expect("serializes");

    assert!(validator.is_valid(&removed), "a real delete must validate");
    assert!(validator.is_valid(&nothing), "and so must a no-op one");
    assert!(
        removed.get("deleted_at").is_some() && nothing.get("deleted_at").is_none(),
        "only the delete that happened carries a timestamp"
    );
}

#[test]
fn the_fitness_config_listing_reports_both_scopes_it_merged() {
    // A name can be tenant-level and overridden per athlete. Reporting only
    // the merged list would hide that, so the two scopes stay on the wire.
    let derived =
        serde_json::to_value(schemars::schema_for!(ListFitnessConfigsResult)).expect("derives");
    let props = derived["properties"].as_object().expect("object schema");
    for field in [
        "configurations",
        "user_specific",
        "tenant_level",
        "total_count",
    ] {
        assert!(
            props.contains_key(field),
            "list_fitness_configs must declare {field}"
        );
    }

    let validator = jsonschema::validator_for(&derived).expect("compiles");
    let value = serde_json::to_value(ListFitnessConfigsResult {
        configurations: vec!["default".to_owned(), "racing".to_owned()],
        user_specific: vec!["racing".to_owned()],
        tenant_level: vec!["default".to_owned(), "racing".to_owned()],
        total_count: 2,
        retrieved_at: "2026-09-06T17:10:00+00:00".to_owned(),
    })
    .expect("serializes");
    assert!(
        validator.is_valid(&value),
        "the merged listing must validate:\n{value:#}"
    );
}

// ============================================================================
// the single-tool surfaces
// ============================================================================

#[test]
fn each_single_tool_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "get_weather_forecast",
            <GetWeatherForecastTool as McpTool<dyn ToolRuntime>>::definition(
                &GetWeatherForecastTool,
            ),
            serde_json::to_value(schemars::schema_for!(WeatherForecastResult)).expect("derives"),
        ),
        (
            "discover_routes",
            <DiscoverRoutesTool as McpTool<dyn ToolRuntime>>::definition(&DiscoverRoutesTool),
            serde_json::to_value(schemars::schema_for!(DiscoverRoutesResult)).expect("derives"),
        ),
        (
            "get_group_member_activities",
            <GetGroupMemberActivitiesTool as McpTool<dyn ToolRuntime>>::definition(
                &GetGroupMemberActivitiesTool,
            ),
            serde_json::to_value(schemars::schema_for!(GroupMemberActivitiesResult))
                .expect("derives"),
        ),
        (
            "push_training_plan",
            <PushTrainingPlanTool as McpTool<dyn ToolRuntime>>::definition(&PushTrainingPlanTool),
            serde_json::to_value(schemars::schema_for!(PushReport)).expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn a_coordinate_forecast_omits_the_place_a_named_one_carries() {
    // The place name is what the caller asked for resolved back to them.
    // A coordinate lookup resolved nothing, so there is nothing to echo, and
    // the key is absent rather than an empty string.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(WeatherForecastResult)).expect("derives"),
    )
    .expect("compiles");

    let named = serde_json::to_value(WeatherForecastResult {
        latitude: 45.5,
        longitude: -73.57,
        timestamp: "2026-09-08T11:00:00+00:00".to_owned(),
        temperature_celsius: 18.5,
        conditions: "partly cloudy".to_owned(),
        humidity_percentage: Some(62.0),
        wind_speed_kmh: Some(14.0),
        place: Some("Montréal".to_owned()),
    })
    .expect("serializes");
    // A provider model that reported neither humidity nor wind for that hour
    // is ordinary, and both are Option because of it.
    let bare = serde_json::to_value(WeatherForecastResult {
        latitude: 45.5,
        longitude: -73.57,
        timestamp: "2026-09-08T11:00:00+00:00".to_owned(),
        temperature_celsius: 18.5,
        conditions: "clear".to_owned(),
        humidity_percentage: None,
        wind_speed_kmh: None,
        place: None,
    })
    .expect("serializes");

    assert!(
        validator.is_valid(&named),
        "named-place forecast:\n{named:#}"
    );
    assert!(validator.is_valid(&bare), "coordinate forecast:\n{bare:#}");
    assert!(named.get("place").is_some(), "a named place is echoed back");
    assert!(
        bare.get("place").is_none(),
        "a coordinate lookup omits place rather than sending an empty string"
    );
}

#[test]
fn a_route_with_almost_no_osm_tags_still_validates() {
    // These come from OpenStreetMap, tagged by volunteers. A way with a name
    // and nothing else is the common case, which is why distance and
    // difficulty are optional — a schema demanding them would reject most of
    // what the tool actually returns.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(DiscoverRoutesResult)).expect("derives"),
    )
    .expect("compiles");
    let value = serde_json::to_value(DiscoverRoutesResult {
        sport_type: "run".to_owned(),
        center: RouteSearchCenter {
            latitude: 45.5,
            longitude: -73.57,
            display_name: None,
        },
        radius_meters: 5_000,
        count: 1,
        routes: vec![DiscoveredRouteEntry {
            name: "Canal de Lachine".to_owned(),
            route_type: "cycling".to_owned(),
            distance_meters: None,
            difficulty: None,
            source: "overpass".to_owned(),
            latitude: 45.47,
            longitude: -73.58,
            distance_from_center_meters: 3_400.0,
        }],
    })
    .expect("serializes");

    assert!(
        validator.is_valid(&value),
        "an untagged OSM way must validate:\n{value:#}"
    );
}

#[test]
fn the_group_projection_carries_no_more_of_a_peer_than_it_should() {
    // This is another athlete's data, shown because they consented to share
    // it with the group. The projection is deliberately narrow, and the
    // schema is where that narrowness becomes checkable: a field added here
    // is a field shared with the peer's whole group.
    let derived =
        serde_json::to_value(schemars::schema_for!(GroupMemberActivity)).expect("derives");
    let declared: Vec<String> = derived["properties"]
        .as_object()
        .expect("object schema")
        .keys()
        .cloned()
        .collect();

    let expected = [
        "average_heart_rate",
        "average_power",
        "calories",
        "distance_km",
        "duration_minutes",
        "elevation_gain_m",
        "id",
        "max_heart_rate",
        "max_power",
        "name",
        "provider",
        "sport",
        "start_date",
    ];
    assert_eq!(
        declared, expected,
        "the group projection changed. Adding a field here shares it with the \
         peer's whole group — confirm that is intended, then update this list."
    );

    // And the whole answer validates, member named by display name only.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(GroupMemberActivitiesResult)).expect("derives"),
    )
    .expect("compiles");
    let value = serde_json::to_value(GroupMemberActivitiesResult {
        member: "Alice".to_owned(),
        group: "Tuesday Track".to_owned(),
        count: 1,
        activities: vec![GroupMemberActivity {
            id: "strava-991".to_owned(),
            name: "Intervals".to_owned(),
            sport: "Run".to_owned(),
            start_date: "2026-09-01T17:00:00+00:00".to_owned(),
            distance_km: Some(10.4),
            duration_minutes: 52,
            elevation_gain_m: Some(48.0),
            average_heart_rate: Some(158),
            max_heart_rate: Some(181),
            average_power: None,
            max_power: None,
            calories: Some(690),
            provider: "strava".to_owned(),
        }],
    })
    .expect("serializes");
    assert!(validator.is_valid(&value), "group activities:\n{value:#}");
}

// ============================================================================
// athlete and stats
// ============================================================================

/// These two answer with the provider model under a named key.
///
/// The key is the contract, not decoration: `GetAthleteResponseSchema` in the
/// TypeScript SDK reads `athlete`, and `provider_backend_resolver_test`
/// asserts it. Consolidating the envelope unwrapped them once and that test
/// caught it — which is the argument for the named envelope over the bare
/// model, however tempting one level fewer looks.
#[test]
fn the_athlete_and_stats_tools_declare_the_models_they_answer_with() {
    for (tool_name, declared, derived) in [
        (
            "get_athlete",
            <GetAthleteTool as McpTool<dyn ToolRuntime>>::definition(&GetAthleteTool),
            serde_json::to_value(schemars::schema_for!(Formatted<GetAthleteResult>))
                .expect("derives"),
        ),
        (
            "get_stats",
            <GetStatsTool as McpTool<dyn ToolRuntime>>::definition(&GetStatsTool),
            serde_json::to_value(schemars::schema_for!(Formatted<GetStatsResult>))
                .expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn the_toon_envelope_key_is_fixed_rather_than_named_after_the_tool() {
    // These two used to build their own TOON envelope keyed `athlete_toon`
    // and `stats_toon`. A property name that changes per tool cannot be
    // stated in a schema at all, which is why the envelope keys are fixed —
    // and this is the assertion that keeps someone from reintroducing one.
    let schema =
        serde_json::to_value(schemars::schema_for!(Formatted<GetAthleteResult>)).expect("derives");

    // The PROPERTY names, not the rendered text. The envelope's own doc
    // comment names the spellings it replaced, so a substring match reads
    // documentation as contract — which is exactly what this test did on its
    // first run, and it failed for that reason rather than a real one.
    let keys: Vec<String> = schema["anyOf"]
        .as_array()
        .expect("the envelope derives a list of arms")
        .iter()
        .filter_map(|arm| arm.get("properties")?.as_object())
        .flat_map(|props| props.keys().cloned())
        .collect();

    for banned in ["athlete_toon", "stats_toon", "result_toon"] {
        assert!(
            !keys.iter().any(|k| k == banned),
            "the envelope must not name its key after the tool: found {banned} in {keys:?}"
        );
    }
    assert!(
        keys.iter().any(|k| k == "toon"),
        "and it must carry the fixed toon key: {keys:?}"
    );
}

// ============================================================================
// the four remaining recipe tools
// ============================================================================

#[test]
fn each_remaining_recipe_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "get_recipe_constraints",
            <GetRecipeConstraintsTool as McpTool<dyn ToolRuntime>>::definition(
                &GetRecipeConstraintsTool,
            ),
            serde_json::to_value(schemars::schema_for!(RecipeConstraintsResult)).expect("derives"),
        ),
        (
            "validate_recipe",
            <ValidateRecipeTool as McpTool<dyn ToolRuntime>>::definition(&ValidateRecipeTool),
            serde_json::to_value(schemars::schema_for!(ValidateRecipeResult)).expect("derives"),
        ),
        (
            "save_recipe",
            <SaveRecipeTool as McpTool<dyn ToolRuntime>>::definition(&SaveRecipeTool),
            serde_json::to_value(schemars::schema_for!(SaveRecipeResult)).expect("derives"),
        ),
        (
            "delete_recipe",
            <DeleteRecipeTool as McpTool<dyn ToolRuntime>>::definition(&DeleteRecipeTool),
            serde_json::to_value(schemars::schema_for!(DeleteRecipeResult)).expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn the_two_tdee_fields_travel_together() {
    // Both come off the athlete's stored energy expenditure, so either both
    // are there or neither is. `tdee_based` is always present and says which
    // case it is, so a client reads one field instead of probing for a key.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(RecipeConstraintsResult)).expect("derives"),
    )
    .expect("compiles");

    let with_tdee = serde_json::to_value(RecipeConstraintsResult {
        calories: 720.0,
        protein_g: Some(45.0),
        carbs_g: Some(80.0),
        fat_g: Some(22.0),
        meal_timing: "postworkout".to_owned(),
        meal_timing_description: "Refuelling after a session".to_owned(),
        prompt_hint: "A 720 kcal recovery meal".to_owned(),
        max_prep_time_mins: Some(20),
        max_cook_time_mins: None,
        tdee_based: true,
        tdee: Some(2_880.0),
        tdee_proportion: Some(0.25),
    })
    .expect("serializes");
    // No stored TDEE: generic targets, and both fields gone. A meal can also
    // be specified by calories alone, which is why the macros are optional.
    let without = serde_json::to_value(RecipeConstraintsResult {
        calories: 600.0,
        protein_g: None,
        carbs_g: None,
        fat_g: None,
        meal_timing: "dinner".to_owned(),
        meal_timing_description: "Evening meal".to_owned(),
        prompt_hint: "A 600 kcal dinner".to_owned(),
        max_prep_time_mins: None,
        max_cook_time_mins: None,
        tdee_based: false,
        tdee: None,
        tdee_proportion: None,
    })
    .expect("serializes");

    assert!(validator.is_valid(&with_tdee), "TDEE-based:\n{with_tdee:#}");
    assert!(validator.is_valid(&without), "generic:\n{without:#}");
    assert_eq!(
        with_tdee.get("tdee").is_some(),
        with_tdee.get("tdee_proportion").is_some(),
        "the two TDEE fields must appear together"
    );
    assert_eq!(
        without.get("tdee").is_some(),
        without.get("tdee_proportion").is_some(),
        "and be absent together"
    );
    assert_eq!(without["tdee_based"], false);
}

#[test]
fn an_unmatched_ingredient_reports_a_null_match_and_no_id() {
    // "Checked, no USDA match" is a different answer from "not checked", so
    // usda_match is an explicit null. There is no id to give in that case, so
    // fdc_id is omitted entirely — and validation_completeness is how the
    // athlete knows the nutrition totals are understated because of it.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(ValidateRecipeResult)).expect("derives"),
    )
    .expect("compiles");
    let value = serde_json::to_value(ValidateRecipeResult {
        validated: true,
        servings: 4,
        nutrition_per_serving: ServingNutrition {
            calories: 512.0,
            protein_g: 31.5,
            carbs_g: 48.2,
            fat_g: 19.4,
            fiber_g: 6.1,
            sodium_mg: 480.0,
            sugar_g: 7.3,
        },
        ingredients: vec![
            ValidatedIngredient {
                name: "chicken breast".to_owned(),
                amount: 400.0,
                unit: "g".to_owned(),
                grams: 400.0,
                fdc_id: Some(171_077),
                usda_match: Some("Chicken, broilers or fryers, breast".to_owned()),
            },
            ValidatedIngredient {
                name: "grandma's spice mix".to_owned(),
                amount: 2.0,
                unit: "tbsp".to_owned(),
                grams: 14.0,
                fdc_id: None,
                usda_match: None,
            },
        ],
        warnings: vec!["No USDA match found for: grandma's spice mix".to_owned()],
        validated_at: "2026-09-06T18:00:00+00:00".to_owned(),
        validation_completeness: 50.0,
        usda_matched_count: 1,
        total_ingredients: 2,
    })
    .expect("serializes");

    assert!(
        validator.is_valid(&value),
        "a partly-matched recipe:\n{value:#}"
    );
    let unmatched = &value["ingredients"][1];
    assert!(
        unmatched["usda_match"].is_null(),
        "an unmatched ingredient reports a null match, not an absent key"
    );
    assert!(
        unmatched.get("fdc_id").is_none(),
        "and omits fdc_id, because there is no identifier to give"
    );
}

// ============================================================================
// compare_activities
// ============================================================================

#[test]
fn compare_activities_declares_a_schema_that_accepts_all_three_modes() {
    // Three modes, one shape, discriminated by comparison_type. As separate
    // untagged variants this could not work: the empty pr_comparison answer
    // requires only keys every other mode also carries, so it would match
    // several arms and a client could not tell which it held.
    let derived = serde_json::to_value(schemars::schema_for!(Formatted<CompareActivitiesResult>))
        .expect("derives");
    assert_eq!(
        <CompareActivitiesTool as McpTool<dyn ToolRuntime>>::definition(&CompareActivitiesTool)
            .output_schema
            .expect("compare_activities must declare an outputSchema"),
        derived,
        "compare_activities declares a schema derived from a DIFFERENT result type"
    );
    let validator = jsonschema::validator_for(&derived).expect("compiles");

    let similar = CompareActivitiesResult {
        activity_id: "strava-1".to_owned(),
        comparison_type: "similar_activities".to_owned(),
        comparison_count: Some(4),
        sport_type: Some("Run".to_owned()),
        comparison_activity_id: None,
        comparison_activity_name: None,
        comparisons: Some(vec![MetricComparison {
            metric: "pace".to_owned(),
            current: 5.1,
            average: Some(5.4),
            comparison: None,
            difference_percent: -5.6,
            improved: Some(true),
        }]),
        pr_comparisons: None,
        error: None,
        insights: vec!["Pace improved by 5.6% compared to similar activities".to_owned()],
    };
    let records = CompareActivitiesResult {
        activity_id: "strava-1".to_owned(),
        comparison_type: "pr_comparison".to_owned(),
        comparison_count: None,
        sport_type: Some("Run".to_owned()),
        comparison_activity_id: None,
        comparison_activity_name: None,
        comparisons: None,
        pr_comparisons: Some(vec![PersonalRecordComparison {
            metric: "distance".to_owned(),
            current: 21_100.0,
            personal_record: 21_100.0,
            is_record: true,
            percent_of_pr: Some(100.0),
        }]),
        error: None,
        insights: vec!["New distance PR! 🎉".to_owned()],
    };
    let specific = CompareActivitiesResult {
        activity_id: "strava-1".to_owned(),
        comparison_type: "specific_activity".to_owned(),
        comparison_count: None,
        sport_type: Some("Run".to_owned()),
        comparison_activity_id: Some("strava-2".to_owned()),
        comparison_activity_name: Some("Last week's tempo".to_owned()),
        comparisons: Some(vec![MetricComparison {
            metric: "average_power".to_owned(),
            current: 265.0,
            average: None,
            comparison: Some(250.0),
            difference_percent: 6.0,
            improved: Some(true),
        }]),
        pr_comparisons: None,
        error: None,
        insights: vec!["Power was 6.0% higher".to_owned()],
    };
    // The named activity did not exist. Reported, not raised: the athlete
    // gave an id and deserves to be told it is wrong.
    let missing = CompareActivitiesResult {
        activity_id: "strava-1".to_owned(),
        comparison_type: "specific_activity".to_owned(),
        comparison_count: None,
        sport_type: None,
        comparison_activity_id: None,
        comparison_activity_name: None,
        comparisons: None,
        pr_comparisons: None,
        error: Some("Activity with ID 'nope' not found".to_owned()),
        insights: vec!["Could not find activity 'nope' for comparison".to_owned()],
    };

    for (label, payload) in [
        ("similar", &similar),
        ("records", &records),
        ("specific", &specific),
        ("missing", &missing),
    ] {
        let value = serde_json::to_value(Formatted::Json(payload)).expect("serializes");
        assert!(
            validator.is_valid(&value),
            "compare_activities' {label} answer must satisfy its declared schema:\n{value:#}"
        );
        assert!(
            value["comparison_type"].is_string(),
            "{label}: comparison_type is the discriminator and is always present"
        );
    }
}

#[test]
fn a_comparison_row_carries_one_baseline_not_both() {
    // `average` is the mean across similar activities; `comparison` is the
    // single activity compared against. A row carries exactly one, and which
    // follows from the mode — a row with both would mean the tool compared
    // against two different baselines at once.
    let similar = serde_json::to_value(MetricComparison {
        metric: "heart_rate".to_owned(),
        current: 158.0,
        average: Some(162.0),
        comparison: None,
        difference_percent: -2.5,
        improved: Some(true),
    })
    .expect("serializes");
    let specific = serde_json::to_value(MetricComparison {
        metric: "heart_rate".to_owned(),
        current: 158.0,
        average: None,
        comparison: Some(164.0),
        difference_percent: -3.7,
        improved: Some(true),
    })
    .expect("serializes");

    assert!(similar.get("average").is_some() && similar.get("comparison").is_none());
    assert!(specific.get("comparison").is_some() && specific.get("average").is_none());

    // Distance and elevation are what the route was, not how well it went,
    // so they carry no verdict.
    let neutral = serde_json::to_value(MetricComparison {
        metric: "elevation_gain".to_owned(),
        current: 240.0,
        average: Some(180.0),
        comparison: None,
        difference_percent: 33.3,
        improved: None,
    })
    .expect("serializes");
    assert!(
        neutral.get("improved").is_none(),
        "a metric with no better direction must omit `improved`, not guess one"
    );
}

// ============================================================================
// physiology
// ============================================================================

#[test]
fn each_physiology_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "set_physiology",
            <SetPhysiologyTool as McpTool<dyn ToolRuntime>>::definition(&SetPhysiologyTool),
            serde_json::to_value(schemars::schema_for!(SetPhysiologyResult)).expect("derives"),
        ),
        (
            "estimate_vo2max",
            <EstimateVo2maxTool as McpTool<dyn ToolRuntime>>::definition(&EstimateVo2maxTool),
            serde_json::to_value(schemars::schema_for!(EstimateVo2maxResult)).expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn an_almost_empty_physiology_profile_still_validates() {
    // A profile is built up over time. An athlete who has given only a
    // resting heart rate has one field set, and every measurement is Option
    // because of it — reporting an unknown as zero would let a coach reason
    // off a fabricated number.
    let derived =
        serde_json::to_value(schemars::schema_for!(SetPhysiologyResult)).expect("derives");
    let validator = jsonschema::validator_for(&derived).expect("compiles");
    let value = serde_json::to_value(SetPhysiologyResult {
        saved: true,
        created: true,
        updated_fields: vec!["resting_hr"],
        profile: PhysiologyProfile {
            ftp_watts: None,
            threshold_pace_sec_per_km: None,
            max_hr: None,
            resting_hr: Some(48),
            lactate_threshold_percentage: None,
            vo2_max: None,
            weight: None,
            age: None,
            fitness_level: FitnessLevel::Intermediate,
            primary_sport: SportType::Run,
            training_experience_years: None,
            hr_zones: None,
            power_zones: None,
        },
    })
    .expect("serializes");

    assert!(
        validator.is_valid(&value),
        "a one-field profile must satisfy the schema:\n{value:#}"
    );
    // hr_zones needs BOTH a resting and a maximum heart rate, so a profile
    // with only one of them has none — absent, not an empty object.
    assert!(
        value["profile"]["hr_zones"].is_null(),
        "zones derived from a pair must be absent when only one is known"
    );
}

#[test]
fn the_updated_fields_list_names_fields_never_measurements() {
    // The measurements are health data. The tool logs and reports which
    // fields were set, by name — and this asserts the list stays names, so
    // nobody "improves" it into a map of what was written.
    let derived =
        serde_json::to_value(schemars::schema_for!(SetPhysiologyResult)).expect("derives");
    let updated = &derived["properties"]["updated_fields"];
    assert_eq!(
        updated["type"], "array",
        "updated_fields is a list of names: {updated:#}"
    );
    assert_eq!(
        updated["items"]["type"], "string",
        "of STRINGS — a list of objects would be the measurements themselves: {updated:#}"
    );
}

#[test]
fn estimate_vo2max_says_it_did_not_save() {
    // The estimate comes off a published equation fitted on a field test, and
    // an athlete should confirm a number before it shapes their zones. So the
    // tool reports saved:false and tells the caller what to do next — both
    // are on the wire, and both are asserted rather than assumed.
    let derived =
        serde_json::to_value(schemars::schema_for!(EstimateVo2maxResult)).expect("derives");
    let validator = jsonschema::validator_for(&derived).expect("compiles");
    let value = serde_json::to_value(EstimateVo2maxResult {
        method: "cooper_12_minute".to_owned(),
        vo2max_ml_kg_min: 52.4,
        formula: "Cooper (1968): VO2max = (distance_m - 504.9) / 44.73".to_owned(),
        defaults_from_profile: vec!["weight", "age"],
        stored_vo2_max: None,
        saved: false,
        to_store: "call set_physiology with vo2_max once the athlete confirms the number"
            .to_owned(),
    })
    .expect("serializes");

    assert!(validator.is_valid(&value), "estimate answer:\n{value:#}");
    assert_eq!(
        value["saved"], false,
        "this tool estimates, it does not write"
    );
    assert!(
        value["to_store"]
            .as_str()
            .is_some_and(|s| s.contains("set_physiology")),
        "and it names the tool that WOULD write it"
    );
    assert!(
        value["stored_vo2_max"].is_null(),
        "an athlete with no stored VO2max gets null, not the estimate echoed back"
    );
}
