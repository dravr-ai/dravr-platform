// ABOUTME: Unit tests for the LLM provider abstraction layer
// ABOUTME: Tests capabilities, message handling, and provider-type parsing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files don't require documentation - this is a rustc lint (not clippy)
#![allow(missing_docs)]

use pierre_config::environment::LlmProviderType;
use pierre_llm::config::LlmChain;
use pierre_llm::{ChatMessage, ChatRequest, LlmCapabilities, MessageRole};

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
// LlmProviderType Tests
// ============================================================================

#[test]
fn test_llm_provider_type_default() {
    let provider_type = LlmProviderType::default();
    assert_eq!(provider_type, LlmProviderType::Gemini);
}

#[test]
fn test_llm_provider_type_from_str() {
    assert_eq!(LlmProviderType::parse("groq"), Some(LlmProviderType::Groq));
    assert_eq!(
        LlmProviderType::parse("gemini"),
        Some(LlmProviderType::Gemini)
    );
    assert_eq!(
        LlmProviderType::parse("google"),
        Some(LlmProviderType::Gemini)
    );
    assert_eq!(
        LlmProviderType::parse("anthropic"),
        Some(LlmProviderType::Anthropic)
    );
    // An unknown value is no provider, never a silent Gemini
    assert_eq!(LlmProviderType::parse("unknown"), None);
}

#[test]
fn test_llm_provider_type_display() {
    assert_eq!(format!("{}", LlmProviderType::Groq), "groq");
    assert_eq!(format!("{}", LlmProviderType::Gemini), "gemini");
}

/// The provider after `provider`, in declaration order; `None` after the
/// last. The match is exhaustive on purpose: a provider added to the server
/// does not compile here until it has its place in the walk, so
/// [`every_provider`] cannot miss one.
const fn provider_after(provider: LlmProviderType) -> Option<LlmProviderType> {
    use LlmProviderType as P;
    match provider {
        P::Groq => Some(P::Gemini),
        P::Gemini => Some(P::Local),
        P::Local => Some(P::OpenRouter),
        P::OpenRouter => Some(P::Cohere),
        P::Cohere => Some(P::Anthropic),
        P::Anthropic => Some(P::ClaudeCode),
        P::ClaudeCode => Some(P::CursorAgent),
        P::CursorAgent => Some(P::OpenCode),
        P::OpenCode => Some(P::Copilot),
        P::Copilot => Some(P::CopilotHeadless),
        P::CopilotHeadless => Some(P::CopilotSdk),
        P::CopilotSdk => Some(P::WarpCli),
        P::WarpCli => Some(P::GeminiCli),
        P::GeminiCli => Some(P::CodexCli),
        P::CodexCli => Some(P::GooseCli),
        P::GooseCli => Some(P::ClineCli),
        P::ClineCli => Some(P::ContinueCli),
        P::ContinueCli => Some(P::KiroCli),
        P::KiroCli => Some(P::KiloCli),
        P::KiloCli => Some(P::Router),
        P::Router => Some(P::OpenAiApi),
        P::OpenAiApi => None,
    }
}

/// Every provider a chain entry may name.
fn every_provider() -> Vec<LlmProviderType> {
    let mut providers = vec![LlmProviderType::Groq];
    while let Some(next) = providers.last().copied().and_then(provider_after) {
        providers.push(next);
    }
    providers
}

/// The provider names the `llm_chain` input accepts: the quoted strings of
/// the list its validation hands to `contains`, in `declaration`.
fn accepted_provider_names(declaration: &str) -> Vec<&str> {
    let list = declaration
        .split("contains([")
        .nth(1)
        .and_then(|after| after.split(']').next())
        .unwrap_or_else(|| {
            panic!("llm_chain no longer validates its provider names against a list: {declaration}")
        });
    list.split(',')
        .map(|name| name.trim().trim_matches('"'))
        .filter(|name| !name.is_empty())
        .collect()
}

