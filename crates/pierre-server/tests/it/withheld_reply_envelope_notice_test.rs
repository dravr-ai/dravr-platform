// ABOUTME: A reply the platform withholds still carries the envelope notice for the writes its turn made
// ABOUTME: Drives a real turn whose model saves an edited family day, then answers as itself

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Founder decision D3: every edit goes through and earns its warning. A turn
//! whose writes reported envelope findings, but whose reply the platform
//! replaced, used to lose the notice with the text (carnet#1495). The withheld
//! identity break is driven here; a Guardian block, a reconnect prompt, a
//! canary hit and an emptied narration scrub make the same
//! `notice_only_blocks` call in the model's place.
//!
//! The turn runs through the chat pipeline against a model that calls
//! `save_training_plan` through the text tool loop, then breaks character.
//! The assertions are on the assistant row the database holds, which is what
//! a reload and every surface read.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use pierre_chat_pipeline::{
    CommandPersistence, InputSource, PipelineHooks, SurfaceId, SurfaceProfile, SurfaceRequest,
    TurnOrigin, TurnRequest,
};
use pierre_contremaitre::messaging_strings::{
    envelope_purpose_key, KEY_ENVELOPE_RELABELLED, KEY_REPLY_WITHHELD,
};
use pierre_core::models::periodization::WorkoutPurpose;
use pierre_core::models::{
    ConversationTurnId, ENVELOPE_NOTICE_BLOCK_TYPE, WITHHELD_REPLY_FINISH_REASON,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use serde_json::{json, Value};

use crate::common::{create_test_server_resources_with_chat_provider, create_test_user_with_plan};
use crate::helpers::recording_llm::RecordingProvider;
use crate::training_plan_tools_test::tabata_style_plan;

/// The 2026-07-22 Telegram break, verbatim: the model answering as itself.
const LEAKED_REPLY: &str = "I'm GitHub Copilot CLI, a terminal-based coding assistant.";

const LOCALE: &str = "fr";

#[tokio::test]
async fn a_withheld_reply_still_carries_the_turns_envelope_notice() {
    let provider = Arc::new(RecordingProvider::answering(LEAKED_REPLY));
    let call = json!({ "name": "save_training_plan", "arguments": tabata_style_plan() });
    provider.answer_next_with(&format!("<tool_call>{call}</tool_call>"));
    let resources = create_test_server_resources_with_chat_provider(provider.clone())
        .await
        .unwrap();
    let (user_id, _user, tenant_id) = create_test_user_with_plan(
        &resources.agent.database,
        "withheld-envelope-notice@example.com",
        "professional",
    )
    .await
    .unwrap();
    let conversation_id = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant_id,
            "Plan",
            "mock-model",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    let ctx = resources.chat_pipeline_context();

    pierre_chat_pipeline::execute(
        &ctx,
        TurnRequest {
            origin: TurnOrigin::Athlete,
            input_source: InputSource::Typed,
            conversation_id: conversation_id.clone(),
            user_id,
            conversation_tenant_id: tenant_id,
            tool_tenant_id: tenant_id,
            content: "Mets un Tabata mardi à la place du tempo.".to_owned(),
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            quote: None,
            channel_type: "web",
            transport: Transport::WebApp,
            is_direct_message: true,
            ambient_group_fallback: false,
            command_persistence: CommandPersistence::Always,
            sender_id: None,
            hooks: PipelineHooks::none(),
            grant: OAuthScope::self_grant(),
        },
        &SurfaceProfile::resolve(&SurfaceRequest {
            surface: SurfaceId::Web,
            locale: LOCALE.to_owned(),
            transport: None,
            prose_contract: None,
        }),
    )
    .await
    .expect("the turn is served");

    let row = resources
        .common
        .repos
        .chat
        .get_messages(&conversation_id, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .into_iter()
        .rfind(|row| row.role == "assistant")
        .expect("the turn persisted a reply");
    assert_eq!(
        row.finish_reason.as_deref(),
        Some(WITHHELD_REPLY_FINISH_REASON),
        "the model broke character, so the platform withheld its reply: {}",
        row.content
    );
    assert_eq!(
        row.content,
        ctx.messaging_strings_registry
            .get(KEY_REPLY_WITHHELD, LOCALE),
        "the athlete reads the platform's apology in the reply's place"
    );
    let blocks: Vec<Value> = serde_json::from_str(
        row.content_blocks
            .as_deref()
            .expect("bug: the withheld reply dropped the envelope notice of the turn's save"),
    )
    .unwrap();
    let notices: Vec<&Value> = blocks
        .iter()
        .filter(|block| block["type"] == ENVELOPE_NOTICE_BLOCK_TYPE)
        .collect();
    assert_eq!(
        notices.len(),
        1,
        "bug: the turn's save earns one notice on the withheld reply: {blocks:#?}"
    );
    let lines: Vec<&str> = notices[0]["lines"]
        .as_array()
        .map(|lines| lines.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    assert_eq!(
        lines.len(),
        1,
        "bug: only the edited day warns, so the notice holds one line: {blocks:#?}"
    );
    assert!(
        lines[0].contains("2026-07-14"),
        "bug: the notice names the edited day: {}",
        lines[0]
    );
    let strings = &ctx.messaging_strings_registry;
    let threshold = strings.get(
        &envelope_purpose_key(WorkoutPurpose::Threshold.as_str()),
        LOCALE,
    );
    assert!(
        !threshold.is_empty(),
        "bug: the {LOCALE} strings name the threshold purpose, or the relabel check below proves nothing"
    );
    assert!(
        lines[0].contains(&strings.render(KEY_ENVELOPE_RELABELLED, LOCALE, &[&threshold])),
        "bug: the notice carries the day's relabel to threshold work: {}",
        lines[0]
    );
}
