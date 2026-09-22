// ABOUTME: An agent introduces itself once per thread, by the title the store shows, as Dravr's agent
// ABOUTME: Pins the thread key, the title choice, the directive's wording and slot, and the reply check that records it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#501: on 2026-09-21 the Half Marathon Agent answered the first
//! message of a web chat straight into analysis, with no name and no role. No
//! introduction rule had ever existed, so this is the rule, and these are the
//! decisions it is made of. The end-to-end half — real turns through the
//! pipeline, a rebind, a guided walk, a room — is
//! `crates/pierre-server/tests/first_reply_introduction_pipeline_test.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::fs;
use std::path::PathBuf;

use pierre_chat_pipeline::stages::introduction::{
    display_title, introduction_directive, introduction_thread, reply_names_agent,
};
use pierre_chat_pipeline::stages::prompt_assembly::IDENTITY_ANCHOR;
use pierre_core::models::ConversationRecord;

fn conversation(group_id: Option<&str>) -> ConversationRecord {
    ConversationRecord {
        id: "conv-501".to_owned(),
        user_id: "user-501".to_owned(),
        tenant_id: "tenant-501".to_owned(),
        title: "Semi".to_owned(),
        model: "mock-model".to_owned(),
        agent_id: Some("half-marathon-agent".to_owned()),
        session_id: None,
        total_tokens: 0,
        created_at: "2026-09-21T23:46:00Z".to_owned(),
        updated_at: "2026-09-21T23:46:00Z".to_owned(),
        group_id: group_id.map(ToOwned::to_owned),
        channel_type: "web".to_owned(),
        onboarding_state: None,
    }
}

#[test]
fn a_one_to_one_conversation_is_its_own_thread() {
    assert_eq!(introduction_thread(&conversation(None)), "conv-501");
}

#[test]
fn a_room_members_row_introduces_the_agent_to_the_room() {
    // Each member of a room has their own conversation row, so keyed by the
    // row every newcomer would hear the agent introduce itself again.
    assert_eq!(
        introduction_thread(&conversation(Some("group-501"))),
        "group-501"
    );
}

#[test]
fn the_store_title_names_the_agent_in_the_athletes_locale() {
    assert_eq!(
        display_title(Some("Agent Semi-Marathon"), "Half Marathon Agent").as_deref(),
        Some("Agent Semi-Marathon"),
        "a French athlete meets the title the store showed them, not the canonical column"
    );
}

#[test]
fn an_agent_with_no_translation_keeps_its_own_title() {
    assert_eq!(
        display_title(None, "Coach Trail Laurentides").as_deref(),
        Some("Coach Trail Laurentides")
    );
    assert_eq!(
        display_title(Some("  "), "Half Marathon Agent").as_deref(),
        Some("Half Marathon Agent"),
        "a blank translation must never leave the agent nameless"
    );
}

#[test]
fn a_title_is_one_line_or_none() {
    assert_eq!(
        display_title(None, "  Half\nMarathon\t Agent ").as_deref(),
        Some("Half Marathon Agent"),
        "the title is spliced into one sentence; a line break would split it"
    );
    assert_eq!(display_title(Some(" \n "), " \n "), None);
}

/// The directive asks for the introduction AND the turn's own task, in that
/// order, and keeps a persona's tool-first protocol intact.
#[test]
fn the_directive_introduces_then_hands_the_reply_back_to_the_turn() {
    let directive = introduction_directive("Agent Semi-Marathon");
    assert!(directive.contains("You have not introduced yourself in this conversation yet."));
    assert!(directive.contains("Open the text of this reply with one short sentence"));
    assert!(
        directive.contains("saying what you help with"),
        "an introduction names the role as well as the name: {directive}"
    );
    assert!(
        directive.contains("then carry on with this turn as the instructions above ask"),
        "the introduction must never replace the turn's own task — an answer, a guided \
         probe, the release after an interview: {directive}"
    );
    assert!(
        directive.contains("Any tool call you need still comes before that sentence"),
        "a builder persona's turn-1 protocol puts tool calls before any prose; the \
         directive must say which comes first: {directive}"
    );
    assert!(
        directive.starts_with('\n') && !directive.contains("\n\n"),
        "it continues the slot's directive block as one more line of it"
    );
}

