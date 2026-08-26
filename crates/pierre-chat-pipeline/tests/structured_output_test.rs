// ABOUTME: Tests the builder-coach structured-output extraction + schema validation stage.
// ABOUTME: Validates against the real dravr-contremaitre structured-workout schema, no LLM needed.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use dravr_contremaitre::schemas::{DRAVR_VIZ_SCHEMA, STRUCTURED_WORKOUT_SCHEMA};
use pierre_chat_pipeline::stages::structured_output::{extract_structured_plan, SchemaTexts};

const SCHEMA_ID: &str = "structured-workout";

/// The full schema set, exactly as production assembles it.
///
/// Every test hands over the same map on purpose: the compiled validators live
/// in a process-wide `OnceLock`, so the first call in a test binary decides
/// what is registered for all of them. That mirrors production, where the set
/// is fixed at startup from compiled-in contremaitre constants.
fn schemas() -> SchemaTexts {
    [
        (
            "structured-workout".to_owned(),
            STRUCTURED_WORKOUT_SCHEMA.to_owned(),
        ),
        ("dravr-viz".to_owned(), DRAVR_VIZ_SCHEMA.to_owned()),
    ]
    .into_iter()
    .collect()
}

/// A minimal but schema-valid plan: 7 days, six rest days plus one easy session.
const VALID_PLAN: &str = r#"{"plan_window":{"start":"2026-05-12","end":"2026-05-18"},"rationale":"7-day aerobic base week","compliance":{"z1_pct":80,"z2_pct":5,"z3_pct":15,"weekly_tss_target":400},"weeks":[{"week_index":1,"days":[{"day":"Mon","session":null},{"day":"Tue","session":null},{"day":"Wed","session":null},{"day":"Thu","session":null},{"day":"Fri","session":null},{"day":"Sat","session":null},{"day":"Sun","session":{"name":"Easy run","duration_min":60,"intensity_factor":0.65,"tss_estimate":40,"blocks":[]}}]}],"evidence_refs":["evidence/test.md"]}"#;

#[test]
fn extracts_and_validates_a_clean_plan() {
    let extraction = extract_structured_plan(Some(SCHEMA_ID), true, &schemas(), VALID_PLAN)
        .expect("a clean, schema-valid plan should be extracted");
    assert!(extraction.structured_content.contains("\"plan_window\""));
    assert!(extraction.structured_content.contains("\"weeks\""));
    // Pure-JSON reply leaves no user-visible prose.
    assert!(extraction.cleaned_text.is_empty());
}

#[test]
fn extracts_plan_despite_narration_prefix() {
    // The exact failure mode from the bug report: the model narrates first,
    // then emits the plan. We still recover the plan object.
    let reply = format!("Pulling the athlete state now, then I'll emit the plan.\n{VALID_PLAN}");
    let extraction = extract_structured_plan(Some(SCHEMA_ID), true, &schemas(), &reply)
        .expect("a plan preceded by narration should still be extracted");
    assert!(extraction.structured_content.contains("\"rationale\""));
}

#[test]
fn extracts_plan_wrapped_in_code_fence() {
    let reply = format!("```json\n{VALID_PLAN}\n```");
    assert!(
        extract_structured_plan(Some(SCHEMA_ID), true, &schemas(), &reply).is_some(),
        "a fenced JSON plan should be extracted"
    );
}

#[test]
fn prose_refusal_is_left_as_text() {
    // Refusals are prose by contract; nothing to extract.
    let reply = "I can't draft this plan: weekly run frequency is below the 7 runs/week guard rail. Confirm a higher frequency and I'll re-emit.";
    assert!(
        extract_structured_plan(Some(SCHEMA_ID), true, &schemas(), reply).is_none(),
        "a prose refusal must not be treated as a plan"
    );
}

#[test]
fn schema_invalid_object_is_rejected() {
    // Missing the required `weeks` array -> fails schema validation -> raw fallback.
    let invalid = r#"{"plan_window":{"start":"2026-05-12","end":"2026-05-18"},"rationale":"x","compliance":{},"evidence_refs":["evidence/x.md"]}"#;
    assert!(
        extract_structured_plan(Some(SCHEMA_ID), true, &schemas(), invalid).is_none(),
        "an object that violates the schema must be rejected"
    );
}

#[test]
fn non_builder_coach_never_extracts() {
    // No output_schema, or a different schema id -> the reply is untouched even
    // when it happens to be a valid plan.
    assert!(extract_structured_plan(None, true, &schemas(), VALID_PLAN).is_none());
    assert!(extract_structured_plan(Some("meal-plan"), true, &schemas(), VALID_PLAN).is_none());
}

#[test]
fn surface_without_a_plan_card_never_extracts() {
    // With no plan-card renderer, stripping the JSON would leave an empty
    // reply, so even a builder coach's valid plan is left in the prose (the
    // coach is steered away from JSON by the withheld directive).
    assert!(
        extract_structured_plan(Some(SCHEMA_ID), false, &schemas(), VALID_PLAN).is_none(),
        "a surface without a plan card must not extract/strip a plan"
    );
}