/// The variable the server reads is the one the deployment sets, from the
/// `llm_chain` input, and that input refuses an entry that names no model or
/// a provider the server does not know.
///
/// A text check on purpose: the two sides are this binary and Cloud Run's
/// environment, which Terraform writes, and nothing in a test process joins
/// them. A rename on either side alone leaves production on the default
/// provider without an error anywhere; a typo in one entry drops that tier
/// with only a boot warning. The chain's value is a private input, absent
/// from this repository, so the entry check is the input's own validation,
/// which fails the plan, and this test pins that the validation is there.
/// The names that validation accepts are a second copy of the parser's, so
/// both directions are held here: every accepted name is one the parser
/// builds a provider from, and every provider's own name is accepted.
#[test]
fn test_llm_chain_env_var_is_the_one_the_deployment_sets() {
    let deployment = include_str!("../../../../infra/environments/dev/main.tf");
    // Terraform aligns the `=` of neighbouring assignments, so the gap
    // between the name and the value is any run of spaces.
    let source = deployment
        .lines()
        .find_map(|line| {
            line.trim_start()
                .strip_prefix(LlmChain::ENV_VAR)?
                .trim_start()
                .strip_prefix('=')
                .map(str::trim)
        })
        .unwrap_or_else(|| {
            panic!(
                "infra/environments/dev/main.tf no longer sets {}, the variable the \
                 server builds its LLM chain from",
                LlmChain::ENV_VAR
            )
        });
    assert_eq!(
        source,
        "var.llm_chain",
        "{} is set from the llm_chain input",
        LlmChain::ENV_VAR
    );

    let variables = include_str!("../../../../infra/environments/dev/variables.tf");
    let after_header = variables
        .split("variable \"llm_chain\"")
        .nth(1)
        .unwrap_or_else(|| {
            panic!("infra/environments/dev/variables.tf no longer declares llm_chain")
        });
    let declaration = after_header
        .split("\nvariable ")
        .next()
        .unwrap_or(after_header);
    assert!(
        declaration.contains("validation {") && declaration.contains("provider=model"),
        "llm_chain validates that every entry is provider=model: {declaration}"
    );
    assert!(
        !declaration
            .lines()
            .any(|line| line.trim_start().starts_with("default")),
        "llm_chain has no default, so a plan without the private value fails: {declaration}"
    );

    let accepted = accepted_provider_names(declaration);
    for name in &accepted {
        assert!(
            LlmProviderType::parse(name).is_some(),
            "llm_chain accepts {name:?}, which the server builds no provider from: the plan \
             would pass and the tier would be skipped at boot"
        );
        let chain = LlmChain::parse(&format!("{name}=some-model"));
        assert!(
            chain.issues().is_empty(),
            "the chain parser refuses an entry llm_chain accepts ({name}=some-model): {:?}",
            chain.issues()
        );
    }
    let providers = every_provider();
    assert_eq!(
        providers.len(),
        22,
        "the walk over the providers stopped early or looped: {providers:?}"
    );
    for provider in providers {
        let name = provider.to_string();
        assert_eq!(
            LlmProviderType::parse(&name),
            Some(provider),
            "{name} is not the name the parser reads {provider:?} from"
        );
        assert!(
            accepted.contains(&name.as_str()),
            "llm_chain refuses {name}, a provider the server knows: add it to the list in \
             infra/environments/dev/variables.tf"
        );
    }
}

#[test]
fn test_llm_provider_type_local_aliases() {
    for alias in ["local", "ollama", "vllm", "localai"] {
        assert_eq!(
            LlmProviderType::parse(alias),
            Some(LlmProviderType::Local),
            "{alias} selects the local OpenAI-compatible provider"
        );
    }
    assert_eq!(LlmProviderType::Local.to_string(), "local");
}

#[test]
fn test_llm_provider_type_case_insensitive() {
    for alias in ["LOCAL", "Ollama", "VLLM", "LocalAI"] {
        assert_eq!(
            LlmProviderType::parse(alias),
            Some(LlmProviderType::Local),
            "{alias} parses regardless of case"
        );
    }
}
