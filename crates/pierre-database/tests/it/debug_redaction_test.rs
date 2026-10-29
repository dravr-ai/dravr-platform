// ABOUTME: Pins pierre-database's hand-written Debug impls that hide keys, credentials and content
// ABOUTME: A future derive(Debug) on a messaging param, seed model or database handle fails here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Each test builds a value with a distinctive sentinel in every hidden field,
//! formats it with `{:?}` and `{:#?}`, and asserts no sentinel comes out.

use std::fmt::Debug;

use chrono::Utc;
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::messaging::{
    CreateChannelLinkParams, CreateLinkStateParams, CreateSessionParams, EnqueueOutboundParams,
    InsertMessageParams, UpsertChannelConfigParams,
};
use pierre_database::repositories::{
    InsertAgentFollowupParams, InsertAgentNoteParams, InsertClaimVerdictParams,
    InsertCompactionBlockParams, SetVerdictDispositionParams, UpsertUserFactParams,
};
use pierre_database::seed_models::{SeedApiKey, SeedDemoUser};
use pierre_memory::claims::{
    ClaimCategory, ClaimStatus, DispositionReason, EvidenceStrength, VerdictDisposition,
    VerdictLayer,
};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_test_support::db::create_test_db_with_key;
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
fn a_channel_config_upsert_never_prints_its_credentials() {
    let params = UpsertChannelConfigParams {
        id: "cfg-1",
        tenant_id: TenantId::generate(),
        channel_type: "telegram",
        api_key: Some("sentinel-api-key-7f3a"),
        api_secret: Some("sentinel-api-secret-7f3a"),
        webhook_secret: Some("sentinel-webhook-7f3a"),
        verify_token: Some("sentinel-verify-7f3a"),
        account_id: Some("account-1"),
        phone_number: Some("+15550000001"),
        bot_token: Some("sentinel-bot-token-7f3a"),
        is_active: true,
    };
    let printed = printed_without(
        &params,
        &[
            "sentinel-api-key-7f3a",
            "sentinel-api-secret-7f3a",
            "sentinel-webhook-7f3a",
            "sentinel-verify-7f3a",
            "sentinel-bot-token-7f3a",
        ],
    );
    assert_eq!(printed.matches("[REDACTED]").count(), 5, "{printed}");
}

#[test]
fn a_new_messaging_session_never_prints_the_channel_user_id() {
    let params = CreateSessionParams {
        id: "session-1",
        user_id: "user-1",
        tenant_id: TenantId::generate(),
        channel_type: "whatsapp",
        channel_user_id: "+15550007f3a",
        channel_conversation_id: Some("chat-1"),
        pierre_conversation_id: Some("conv-1"),
    };
    let printed = printed_without(&params, &["+15550007f3a"]);
    assert!(
        printed.contains("channel_user_id: \"[REDACTED]\""),
        "{printed}"
    );
}

#[test]
fn an_outbound_queue_entry_prints_the_size_of_its_payload_never_the_payload() {
    let payload = r#"{"text":"Reconnect: https://api.dravr.ai/r/sentinel-link-7f3a"}"#;
    let params = EnqueueOutboundParams {
        id: "queue-1",
        message_id: Some("msg-1"),
        tenant_id: TenantId::generate(),
        user_id: Some("user-1"),
        channel_type: "telegram",
        payload,
        expires_at: Some(Utc::now()),
        reauth: None,
    };
    let printed = printed_without(&params, &["sentinel-link-7f3a"]);
    assert!(
        printed.contains(&format!("payload_len: {}", payload.len())),
        "{printed}"
    );
}

#[test]
fn a_stored_channel_message_never_prints_its_sender_body_or_raw_webhook() {
    let body = "sentinel-body-7f3a my knee hurts";
    let raw = r#"{"from":"sentinel-raw-7f3a"}"#;
    let params = InsertMessageParams {
        id: "msg-1",
        tenant_id: TenantId::generate(),
        session_id: "session-1",
        direction: "inbound",
        channel_type: "whatsapp",
        channel_message_id: "wamid-1",
        sender_id: "+15550007f3a",
        content_type: "text",
        content_body: Some(body),
        correlation_id: "corr-1",
        raw_payload: Some(raw),
        chat_message_id: Some("chat-msg-1"),
    };
    let printed = printed_without(
        &params,
        &["+15550007f3a", "sentinel-body-7f3a", "sentinel-raw-7f3a"],
    );
    assert!(printed.contains("sender_id: \"[REDACTED]\""), "{printed}");
    assert!(
        printed.contains(&format!("content_body_len: Some({})", body.len())),
        "{printed}"
    );
    assert!(
        printed.contains(&format!("raw_payload_len: Some({})", raw.len())),
        "{printed}"
    );
}

