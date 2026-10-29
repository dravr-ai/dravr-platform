// ABOUTME: Pins pierre-core's hand-written Debug impls that hide athlete content
// ABOUTME: A future derive(Debug) on a message write prints the athlete's words; this fails first
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Each test builds a value with a distinctive sentinel in every hidden field,
//! formats it with `{:?}` and `{:#?}`, and asserts no sentinel comes out.

use std::fmt::Debug;

use chrono::Utc;
use pierre_core::models::agents::AgentCategory;
use pierre_core::models::groups::GroupInviteKind;
use pierre_core::models::{
    AddMessageParams, AgentRuntimeContext, ConversationLastMessage, ConversationRecord,
    ConversationSummary, DeviceAuthorization, GroupInvite, MessageFeedbackRecord, MessagePage,
    MessageRecord, OAuth2AuthCode, OAuth2State, OAuthClientState, TenantId, ThreadTopic,
    UpsertMessageFeedbackParams,
};
use pierre_core::transport::TransportPolicy;
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
fn a_message_write_prints_the_size_of_its_content_never_the_content() {
    let content = "sentinel-message-7f3a my ferritin came back at 9";
    let blocks = r#"[{"kind":"chart","title":"sentinel-block-7f3a"}]"#;
    let params = AddMessageParams {
        tenant_id: TenantId::generate(),
        conversation_id: "conv-1",
        user_id: "user-1",
        role: "user",
        content,
        token_count: Some(12),
        finish_reason: Some("stop"),
        prompt_tokens: Some(40),
        model: Some("model-1"),
        content_blocks: Some(blocks),
        transport_policy: TransportPolicy::AnyTransport,
        channel_type: Some("web"),
        about_user_id: None,
    };
    let compact = format!("{params:?}");
    let pretty = format!("{params:#?}");
    for printed in [&compact, &pretty] {
        assert!(!printed.contains("sentinel-message-7f3a"), "{printed}");
        assert!(!printed.contains("sentinel-block-7f3a"), "{printed}");
    }
    assert!(
        compact.contains(&format!("content_len: {}", content.len())),
        "{compact}"
    );
    assert!(
        compact.contains(&format!("content_blocks_len: Some({})", blocks.len())),
        "{compact}"
    );
}

#[test]
fn a_feedback_write_prints_the_size_of_its_comment_never_the_comment() {
    let comment = "sentinel-feedback-7f3a you ignored my knee injury";
    let params = UpsertMessageFeedbackParams {
        tenant_id: TenantId::generate(),
        conversation_id: "conv-1",
        message_id: "msg-1",
        user_id: "user-1",
        rating: "down",
        comment: Some(comment),
    };
    let printed = printed_without(&params, &["sentinel-feedback-7f3a"]);
    assert!(
        printed.contains(&format!("comment_len: Some({})", comment.len())),
        "{printed}"
    );
    assert!(printed.contains("rating: \"down\""), "{printed}");
}

#[test]
fn a_group_invite_never_prints_its_code() {
    let invite = GroupInvite {
        id: Uuid::new_v4(),
        group_id: Uuid::new_v4(),
        tenant_id: "tenant-1".to_owned(),
        code: "sentinel-invite-7f3a".to_owned(),
        kind: GroupInviteKind::Member,
        created_by: Uuid::new_v4(),
        expires_at: None,
        max_uses: Some(3),
        use_count: 0,
        is_active: true,
        approves_signup: false,
        created_at: Utc::now(),
    };
    let printed = printed_without(&invite, &["sentinel-invite-7f3a"]);
    assert!(printed.contains("GroupInvite"), "{printed}");
    assert!(printed.contains("[REDACTED]"), "{printed}");
}

#[test]
fn a_device_authorization_never_prints_its_user_code() {
    let device = DeviceAuthorization {
        device_code_hash: "hash-1".to_owned(),
        user_code: "sentinel-usercode-7f3a".to_owned(),
        status: "pending".to_owned(),
        approved_by: None,
        created_at: 1,
        expires_at: 2,
    };
    let printed = printed_without(&device, &["sentinel-usercode-7f3a"]);
    assert!(printed.contains("DeviceAuthorization"), "{printed}");
}

#[test]
fn an_auth_code_never_prints_its_code_or_state() {
    let auth_code = OAuth2AuthCode {
        code: "sentinel-authcode-7f3a".to_owned(),
        client_id: "client-1".to_owned(),
        user_id: Uuid::new_v4(),
        tenant_id: "tenant-1".to_owned(),
        redirect_uri: "https://example.test/cb".to_owned(),
        scope: None,
        expires_at: Utc::now(),
        used: false,
        state: Some("sentinel-authstate-7f3a".to_owned()),
        code_challenge: None,
        code_challenge_method: None,
        resource: None,
    };
    let printed = printed_without(
        &auth_code,
        &["sentinel-authcode-7f3a", "sentinel-authstate-7f3a"],
    );
    assert!(printed.contains("OAuth2AuthCode"), "{printed}");
    assert!(printed.contains("state: Some(\"[REDACTED]\")"), "{printed}");
}

