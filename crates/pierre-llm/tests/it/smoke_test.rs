// ABOUTME: Smoke test for pierre-llm's public API (ChatMessage, MessageRole, LlmCapabilities)
// ABOUTME: Guards the trait + type contracts that LLM provider implementations rely on
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Smoke integration tests for the crate public API.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use pierre_llm::{ChatMessage, LlmCapabilities, LlmProviderRegistry, MessageRole};

#[test]
fn message_role_serializes_to_lowercase_strings() {
    let json = serde_json::to_value(MessageRole::User).expect("MessageRole serialize");
    assert_eq!(json, serde_json::json!("user"));

    let json = serde_json::to_value(MessageRole::System).expect("MessageRole serialize");
    assert_eq!(json, serde_json::json!("system"));

    let json = serde_json::to_value(MessageRole::Assistant).expect("MessageRole serialize");
    assert_eq!(json, serde_json::json!("assistant"));

    let json = serde_json::to_value(MessageRole::Tool).expect("MessageRole serialize");
    assert_eq!(json, serde_json::json!("tool"));
}

#[test]
fn chat_message_serde_round_trip() {
    let msg = ChatMessage {
        role: MessageRole::User,
        content: "Hello, coach.".to_owned(),
        images: None,
        tool_calls: None,
        tool_call_id: None,
        name: None,
    };
    let json = serde_json::to_string(&msg).expect("ChatMessage serialize");
    let parsed: ChatMessage = serde_json::from_str(&json).expect("ChatMessage deserialize");
    assert!(matches!(parsed.role, MessageRole::User));
    assert_eq!(parsed.content, "Hello, coach.");
}

#[test]
fn llm_capabilities_bitflags_compose() {
    let caps = LlmCapabilities::STREAMING | LlmCapabilities::FUNCTION_CALLING;
    assert!(caps.contains(LlmCapabilities::STREAMING));
    assert!(caps.contains(LlmCapabilities::FUNCTION_CALLING));
    assert!(!caps.contains(LlmCapabilities::VISION));
}

#[test]
fn provider_registry_returns_none_for_unknown_lookup() {
    let registry = LlmProviderRegistry::new();
    assert!(
        registry.get("nonexistent-provider").is_none(),
        "empty registry must return None for any provider name"
    );
}
