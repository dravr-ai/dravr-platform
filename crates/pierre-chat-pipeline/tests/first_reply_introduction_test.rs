// ABOUTME: An agent's first reply opens with its introduction, named in the athlete's locale; later replies never do
// ABOUTME: Pins the decision (first reply, bound agent, one-to-one), the localized title and the Stage 7g.3 placement
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#501: on 2026-09-21 the Half Marathon Agent answered the first
//! message of a web chat straight into analysis, with no name and no role. No
//! introduction rule had ever existed, so this is the rule, and these are the
//! decisions it is made of. The end-to-end half — the rule reaching a real
//! assembled prompt on turn 1 and not on turn 2 — is
//! `crates/pierre-server/tests/first_reply_introduction_pipeline_test.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::fs;
use std::path::PathBuf;

use pierre_chat_pipeline::stages::introduction::{
    agent_display_title, first_reply_introduction, introduction_directive, is_first_reply,
};
use pierre_core::models::agents::AgentCategory;
use pierre_core::models::AgentRuntimeContext;
use pierre_database::database::MessageRecord;

/// The French persona's frontmatter as the pinned dravr-contremaitre ships it
/// (`prompts/agents/training/half-marathon-agent/fr.md` at 799fbae), so the
/// parser is exercised against the real shape — `startup.visuals`, `replaces`,
/// a nested `data_requirements` — and not a trimmed-down stand-in.
const HALF_MARATHON_FR: &str = r#"---
name: half-marathon-agent
title: Agent Semi-Marathon
category: training
tags: [course-a-pied, semi-marathon, 13.1, tempo, endurance, competition]
prerequisites:
  providers: [strava, garmin, fitbit, whoop, coros, terra]
  min_activities: 8
  activity_types: [Run]
visibility: tenant
replaces: [half-marathon-coach]
startup:
  visuals: [chart, table, route]
  query: "Résume mon volume d'entraînement récent, le travail tempo et les distances de sorties longues."
  data_requirements:
    activities:
      count: 20
      time_frame: 12w
      mode: summary
      analysis_type: race_preparation
---

## Purpose
Spécialiste de la préparation et du pacing sur 21,1 km.

## Instructions
Tu es spécialiste du semi-marathon et tu aides les coureurs à préparer le 21,1 km.
"#;

fn row(role: &str, content: &str) -> MessageRecord {
    MessageRecord {
        id: format!("{role}-{}", content.len()),
        conversation_id: "conv-501".to_owned(),
        role: role.to_owned(),
        content: content.to_owned(),
        token_count: None,
        prompt_tokens: None,
        model: None,
        finish_reason: None,
        content_blocks: None,
        created_at: "2026-09-21T23:46:00Z".to_owned(),
    }
}

/// The transcript prompt assembly receives on the first message: that message
/// alone.
fn first_message() -> Vec<MessageRecord> {
    vec![row(
        "user",
        "Montre-moi mes sorties de mars 2026 avec le dénivelé.",
    )]
}

/// The transcript on the second message: the first exchange, then the new ask.
fn second_message() -> Vec<MessageRecord> {
    vec![
        row(
            "user",
            "Montre-moi mes sorties de mars 2026 avec le dénivelé.",
        ),
        row(
            "assistant",
            "Mars a été un mois de fond plutôt que de spécifique semi.",
        ),
        row("user", "Et avril ?"),
    ]
}

fn agent(source: &str, title: &str) -> AgentRuntimeContext {
    AgentRuntimeContext {
        slug: "half-marathon-agent".to_owned(),
        title: title.to_owned(),
        source: source.to_owned(),
        system_prompt: "Tu es spécialiste du semi-marathon.".to_owned(),
        startup_query: None,
        data_requirements: None,
        visuals: Vec::new(),
        max_tool_iterations: None,
        temperature: None,
        category: AgentCategory::Training,
    }
}

#[test]
fn only_an_unanswered_conversation_is_a_first_reply() {
    assert!(
        is_first_reply(&first_message()),
        "a conversation holding only the athlete's first message has never been answered"
    );
    assert!(
        !is_first_reply(&second_message()),
        "a conversation holding an assistant row has already been answered"
    );
}

#[test]
fn a_catalogue_agent_is_named_by_its_persona_title_in_the_turns_locale() {
    let title = agent_display_title(
        &agent("contremaitre", "Half Marathon Agent"),
        HALF_MARATHON_FR,
    );
    assert_eq!(
        title.as_deref(),
        Some("Agent Semi-Marathon"),
        "a French turn loads the French persona, and its title is the name the athlete \
         reads in the store — not the canonical English column"
    );
}

#[test]
fn a_custom_agent_is_named_by_its_own_title() {
    // A custom agent's prompt is the author's text, never parsed: only a
    // catalogue persona is a file with frontmatter.
    let title = agent_display_title(
        &agent("custom", "Coach Trail Laurentides"),
        HALF_MARATHON_FR,
    );
    assert_eq!(title.as_deref(), Some("Coach Trail Laurentides"));
}

