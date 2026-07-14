// ABOUTME: Tests later-turn activity grounding — the intent predicate, the refresh gate, and the
// ABOUTME: injection contract that keeps a coach's plans anchored in real activities past turn 1.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Later-turn activity grounding.
//!
//! [`inject_startup_context`] deterministically grounds a coach conversation in
//! real activities only on its first message; on later turns the model had to
//! choose to call `get_activities` itself, and when it skipped that call a
//! "fais-moi un plan" / "analyse ma charge" ask was answered from the persona
//! prompt alone — the generic, ungrounded plan this stage fixes. These tests
//! pin the three pure pieces of the fix: the [`needs_activity_grounding`]
//! intent predicate, the [`should_refresh_activity_context`] gate (turn number,
//! coach data-requirements, and intent), and the [`inject_activity_refresh`]
//! contract (inject fresh data just before the ask; never inject over an empty
//! window; inject conservatively when the window cannot be parsed).

use pierre_chat_pipeline::stages::prefetch::{
    inject_activity_refresh, needs_activity_grounding, should_refresh_activity_context,
};
use pierre_core::models::{CoachCategory, CoachRuntimeContext};
use pierre_llm::{ChatMessage, MessageRole};

/// A coach runtime context carrying only the `data_requirements` the gate
/// reads; every other field is an inert placeholder.
fn coach(data_requirements: Option<&str>) -> CoachRuntimeContext {
    CoachRuntimeContext {
        slug: "endurance-coach".to_owned(),
        source: "contremaitre".to_owned(),
        system_prompt: "You are a coach.".to_owned(),
        startup_query: None,
        data_requirements: data_requirements.map(ToOwned::to_owned),
        output_schema: None,
        max_tool_iterations: None,
        temperature: None,
        category: CoachCategory::Training,
    }
}

/// A coach whose requirements include an activity window (the grounded case).
const WITH_ACTIVITIES: &str = r#"{"activities":{"count":30,"time_frame":"12w"}}"#;

#[test]
fn grounding_intent_matches_plan_analysis_and_recommendation_asks() {
    // fr coaching surface (the primary one) + en.
    for message in [
        "Fait moi donc un plan d'entraînement pour la semaine?",
        "analyse ma charge d'entraînement",
        "Compare le avec ton approche",
        "que dois-je faire cette semaine",
        "comment je m'en sors ?",
        "recommande-moi une séance",
        "montre-moi ma progression",
        "how am I doing this week",
        "what should I do tomorrow",
        "give me a training plan",
    ] {
        assert!(
            needs_activity_grounding(message),
            "expected a grounding intent for: {message:?}"
        );
    }
}

#[test]
fn grounding_intent_ignores_acknowledgments_and_smalltalk() {
    for message in ["merci beaucoup!", "ok parfait", "oui", "salut ça va", "👍"] {
        assert!(
            !needs_activity_grounding(message),
            "expected NO grounding intent for: {message:?}"
        );
    }
}

#[test]
fn refresh_gate_fires_only_on_a_later_grounding_turn_with_an_activity_window() {
    let with_activities = coach(Some(WITH_ACTIVITIES));

    // The happy path: a later turn, a coach with an activity window, a plan ask.
    assert!(should_refresh_activity_context(
        3,
        Some(&with_activities),
        "fais-moi un plan pour la semaine"
    ));

    // Turn 1 is owned by inject_startup_context, never this stage.
    assert!(
        !should_refresh_activity_context(1, Some(&with_activities), "fais-moi un plan"),
        "turn 1 must be left to inject_startup_context"
    );

    // No coach bound → nothing to ground against.
    assert!(
        !should_refresh_activity_context(3, None, "fais-moi un plan"),
        "no coach context → no refresh"
    );

    // A grounding ask but no activity window (e.g. a profile-only coach).
    let no_window = coach(Some(r#"{"athlete_profile":true}"#));
    assert!(
        !should_refresh_activity_context(3, Some(&no_window), "fais-moi un plan"),
        "a coach without an activity window is left untouched"
    );

    // No data_requirements at all.
    let no_reqs = coach(None);
    assert!(
        !should_refresh_activity_context(3, Some(&no_reqs), "fais-moi un plan"),
        "a coach without data_requirements is left untouched"
    );

    // A later turn with an activity window but a non-grounding message.
    assert!(
        !should_refresh_activity_context(3, Some(&with_activities), "merci beaucoup!"),
        "an acknowledgment turn must not re-fetch"
    );
}

#[test]
fn injection_places_fresh_data_just_before_the_ask() {
    let mut messages = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("fais-moi un plan"),
    ];
    let window = r#"{"count":1,"activity_list":"1. [Run] Chair à mouches - 2026-07-09 - 8.20 km","activities":[]}"#;

    let injected = inject_activity_refresh(&mut messages, window);

    assert!(injected, "a non-empty window must be injected");
    assert_eq!(messages.len(), 3, "injection adds exactly one message");
    // Inserted at index 1 (just before the trailing user ask).
    assert!(
        messages[1].role == MessageRole::System,
        "the fresh block is a system message"
    );
    assert!(
        messages[1].content.contains("freshly loaded for this turn"),
        "the block must instruct the model to ground on the fresh data"
    );
    assert!(
        messages[1].content.contains("Chair à mouches"),
        "the block must carry the actual activity data"
    );
    assert!(
        messages[2].role == MessageRole::User,
        "the athlete's ask stays last, right after the data"
    );
}

#[test]
fn injection_skips_an_empty_window_so_the_model_is_never_told_it_has_data() {
    let mut messages = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("analyse ma charge"),
    ];
    let empty = r#"{"count":0,"activity_list":"","activities":[]}"#;

    let injected = inject_activity_refresh(&mut messages, empty);

    assert!(!injected, "an empty window must not be injected");
    assert_eq!(messages.len(), 2, "no message is added for an empty window");
}

#[test]
fn injection_is_conservative_when_the_window_cannot_be_parsed() {
    // A serialization hiccup must not silently drop real grounding data: when
    // we can't positively confirm emptiness, we inject.
    let mut messages = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("plan ma semaine"),
    ];

    let injected = inject_activity_refresh(&mut messages, "not json at all");

    assert!(injected, "unparseable content is treated as non-empty");
    assert_eq!(messages.len(), 3);
    assert!(messages[1].content.contains("not json at all"));
}
