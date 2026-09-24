// ABOUTME: Unit tests for memory extraction — predicate resolution, the stated_by parser and the schedule gate
// ABOUTME: Pins the prompt addenda to exactly the kinds, codes and provenance field the parser and gate accept

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_memory::{FactKind, FactSource, PredicateCode};
use pierre_services::memory_extraction::{
    is_agent_prescription, parse_raw_facts, resolve_predicate, RawFact, EXTRACTABLE_KINDS,
    PREDICATE_CODES_ADDENDUM, PROVENANCE_ADDENDUM,
};

fn raw(
    code: Option<&str>,
    predicate: Option<&str>,
    subject: Option<&str>,
    object: &str,
) -> RawFact {
    RawFact {
        kind: "goal".to_owned(),
        predicate_code: code.map(str::to_owned),
        predicate: predicate.map(str::to_owned),
        subject: subject.map(str::to_owned),
        object: object.to_owned(),
        confidence: 0.9,
        stated_by: Some("user".to_owned()),
        same_as: None,
    }
}

#[test]
fn the_new_prompt_shape_keeps_the_code_and_the_athletes_words() {
    let (code, object) = resolve_predicate(
        &raw(
            Some("training_for"),
            None,
            None,
            "un ultra de 26 km au Mont Albert",
        ),
        FactKind::Goal,
    );
    assert_eq!(code, PredicateCode::TrainingFor);
    assert_eq!(object, "un ultra de 26 km au Mont Albert");
}

#[test]
fn a_code_from_another_kind_or_an_unknown_code_falls_to_states() {
    let (code, object) =
        resolve_predicate(&raw(Some("parq_yes"), None, None, "Boston"), FactKind::Goal);
    assert_eq!((code, object.as_str()), (PredicateCode::States, "Boston"));
    let (code, _) = resolve_predicate(&raw(Some("targets"), None, None, "Boston"), FactKind::Goal);
    assert_eq!(code, PredicateCode::States);
}

#[test]
fn the_old_prompt_shape_survives_the_switch_over() {
    // A server phrase maps to its code; an extractor phrase folds into the
    // object under `states`, dropping the "you" subject and keeping a
    // third-party one — nothing the athlete said is lost.
    let (code, object) = resolve_predicate(
        &raw(None, Some("are working toward"), Some("you"), "a 5k"),
        FactKind::Goal,
    );
    assert_eq!(
        (code, object.as_str()),
        (PredicateCode::WorkingToward, "a 5k")
    );
    let (code, object) = resolve_predicate(
        &raw(
            None,
            Some("are racing"),
            Some("you"),
            "Big Red on 2026-08-08",
        ),
        FactKind::Goal,
    );
    assert_eq!(
        (code, object.as_str()),
        (PredicateCode::States, "are racing Big Red on 2026-08-08")
    );
    let (code, object) = resolve_predicate(
        &raw(
            None,
            Some("recommends"),
            Some("Coach Sarah"),
            "cadence drills",
        ),
        FactKind::Goal,
    );
    assert_eq!(
        (code, object.as_str()),
        (
            PredicateCode::States,
            "Coach Sarah recommends cadence drills"
        )
    );
}

#[test]
fn schedule_gate_drops_coach_prescriptions_once_the_plan_is_stored() {
    // The 1b6199d8 shape: a schedule fact the extractor did not attribute
    // to the user. Absent stated_by is treated as agent-stated. Dropped
    // only because `save_training_plan` ran and holds the plan.
    assert!(is_agent_prescription(
        FactKind::Schedule,
        None,
        FactSource::Conversation,
        true
    ));
    assert!(is_agent_prescription(
        FactKind::Schedule,
        Some("coach"),
        FactSource::Conversation,
        true
    ));
    // User-stated availability constraints still persist.
    assert!(!is_agent_prescription(
        FactKind::Schedule,
        Some("user"),
        FactSource::Conversation,
        true
    ));
    // …including when the extractor drifts the casing/spacing of "user".
    for variant in ["User", "USER", " user ", "User "] {
        assert!(
            !is_agent_prescription(
                FactKind::Schedule,
                Some(variant),
                FactSource::Conversation,
                true
            ),
            "user-stated fact dropped on casing variant {variant:?}"
        );
    }
    // Other kinds are not gated (goal write-back is the save tool's job,
    // but user-stated goals from chat remain extractable).
    assert!(!is_agent_prescription(
        FactKind::Goal,
        Some("coach"),
        FactSource::Conversation,
        true
    ));
    // The guided onboarding walk records the user's own answers even
    // when the extractor forgets the provenance field.
    assert!(!is_agent_prescription(
        FactKind::Schedule,
        None,
        FactSource::Onboarding,
        true
    ));
}