#[test]
fn an_oauth2_state_never_prints_its_state() {
    let state = OAuth2State {
        state: "sentinel-csrf-7f3a".to_owned(),
        client_id: "client-1".to_owned(),
        user_id: None,
        tenant_id: None,
        redirect_uri: "https://example.test/cb".to_owned(),
        scope: None,
        code_challenge: None,
        code_challenge_method: None,
        created_at: Utc::now(),
        expires_at: Utc::now(),
        used: false,
    };
    let printed = printed_without(&state, &["sentinel-csrf-7f3a"]);
    assert!(printed.contains("OAuth2State"), "{printed}");
}

#[test]
fn an_oauth_client_state_never_prints_its_state_or_pkce_verifier() {
    let state = OAuthClientState {
        state: "sentinel-clientstate-7f3a".to_owned(),
        provider: "strava".to_owned(),
        user_id: None,
        tenant_id: None,
        redirect_uri: "https://example.test/cb".to_owned(),
        scope: None,
        pkce_code_verifier: Some("sentinel-verifier-7f3a".to_owned()),
        oauth_app_client_id: None,
        created_at: Utc::now(),
        expires_at: Utc::now(),
        used: false,
    };
    let printed = printed_without(
        &state,
        &["sentinel-clientstate-7f3a", "sentinel-verifier-7f3a"],
    );
    assert!(printed.contains("OAuthClientState"), "{printed}");
}

fn message_record(content: &str, blocks: &str) -> MessageRecord {
    MessageRecord {
        id: "msg-1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        role: "user".to_owned(),
        content: content.to_owned(),
        token_count: Some(12),
        prompt_tokens: Some(40),
        model: Some("model-1".to_owned()),
        finish_reason: Some("stop".to_owned()),
        content_blocks: Some(blocks.to_owned()),
        created_at: "2026-10-24T00:00:00Z".to_owned(),
        transport_policy: TransportPolicy::AnyTransport,
        channel_type: Some("web".to_owned()),
        about_user_id: None,
    }
}

#[test]
fn a_message_record_prints_the_size_of_its_content_never_the_content() {
    let content = "sentinel-record-7f3a my ferritin came back at 9";
    let blocks = r#"[{"kind":"chart","title":"sentinel-recblock-7f3a"}]"#;
    let record = message_record(content, blocks);
    let printed = printed_without(&record, &["sentinel-record-7f3a", "sentinel-recblock-7f3a"]);
    assert!(printed.contains("MessageRecord"), "{printed}");
    assert!(
        printed.contains(&format!("content_len: {}", content.len())),
        "{printed}"
    );
    assert!(
        printed.contains(&format!("content_blocks_len: Some({})", blocks.len())),
        "{printed}"
    );
    assert!(printed.contains("id: \"msg-1\""), "{printed}");
}

#[test]
fn a_message_page_prints_its_rows_without_their_content() {
    let page = MessagePage {
        messages: vec![message_record("sentinel-page-7f3a", "[]")],
        has_more: true,
    };
    let printed = printed_without(&page, &["sentinel-page-7f3a"]);
    assert!(printed.contains("MessagePage"), "{printed}");
    assert!(printed.contains("MessageRecord"), "{printed}");
}

#[test]
fn a_feedback_record_prints_the_size_of_its_comment_never_the_comment() {
    let comment = "sentinel-fbrecord-7f3a you ignored my knee injury";
    let record = MessageFeedbackRecord {
        id: "fb-1".to_owned(),
        message_id: "msg-1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        user_id: "user-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        rating: "down".to_owned(),
        comment: Some(comment.to_owned()),
        created_at: "2026-10-24T00:00:00Z".to_owned(),
        updated_at: "2026-10-24T00:00:00Z".to_owned(),
    };
    let printed = printed_without(&record, &["sentinel-fbrecord-7f3a"]);
    assert!(printed.contains("MessageFeedbackRecord"), "{printed}");
    assert!(
        printed.contains(&format!("comment_len: Some({})", comment.len())),
        "{printed}"
    );
    assert!(printed.contains("rating: \"down\""), "{printed}");
}

