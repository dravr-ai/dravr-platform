// ABOUTME: Tests inline dravr-viz block extraction — ordering, markers, and failure modes
// ABOUTME: An invalid block must stay visible in the reply, never be silently dropped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use dravr_contremaitre::schemas::{DRAVR_VIZ_SCHEMA, STRUCTURED_WORKOUT_SCHEMA};
use pierre_chat_pipeline::stages::prefetch::PREFETCH_TOOL;
use pierre_chat_pipeline::stages::structured_output::SchemaTexts;
use pierre_chat_pipeline::stages::viz_blocks::{
    extract_viz_blocks, granted_visuals, marker, markers_intact, strip_fences, DEFAULT_VISUALS,
};

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

/// Assert a reply's fences were all refused: nothing is lifted, and — the part
/// that matters to the athlete — no JSON survives in the text.
///
/// The old contract left a refused fence in place as literal text. That shipped
/// a screenful of raw JSON to the athlete AND persisted it as the assistant
/// message, so the coach read its own transcript on the next turn, believed it
/// had already drawn a chart, and refused to draw again (Telegram 2026-08-21).
fn assert_all_refused_with(granted: &[String], tools: &[String], reply: &str) -> String {
    let out = extract_viz_blocks(&schemas(), granted, tools, reply)
        .expect("a reply containing a fence is still processed, to strip it");
    assert!(
        out.blocks.is_empty(),
        "no block may be lifted; got {:?}",
        out.blocks
    );
    assert!(
        !out.text.contains("dravr-viz"),
        "the fence must not survive into the reply: {}",
        out.text
    );
    assert!(
        !out.text.contains("\"type\":"),
        "no block JSON may survive into the reply: {}",
        out.text
    );
    out.text
}

/// [`assert_all_refused_with`] for the common grant and provenance.
fn assert_all_refused(reply: &str) -> String {
    assert_all_refused_with(&granted(), &tools_called(), reply)
}

