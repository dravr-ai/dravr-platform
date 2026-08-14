// ABOUTME: Tests inline dravr-viz block extraction — ordering, markers, and failure modes
// ABOUTME: An invalid block must stay visible in the reply, never be silently dropped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use dravr_contremaitre::schemas::{DRAVR_VIZ_SCHEMA, STRUCTURED_WORKOUT_SCHEMA};
use pierre_chat_pipeline::stages::structured_output::SchemaTexts;
use pierre_chat_pipeline::stages::viz_blocks::{extract_viz_blocks, marker, markers_intact};

/// The full schema set, as production assembles it. Every test hands over the
/// same map: compiled validators live in a process-wide `OnceLock`, so the
/// first call in a binary decides what is registered for all of them.
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

/// Both kinds granted — the common case for the fixtures below.
fn granted() -> Vec<String> {
    vec!["chart".to_owned(), "table".to_owned()]
}

/// The tools the fixtures' blocks cite, as the tool loop would record them.
fn tools_called() -> Vec<String> {
    vec![
        "analyze_training_load".to_owned(),
        "get_activities".to_owned(),
    ]
}

const CHART: &str = r#"{"type":"chart","kind":"line","source_tool":"analyze_training_load","x":{"label":"Date","type":"time"},"series":[{"label":"CTL","points":[["2026-07-01",42.0],["2026-07-02",43.1]]}]}"#;

const TABLE: &str = r#"{"type":"table","source_tool":"get_activities","columns":["Day","Distance"],"rows":[["Tue","12 km"],["Sun","24 km"]]}"#;

fn fenced(body: &str) -> String {
    format!("```dravr-viz\n{body}\n```")
}

#[test]
fn lifts_a_block_and_leaves_a_marker() {
    let reply = format!(
        "Ta charge grimpe depuis trois semaines.\n\n{}\n\nC'est pourquoi on coupe jeudi.",
        fenced(CHART)
    );
    let out = extract_viz_blocks(&schemas(), &granted(), &tools_called(), &reply)
        .expect("a valid block must be extracted");

    assert_eq!(out.blocks.len(), 1);
    assert_eq!(out.blocks[0]["kind"], "line");
    assert!(out.text.contains(&marker(0)), "prose keeps a marker");
    assert!(
        out.text.contains("Ta charge grimpe"),
        "prose before the block survives"
    );
    assert!(
        out.text.contains("on coupe jeudi"),
        "prose after the block survives"
    );
    assert!(
        !out.text.contains("dravr-viz"),
        "the fence itself is gone from the prose"
    );
}

#[test]
fn preserves_order_across_several_blocks() {
    let reply = format!(
        "Un.\n\n{}\n\nDeux.\n\n{}\n\nTrois.",
        fenced(CHART),
        fenced(TABLE)
    );
    let out = extract_viz_blocks(&schemas(), &granted(), &tools_called(), &reply)
        .expect("both blocks must be extracted");

    assert_eq!(out.blocks.len(), 2);
    assert_eq!(out.blocks[0]["type"], "chart");
    assert_eq!(out.blocks[1]["type"], "table");
    let first = out.text.find(&marker(0)).expect("marker 0 present");
    let second = out.text.find(&marker(1)).expect("marker 1 present");
    assert!(
        first < second,
        "markers keep the order the blocks appeared in"
    );
}

#[test]
fn an_invalid_block_stays_visible_in_the_reply() {
    // Missing source_tool — schema-invalid. It must NOT be deleted: silently
    // dropping content the coach meant to show is worse than showing a fence.
    let bad = r#"{"type":"chart","kind":"line","x":{"label":"Date","type":"time"},"series":[{"label":"CTL","points":[["a",1],["b",2]]}]}"#;
    let reply = format!("Voici.\n\n{}\n\nFin.", fenced(bad));

    assert!(
        extract_viz_blocks(&schemas(), &granted(), &tools_called(), &reply).is_none(),
        "no valid blocks means the reply is left untouched"
    );
}

#[test]
fn a_valid_block_survives_alongside_an_invalid_one() {
    let bad = r#"{"type":"chart","kind":"scatter","source_tool":"t","x":{"label":"D","type":"time"},"series":[{"label":"C","points":[["a",1],["b",2]]}]}"#;
    let reply = format!("A.\n\n{}\n\nB.\n\n{}\n\nC.", fenced(bad), fenced(CHART));
    let out = extract_viz_blocks(&schemas(), &granted(), &tools_called(), &reply)
        .expect("the good block must be extracted");

    assert_eq!(out.blocks.len(), 1, "only the valid block is lifted");
    assert!(
        out.text.contains("scatter"),
        "the rejected block stays in the reply as literal text"
    );
    assert!(
        out.text.contains(&marker(0)),
        "the valid block leaves a marker"
    );
}

#[test]
fn rejects_a_table_whose_rows_do_not_match_its_columns() {
    // JSON Schema cannot express this relation, so the stage checks it.
    let ragged = r#"{"type":"table","source_tool":"get_activities","columns":["Day","Distance"],"rows":[["Tue","12 km"],["Sun"]]}"#;
    let reply = format!("Voici.\n\n{}", fenced(ragged));

    assert!(
        extract_viz_blocks(&schemas(), &granted(), &tools_called(), &reply).is_none(),
        "a ragged table must be rejected, not rendered with holes"
    );
}

