// ABOUTME: Pins the chat pipeline's hand-written Debug impls that hide links and athlete text
// ABOUTME: Reply blocks, reconnect prompts and turn requests must never print a signed URL or a message
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Each test builds a value with a distinctive sentinel in every hidden field,
//! formats it with `{:?}` and `{:#?}`, and asserts no sentinel comes out.

use std::fmt::Debug;

use pierre_chat_pipeline::stages::viz_blocks::VizExtraction;
use pierre_chat_pipeline::turn::{AmbientContext, UserMessageResult};
use pierre_chat_pipeline::{
    CommandPersistence, InputSource, PipelineHooks, QuotaState, ReconnectPrompt, ReplyBlock,
    SlashRequest, SurfaceId, TurnInput, TurnOrigin, TurnRequest, VerdictChip,
};
use pierre_core::models::{ConversationRecord, ConversationTurnId, MessageRecord, TenantId};
use pierre_core::transport::{Transport, TransportPolicy};
use uuid::Uuid;

/// Formats `value` both ways, asserts none of `secrets` appears in either,
/// and returns the compact form for further checks.
fn printed_without(value: &impl Debug, secrets: &[&str]) -> String {
    let compact = format!("{value:?}");
    let pretty = format!("{value:#?}");
    for secret in secrets {
        assert!(!compact.contains(secret), "{secret} leaked: {compact}");
        assert!(!pretty.contains(secret), "{secret} leaked: {pretty}");
    }
    compact
}

#[test]
fn a_scene_image_block_never_prints_its_signed_url() {
    let block = ReplyBlock::SceneImage {
        url: "https://charts.dravr.ai/c/abc?sig=sentinel-signature-7f3a".to_owned(),
        mime_type: "image/png".to_owned(),
        caption: Some("Weekly load".to_owned()),
    };
    let printed = printed_without(&block, &["sentinel-signature-7f3a"]);
    assert!(printed.contains("url: \"[REDACTED]\""), "{printed}");
    assert!(printed.contains("Weekly load"), "{printed}");
}

#[test]
fn a_reconnect_block_never_prints_its_link_or_the_text_quoting_it() {
    let block = ReplyBlock::Reconnect {
        provider: "whoop".to_owned(),
        display_name: "WHOOP".to_owned(),
        url: "https://api.dravr.ai/r/sentinel-link-7f3a".to_owned(),
        text: "Reconnect WHOOP: https://api.dravr.ai/r/sentinel-text-7f3a".to_owned(),
    };
    let printed = printed_without(&block, &["sentinel-link-7f3a", "sentinel-text-7f3a"]);
    assert_eq!(printed.matches("[REDACTED]").count(), 2, "{printed}");
    assert!(printed.contains("WHOOP"), "{printed}");
}

#[test]
fn a_reconnect_prompt_never_prints_its_link_or_the_text_quoting_it() {
    let prompt = ReconnectPrompt {
        provider: "strava".to_owned(),
        display_name: "Strava".to_owned(),
        url: "https://api.dravr.ai/r/sentinel-link-7f3a".to_owned(),
        text: "Reconnect Strava: https://api.dravr.ai/r/sentinel-text-7f3a".to_owned(),
    };
    let printed = printed_without(&prompt, &["sentinel-link-7f3a", "sentinel-text-7f3a"]);
    assert_eq!(printed.matches("[REDACTED]").count(), 2, "{printed}");
    assert!(printed.contains("Strava"), "{printed}");
}

#[test]
fn a_turn_request_prints_the_size_of_the_message_never_its_text_or_sender() {
    let content = "sentinel-message-7f3a my resting heart rate is 41";
    let request = TurnRequest {
        conversation_id: "conv-1".to_owned(),
        user_id: Uuid::new_v4(),
        conversation_tenant_id: TenantId::generate(),
        tool_tenant_id: TenantId::generate(),
        content: content.to_owned(),
        origin: TurnOrigin::Athlete,
        input_source: InputSource::Typed,
        turn_id: ConversationTurnId::new(),
        ambient_context: Some(AmbientContext {
            text: "sentinel-room-7f3a said hello".to_owned(),
            transport_policy: TransportPolicy::AnyTransport,
        }),
        quote: None,
        channel_type: "telegram",
        is_direct_message: false,
        ambient_group_fallback: true,
        command_persistence: CommandPersistence::Always,
        sender_id: Some("+15550007f3a"),
        transport: Transport::WebApp,
        hooks: PipelineHooks::none(),
    };
    let printed = printed_without(
        &request,
        &[
            "sentinel-message-7f3a",
            "sentinel-room-7f3a",
            "+15550007f3a",
        ],
    );
    assert!(
        printed.contains(&format!("content_len: {}", content.len())),
        "{printed}"
    );
    assert!(
        printed.contains("sender_id: Some(\"[REDACTED]\")"),
        "{printed}"
    );
}

