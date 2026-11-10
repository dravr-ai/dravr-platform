// ABOUTME: Pins what a detail row carries of the athlete's own text: description, gear, private note
// ABOUTME: The private note reaches only the athlete's own reads; an external client or a human coach never sees it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The activities of the last fourteen days reach the model in detail
//! (carnet#1480): the athlete's description, the gear, and their private
//! note. The note is the athlete's alone.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, GearRef, SportType};
use pierre_core::transport::Transport;
use pierre_providers::ai_scope;
use pierre_tool_runtime::implementations::activity_summary::{
    detail_json, summary_json, ActivitySummary,
};
use serde_json::Value;

fn outing() -> Activity {
    ActivityBuilder::new(
        "16001",
        "Bûcheronner !",
        SportType::Workout,
        Utc.with_ymd_and_hms(2026, 11, 7, 13, 0, 0).unwrap(),
        13_320,
        "strava",
    )
    .moving_time_seconds(4_080)
    .description("Mal aux bras ! Trois heures de bûches.".to_owned())
    .private_note("Genou gauche un peu sensible")
    .gear(GearRef {
        id: Some("b42".to_owned()),
        name: "Epic\nEVO".to_owned(),
    })
    .build()
}

fn row() -> Value {
    detail_json(&[outing()]).unwrap()[0].clone()
}

#[test]
fn the_athletes_own_read_carries_the_private_note_fenced() {
    let row = row();
    let note = row["private_note"]
        .as_str()
        .expect("the athlete sees their note");
    assert!(note.contains("Genou gauche"), "bug: note text lost: {note}");
    assert!(
        note.starts_with("<athlete_text"),
        "bug: the note must be fenced: {note}"
    );
    assert_eq!(
        row["gear"]["name"], "Epic EVO",
        "bug: gear name is one defanged line"
    );
}

#[tokio::test]
async fn an_external_client_never_sees_the_private_note() {
    let row = ai_scope::serve_over(Transport::McpHttp, async { row() }).await;
    assert!(
        row.get("private_note").is_none(),
        "bug: the private note leaked to an external client: {row}"
    );
    assert!(
        row.get("description").is_some(),
        "bug: the description is not withheld"
    );
}

#[tokio::test]
async fn a_human_coach_never_sees_the_private_note() {
    let row = ai_scope::for_human_coach(async { row() }).await;
    assert!(
        row.get("private_note").is_none(),
        "bug: the private note leaked to a human coach: {row}"
    );
}

#[test]
fn a_summary_row_says_the_athlete_wrote_something() {
    let long = ActivityBuilder::new(
        "1",
        "Run",
        SportType::Run,
        Utc.with_ymd_and_hms(2026, 10, 1, 13, 0, 0).unwrap(),
        1_800,
        "strava",
    )
    .description("x".repeat(400))
    .build();
    let rows = summary_json(&[
        ActivitySummary::from(&outing()),
        ActivitySummary::from(&long),
    ])
    .unwrap();
    let excerpt = rows[0]["description_excerpt"].as_str().expect("an excerpt");
    assert!(excerpt.contains("Mal aux bras"), "bug: {excerpt}");
    assert!(
        rows[0].get("description_truncated").is_none(),
        "bug: a short text is whole"
    );
    assert_eq!(
        rows[1]["description_truncated"], true,
        "bug: a long text is marked cut"
    );
    assert_eq!(
        rows[0]["gear"], "Epic EVO",
        "bug: the summary names the gear"
    );
    assert!(
        rows[0].get("private_note").is_none(),
        "bug: a summary never carries the note"
    );
}
