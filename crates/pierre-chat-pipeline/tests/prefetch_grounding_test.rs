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
    get_startup_context_if_applicable, inject_activity_refresh, needs_activity_grounding,
    should_refresh_activity_context, startup_query_preview,
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
        visuals: Vec::new(),
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
        // Temporal / meal / outing asks — the 2026-07-24 incident phrasing,
        // which previously matched no grounding term so the coach answered a
        // recommendation from memory (or declined to fetch).
        "Tu me recommandes quoi comme dîner et course en ce 24 juillet",
        "qu'est-ce que je mange ce soir?",
        "je fais quoi comme course aujourd'hui",
        "ma sortie de demain, ravito ou juste de l'eau?",
        "c'est quoi mon souper de récup ce soir",
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
        "fais-moi un plan pour la semaine",
        false
    ));

    // Turn 1 is owned by inject_startup_context, never this stage.
    assert!(
        !should_refresh_activity_context(1, Some(&with_activities), "fais-moi un plan", false),
        "turn 1 must be left to inject_startup_context"
    );

    // No coach bound → nothing to ground against.
    assert!(
        !should_refresh_activity_context(3, None, "fais-moi un plan", false),
        "no coach context → no refresh"
    );

    // A grounding ask but no activity window (e.g. a profile-only coach).
    let no_window = coach(Some(r#"{"athlete_profile":true}"#));
    assert!(
        !should_refresh_activity_context(3, Some(&no_window), "fais-moi un plan", false),
        "a coach without an activity window is left untouched"
    );

    // No data_requirements at all.
    let no_reqs = coach(None);
    assert!(
        !should_refresh_activity_context(3, Some(&no_reqs), "fais-moi un plan", false),
        "a coach without data_requirements is left untouched"
    );

    // A later turn with an activity window but a non-grounding message.
    assert!(
        !should_refresh_activity_context(3, Some(&with_activities), "merci beaucoup!", false),
        "an acknowledgment turn must not re-fetch"
    );
}

#[test]
fn refresh_gate_never_fires_while_a_guided_flow_owns_the_turn() {
    let with_activities = coach(Some(WITH_ACTIVITIES));

    // Every one of these is a plausible pillar answer that happens to contain a
    // GROUNDING_INTENT_TERMS word. Mid-walk they must not pull an activity dump
    // (whose instruction reads "base your analysis and any plan on these
    // specific activities") into a profile question's turn.
    for answer in [
        "je m'entraîne 10h par semaine",
        "ma course objectif c'est l'Unbound",
        "je veux progresser en montagne",
        "la performance compte moins que le plaisir",
    ] {
        assert!(
            needs_activity_grounding(answer),
            "precondition: {answer:?} must look like a grounding ask"
        );
        assert!(
            !should_refresh_activity_context(2, Some(&with_activities), answer, true),
            "guided flow active must suppress the refresh for: {answer:?}"
        );
        // Same message outside the walk still grounds — the gate is the flow,
        // not the wording.
        assert!(
            should_refresh_activity_context(2, Some(&with_activities), answer, false),
            "outside a guided flow the same message must still ground: {answer:?}"
        );
    }
}

#[test]
fn startup_gate_never_fires_while_a_guided_flow_owns_the_turn() {
    let with_activities = coach(Some(WITH_ACTIVITIES));

    // The 2026-07-24 shape: history_len == 1 on the athlete's first answer with a
    // builder coach bound. Without the gate this injects the activity dump AND
    // the coach's own startup query as a synthetic user message.
    assert!(
        get_startup_context_if_applicable(1, Some(&with_activities), true).is_none(),
        "guided flow active must suppress startup grounding"
    );
    assert!(
        get_startup_context_if_applicable(1, Some(&with_activities), false).is_some(),
        "outside a guided flow the first turn still grounds"
    );
}

/// A fr-first `startup_query` whose 50th byte lands inside the `è` of
/// "athlète" — the geometry that made the log preview's byte slice panic.
const ACCENTED_STARTUP_QUERY: &str =
    "Analyse les douze dernières semaines de cet athlète: régularité, pics, récupération.";

#[test]
fn the_startup_query_preview_cuts_on_a_character_not_a_byte() {
    // `startup_query` is accepted verbatim by the custom-coach create/update
    // API on a fr-first platform, and the log line previewed it with a raw byte
    // slice. That runs on the `history_len == 1` path, before dispatch, with no
    // panic boundary between it and the turn — the same defect class that
    // destroyed a production turn in the deterministic-bounds scanner.
    //
    // The geometry is asserted before the function is exercised, so a reworded
    // fixture cannot silently stop reproducing the bug.
    let query = ACCENTED_STARTUP_QUERY;
    assert_eq!(
        query.len(),
        90,
        "the fixture must be longer than the preview"
    );
    assert!(
        !query.is_char_boundary(50),
        "byte 50 must land inside a character ('è' spans bytes 49..51) or this proves nothing"
    );
    assert!(query.is_char_boundary(49) && query.is_char_boundary(51));

    let preview = startup_query_preview(query);
    assert_eq!(
        preview, "Analyse les douze dernières semaines de cet athlèt",
        "the preview must be the first 50 characters, whole"
    );
    assert_eq!(preview.chars().count(), 50);
    assert_eq!(
        preview.len(),
        52,
        "50 characters of accented French are 52 bytes — a byte cut would have split the 'è'"
    );

    // A query shorter than the window is previewed whole, accents and all.
    assert_eq!(
        startup_query_preview("Résume la semaine."),
        "Résume la semaine."
    );
}

#[test]
fn a_startup_query_with_an_accent_at_the_preview_boundary_still_grounds_the_turn() {
    let mut with_startup_query = coach(None);
    with_startup_query.startup_query = Some(ACCENTED_STARTUP_QUERY.to_owned());

    let (returned, data_reqs) =
        get_startup_context_if_applicable(1, Some(&with_startup_query), false)
            .expect("a coach carrying a startup query must still ground its first turn");

    assert_eq!(
        returned.as_deref(),
        Some(ACCENTED_STARTUP_QUERY),
        "the query reaching the caller must be the coach's, untruncated"
    );
    assert!(
        data_reqs.is_none(),
        "this coach declares no data_requirements"
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
        messages[1].role == MessageRole::User,
        "the fresh block must be a USER message — the live provider keeps only \
         the first system message and drops every other one, so a System block \
         here never reached the model at all"
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
