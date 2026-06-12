// ABOUTME: Integration tests for the per-turn tool intent pre-filter adapter.
// ABOUTME: Verifies narrowing against the real built-in registry — pinned floor, coach scope, keyword activation, and fallback.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![doc = "Integration tests for the per-turn tool intent pre-filter adapter."]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use pierre_chat_pipeline::ToolPrefilter;
use pierre_config::tool_intent_prefilter::ToolIntentPrefilterConfig;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::registry::ToolRegistry;

fn registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    registry
}

fn enabled_prefilter() -> ToolPrefilter {
    ToolPrefilter::from_config(&ToolIntentPrefilterConfig {
        enabled: true,
        min_keep: 8,
    })
    .expect("enabled config yields a prefilter")
}

fn contains(names: &[String], name: &str) -> bool {
    names.iter().any(|n| n == name)
}

#[test]
fn disabled_config_yields_no_prefilter() {
    assert!(ToolPrefilter::from_config(&ToolIntentPrefilterConfig::default()).is_none());
}

#[tokio::test]
async fn nutrition_turn_drops_mobility_keeps_pinned_and_nutrition() {
    let registry = registry();
    let full = registry.chat_callable_schemas().len();
    let outcome = enabled_prefilter()
        .select(
            &registry,
            "what should I eat after my long run?",
            Some("nutrition"),
        )
        .await;

    // Pinned core survives every turn.
    assert!(contains(&outcome.keep, "get_activities"));
    // Nutrition coach scope + "eat" keyword keep the nutrition tools.
    assert!(contains(&outcome.keep, "search_food"));
    // Mobility is irrelevant to this turn and is dropped.
    assert!(!contains(&outcome.keep, "suggest_yoga_sequence"));
    // The set was genuinely narrowed.
    assert!(outcome.keep.len() < full);
    assert!(!outcome.dropped.is_empty());
}

#[tokio::test]
async fn analysis_turn_keeps_data_analytics_drops_recipes() {
    let registry = registry();
    let outcome = enabled_prefilter()
        .select(
            &registry,
            "analyze my training load trend this month",
            Some("analysis"),
        )
        .await;

    assert!(contains(&outcome.keep, "analyze_training_load"));
    assert!(contains(&outcome.keep, "get_activities"));
    assert!(!contains(&outcome.keep, "save_recipe"));
}

#[tokio::test]
async fn vague_turn_with_no_signal_falls_back_to_full_set() {
    let registry = registry();
    let full = registry.chat_callable_schemas().len();
    // No keyword hits and no coach scope → only the pinned core would survive,
    // which is below min_keep, so the selector keeps the full set.
    let outcome = enabled_prefilter()
        .select(&registry, "hey there", None)
        .await;

    assert_eq!(outcome.keep.len(), full);
    assert!(outcome.dropped.is_empty());
}

#[tokio::test]
async fn data_grounded_flag_set_when_activity_reads_survive() {
    let registry = registry();
    let outcome = enabled_prefilter()
        .select(&registry, "how did my ride go?", Some("training"))
        .await;

    // get_activities (reads provider data) is kept → the turn is data-grounded.
    assert!(contains(&outcome.keep, "get_activities"));
    assert!(outcome.needs_grounded_data);
}