/// The whole justification for the drop is that the plan store has the
/// plan. Without the tool call it does not, and the drop deletes the only
/// copy.
///
/// Live 2026-09-02: the athlete asked for a dated plan to a 3 700 m race on
/// 11 October. The agent wrote a week-by-week build-up in prose, this gate
/// logged three drops, and `save_training_plan` was never called — zero
/// `training_plan.saved` events that day. The plan survived only in a
/// conversation whose history was being raw-dropped every turn, and was
/// unrecoverable by the end of the session (registre#203).
#[test]
fn a_prescription_is_retained_when_save_training_plan_did_not_run() {
    assert!(
        !is_agent_prescription(FactKind::Schedule, None, FactSource::Conversation, false),
        "with no plan stored, the fact is the only record of the prescription"
    );
    assert!(
        !is_agent_prescription(
            FactKind::Schedule,
            Some("coach"),
            FactSource::Conversation,
            false
        ),
        "an explicitly coach-stated schedule is exactly the one worth keeping \
         when nothing else holds it"
    );
}

#[test]
fn parser_accepts_stated_by_and_tolerates_its_absence() {
    let with = r#"[{"kind":"schedule","subject":"you","predicate":"can train on","object":"Tuesday and Thursday evenings","confidence":0.9,"stated_by":"user"}]"#;
    let facts = parse_raw_facts(with);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].stated_by.as_deref(), Some("user"));
    assert_eq!(facts[0].object, "Tuesday and Thursday evenings");

    let without = r#"[{"kind":"goal","subject":"you","predicate":"are racing","object":"Big Red on 2026-08-08","confidence":0.95}]"#;
    let facts = parse_raw_facts(without);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].stated_by, None);
}

#[test]
fn predicate_codes_addendum_lists_exactly_what_the_parser_accepts() {
    let addendum = PREDICATE_CODES_ADDENDUM.as_str();
    assert!(addendum.contains("\"predicate_code\""));
    for kind in EXTRACTABLE_KINDS {
        assert!(
            addendum.contains(&format!("\n- {}: ", kind.as_str())),
            "kind {} missing from the addendum",
            kind.as_str()
        );
    }
    for code in PredicateCode::ALL {
        let quoted = format!("\"{}\"", code.as_str());
        let offered =
            code.extractable() && EXTRACTABLE_KINDS.iter().any(|kind| code.allowed_for(*kind));
        assert_eq!(
            addendum.contains(&quoted),
            offered,
            "{} is {}offered but {}listed",
            code.as_str(),
            if offered { "" } else { "not " },
            if offered { "not " } else { "" }
        );
    }
    // `states` is the honest catch-all on every kind the model may pick.
    for line in addendum.lines().filter(|line| line.starts_with("- ")) {
        assert!(line.contains("\"states\""), "no states on {line}");
    }
}

#[test]
fn a_tool_only_code_from_the_model_is_stored_as_states() {
    // target_race passes allowed_for(Goal); only the extractable gate
    // keeps the model from passing a chat remark off as the plan tool's.
    let fact = raw(Some("target_race"), None, None, "Boston in April");
    let (code, object) = resolve_predicate(&fact, FactKind::Goal);
    assert_eq!(code, PredicateCode::States);
    assert_eq!(object, "Boston in April");
}

#[test]
fn provenance_addendum_defines_the_field_it_enforces() {
    // The gate keys on stated_by == "user"; the appended prompt must
    // actually instruct the extractor to emit that field and value.
    assert!(PROVENANCE_ADDENDUM.contains("\"stated_by\""));
    assert!(PROVENANCE_ADDENDUM.contains("\"user\""));
    assert!(PROVENANCE_ADDENDUM.contains("\"coach\""));
}
