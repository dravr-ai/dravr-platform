// ABOUTME: What every built-in tool reads, declared once: its data class, the streams it serves, and written text
// ABOUTME: Feeds the coach-read gate and the trust label a result carries; every registered tool has a row

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What each built-in tool reads ([`ToolData`]), declared on the registry
//! after every tool is registered.
//!
//! The class decides whose data a coach's call may read through a tool
//! (carnet#1387): a training-class tool reads a coached athlete's training
//! when a call names one, the athlete of a coach's thread about them or
//! `athlete=` over MCP and A2A, and a wellness-class tool their wellness the
//! same two ways, under their `coach_wellness_read` too. A tool that reads no
//! person's data stays callable in that thread. A tool declared under no
//! class ([`ToolData::own`]) reads only its caller's own, and the thread
//! withholds it.
//!
//! The streams set the tier of the trust label on a result whose caller
//! writes its own reply (owl phase 1), and `with_athlete_text` marks a tool
//! that passes provider- or athlete-written text through, which makes the
//! label `untrusted`. A tool serving no stream declares none: its result is
//! labelled only when it served an item anyway, and then at T3.

use dravr_barrage::Stream::{
    ActivityDaily, Body, CardioDaily, CardioRaw, Fitness, Profile, RecoveryDaily, RestrictedNotes,
    Sleep, TrainingLoad, Workout, WorkoutRaw, WorkoutRoute,
};
use pierre_tool_runtime::read_subject::DataClass::{NoSubject, Training, Wellness};
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::tool_data::ToolData;