#[test]
fn a_persisted_user_message_prints_its_size_never_its_text() {
    let content = "sentinel-message-7f3a my knee hurts on the stairs";
    let result = UserMessageResult {
        message: MessageRecord {
            id: "msg-1".to_owned(),
            conversation_id: "conv-1".to_owned(),
            role: "user".to_owned(),
            content: content.to_owned(),
            token_count: None,
            prompt_tokens: None,
            model: None,
            finish_reason: None,
            content_blocks: None,
            created_at: "2026-10-23T21:00:00Z".to_owned(),
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
        },
        conversation: ConversationRecord {
            id: "conv-1".to_owned(),
            user_id: "user-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            title: "sentinel-title-7f3a".to_owned(),
            model: "mock-model".to_owned(),
            agent_id: None,
            session_id: None,
            total_tokens: 0,
            created_at: "2026-10-23T21:00:00Z".to_owned(),
            updated_at: "2026-10-23T21:00:00Z".to_owned(),
            group_id: None,
            channel_type: "web".to_owned(),
            onboarding_state: None,
            archived_at: None,
            personal: true,
            subject_user_id: None,
            subject_group_id: None,
        },
    };
    let printed = printed_without(&result, &["sentinel-message-7f3a", "sentinel-title-7f3a"]);
    assert!(
        printed.contains(&format!("message_len: {}", content.len())),
        "{printed}"
    );
    assert!(
        printed.contains("msg-1") && printed.contains("conv-1"),
        "{printed}"
    );
}

#[test]
fn a_slash_request_prints_the_size_of_the_command_never_its_text_or_sender() {
    let text = "/group join sentinel-invite-7f3a";
    let request = SlashRequest {
        user_id: Uuid::new_v4(),
        tenant_id: TenantId::generate(),
        conversation_id: "conv-1",
        conversation_tenant_id: TenantId::generate(),
        channel_type: "whatsapp",
        locale: "en",
        is_direct_message: true,
        ambient_group_fallback: false,
        persistence: CommandPersistence::Always,
        sender_id: Some("+15550007f3a"),
        text,
        input_source: InputSource::Typed,
        origin: TurnOrigin::Athlete,
        transport: Transport::WebApp,
        surface: SurfaceId::WhatsApp,
        renders_buttons: false,
    };
    let printed = printed_without(&request, &["sentinel-invite-7f3a", "+15550007f3a"]);
    assert!(
        printed.contains(&format!("text_len: {}", text.len())),
        "the command's size prints in place of its text: {printed}"
    );
    assert!(
        printed.contains("sender_id: Some(\"[REDACTED]\")"),
        "the sender prints as a redaction: {printed}"
    );
}

#[test]
fn a_prose_block_prints_the_size_of_the_reply_never_its_text() {
    let text = "sentinel-prose-7f3a your threshold rose to 287 W";
    let block = ReplyBlock::Prose {
        text: text.to_owned(),
    };
    let printed = printed_without(&block, &["sentinel-prose-7f3a"]);
    assert!(printed.starts_with("Prose"), "{printed}");
    assert!(
        printed.contains(&format!("text_len: {}", text.len())),
        "{printed}"
    );
}

#[test]
fn an_activity_list_block_prints_its_size_never_the_activities() {
    let text = "sentinel-activities-7f3a Morning ride, 42 km";
    let block = ReplyBlock::ActivityList {
        text: text.to_owned(),
    };
    let printed = printed_without(&block, &["sentinel-activities-7f3a"]);
    assert!(printed.starts_with("ActivityList"), "{printed}");
    assert!(
        printed.contains(&format!("text_len: {}", text.len())),
        "{printed}"
    );
}

