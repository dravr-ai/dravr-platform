// ABOUTME: Pins pierre-services' hand-written Debug impls that hide tokens, codes and athlete text
// ABOUTME: A future derive(Debug) on an invitation, link token, signup code or extraction request fails here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Each test builds a value with a distinctive sentinel in every hidden field,
//! formats it with `{:?}` and `{:#?}`, and asserts no sentinel comes out.

use std::fmt::Debug;

use chrono::Utc;
use pierre_core::models::{Pillar, SignupCode, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_memory::{FactKind, FactSource};
use pierre_services::invitation::IssuedInvitation;
use pierre_services::link_token::generate_link_token;
use pierre_services::memory_extraction::ExtractionRequest;
use pierre_services::signup_codes::MintedSignupCode;
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
fn an_issued_invitation_never_prints_its_token() {
    let invitation = IssuedInvitation {
        email: "invitee@example.com".to_owned(),
        token: "sentinel-selector.sentinel-verifier-7f3a".to_owned(),
        expires_at: Utc::now(),
    };
    let printed = printed_without(
        &invitation,
        &["sentinel-selector", "sentinel-verifier-7f3a"],
    );
    assert!(printed.contains("token: \"[REDACTED]\""), "{printed}");
    assert!(printed.contains("invitee@example.com"), "{printed}");
}

#[test]
fn a_generated_link_token_never_prints_its_verifier() {
    let generated = generate_link_token();
    let (_, verifier) = generated.token.split_once('.').unwrap();
    let printed = printed_without(&generated, &[&generated.token, verifier]);
    assert!(printed.contains("token: \"[REDACTED]\""), "{printed}");
    assert!(printed.contains(&generated.selector), "{printed}");
}

#[test]
fn a_minted_signup_code_never_prints_the_code() {
    let minted = MintedSignupCode {
        code: "SENTINEL-CODE-7F3A-9QX2".to_owned(),
        record: SignupCode {
            id: Uuid::new_v4(),
            code_hint: "9QX2".to_owned(),
            note: Some("spring cohort".to_owned()),
            created_by: None,
            expires_at: None,
            max_uses: Some(10),
            use_count: 0,
            revoked_at: None,
            created_at: Utc::now(),
        },
    };
    let printed = printed_without(&minted, &["SENTINEL-CODE-7F3A"]);
    assert!(printed.contains("code: \"[REDACTED]\""), "{printed}");
    assert!(printed.contains("spring cohort"), "{printed}");
}

#[test]
fn an_extraction_request_prints_the_size_of_the_turn_never_its_text() {
    let user_message = "sentinel-question-7f3a I was diagnosed with asthma";
    let assistant_reply = "sentinel-reply-7f3a noted, we will adapt the plan";
    let request = ExtractionRequest {
        tenant_id: TenantId::generate(),
        user_id: "user-1",
        agent_id: Some("agent-1"),
        user_message,
        assistant_reply,
        source_msg_id: Some("msg-1"),
        pillar: Some(Pillar::SleepAndRecovery),
        source: FactSource::Conversation,
        force_kind: Some(FactKind::Goal),
        plan_was_saved: false,
        transport_policy: TransportPolicy::AnyTransport,
        recorder: None,
    };
    let printed = printed_without(&request, &["sentinel-question-7f3a", "sentinel-reply-7f3a"]);
    assert!(
        printed.contains(&format!("user_message_len: {}", user_message.len())),
        "{printed}"
    );
    assert!(
        printed.contains(&format!("assistant_reply_len: {}", assistant_reply.len())),
        "{printed}"
    );
}

#[cfg(feature = "health-sync")]
#[test]
fn delegated_wellness_never_prints_the_coach_credentials() {
    use dravr_enforme::models::connection::{CredentialKind, ProviderCredentials};
    use pierre_services::delegated_wellness::{DelegatedWellness, PrivateWellness};

    let wellness = DelegatedWellness {
        credentials: ProviderCredentials {
            access_token: "sentinel-access-7f3a".to_owned(),
            refresh_token: Some("sentinel-refresh-7f3a".to_owned()),
            expires_at: None,
            scopes: vec!["wellness:read".to_owned()],
            user_id: "user-1".to_owned(),
            provider: "intervals_icu".to_owned(),
            provider_user_id: Some("i12345".to_owned()),
            kind: CredentialKind::default(),
            request_gate: None,
        },
        private: PrivateWellness::default(),
    };
    let printed = printed_without(
        &wellness,
        &["sentinel-access-7f3a", "sentinel-refresh-7f3a"],
    );
    assert!(printed.contains("credentials: \"[REDACTED]\""), "{printed}");
}

#[cfg(feature = "client-messaging")]
#[test]
fn a_status_placeholder_request_never_prints_the_channel_credentials() {
    use dravr_canot::models::{ChannelConfig, ChannelType};
    use pierre_services::messaging_status_bridge::OpenStatusParams;

    let config = ChannelConfig {
        id: "cfg-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        channel_type: ChannelType::Telegram,
        api_key: Some("sentinel-api-key-7f3a".to_owned()),
        api_secret: Some("sentinel-api-secret-7f3a".to_owned()),
        webhook_secret: Some("sentinel-webhook-7f3a".to_owned()),
        verify_token: Some("sentinel-verify-7f3a".to_owned()),
        account_id: Some("account-1".to_owned()),
        phone_number: Some("+15550007f3a".to_owned()),
        bot_token: Some("sentinel-bot-token-7f3a".to_owned()),
        is_active: true,
    };
    let params = OpenStatusParams {
        channel_type: ChannelType::Telegram,
        channel_config: &config,
        conversation_id: "chat-1",
        thread_id: Some("thread-1"),
        placeholder_text: "thinking…",
        api_base_override: None,
    };
    let printed = printed_without(
        &params,
        &[
            "sentinel-api-key-7f3a",
            "sentinel-api-secret-7f3a",
            "sentinel-webhook-7f3a",
            "sentinel-verify-7f3a",
            "+15550007f3a",
            "sentinel-bot-token-7f3a",
        ],
    );
    assert!(
        printed.contains("channel_config_id: \"cfg-1\""),
        "{printed}"
    );
}