#[test]
fn an_invalid_block_is_stripped_from_the_reply() {
    // Missing source_tool — schema-invalid.
    let bad = r#"{"type":"chart","kind":"line","x":{"label":"Date","type":"time"},"series":[{"label":"CTL","points":[["a",1],["b",2]]}]}"#;
    let reply = format!("Voici.\n\n{}\n\nFin.", fenced(bad));

    let text = assert_all_refused(&reply);
    assert!(
        text.contains("Voici.") && text.contains("Fin."),
        "the coach's prose must survive untouched: {text}"
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
        !out.text.contains("scatter"),
        "the rejected block is stripped, not shown as JSON: {}",
        out.text
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

    assert_all_refused(&reply);
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

    {
        let out = extract_viz_blocks(&schemas(), &granted(), &only_activities, &reply)
            .expect("a reply containing a fence is still processed, to strip it");
        assert!(
            out.blocks.is_empty(),
            "a source_tool that did not run must reject the block"
        );
        assert!(
            !out.text.contains("dravr-viz"),
            "the refused fence must not survive into the reply: {}",
            out.text
        );
    }
}

#[test]
fn a_turn_with_no_tool_calls_renders_no_blocks() {
    // No tool ran, so there is no data any visual could truthfully be built
    // from. Every block must be refused, whatever it claims.
    let reply = format!("Voici.\n\n{}", fenced(CHART));
    {
        let out = extract_viz_blocks(&schemas(), &granted(), &[], &reply)
            .expect("a reply containing a fence is still processed, to strip it");
        assert!(
            out.blocks.is_empty(),
            "an empty tool record must reject every block"
        );
        assert!(
            !out.text.contains("dravr-viz"),
            "the refused fence must not survive into the reply: {}",
            out.text
        );
    }
}

#[test]
fn source_tool_matching_is_exact() {
    // Case-insensitive or fuzzy matching would let "Get_Activities" pass while
    // remaining uncheckable against the recorded name. A near-miss is a miss.
    let reply = format!("Voici.\n\n{}", fenced(TABLE));
    let near_miss = vec!["Get_Activities".to_owned()];
    {
        let out = extract_viz_blocks(&schemas(), &granted(), &near_miss, &reply)
            .expect("a reply containing a fence is still processed, to strip it");
        assert!(
            out.blocks.is_empty(),
            "source_tool comparison must be exact"
        );
        assert!(
            !out.text.contains("dravr-viz"),
            "the refused fence must not survive into the reply: {}",
            out.text
        );
    }
}

#[test]
fn a_kind_outside_the_grant_is_rejected() {
    // Granted tables only: a chart is outside the grant even though it is
    // schema-valid and correctly attributed. The frontmatter list is a
    // permission set, not a boolean.
    let tables_only = vec!["table".to_owned()];
    let chart_reply = format!("Voici.\n\n{}", fenced(CHART));
    assert_all_refused_with(&tables_only, &tools_called(), &chart_reply);

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

/// The provenance a prefetched turn carries: the platform ran `get_activities`
/// itself in [`pierre_chat_pipeline::stages::prefetch`], and the tool loop
/// recorded nothing because the model never had to call anything.
fn prefetch_only_provenance() -> Vec<String> {
    vec![PREFETCH_TOOL.to_owned()]
}

/// A chart built from pre-loaded activities must render.
///
/// The platform prefetches the athlete's activity window before dispatch and
/// then tells the coach to use those rows *without* re-fetching. A coach that
/// obeys calls no tool, so the tool loop reported an empty `tools_called` and
/// this gate refused the chart as unsourced — leaving the raw ```dravr-viz```
/// fence in the reply. Observed on Slack 2026-08-20: the athlete asked for a
/// graph and got a wall of JSON.
///
/// The data was real and the citation was accurate; only the bookkeeping was
/// missing. The gate must accept the prefetch's own run as the source.
#[test]
fn a_chart_sourced_from_the_prefetch_is_lifted() {
    let reply = format!("Voici tes distances par semaine.\n\n{}", fenced(TABLE));
    let out = extract_viz_blocks(&schemas(), &granted(), &prefetch_only_provenance(), &reply)
        .expect("a block citing the prefetched tool must be extracted");

    assert_eq!(out.blocks.len(), 1, "exactly one block was fenced");
    assert_eq!(out.blocks[0]["source_tool"], "get_activities");
    assert_eq!(
        out.blocks[0]["rows"].as_array().map(Vec::len),
        Some(2),
        "the block must survive extraction with its rows intact"
    );
    assert!(
        out.text.contains(&marker(0)),
        "prose keeps a positional marker where the block sat: {}",
        out.text
    );
    assert!(
        !out.text.contains("dravr-viz"),
        "the fence must not survive as literal text: {}",
        out.text
    );
}

/// Prefetch provenance is not a blanket pass: a block citing a tool that
/// neither the prefetch nor the model ran is still refused. Otherwise recording
/// the prefetch would have widened the gate into an escape hatch.
#[test]
fn prefetch_provenance_does_not_excuse_an_uncited_tool() {
    let reply = format!("Ta charge grimpe.\n\n{}", fenced(CHART));
    assert_all_refused_with(&granted(), &prefetch_only_provenance(), &reply);
}

/// The reply the athlete reads must never contain a chart spec.
///
/// This is the compound failure observed on Telegram, 2026-08-21. A block was
/// refused, the fence stayed in the reply as literal text, and that text became
/// the persisted assistant message. The athlete saw a screenful of JSON; worse,
/// on the next turn the coach read its own transcript, concluded it had already
/// drawn the chart, and answered "le graphique est déjà juste au-dessus" —
/// refusing to draw a real one. One refusal poisoned every turn after it.
///
/// Asserting on the shape of the surviving text rather than on `is_none()`
/// keeps this honest: what matters is that nothing machine-shaped reaches the
/// athlete or the transcript, however the refusal was decided.
#[test]
fn a_refused_block_leaves_no_machine_text_for_the_next_turn_to_read() {
    // Cites a tool that did not run — the exact gate that fired on 2026-08-21.
    let uncited = r#"{"type":"chart","kind":"bar","source_tool":"analyze_training_load","title":"Volume hebdo (km) — 12 semaines","x":{"label":"Semaine","type":"time"},"series":[{"label":"km/semaine","accent":"activity","points":[["2026-05-18",25.5],["2026-05-25",33.6]]}]}"#;
    let reply = format!(
        "Voici ton volume hebdomadaire.\n\n{}\n\nDeux trous complets début juin.",
        fenced(uncited)
    );

    let text = assert_all_refused_with(&granted(), &prefetch_only_provenance(), &reply);

    // The coaching survives in full — the visual contract already requires the
    // prose to carry the interpretation on its own.
    assert!(
        text.contains("Voici ton volume hebdomadaire.") && text.contains("Deux trous complets"),
        "the prose must survive intact: {text}"
    );
    // Nothing a later turn could mistake for "I already showed a chart".
    for shard in [
        "dravr-viz",
        "source_tool",
        "\"kind\"",
        "points",
        "Volume hebdo",
        "```",
    ] {
        assert!(
            !text.contains(shard),
            "refused-block text {shard:?} must not reach the athlete or the transcript: {text}"
        );
    }
}

/// Stored history must never replay a fence back to the coach.
///
/// Fixes ship forward, conversations do not: every transcript that already
/// carries a leaked fence would keep telling the coach it had drawn a chart.
/// Stripping on replay heals those conversations instead of requiring surgery
/// on the message table.
#[test]
fn a_stored_fence_is_stripped_before_the_coach_reads_its_own_transcript() {
    let poisoned = format!(
        "Voici ton volume hebdomadaire.\n\n{}\n\nDeux trous complets début juin.",
        fenced(CHART)
    );

    let clean = strip_fences(&poisoned);

    assert!(
        !clean.contains("dravr-viz") && !clean.contains("source_tool"),
        "no fence may survive into replayed history: {clean}"
    );
    assert!(
        clean.contains("Voici ton volume hebdomadaire.") && clean.contains("Deux trous complets"),
        "the coach's own prose must survive: {clean}"
    );
}

/// Ordinary replies are handed back untouched — no allocation, no rewriting.
#[test]
fn history_without_a_fence_is_left_exactly_as_it_was() {
    let plain = "Ta charge grimpe depuis trois semaines. On coupe jeudi.";
    assert_eq!(strip_fences(plain), plain);

    // A marker is not a fence: it means a chart really was delivered, and the
    // coach may legitimately remember showing it.
    let with_marker = format!("Voici.\n\n{}\n\nEt donc.", marker(0));
    assert_eq!(strip_fences(&with_marker), with_marker);
}

/// A chat with no coach bound may still draw.
///
/// The `visuals:` grant belongs to a coach author, so it only exists when a
/// coach is bound. A Telegram group binds none — the platform answers directly
/// — and reading that as an empty grant withheld the visual contract from the
/// prompt. The model then told the group it had no way to draw a chart
/// ("pas d'outil pour ça de mon côté", 2026-08-21) *after* successfully calling
/// `get_activities`: it had the data and no permission to picture it.
#[test]
fn a_conversation_with_no_coach_falls_back_to_the_platform_grant() {
    let grant = granted_visuals(None);

    assert!(
        !grant.is_empty(),
        "no coach bound must not read as 'no visuals'"
    );
    for kind in DEFAULT_VISUALS {
        assert!(
            grant.iter().any(|g| g == kind),
            "the platform baseline must include {kind:?}"
        );
    }

    // And the grant is live: a chart extracts under it.
    let reply = format!("Voici ton volume.\n\n{}", fenced(CHART));
    let out = extract_viz_blocks(&schemas(), &grant, &tools_called(), &reply)
        .expect("a coach-less turn must still lift a valid chart");
    assert_eq!(out.blocks.len(), 1);
    assert_eq!(out.blocks[0]["kind"], "line");
}

/// A bound coach still governs its own reply — including choosing not to draw.
///
/// The fallback must not become "everyone draws". An author who ships a coach
/// with no `visuals:` made a decision, and 13 of the 26 catalogue coaches have
/// made exactly that one.
#[test]
fn a_bound_coach_governs_its_own_grant() {
    assert!(
        granted_visuals(Some(&[])).is_empty(),
        "a coach that declares no visuals must not inherit the platform default"
    );

    let tables_only = vec!["table".to_owned()];
    let grant = granted_visuals(Some(&tables_only));
    assert_eq!(
        grant, tables_only,
        "a declared grant passes through verbatim"
    );

    // ...and it is still enforced.
    let chart_reply = format!("Voici.\n\n{}", fenced(CHART));
    assert_all_refused_with(&grant, &tools_called(), &chart_reply);
}