/// The agent introduces itself as Dravr's, which is the one identity the tail
/// anchor allows. A bare "introduce yourself as Agent Semi-Marathon" would be
/// answered by the anchor's "never claim any identity other than Dravr", and
/// the anchor is the instruction measured to win.
#[test]
fn the_introduction_stays_inside_the_identity_the_anchor_asserts() {
    assert!(
        IDENTITY_ANCHOR.contains("never claim any identity other than Dravr"),
        "the premise of this test is the anchor's own rule"
    );
    let directive = introduction_directive("Agent Semi-Marathon");
    assert!(
        directive.contains("introducing yourself as Dravr's Agent Semi-Marathon"),
        "the title must be introduced as Dravr's: {directive}"
    );
}

/// The same bans `turn_directive_test` holds `TURN_DIRECTIVE` to, because the
/// directive is appended to the slot and reaches the model as part of it.
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

#[test]
fn a_reply_opening_on_the_agents_name_introduced_it() {
    assert!(reply_names_agent(
        "Salut, je suis l'Agent Semi-Marathon de Dravr, là pour préparer ton 21,1 km. \
         Mars a été un mois de fond.",
        "Agent Semi-Marathon"
    ));
    assert!(
        reply_names_agent(
            "Je suis l'agent semi marathon de Dravr. Mars a été un mois de fond.",
            "Agent Semi-Marathon"
        ),
        "case and punctuation are not what makes a name"
    );
}

#[test]
fn a_reply_that_never_names_the_agent_introduced_nobody() {
    assert!(
        !reply_names_agent(
            "Mars a été un mois de fond plutôt que de spécifique semi.",
            "Agent Semi-Marathon"
        ),
        "the 2026-09-21 reply itself: straight into analysis"
    );
    assert!(
        !reply_names_agent(
            "Mon lien avec Strava a expiré — reconnecte-le ici pour que je voie tes sorties.",
            "Agent Semi-Marathon"
        ),
        "platform text standing in for the reply names no agent"
    );
    assert!(!reply_names_agent("Anything at all", " - "));
}

#[test]
fn a_name_deep_in_the_reply_is_a_mention_not_an_introduction() {
    let reply = format!(
        "{} Au fait, l'Agent Semi-Marathon peut aussi t'aider.",
        "Mars a été un mois de fond. ".repeat(20)
    );
    assert!(
        !reply_names_agent(&reply, "Agent Semi-Marathon"),
        "the introduction opens the reply; a name hundreds of characters in did not"
    );
}

/// The introduction follows the Stage 7g.3 slot's match, so it rides whichever
/// arm owns the turn — the ordinary directive, a guided probe, the release —
/// and no arm can carry it twice or drop it.
#[test]
fn the_introduction_follows_every_arm_of_the_slot() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/stages/prompt_assembly.rs");
    let source = fs::read_to_string(&path).expect("read prompt_assembly.rs");
    let start = source.find("// Stage 7g.3:").expect("Stage 7g.3 marker");
    let end = source.find("// Stage 7g.3b:").expect("Stage 7g.3b marker");
    let slot = &source[start..end];

    let resolved = slot
        .find("super::introduction::resolve(")
        .expect("Stage 7g.3 must decide the introduction from the turn itself");
    let arms = slot
        .find("let raw_system_prompt = match onboarding {")
        .expect("the slot's arm match");
    let ordinary = slot
        .find("None => format!(\"{raw_system_prompt}{TURN_DIRECTIVE}\"),")
        .expect("the ordinary arm");
    let appended = slot
        .find("} + introduction_line.as_str();")
        .expect("the introduction is appended to the result of the arm match");
    assert!(
        resolved < arms && arms < ordinary && ordinary < appended,
        "the introduction must be appended to the whole match, not inside one arm"
    );
    assert_eq!(
        slot.matches("introduction_line").count(),
        2,
        "the introduction is built once and appended once"
    );
}