#[test]
fn a_link_state_never_prints_its_code_sender_or_display_name() {
    let params = CreateLinkStateParams {
        id: "state-1",
        tenant_id: TenantId::generate(),
        user_id: Some("user-1"),
        channel_type: "telegram",
        code: "sentinel-code-7f3a",
        method: "deep_link",
        channel_user_id: Some("sentinel-channel-user-7f3a"),
        sender_name: Some("sentinel-name-7f3a"),
        expires_at: "2026-10-23T00:00:00Z",
    };
    let printed = printed_without(
        &params,
        &[
            "sentinel-code-7f3a",
            "sentinel-channel-user-7f3a",
            "sentinel-name-7f3a",
        ],
    );
    assert_eq!(printed.matches("[REDACTED]").count(), 3, "{printed}");
}

#[test]
fn a_channel_link_never_prints_the_channel_user_id_or_display_name() {
    let params = CreateChannelLinkParams {
        id: "link-1",
        tenant_id: TenantId::generate(),
        user_id: "user-1",
        channel_type: "telegram",
        channel_user_id: "sentinel-channel-user-7f3a",
        display_name: Some("sentinel-name-7f3a"),
    };
    let printed = printed_without(
        &params,
        &["sentinel-channel-user-7f3a", "sentinel-name-7f3a"],
    );
    assert_eq!(printed.matches("[REDACTED]").count(), 2, "{printed}");
}

#[test]
fn a_seeded_demo_user_never_prints_its_password_hash() {
    let user = SeedDemoUser {
        id: Uuid::new_v4(),
        email: "demo@example.com".to_owned(),
        display_name: "Demo".to_owned(),
        password_hash: "$2b$12$sentinel-hash-7f3a".to_owned(),
        tier: "starter".to_owned(),
        status: "active".to_owned(),
        is_admin: false,
        locale: "en".to_owned(),
        created_at: Utc::now(),
    };
    let printed = printed_without(&user, &["sentinel-hash-7f3a"]);
    assert!(
        printed.contains("password_hash: \"[REDACTED]\""),
        "{printed}"
    );
    assert!(printed.contains("demo@example.com"), "{printed}");
}

#[test]
fn a_seeded_api_key_never_prints_its_hash() {
    let key = SeedApiKey {
        id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        name: "demo key".to_owned(),
        description: "seeded".to_owned(),
        key_hash: "sentinel-key-hash-7f3a".to_owned(),
        key_prefix: "pk_live_ab".to_owned(),
        tier: "starter".to_owned(),
        rate_limit: Some(100),
        expires_at: None,
        created_at: Utc::now(),
    };
    let printed = printed_without(&key, &["sentinel-key-hash-7f3a"]);
    assert!(printed.contains("key_hash: \"[REDACTED]\""), "{printed}");
    assert!(printed.contains("pk_live_ab"), "{printed}");
}

#[tokio::test]
async fn a_database_handle_never_prints_its_encryption_keys() {
    let database = create_test_db_with_key(vec![0x7f; 32]).await.unwrap();
    let printed = printed_without(&database, &["127, 127, 127, 127"]);
    assert!(
        printed.contains("encryption_key: \"[REDACTED]\""),
        "{printed}"
    );
    assert!(
        printed.contains("blind_index_key: \"[REDACTED]\""),
        "{printed}"
    );
}

#[test]
fn a_compaction_block_prints_the_size_of_its_summary_never_the_summary() {
    let summary = "sentinel-summary-7f3a the athlete reported a sore achilles";
    let params = InsertCompactionBlockParams {
        tenant_id: TenantId::generate(),
        conversation_id: "conv-1",
        summary,
        summary_tokens: 12,
        original_tokens: 900,
        first_message_id: "msg-1",
        last_message_id: "msg-9",
        transport_policy: TransportPolicy::AnyTransport,
        about_user_id: None,
    };
    let printed = printed_without(&params, &["sentinel-summary-7f3a", "achilles"]);
    assert!(
        printed.contains(&format!("summary_len: {}", summary.len())),
        "the summary's size prints in place of its text: {printed}"
    );
    assert!(
        printed.contains("msg-1") && printed.contains("msg-9"),
        "the compacted range still prints: {printed}"
    );
}

