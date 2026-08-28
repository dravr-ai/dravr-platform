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
    build_prefetch_params, get_startup_context_if_applicable, inject_activity_refresh,
    should_refresh_activity_context, startup_query_preview,
};
use pierre_core::models::coaches::ActivityDataRequirements;
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
fn refresh_gate_fires_on_any_later_turn_with_an_activity_window() {
    let with_activities = coach(Some(WITH_ACTIVITIES));

    // The happy path: a later turn and a coach that declared a window.
    assert!(should_refresh_activity_context(
        3,
        Some(&with_activities),
        false
    ));

    // The phrasing is no longer consulted, and this is the case that proves it.
    // "Montre-moi l'évolution de mon volume hebdomadaire sur les 3 derniers
    // mois" matched none of the 61 terms the old gate tested, so a real
    // Telegram turn reached the model with no activity data at all and the
    // coach answered from conversation history (2026-08-21). Nothing about the
    // wording may decide whether the athlete's own data is fetched.
    assert!(
        should_refresh_activity_context(3, Some(&with_activities), false),
        "grounding must not depend on how the athlete phrased the question"
    );

    // Turn 1 is owned by inject_startup_context, never this stage.
    assert!(
        !should_refresh_activity_context(1, Some(&with_activities), false),
        "turn 1 must be left to inject_startup_context"
    );

    // A guided pillar interview must not pull in an activity dump.
    assert!(
        !should_refresh_activity_context(3, Some(&with_activities), true),
        "a guided flow owns its turn"
    );

    // No coach bound → nothing to ground against.
    assert!(
        !should_refresh_activity_context(3, None, false),
        "no coach context → no refresh"
    );

    // A coach with no activity window (e.g. a profile-only coach).
    let no_window = coach(Some(r#"{"athlete_profile":true}"#));
    assert!(
        !should_refresh_activity_context(3, Some(&no_window), false),
        "a coach without an activity window is left untouched"
    );

    // No data_requirements at all.
    let no_reqs = coach(None);
    assert!(
        !should_refresh_activity_context(3, Some(&no_reqs), false),
        "a coach without data_requirements is left untouched"
    );
}

#[test]
fn refresh_gate_never_fires_while_a_guided_flow_owns_the_turn() {
    let with_activities = coach(Some(WITH_ACTIVITIES));

    // Mid-walk, a pillar answer must not pull an activity dump (whose
    // instruction reads "base your analysis and any plan on these specific
    // activities") into a profile question's turn. This gate used to share the
    // work with an intent predicate; now that grounding no longer reads the
    // wording, the guided flag is the only thing standing between an interview
    // turn and a 12 KB activity block, so it carries the whole load.
    assert!(
        !should_refresh_activity_context(2, Some(&with_activities), true),
        "guided flow active must suppress the refresh"
    );

    // Outside the walk, the same coach and turn number ground normally.
    assert!(
        should_refresh_activity_context(2, Some(&with_activities), false),
        "outside a guided flow the coach's declared window is honoured"
    );
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

/// A realistic `get_activities` payload: the same activities three times over.
///
/// The tool answers with a pre-rendered `activity_list`, the structured
/// `activities` array, and a `retrieval_context` sidecar. Shapes mirror what a
/// live dev turn returned on 2026-08-21.
fn tool_payload(n: usize) -> String {
    let prose: Vec<String> = (1..=n)
        .map(|i| {
            format!(
                "{i}. [course à pied] Morning Run #{i} - 2026-08-{:02} - 12.34 km - 58:33 - +236m - 14°C",
                (i % 28) + 1
            )
        })
        .collect();
    let structured: Vec<serde_json::Value> = (1..=n)
        .map(|i| {
            serde_json::json!({
                "id": format!("17836429807609663{i:03}"),
                "name": format!("Morning Run #{i}"),
                "sport_type": "run",
                "start_date": format!("2026-08-{:02}T11:03:00Z", (i % 28) + 1),
                "distance_meters": 12_340.0,
                "moving_time_seconds": 3513,
                "total_elevation_gain": 236.0,
                "average_heartrate": 137.0,
                "max_heartrate": 168.0,
                "average_speed": 3.51,
                "provider": "strava",
            })
        })
        .collect();
    serde_json::json!({
        "activity_list": prose.join("\n"),
        "activities": structured,
        "provider": "strava",
        "count": n,
        "mode": "full",
        "format": "json",
        "retrieval_context": {
            "analysis_type": "general",
            "sufficiency": "sufficient",
            "fragment_dedup": { "groups": [], "has_fragments": false },
        },
    })
    .to_string()
}

/// Grounding must inject the activities once, not three times.
///
/// The prefetch used to serialize the whole tool response into the prompt —
/// 12,448 bytes for a 30-activity window on a live dev turn, roughly 3k tokens,
/// every grounded turn. Now that grounding fires on every qualifying turn rather
/// than only on a keyword match, that payload is paid far more often, so the
/// duplication is worth removing rather than tolerating.
#[test]
fn grounding_injects_the_readable_list_not_the_whole_tool_response() {
    let payload = tool_payload(30);
    let mut messages = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("montre-moi l'évolution de mon volume hebdomadaire"),
    ];

    assert!(inject_activity_refresh(&mut messages, &payload));
    let injected = &messages[1].content;

    // What the coach cites survives: name, date, distance, duration, elevation.
    assert!(
        injected.contains("Morning Run #1")
            && injected.contains("12.34 km")
            && injected.contains("+236m"),
        "the citable activity lines must reach the model"
    );

    // The structured duplicate does not.
    for key in [
        "distance_meters",
        "moving_time_seconds",
        "average_heartrate",
        "retrieval_context",
        "sufficiency",
    ] {
        assert!(
            !injected.contains(key),
            "{key:?} is the second copy of the same data and must not be injected"
        );
    }

    assert!(
        injected.len() * 3 < payload.len(),
        "expected a large reduction; injected {} bytes from a {} byte payload",
        injected.len(),
        payload.len()
    );
}

/// An unrecognised payload still reaches the model whole — the reducer must
/// never be the reason a coach is left with nothing.
#[test]
fn an_unrecognised_payload_is_injected_verbatim() {
    let odd = r#"{"count":3,"rows":"something this stage has never seen"}"#;
    let mut messages = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("analyse ma charge"),
    ];

    assert!(inject_activity_refresh(&mut messages, odd));
    assert!(
        messages[1]
            .content
            .contains("something this stage has never seen"),
        "a shape without activity_list must fall back to the raw payload"
    );
}

/// The declared window reaches `get_activities` unnarrowed by sport.
///
/// A Marathon Coach declares `sport_types: ["Run"]`, and the prefetch used to
/// forward that as a `sport_type` filter whenever exactly one sport was named.
/// On 2026-08-27 that turned a 106-activity window into 24 run-family sessions,
/// which were then injected under a block instructing the model to "infer the
/// sport mix from them rather than asking" — so the coach told an athlete who
/// had ridden 18 km of singletrack that morning he had no mountain-bike history
/// and was 100% trail running. The coach's specialization belongs in its
/// prompt, never in a filter over the athlete's training.
#[test]
fn a_single_sport_coach_still_gets_an_unfiltered_window() {
    let marathon_coach = ActivityDataRequirements {
        count: 30,
        sport_types: vec!["Run".to_owned()],
        time_frame: Some("16w".to_owned()),
        mode: "summary".to_owned(),
        format: "toon".to_owned(),
        analysis_type: "race_preparation".to_owned(),
    };

    let params = build_prefetch_params(&marathon_coach);

    assert!(
        params.get("sport_type").is_none(),
        "a coach's sport_types must never narrow the grounding window, got {params}"
    );
    assert_eq!(
        params.get("limit").and_then(serde_json::Value::as_u64),
        Some(30),
        "the declared count still bounds the window"
    );
    assert!(
        params.get("after").is_some() && params.get("before").is_some(),
        "the declared time_frame still bounds the window as a paired range, got {params}"
    );
}

/// The multi-sport case was already unfiltered and must stay that way.
#[test]
fn a_multi_sport_coach_still_gets_an_unfiltered_window() {
    let endurance_coach = ActivityDataRequirements {
        count: 30,
        sport_types: vec!["Run".to_owned(), "Ride".to_owned()],
        time_frame: Some("4w".to_owned()),
        mode: "detailed".to_owned(),
        format: "toon".to_owned(),
        analysis_type: "race_preparation".to_owned(),
    };

    let params = build_prefetch_params(&endurance_coach);

    assert!(
        params.get("sport_type").is_none(),
        "a multi-sport coach must not acquire a filter either, got {params}"
    );
}