/// One row per built-in tool, under every feature that registers it.
const TOOL_DATA: &[(&str, ToolData)] = &[
    // The daily load history and the activities it is computed from.
    (
        "get_training_history",
        ToolData::of(Training, &[TrainingLoad, Workout]),
    ),
    (
        "analyze_training_load",
        ToolData::of(Training, &[TrainingLoad, Workout]),
    ),
    (
        "calculate_fitness_score",
        ToolData::of(Training, &[TrainingLoad, Workout]),
    ),
    // The sessions themselves: their names are the athlete's or their
    // provider's text, and their start coordinates and city place them.
    (
        "get_activities",
        ToolData::of(Training, &[Workout, WorkoutRoute]).with_athlete_text(),
    ),
    (
        "export_latest_snapshot",
        ToolData::of(Training, &[Workout, WorkoutRoute]),
    ),
    (
        "analyze_weather_impact",
        ToolData::of(Training, &[Workout, WorkoutRoute]).with_athlete_text(),
    ),
    ("export_routes", ToolData::of(Training, &[WorkoutRoute])),
    // Each session's detail and sensor streams, without its position.
    (
        "analyze_activity",
        ToolData::of(Training, &[Workout, WorkoutRaw]).with_athlete_text(),
    ),
    // A session it was not asked about is picked for it, and named, with
    // the sessions it could have picked.
    (
        "get_activity_intelligence",
        ToolData::of(Training, &[Workout, WorkoutRaw]).with_athlete_text(),
    ),
    (
        "extract_activity_streams",
        ToolData::of(Training, &[Workout, WorkoutRaw]),
    ),
    (
        "export_intervals",
        ToolData::of(Training, &[Workout, WorkoutRaw]),
    ),
    (
        "calculate_metrics",
        ToolData::of(Training, &[Workout, WorkoutRaw]),
    ),
    (
        "compare_activities",
        ToolData::of(Training, &[Workout, WorkoutRaw]).with_athlete_text(),
    ),
    // One segment's efforts across the sessions, by the segment's name.
    (
        "get_segment_history",
        ToolData::of(Training, &[Workout, WorkoutRaw]).with_athlete_text(),
    ),
    // Trends, best efforts, predictions and the advice derived from the
    // sessions; a mode reading the athlete's wellness is refused on a coach's
    // read.
    (
        "analyze_performance_trends",
        ToolData::of(Training, &[Workout, TrainingLoad, CardioDaily]),
    ),
    (
        "detect_patterns",
        ToolData::of(Training, &[Workout, TrainingLoad, CardioDaily]),
    ),
    (
        "predict_performance",
        ToolData::of(Training, &[Workout, TrainingLoad, CardioDaily]),
    ),
    (
        "generate_recommendations",
        ToolData::of(Training, &[Workout, TrainingLoad, CardioDaily]).with_athlete_text(),
    ),
    // Goals and the progress toward them.
    (
        "suggest_goals",
        ToolData::of(Training, &[Workout, TrainingLoad, CardioDaily]),
    ),
    (
        "track_progress",
        ToolData::of(Training, &[Workout, TrainingLoad, CardioDaily]),
    ),
    (
        "analyze_goal_feasibility",
        ToolData::of(Training, &[Workout, TrainingLoad, CardioDaily]),
    ),
    // Training physiology, the estimates read against it, and the zones and
    // configuration it sets.
    (
        "estimate_vo2max",
        ToolData::of(Training, &[Fitness, Profile]),
    ),
    (
        "estimate_lactate_thresholds",
        ToolData::of(Training, &[Fitness, Profile]),
    ),
    (
        "calculate_personalized_zones",
        ToolData::of(Training, &[Fitness, Profile]),
    ),
    (
        "get_user_configuration",
        ToolData::of(Training, &[Fitness, Profile]),
    ),
    // The athlete's provider profile, totals and counts, their connected
    // sources and how fresh each is. A count's breakdown names the gear and
    // routes the athlete named.
    ("get_athlete", ToolData::of(Training, &[Profile])),
    ("get_stats", ToolData::of(Training, &[ActivityDaily])),
    (
        "query_activities",
        ToolData::of(Training, &[ActivityDaily, Workout]).with_athlete_text(),
    ),
    ("list_data_sources", ToolData::of(Training, &[])),
    ("get_data_freshness", ToolData::of(Training, &[])),
    // The athlete's wellness: sleep, recovery and readiness — the day's note
    // they typed into their source's log included — health snapshots and
    // intraday heart rate, and what is derived from them.
    ("get_sleep_sessions", ToolData::of(Wellness, &[Sleep])),
    ("analyze_sleep_quality", ToolData::of(Wellness, &[Sleep])),
    ("track_sleep_trends", ToolData::of(Wellness, &[Sleep])),
    ("optimize_sleep_schedule", ToolData::of(Wellness, &[Sleep])),
    (
        "get_recovery_metrics",
        ToolData::of(Wellness, &[RecoveryDaily, CardioDaily]).with_athlete_text(),
    ),
    (
        "calculate_recovery_score",
        ToolData::of(Wellness, &[RecoveryDaily, CardioDaily]),
    ),
    (
        "suggest_rest_day",
        ToolData::of(Wellness, &[RecoveryDaily, CardioDaily]),
    ),
    (
        "get_health_snapshots",
        ToolData::of(Wellness, &[CardioDaily, Body]),
    ),
    (
        "get_heart_rate_series",
        ToolData::of(Wellness, &[CardioRaw]),
    ),
    // Nutrition targets, which the athlete's medical flag governs; the flag
    // itself is not in the output (owl Q8).
    (
        "calculate_daily_nutrition",
        ToolData::of(Wellness, &[Body, Profile]),
    ),
    (
        "get_nutrient_timing",
        ToolData::of(Wellness, &[Body, Profile]),
    ),
    // Route and weather lookups: OpenStreetMap and forecast data, cached for
    // the caller's own next call.
    ("discover_routes", ToolData::of(NoSubject, &[])),
    ("compose_route", ToolData::of(NoSubject, &[])),
    ("get_weather_forecast", ToolData::of(NoSubject, &[])),
    // The caller's own catalogue of workouts and agents, not anyone's
    // training: what a coach prescribes from.
    ("list_workout_templates", ToolData::of(NoSubject, &[])),
    ("browse_agent_store", ToolData::of(NoSubject, &[])),
    ("search_agent_store", ToolData::of(NoSubject, &[])),
    // Food references and pure calculators.
    ("search_food", ToolData::of(NoSubject, &[])),
    ("get_food_details", ToolData::of(NoSubject, &[])),
    ("analyze_meal_nutrition", ToolData::of(NoSubject, &[])),
    ("get_recipe_constraints", ToolData::of(NoSubject, &[])),
    ("validate_recipe", ToolData::of(NoSubject, &[])),
    // The shared stretching and yoga references.
    ("get_stretching_exercise", ToolData::of(NoSubject, &[])),
    ("get_yoga_pose", ToolData::of(NoSubject, &[])),
    ("list_stretching_exercises", ToolData::of(NoSubject, &[])),
    ("list_yoga_poses", ToolData::of(NoSubject, &[])),
    (
        "suggest_stretches_for_activity",
        ToolData::of(NoSubject, &[]),
    ),
    ("suggest_yoga_sequence", ToolData::of(NoSubject, &[])),
    // Everything below reads only its caller's own data, if any: a coach's
    // thread about an athlete withholds it.
    //
    // The agent's own memory of the athlete and the playbooks it learned.
    ("recall_user_memory", ToolData::own(&[RestrictedNotes])),
    ("remember_fact", ToolData::own(&[RestrictedNotes])),
    ("list_coaching_playbooks", ToolData::own(&[RestrictedNotes])),
    ("forget_playbook", ToolData::own(&[RestrictedNotes])),
    // The composed dossier carries the durable facts beside the training and
    // physiology; the claim checker reads the sessions and physiology a
    // claim names.
    (
        "export_dossier",
        ToolData::own(&[Workout, TrainingLoad, Fitness, Profile, RestrictedNotes]),
    ),
    (
        "verify_claim",
        ToolData::own(&[Workout, WorkoutRaw, Fitness, Profile]),
    ),
    // The history recomputed from the sessions.
    (
        "compute_training_history",
        ToolData::own(&[TrainingLoad, Workout]),
    ),
    // Plans, prescriptions and calendar entries: the planned sessions, whose
    // titles and descriptions a provider or coach wrote.
    (
        "get_planned_workouts",
        ToolData::own(&[Workout]).with_athlete_text(),
    ),
    ("get_training_plan", ToolData::own(&[Workout])),
    ("save_training_plan", ToolData::own(&[Workout])),
    ("publish_training_plan", ToolData::own(&[Workout])),
    ("push_training_plan", ToolData::own(&[Workout])),
    ("prescribe_workout", ToolData::own(&[Workout])),
    ("withdraw_prescribed_workout", ToolData::own(&[Workout])),
    (
        "recommend_plan_flavour",
        ToolData::own(&[Workout, TrainingLoad]),
    ),
    ("set_preferred_calendar", ToolData::own(&[])),
    // The places the athlete trains from, by the names they gave them.
    (
        "name_place",
        ToolData::own(&[WorkoutRoute]).with_athlete_text(),
    ),
    // Goals and commitments the athlete set.
    ("set_goal", ToolData::own(&[Workout, TrainingLoad])),
    ("commitment_create", ToolData::own(&[])),
    ("commitment_cancel", ToolData::own(&[])),
    // A coach's group: its members' sessions by name, and their plans.
    (
        "get_group_member_activities",
        ToolData::own(&[Workout, WorkoutRoute]).with_athlete_text(),
    ),
    (
        "get_roster_overview",
        ToolData::own(&[Workout, WorkoutRoute]).with_athlete_text(),
    ),
    ("get_group_member_plan_status", ToolData::own(&[Workout])),
    ("propose_athlete_link", ToolData::own(&[])),
    // Physiology the athlete set or imported, and their fitness
    // configurations.
    ("set_physiology", ToolData::own(&[Fitness, Profile])),
    (
        "import_provider_thresholds",
        ToolData::own(&[Fitness, Profile]),
    ),
    ("get_fitness_config", ToolData::own(&[Fitness, Profile])),
    ("list_fitness_configs", ToolData::own(&[Fitness, Profile])),
    ("set_fitness_config", ToolData::own(&[Fitness, Profile])),
    ("delete_fitness_config", ToolData::own(&[])),
    (
        "update_user_configuration",
        ToolData::own(&[Fitness, Profile]),
    ),
    ("get_configuration_catalog", ToolData::own(&[])),
    ("get_configuration_profiles", ToolData::own(&[])),
    ("validate_configuration", ToolData::own(&[])),
    // Connections: which providers are linked, and the calls that link,
    // unlink and refresh them.
    ("connect_provider", ToolData::own(&[])),
    ("disconnect_provider", ToolData::own(&[])),
    ("get_connection_status", ToolData::own(&[])),
    ("refresh_provider_data", ToolData::own(&[])),
    ("get_provider_roster", ToolData::own(&[])),
    // The caller's own recipes.
    ("get_recipe", ToolData::own(&[])),
    ("list_recipes", ToolData::own(&[])),
    ("search_recipes", ToolData::own(&[])),
    ("save_recipe", ToolData::own(&[])),
    ("delete_recipe", ToolData::own(&[])),
    // The agent catalogue and the caller's own agents.
    ("list_agents", ToolData::own(&[])),
    ("search_agents", ToolData::own(&[])),
    ("get_agent", ToolData::own(&[])),
    ("get_active_agent", ToolData::own(&[])),
    ("list_hidden_agents", ToolData::own(&[])),
    ("create_agent", ToolData::own(&[])),
    ("update_agent", ToolData::own(&[])),
    ("delete_agent", ToolData::own(&[])),
    ("activate_agent", ToolData::own(&[])),
    ("deactivate_agent", ToolData::own(&[])),
    ("hide_agent", ToolData::own(&[])),
    ("show_agent", ToolData::own(&[])),
    ("toggle_agent_favorite", ToolData::own(&[])),
    ("install_agent_from_store", ToolData::own(&[])),
    // Operator tools over the system agents.
    ("admin_list_system_agents", ToolData::own(&[])),
    ("admin_get_system_agent", ToolData::own(&[])),
    ("admin_create_system_agent", ToolData::own(&[])),
    ("admin_update_system_agent", ToolData::own(&[])),
    ("admin_delete_system_agent", ToolData::own(&[])),
    ("admin_assign_agent", ToolData::own(&[])),
    ("admin_unassign_agent", ToolData::own(&[])),
    ("admin_list_agent_assignments", ToolData::own(&[])),
    ("admin_impersonate_start", ToolData::own(&[])),
    ("admin_impersonate_end", ToolData::own(&[])),
];

