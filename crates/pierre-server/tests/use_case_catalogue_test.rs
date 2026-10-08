// ABOUTME: carnet#828 — the pinned use-case catalogue loads whole, its words exist in every locale, and its commands are real
// ABOUTME: Each command starter is tapped through the chat route and must reach a handler, never the unknown-command reply

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The catalogue's structure is compiled in from the pinned dravr-contremaitre
//! and its words ride the strings, so the two can drift apart only here: a
//! starter whose label or prompt the platform cannot read, or whose command no
//! handler answers, would reach an athlete's welcome. A catalogue that fails
//! to load reads as empty at runtime (welcomes fall back to the agents'
//! examples), so this is also the test that keeps that from passing CI.

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;

use common::{
    create_test_server_resources_with_chat_provider, create_test_user_with_plan,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use helpers::recording_llm::RecordingProvider;
use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_UNKNOWN_COMMAND};
use pierre_contremaitre::use_case_catalogue::{UseCaseCatalogue, UseCaseRun};
use pierre_core::models::{COMMAND_FINISH_REASON, SUPPORTED_LOCALES};
use pierre_mcp_server::routes::chat::{ChatRoutes, ConversationResponse, TurnResponse};
use pierre_services::locale::resolve_user_locale;

/// `WhatsApp` caps a reply button's title at 20 characters.
const MAX_LABEL_CHARS: usize = 20;

#[test]
fn the_pinned_catalogue_loads_whole() {
    // The load is all or nothing, so any entry at all means every entry
    // parsed. Which starters the catalogue holds is contremaitre's to change
    // through a bump; the tests below judge each one it holds.
    assert!(
        !UseCaseCatalogue::pinned().entries().is_empty(),
        "the pinned catalogue failed to parse and reads as empty"
    );
}

#[test]
fn every_starter_has_its_words_in_every_locale() {
    let strings = MessagingStringsRegistry::new();
    for entry in UseCaseCatalogue::pinned().entries() {
        for locale in SUPPORTED_LOCALES {
            let label = strings.get(&entry.label_key(), locale);
            assert!(
                !label.trim().is_empty(),
                "{locale}: {} has no label",
                entry.id
            );
            assert!(
                label.chars().count() <= MAX_LABEL_CHARS,
                "{locale}: {}'s label {label:?} is over {MAX_LABEL_CHARS} characters",
                entry.id
            );
            let prompt = strings.get(&entry.prompt_key(), locale);
            match entry.run {
                UseCaseRun::Prompt => {
                    assert!(
                        !prompt.trim().is_empty(),
                        "{locale}: {} has no prompt",
                        entry.id
                    );
                }
                UseCaseRun::Command(_) => {
                    assert!(prompt.is_empty(), "{locale}: {} runs a command", entry.id);
                }
            }
        }
    }
}

#[tokio::test]
async fn every_command_starter_reaches_a_handler() {
    let provider = Arc::new(RecordingProvider::answering("Noted."));
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let (user_id, user, _) = create_test_user_with_plan(
        &resources.agent.database,
        "use-case-commands@test.com",
        "professional",
    )
    .await
    .unwrap();
    let auth = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), user_id).await;
    let unknown = resources
        .mcp
        .messaging_strings_registry
        .get(KEY_UNKNOWN_COMMAND, &locale);

    let commands: Vec<(&str, &str)> = UseCaseCatalogue::pinned()
        .entries()
        .iter()
        .filter_map(|e| match &e.run {
            UseCaseRun::Command(command) => Some((e.id.as_str(), command.as_str())),
            UseCaseRun::Prompt => None,
        })
        .collect();
    assert!(!commands.is_empty(), "the catalogue runs commands");

    for (id, command) in commands {
        // A fresh thread each: a command that opens a guided walk owns its thread.
        let created = AxumTestRequest::post("/api/chat/conversations")
            .header("authorization", &auth)
            .json(&json!({}))
            .send(ChatRoutes::routes(Arc::clone(&resources)))
            .await;
        assert_eq!(created.status_code(), StatusCode::CREATED);
        let conv = created.json::<ConversationResponse>().id;

        let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv}/messages"))
            .header("authorization", &auth)
            .json(&json!({ "content": format!("uc:0:{id}") }))
            .send(ChatRoutes::routes(Arc::clone(&resources)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{id}");
        let turn: TurnResponse = resp.json();
        assert_eq!(turn.user_message.content, command, "{id} runs {command}");
        assert_eq!(
            turn.assistant.finish_reason.as_deref(),
            Some(COMMAND_FINISH_REASON),
            "{id}: {command} is answered as a command"
        );
        assert_ne!(
            turn.assistant.message.content, unknown,
            "{id}: no handler answers {command}"
        );
    }
}