#[test]
fn a_claim_verdict_prints_the_size_of_its_claim_and_explanation_never_the_text() {
    let claim = "sentinel-claim-7f3a your resting HR dropped to 41";
    let explanation = "sentinel-explanation-7f3a the athlete's log says 48";
    let params = InsertClaimVerdictParams {
        tenant_id: TenantId::generate(),
        user_id: "user-1",
        agent_id: Some("agent-1"),
        conversation_id: Some("conv-1"),
        message_id: Some("msg-1"),
        claim_text: claim,
        category: ClaimCategory::AthleteData,
        status: ClaimStatus::Unverifiable,
        evidence_strength: EvidenceStrength::None,
        confidence: 0.4,
        layer_fired: VerdictLayer::AthleteData,
        explanation: Some(explanation),
        evidence_refs: Some("doi:10.1000/182"),
        transport_policy: TransportPolicy::AnyTransport,
    };
    let printed = printed_without(
        &params,
        &["sentinel-claim-7f3a", "sentinel-explanation-7f3a"],
    );
    assert!(printed.contains("InsertClaimVerdictParams"), "{printed}");
    assert!(
        printed.contains(&format!("claim_text_len: {}", claim.len())),
        "{printed}"
    );
    assert!(
        printed.contains(&format!("explanation_len: Some({})", explanation.len())),
        "{printed}"
    );
    assert!(printed.contains("doi:10.1000/182"), "{printed}");
}

#[test]
fn a_verdict_disposition_prints_the_size_of_its_note_never_the_note() {
    let note = "sentinel-note-7f3a the claim quoted their knee injury";
    let params = SetVerdictDispositionParams {
        tenant_id: TenantId::generate(),
        verdict_id: "verdict-1",
        disposition: VerdictDisposition::FalsePositive,
        reason: Some(DispositionReason::MissingKeyword),
        note: Some(note),
        disposed_by: "admin_console",
        disposed_at: Utc::now(),
    };
    let printed = printed_without(&params, &["sentinel-note-7f3a"]);
    assert!(printed.contains("SetVerdictDispositionParams"), "{printed}");
    assert!(
        printed.contains(&format!("note_len: Some({})", note.len())),
        "{printed}"
    );
    assert!(printed.contains("verdict-1"), "{printed}");
}

#[test]
fn a_user_fact_upsert_prints_the_size_of_the_athletes_words_never_the_words() {
    let object = "sentinel-object-7f3a a marathon in Montreal next spring";
    let params = UpsertUserFactParams {
        tenant_id: TenantId::generate(),
        user_id: "user-1",
        agent_id: None,
        scope: MemoryScope::User,
        kind: FactKind::Preference,
        pillar: None,
        predicate_code: PredicateCode::TrainingFor,
        object,
        confidence: 0.8,
        source: FactSource::Conversation,
        valid_until: None,
        source_msg_id: Some("msg-1"),
        transport_policy: TransportPolicy::AnyTransport,
    };
    let printed = printed_without(&params, &["sentinel-object-7f3a", "Montreal"]);
    assert!(printed.contains("UpsertUserFactParams"), "{printed}");
    assert!(
        printed.contains(&format!("object_len: {}", object.len())),
        "{printed}"
    );
}

#[test]
fn an_agent_note_prints_the_size_of_its_content_never_the_content() {
    let content = "sentinel-note-content-7f3a athlete mentioned poor sleep";
    let params = InsertAgentNoteParams {
        tenant_id: TenantId::generate(),
        user_id: "user-1",
        agent_id: "agent-1",
        conversation_id: Some("conv-1"),
        scope: MemoryScope::User,
        content,
        transport_policy: TransportPolicy::AnyTransport,
    };
    let printed = printed_without(&params, &["sentinel-note-content-7f3a"]);
    assert!(printed.contains("InsertAgentNoteParams"), "{printed}");
    assert!(
        printed.contains(&format!("content_len: {}", content.len())),
        "{printed}"
    );
}

#[test]
fn an_agent_followup_prints_the_size_of_its_content_never_the_content() {
    let content = "sentinel-followup-7f3a ask how the long run felt";
    let params = InsertAgentFollowupParams {
        tenant_id: TenantId::generate(),
        user_id: "user-1",
        agent_id: "agent-1",
        conversation_id: None,
        content,
        due_at: Some(Utc::now()),
        transport_policy: TransportPolicy::AnyTransport,
    };
    let printed = printed_without(&params, &["sentinel-followup-7f3a"]);
    assert!(printed.contains("InsertAgentFollowupParams"), "{printed}");
    assert!(
        printed.contains(&format!("content_len: {}", content.len())),
        "{printed}"
    );
}