#[test]
fn a_last_message_preview_prints_the_size_of_its_head_never_the_head() {
    let head = "sentinel-head-7f3a I think I tore something";
    let last = ConversationLastMessage {
        content_head: head.to_owned(),
        role: "user".to_owned(),
        created_at: "2026-10-24T00:00:00Z".to_owned(),
        transport_policy: TransportPolicy::AnyTransport,
    };
    let printed = printed_without(&last, &["sentinel-head-7f3a"]);
    assert!(printed.contains("ConversationLastMessage"), "{printed}");
    assert!(
        printed.contains(&format!("content_head_len: {}", head.len())),
        "{printed}"
    );
}

#[test]
fn a_thread_topic_prints_the_size_of_its_first_words_never_the_words() {
    let words = "sentinel-topic-7f3a my marathon in two weeks";
    let topic = ThreadTopic {
        divider_id: Some("msg-2".to_owned()),
        started_at: "2026-10-24T00:00:00Z".to_owned(),
        first_words: Some(words.to_owned()),
    };
    let printed = printed_without(&topic, &["sentinel-topic-7f3a"]);
    assert!(printed.contains("ThreadTopic"), "{printed}");
    assert!(
        printed.contains(&format!("first_words_len: Some({})", words.len())),
        "{printed}"
    );
}

#[test]
fn a_conversation_record_prints_the_size_of_its_title_never_the_title() {
    let title = "sentinel-convtitle-7f3a my eating disorder";
    let record = ConversationRecord {
        id: "conv-1".to_owned(),
        user_id: "user-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        title: title.to_owned(),
        model: "model-1".to_owned(),
        agent_id: None,
        session_id: None,
        total_tokens: 7,
        created_at: "2026-10-24T00:00:00Z".to_owned(),
        updated_at: "2026-10-24T00:00:00Z".to_owned(),
        group_id: None,
        channel_type: "web".to_owned(),
        onboarding_state: None,
        archived_at: None,
        personal: true,
        subject_user_id: None,
        subject_group_id: None,
    };
    let printed = printed_without(&record, &["sentinel-convtitle-7f3a"]);
    assert!(printed.contains("ConversationRecord"), "{printed}");
    assert!(
        printed.contains(&format!("title_len: {}", title.len())),
        "{printed}"
    );
}

#[test]
fn a_conversation_summary_prints_neither_its_title_nor_its_preview() {
    let title = "sentinel-sumtitle-7f3a my eating disorder";
    let summary = ConversationSummary {
        id: "conv-1".to_owned(),
        title: title.to_owned(),
        model: "model-1".to_owned(),
        message_count: 2,
        total_tokens: 7,
        agent_id: None,
        agent_handle: None,
        agent_title: None,
        group_id: None,
        group_name: None,
        channel_type: Some("web".to_owned()),
        archived_at: None,
        personal: false,
        subject_user_id: None,
        subject_group_id: None,
        subject_display_name: None,
        last_message: Some(ConversationLastMessage {
            content_head: "sentinel-sumhead-7f3a".to_owned(),
            role: "assistant".to_owned(),
            created_at: "2026-10-24T00:00:00Z".to_owned(),
            transport_policy: TransportPolicy::AnyTransport,
        }),
        unread_count: 1,
        created_at: "2026-10-24T00:00:00Z".to_owned(),
        updated_at: "2026-10-24T00:00:00Z".to_owned(),
    };
    let printed = printed_without(
        &summary,
        &["sentinel-sumtitle-7f3a", "sentinel-sumhead-7f3a"],
    );
    assert!(printed.contains("ConversationSummary"), "{printed}");
    assert!(
        printed.contains(&format!("title_len: {}", title.len())),
        "{printed}"
    );
}

#[test]
fn an_agent_context_prints_the_size_of_its_prompt_never_the_prompt() {
    let prompt = "sentinel-prompt-7f3a you coach my daughter";
    let startup = "sentinel-startup-7f3a ask about her period";
    let context = AgentRuntimeContext {
        slug: "custom-1".to_owned(),
        title: "Custom".to_owned(),
        source: "custom".to_owned(),
        system_prompt: prompt.to_owned(),
        startup_query: Some(startup.to_owned()),
        data_requirements: None,
        visuals: vec!["chart".to_owned()],
        max_tool_iterations: Some(4),
        temperature: None,
        category: AgentCategory::Training,
    };
    let printed = printed_without(&context, &["sentinel-prompt-7f3a", "sentinel-startup-7f3a"]);
    assert!(printed.contains("AgentRuntimeContext"), "{printed}");
    assert!(
        printed.contains(&format!("system_prompt_len: {}", prompt.len())),
        "{printed}"
    );
    assert!(
        printed.contains(&format!("startup_query_len: Some({})", startup.len())),
        "{printed}"
    );
}