#[test]
fn a_turn_input_prints_the_size_of_the_message_never_its_text_or_the_room() {
    let content = "sentinel-message-7f3a my resting heart rate is 41";
    let input = TurnInput {
        conversation_id: "conv-1".to_owned(),
        user_id: "user-1".to_owned(),
        conversation_tenant_id: TenantId::generate(),
        tool_tenant_id: TenantId::generate(),
        is_direct_message: true,
        content: content.to_owned(),
        origin: TurnOrigin::Athlete,
        input_source: InputSource::Typed,
        turn_id: ConversationTurnId::new(),
        ambient_context: Some(AmbientContext {
            text: "sentinel-room-7f3a said hello".to_owned(),
            transport_policy: TransportPolicy::AnyTransport,
        }),
        quote: None,
        quota: QuotaState::default(),
        mentioned_agent: None,
        steps_after: None,
    };
    let printed = printed_without(&input, &["sentinel-message-7f3a", "sentinel-room-7f3a"]);
    assert!(printed.starts_with("TurnInput"), "{printed}");
    assert!(
        printed.contains(&format!("content_len: {}", content.len())),
        "{printed}"
    );
    assert!(printed.contains("has_ambient_context: true"), "{printed}");
    assert!(printed.contains("conv-1"), "{printed}");
}

#[test]
fn a_viz_extraction_prints_the_size_of_the_reply_never_its_text() {
    let text = "sentinel-viz-7f3a here is your week [[viz:0]]";
    let extraction = VizExtraction {
        text: text.to_owned(),
        blocks: vec![serde_json::json!({ "kind": "table" })],
        refusals: vec!["schema mismatch".to_owned()],
    };
    let printed = printed_without(&extraction, &["sentinel-viz-7f3a"]);
    assert!(printed.starts_with("VizExtraction"), "{printed}");
    assert!(
        printed.contains(&format!("text_len: {}", text.len())),
        "{printed}"
    );
    assert!(printed.contains("schema mismatch"), "{printed}");
}

#[cfg(feature = "tools-verification")]
#[test]
fn a_claim_verification_outcome_prints_sizes_never_the_reply_or_its_claims() {
    use pierre_chat_pipeline::stages::verification::ClaimVerificationOutcome;
    use pierre_chat_pipeline::VerdictChip;
    use pierre_evals::{ClaimSource, ExtractedClaim, VerdictOutcome};
    use pierre_memory::claims::ClaimCategory;

    let content = "sentinel-reply-7f3a your VO2max is 71";
    let outcome = ClaimVerificationOutcome {
        content: content.to_owned(),
        pending_verdicts: vec![(
            ExtractedClaim {
                text: "sentinel-claim-7f3a your VO2max is 71".to_owned(),
                category: ClaimCategory::Physiological,
                source: ClaimSource::Reply,
            },
            VerdictOutcome::judge_error(),
        )],
        chips: vec![VerdictChip {
            claim: "sentinel-chip-7f3a your VO2max is 71".to_owned(),
            contradicted: true,
        }],
    };
    let printed = printed_without(
        &outcome,
        &[
            "sentinel-reply-7f3a",
            "sentinel-claim-7f3a",
            "sentinel-chip-7f3a",
        ],
    );
    assert!(printed.starts_with("ClaimVerificationOutcome"), "{printed}");
    assert!(
        printed.contains(&format!("content_len: {}", content.len())),
        "{printed}"
    );
    assert!(printed.contains("pending_verdicts_len: 1"), "{printed}");
    assert!(printed.contains("chips_len: 1"), "{printed}");
}

#[test]
fn a_verdicts_block_prints_the_size_of_each_claim_never_the_claim() {
    let block = ReplyBlock::Verdicts {
        chips: vec![VerdictChip {
            claim: "sentinel-claim-7f3a".to_owned(),
            contradicted: true,
        }],
    };

    let printed = printed_without(&block, &["sentinel-claim-7f3a"]);
    assert!(
        printed.contains("claim_len"),
        "the chip states the claim's size: {printed}"
    );
}

#[test]
fn an_ambient_context_prints_the_size_of_the_room_never_its_transcript() {
    let ambient = AmbientContext {
        text: "sentinel-room-7f3a".to_owned(),
        transport_policy: TransportPolicy::default(),
    };

    let printed = printed_without(&ambient, &["sentinel-room-7f3a"]);
    assert!(
        printed.contains("text_len"),
        "the context states the transcript's size: {printed}"
    );
}