#[test]
fn a_catalogue_persona_without_frontmatter_falls_back_to_the_column() {
    let title = agent_display_title(
        &agent("contremaitre", "Half Marathon Agent"),
        "Tu es spécialiste du semi-marathon.",
    );
    assert_eq!(
        title.as_deref(),
        Some("Half Marathon Agent"),
        "a persona that does not parse must never leave the agent nameless"
    );
}

#[test]
fn a_title_is_one_line_or_none() {
    let title = agent_display_title(&agent("custom", "  Half\nMarathon\t Agent "), "");
    assert_eq!(
        title.as_deref(),
        Some("Half Marathon Agent"),
        "the title is spliced into one sentence; a line break would split it"
    );
    assert_eq!(agent_display_title(&agent("custom", " \n "), ""), None);
}

#[test]
fn a_bound_agent_is_introduced_on_its_first_reply_and_never_after() {
    let bound = agent("contremaitre", "Half Marathon Agent");

    let first = first_reply_introduction(&first_message(), Some(&bound), HALF_MARATHON_FR, true)
        .expect("the first reply of a one-to-one conversation with a bound agent is introduced");
    assert!(
        first.contains("introducing yourself by your title, Agent Semi-Marathon,"),
        "the directive must name the agent by its localized title: {first}"
    );

    assert_eq!(
        first_reply_introduction(&second_message(), Some(&bound), HALF_MARATHON_FR, true),
        None,
        "a later reply must not introduce the agent again"
    );
}

#[test]
fn a_coachless_turn_keeps_its_own_voice() {
    assert_eq!(
        first_reply_introduction(&first_message(), None, "", true),
        None,
        "with no agent bound Dravr answers under the identity anchor, as it always has"
    );
}

#[test]
fn a_shared_room_is_never_introduced() {
    // Each member of a room has their own conversation row, so "no reply in
    // this row yet" is true for every newcomer while the room has heard the
    // agent many times.
    let bound = agent("custom", "Eval Coach");
    assert_eq!(
        first_reply_introduction(&first_message(), Some(&bound), "", false),
        None
    );
}

#[test]
fn a_nameless_agent_is_not_introduced() {
    let bound = agent("custom", "   ");
    assert_eq!(
        first_reply_introduction(&first_message(), Some(&bound), "", true),
        None,
        "an introduction with no name in it is not an introduction"
    );
}

/// The directive asks for the introduction AND the answer, in that order.
#[test]
fn the_directive_introduces_then_answers() {
    let directive = introduction_directive("Agent Semi-Marathon");
    assert!(directive.contains("first reply in this conversation"));
    assert!(directive.contains("Open it with one short sentence"));
    assert!(
        directive.contains("what you help them with"),
        "an introduction names the role as well as the name: {directive}"
    );
    assert!(
        directive.contains("then answer their question in the same reply"),
        "the introduction must never replace the answer: {directive}"
    );
    assert!(
        directive.starts_with('\n') && !directive.contains("\n\n"),
        "it continues the turn directive's block as one more line of it"
    );
}

/// The same bans `turn_directive_test` holds `TURN_DIRECTIVE` to, because the
/// directive is appended to it and reaches the model as part of it.
#[test]
fn the_directive_asserts_no_identity_and_sets_no_format() {
    let lower = introduction_directive("Agent Semi-Marathon").to_lowercase();
    for banned in [
        "you are",
        "you are not",
        "never reveal",
        "never claim",
        "role-play",
        "language model",
        "coding assistant",
        "command-line",
        "your identity",
        "real identity",
        "underlying model",
        "characters",
        "markdown",
        "json",
        "plain text",
        "bullet",
    ] {
        assert!(
            !lower.contains(banned),
            "the introduction directive must state a task, never an identity or a \
             format: found {banned:?}"
        );
    }
    // The language is Stage 7g.3b's alone; a second language rule here would
    // be the inference that block replaced.
    for language_rule in ["language", "french", "english", "locale"] {
        assert!(
            !lower.contains(language_rule),
            "the directive must not carry a language rule: found {language_rule:?}"
        );
    }
}

/// The introduction rides the ordinary arm of Stage 7g.3 and no other.
///
/// A guided flow's directive claims the slot on its own turns and forbids
/// anything but its probe; an introduction stacked on it would compete with
/// the one instruction that must win there.
#[test]
fn the_introduction_rides_only_the_ordinary_turn_arm() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/stages/prompt_assembly.rs");
    let source = fs::read_to_string(&path).expect("read prompt_assembly.rs");
    let start = source.find("// Stage 7g.3:").expect("Stage 7g.3 marker");
    let end = source.find("// Stage 7g.3b:").expect("Stage 7g.3b marker");
    let slot = &source[start..end];

    assert!(
        slot.contains("super::introduction::first_reply_introduction("),
        "Stage 7g.3 must decide the introduction from the turn itself"
    );
    assert_eq!(
        slot.matches("{introduction}").count(),
        1,
        "the introduction is interpolated exactly once in the slot"
    );
    let arm = slot
        .lines()
        .find(|line| line.contains("{introduction}"))
        .expect("checked above");
    assert!(
        arm.trim_start()
            .starts_with("None => format!(\"{raw_system_prompt}{TURN_DIRECTIVE}"),
        "the introduction must follow TURN_DIRECTIVE in the ordinary arm, found: {arm}"
    );
}