#[test]
fn ignores_other_fenced_languages() {
    let reply = "Here is some code:\n\n```json\n{\"type\":\"chart\"}\n```\n\nDone.";
    assert!(
        extract_viz_blocks(&schemas(), &granted(), &tools_called(), reply).is_none(),
        "a ```json fence is not a viz block"
    );
}

#[test]
fn a_prose_mention_does_not_open_a_block() {
    let reply = "I can render a dravr-viz block if you want one.";
    assert!(
        extract_viz_blocks(&schemas(), &granted(), &tools_called(), reply).is_none(),
        "the info string only counts on a fence opener line"
    );
}

#[test]
fn a_viz_block_inside_another_fence_is_not_extracted() {
    // Documentation showing the syntax must not be mistaken for a real block.
    let reply = format!(
        "To emit one, write:\n\n```text\n{}\n```\n\nUnderstood?",
        fenced(CHART)
    );
    assert!(
        extract_viz_blocks(&schemas(), &granted(), &tools_called(), &reply).is_none(),
        "a viz fence nested in another fence is documentation, not a block"
    );
}

#[test]
fn a_reply_with_no_fence_is_left_alone() {
    assert!(extract_viz_blocks(
        &schemas(),
        &granted(),
        &tools_called(),
        "Repose-toi aujourd'hui."
    )
    .is_none());
}

#[test]
fn an_unterminated_fence_is_not_extracted() {
    let reply = format!("Voici.\n\n```dravr-viz\n{CHART}");
    assert!(
        extract_viz_blocks(&schemas(), &granted(), &tools_called(), &reply).is_none(),
        "a truncated reply must not yield a half-parsed block"
    );
}

#[test]
fn a_block_citing_a_tool_that_did_not_run_is_rejected() {
    // The chart cites analyze_training_load, but this turn only ran
    // get_activities — the citation is unverifiable, so the block must not
    // render as if it were measured.
    let reply = format!("Voici.\n\n{}\n\nFin.", fenced(CHART));
    let only_activities = vec!["get_activities".to_owned()];

    assert!(
        extract_viz_blocks(&schemas(), &granted(), &only_activities, &reply).is_none(),
        "a source_tool that did not run must reject the block"
    );
}

#[test]
fn a_turn_with_no_tool_calls_renders_no_blocks() {
    // No tool ran, so there is no data any visual could truthfully be built
    // from. Every block must be refused, whatever it claims.
    let reply = format!("Voici.\n\n{}", fenced(CHART));
    assert!(
        extract_viz_blocks(&schemas(), &granted(), &[], &reply).is_none(),
        "an empty tool record must reject every block"
    );
}

#[test]
fn source_tool_matching_is_exact() {
    // Case-insensitive or fuzzy matching would let "Get_Activities" pass while
    // remaining uncheckable against the recorded name. A near-miss is a miss.
    let reply = format!("Voici.\n\n{}", fenced(TABLE));
    let near_miss = vec!["Get_Activities".to_owned()];
    assert!(
        extract_viz_blocks(&schemas(), &granted(), &near_miss, &reply).is_none(),
        "source_tool comparison must be exact"
    );
}

#[test]
fn a_kind_outside_the_grant_is_rejected() {
    // Granted tables only: a chart is outside the grant even though it is
    // schema-valid and correctly attributed. The frontmatter list is a
    // permission set, not a boolean.
    let tables_only = vec!["table".to_owned()];
    let chart_reply = format!("Voici.\n\n{}", fenced(CHART));
    assert!(
        extract_viz_blocks(&schemas(), &tables_only, &tools_called(), &chart_reply).is_none(),
        "a chart from a tables-only coach must be refused"
    );

    let table_reply = format!("Voici.\n\n{}", fenced(TABLE));
    assert!(
        extract_viz_blocks(&schemas(), &tables_only, &tools_called(), &table_reply).is_some(),
        "a table from the same coach must still pass"
    );
}

#[test]
fn markers_intact_detects_a_rewritten_reply() {
    // The guard behind the post-process parity check. A stage that replaces the
    // reply wholesale — guardrail blocked-topic, too-long truncation, the
    // verification fallback — drops the markers while the blocks survive; the
    // athlete would then get a chart rendered under a refusal.
    let with_markers = format!("Voici. {} Et {}", marker(0), marker(1));
    assert!(markers_intact(&with_markers, 2));

    let canned = "Je ne peux pas discuter de ça.";
    assert!(
        !markers_intact(canned, 2),
        "a canned replacement has no markers, so its blocks must be dropped"
    );

    let truncated = format!("Voici. {}", marker(0));
    assert!(
        !markers_intact(&truncated, 2),
        "truncation that cuts the second marker must drop the blocks"
    );

    // Zero blocks is vacuously intact — nothing to place.
    assert!(markers_intact(canned, 0));
}