/// Declare [`TOOL_DATA`] on the tools this build registered.
pub(crate) fn declare_tool_data(registry: &mut ToolRegistry) {
    for (tool, data) in TOOL_DATA {
        registry.declare_data(tool, *data);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::TOOL_DATA;
    use crate::tools::registry_builtin::register_builtin_tools;
    use pierre_tool_runtime::registry::ToolRegistry;

    fn registered() -> BTreeSet<String> {
        let mut registry = ToolRegistry::new();
        register_builtin_tools(&mut registry);
        registry
            .tool_names()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn declared() -> BTreeSet<String> {
        TOOL_DATA
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect()
    }

    #[test]
    fn every_registered_tool_declares_what_it_reads() {
        let missing: Vec<String> = registered().difference(&declared()).cloned().collect();
        assert!(
            missing.is_empty(),
            "a registered tool has no TOOL_DATA row, so its result's trust label cannot be \
             placed: {missing:?}"
        );
    }

    #[test]
    fn every_row_is_declared_once() {
        assert_eq!(
            declared().len(),
            TOOL_DATA.len(),
            "a tool appears twice in TOOL_DATA; the later row would win silently"
        );
    }

    /// Only a build registering every tool family can tell a stale row from
    /// one whose family this build leaves out.
    #[cfg(all(
        feature = "tools-connection",
        feature = "tools-data",
        feature = "tools-analytics",
        feature = "tools-goals",
        feature = "tools-groups",
        feature = "tools-config",
        feature = "tools-nutrition",
        feature = "tools-sleep",
        feature = "tools-recipes",
        feature = "tools-agents",
        feature = "tools-admin",
        feature = "tools-mobility",
        feature = "tools-memory",
        feature = "tools-verification",
    ))]
    #[test]
    fn no_row_names_a_tool_the_full_build_does_not_register() {
        let stale: Vec<String> = declared().difference(&registered()).cloned().collect();
        assert!(
            stale.is_empty(),
            "TOOL_DATA names a tool no build registers; delete its row: {stale:?}"
        );
    }
}
