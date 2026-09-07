// ABOUTME: A model-invented series accent must not reject an otherwise valid chart
// ABOUTME: The exact block prod stripped on 2026-08-23 must survive with the accent dropped

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Live incident 2026-08-23 (Telegram group): the coach emitted a valid
//! two-series comparison chart but pinned `"accent": "neutral"` — a
//! plausible word, not a schema value — and the whole block failed schema
//! validation and was stripped; the athlete asked for a graph and got prose.
//! An accent is a styling hint: an unknown value drops the FIELD, never the
//! chart, and the renderer's cycle colours take over.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::BTreeMap;

use dravr_contremaitre::schemas::DRAVR_VIZ_SCHEMA;
use pierre_chat_pipeline::stages::structured_output::{SchemaTexts, DRAVR_VIZ};
use pierre_chat_pipeline::stages::viz_blocks::extract_viz_blocks;
use pierre_chat_pipeline::stages::viz_route::RouteTracks;
use serde_json::Value;

fn schemas() -> SchemaTexts {
    let mut map = BTreeMap::new();
    map.insert(DRAVR_VIZ.to_owned(), DRAVR_VIZ_SCHEMA.to_owned());
    map
}

/// The block prod rejected, verbatim from the 2026-08-23 23:03 UTC logs.
const LIVE_REJECTED_BLOCK: &str = r#"{"type":"chart","kind":"line","source_tool":"get_activities","title":"Heures d'entraînement / semaine","x":{"label":"Semaine du","type":"time"},"series":[{"label":"Toi","accent":"activity","points":[["2026-07-27",22.1],["2026-08-03",11.2],["2026-08-10",10.2],["2026-08-17",18.0]]},{"label":"Philippe","accent":"neutral","points":[["2026-07-27",8.1],["2026-08-03",15.7],["2026-08-10",12.2],["2026-08-17",15.4]]}]}"#;

#[test]
fn an_unknown_accent_is_dropped_and_the_chart_survives() {
    let reply = format!("Voici le portrait 👇\n```dravr-viz\n{LIVE_REJECTED_BLOCK}\n```");
    let granted = vec!["chart".to_owned(), "table".to_owned()];
    let tools = vec!["get_activities".to_owned()];

    let extraction = extract_viz_blocks(&schemas(), &granted, &tools, &RouteTracks::new(), &reply)
        .expect("the reply carries a dravr-viz fence");

    assert_eq!(
        extraction.blocks.len(),
        1,
        "the live-rejected chart must now survive validation"
    );
    let series = extraction.blocks[0]
        .get("series")
        .and_then(Value::as_array)
        .expect("series present");
    assert_eq!(
        series[0].get("accent").and_then(Value::as_str),
        Some("activity"),
        "a valid accent is untouched"
    );
    assert!(
        series[1].get("accent").is_none(),
        "the invented 'neutral' accent is dropped, not the chart"
    );
}

/// A genuinely invalid block (bad top-level type) still gets refused — the
/// tolerance is accent-scoped, not a validation bypass.
#[test]
fn a_truly_invalid_block_is_still_refused() {
    let reply = "Voici 👇\n```dravr-viz\n{\"type\":\"hologram\",\"series\":[]}\n```";
    let granted = vec!["chart".to_owned()];
    let tools = vec!["get_activities".to_owned()];

    let extraction = extract_viz_blocks(&schemas(), &granted, &tools, &RouteTracks::new(), reply)
        .expect("the reply carries a dravr-viz fence");
    assert!(
        extraction.blocks.is_empty(),
        "schema validation still refuses genuinely invalid blocks"
    );
}
