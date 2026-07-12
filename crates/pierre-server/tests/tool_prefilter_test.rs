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

#[cfg(feature = "tools-groups")]
#[tokio::test]
async fn peer_fetch_tool_survives_prefilter_via_pin() {
    // Peer-fetch intent is a person's name ("Raph"), which activates no keyword
    // category, and the tool's "groups" category is in neither KEYWORD_RULES nor
    // any coach scope. It stays callable only because it is in PINNED_CORE — so a
    // group-chat turn asking about a peer must keep it regardless of phrasing.
    let registry = registry();
    let outcome = enabled_prefilter()
        .select(
            &registry,
            "show me Raph's runs from last weekend",
            Some("training"),
        )
        .await;

    assert!(
        contains(&outcome.keep, "get_group_member_activities"),
        "consent-gated peer fetch was dropped by the prefilter; it must be pinned \
         (PINNED_CORE) because peer-fetch intent activates no keyword category. \
         kept={:?}",
        outcome.keep
    );
}

#[tokio::test]
async fn plan_request_turn_keeps_training_plan_tools() {
    let registry = registry();
    let outcome = enabled_prefilter()
        .select(
            &registry,
            "fais-moi un plan d'entraînement jusqu'au 8 août",
            Some("training"),
        )
        .await;

    // "plan" maps to the memory category: the persistence pair must be
    // offered on exactly the turns where the coach commits to a plan.
    assert!(contains(&outcome.keep, "save_training_plan"));
    assert!(contains(&outcome.keep, "get_training_plan"));
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
