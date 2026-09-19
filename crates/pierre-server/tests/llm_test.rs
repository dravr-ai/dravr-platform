// ABOUTME: Unit tests for the LLM provider abstraction layer
// ABOUTME: Tests capabilities, message handling, provider registry, and provider-type parsing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files don't require documentation - this is a rustc lint (not clippy)
#![allow(missing_docs)]

use pierre_config::environment::LlmProviderType;
use pierre_llm::{ChatMessage, ChatRequest, LlmCapabilities, LlmProviderRegistry, MessageRole};

// ============================================================================
// LlmCapabilities Tests
// ============================================================================

#[test]
fn test_capabilities_text_only() {
    let caps = LlmCapabilities::text_only();
    assert!(caps.supports_streaming());
    assert!(caps.supports_system_messages());
    assert!(!caps.supports_function_calling());
    assert!(!caps.supports_vision());
}

#[test]
fn test_capabilities_full_featured() {
    let caps = LlmCapabilities::full_featured();
    assert!(caps.supports_streaming());
    assert!(caps.supports_function_calling());
    assert!(caps.supports_vision());
    assert!(caps.supports_json_mode());
    assert!(caps.supports_system_messages());
}

// ============================================================================
// MessageRole Tests
// ============================================================================

#[test]
fn test_message_role_as_str() {
    assert_eq!(MessageRole::System.as_str(), "system");
    assert_eq!(MessageRole::User.as_str(), "user");
    assert_eq!(MessageRole::Assistant.as_str(), "assistant");
}

// ============================================================================
// ChatMessage Tests
// ============================================================================

#[test]
fn test_chat_message_constructors() {
    let system = ChatMessage::system("You are helpful");
    assert_eq!(system.role, MessageRole::System);
    assert_eq!(system.content, "You are helpful");

    let user = ChatMessage::user("Hello");
    assert_eq!(user.role, MessageRole::User);

    let assistant = ChatMessage::assistant("Hi there!");
    assert_eq!(assistant.role, MessageRole::Assistant);
}

// ============================================================================
// ChatRequest Tests
// ============================================================================

#[test]
fn test_chat_request_builder() {
    let request = ChatRequest::new(vec![ChatMessage::user("Hello")])
        .with_model("gemini-pro")
        .with_temperature(0.7)
        .with_max_tokens(1000)
        .with_streaming();

    assert_eq!(request.model, Some("gemini-pro".to_owned()));
    assert_eq!(request.temperature, Some(0.7));
    assert_eq!(request.max_tokens, Some(1000));
    assert!(request.stream);
}

// ============================================================================
// LlmProviderRegistry Tests
// ============================================================================

#[test]
fn test_registry_operations() {
    let registry = LlmProviderRegistry::new();
    assert!(registry.list().is_empty());
    assert!(registry.default_provider().is_none());
}

// ============================================================================
// LlmProviderType Tests
// ============================================================================

#[test]
fn test_llm_provider_type_default() {
    let provider_type = LlmProviderType::default();
    assert_eq!(provider_type, LlmProviderType::Gemini);
}

#[test]
fn test_llm_provider_type_from_str() {
    assert_eq!(
        LlmProviderType::from_str_or_default("groq"),
        LlmProviderType::Groq
    );
    assert_eq!(
        LlmProviderType::from_str_or_default("gemini"),
        LlmProviderType::Gemini
    );
    assert_eq!(
        LlmProviderType::from_str_or_default("google"),
        LlmProviderType::Gemini
    );
    // Unknown values default to Gemini
    assert_eq!(
        LlmProviderType::from_str_or_default("unknown"),
        LlmProviderType::Gemini
    );
}

#[test]
fn test_llm_provider_type_display() {
    assert_eq!(format!("{}", LlmProviderType::Groq), "groq");
    assert_eq!(format!("{}", LlmProviderType::Gemini), "gemini");
}

#[test]
fn test_llm_provider_type_env_var_name() {
    assert_eq!(LlmProviderType::ENV_VAR, "PIERRE_LLM_PROVIDER");
}

#[test]
fn test_llm_provider_type_local_aliases() {
    for alias in ["local", "ollama", "vllm", "localai"] {
        assert_eq!(
            LlmProviderType::from_str_or_default(alias),
            LlmProviderType::Local,
            "{alias} selects the local OpenAI-compatible provider"
        );
    }
    assert_eq!(LlmProviderType::Local.to_string(), "local");
}

#[test]
fn test_llm_provider_type_case_insensitive() {
    for alias in ["LOCAL", "Ollama", "VLLM", "LocalAI"] {
        assert_eq!(
            LlmProviderType::from_str_or_default(alias),
            LlmProviderType::Local,
            "{alias} parses regardless of case"
        );
    }
}
