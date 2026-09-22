// ABOUTME: Pins the shape of an EmbacleProvider chain: head-tier identity, tail depth, and how missing tiers assemble
// ABOUTME: Replaces the in-source compose_* tests that drove the nested two-provider chain
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A chain reports its head tier — `name()` keys `llm_usage.provider` and the
//! price table, `capabilities()` picks the tool loop — and exposes the rest
//! as its tail for the headless loop's re-run. `assemble_runtime_chain` turns
//! whatever tiers built at boot into that chain, promoting a tertiary over a
//! secondary that failed and collapsing to a solo provider when only one side
//! built.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use async_trait::async_trait;
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError,
};
use pierre_llm::config::LlmProviderType;
use pierre_llm::errors::{AppError, ErrorCode};
use pierre_llm::{ChainTiers, ChatProvider, EmbacleProvider, LlmProvider};

/// A scripted embacle runner with a name and a capability set of its own.
struct Scripted {
    name: &'static str,
    capabilities: LlmCapabilities,
    models: Vec<String>,
}

impl Scripted {
    fn new(name: &'static str, capabilities: LlmCapabilities) -> Self {
        Self {
            name,
            capabilities,
            models: vec![format!("{name}-model")],
        }
    }
}

#[async_trait]
impl EmbacleLlmProvider for Scripted {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &str {
        self.name
    }

    fn capabilities(&self) -> LlmCapabilities {
        self.capabilities
    }

    fn default_model(&self) -> &str {
        &self.models[0]
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        Ok(ChatResponse {
            content: format!("from {}", self.name),
            model: self.models[0].clone(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal("scripted runners do not stream"))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
}

/// A Copilot-shaped head: SDK tool calling, no native function calling.
fn copilot_like() -> EmbacleProvider {
    EmbacleProvider::from_runner(
        Box::new(Scripted::new(
            "copilot_headless",
            LlmCapabilities::STREAMING | LlmCapabilities::SDK_TOOL_CALLING,
        )),
        "Copilot (scripted)",
    )
}

/// A Cohere-shaped tier: native function calling.
fn cohere_like() -> EmbacleProvider {
    EmbacleProvider::from_runner(
        Box::new(Scripted::new(
            "cohere",
            LlmCapabilities::STREAMING | LlmCapabilities::FUNCTION_CALLING,
        )),
        "Cohere (scripted)",
    )
}

/// A Gemini-shaped tier: full featured.
fn gemini_like() -> EmbacleProvider {
    EmbacleProvider::from_runner(
        Box::new(Scripted::new("gemini", LlmCapabilities::full_featured())),
        "Gemini (scripted)",
    )
}

#[test]
fn zero_tiers_is_a_config_error() {
    let err = EmbacleProvider::chain(vec![]).expect_err("no tiers, no chain");
    assert_eq!(err.code, ErrorCode::ConfigError);
}

#[test]
fn one_tier_is_that_tier_unchained() {
    let solo = EmbacleProvider::chain(vec![cohere_like()]).unwrap();
    assert_eq!(solo.name(), "cohere");
    assert_eq!(solo.display_name(), "Cohere (scripted)");
    assert!(
        solo.fallback_tail().is_none(),
        "a single tier has nothing to fall back to"
    );
}

#[test]
fn a_chain_reports_its_head_tier_not_the_union() {
    let chain = EmbacleProvider::chain(vec![copilot_like(), cohere_like()]).unwrap();

    assert_eq!(
        chain.name(),
        "copilot_headless",
        "llm_usage.provider and the price table are keyed on the head's name"
    );
    assert_eq!(chain.display_name(), "Copilot (scripted)");
    assert_eq!(chain.default_model(), "copilot_headless-model");
    assert_eq!(
        chain.available_models(),
        ["copilot_headless-model".to_owned()]
    );

    let capabilities = chain.capabilities();
    assert!(
        capabilities.supports_sdk_tool_calling(),
        "the head's SDK tool calling routes the turn to the headless loop"
    );
    assert!(
        !capabilities.supports_function_calling(),
        "the tail's FUNCTION_CALLING must not leak into the head's capabilities, \
         or Copilot turns would be sent down the API loop: {capabilities:?}"
    );
}

#[test]
fn the_tail_of_a_three_tier_chain_is_two_deep() {
    let chain = EmbacleProvider::chain(vec![copilot_like(), cohere_like(), gemini_like()]).unwrap();

    let tail = chain.fallback_tail().expect("a chain has a tail");
    assert_eq!(
        tail.name(),
        "cohere",
        "the tail is headed by the second tier"
    );
    assert!(
        tail.capabilities().supports_function_calling(),
        "the tail reports ITS head's capabilities, so the re-run takes the API loop"
    );

    let tail_of_tail = tail
        .fallback_tail()
        .expect("the tail of three tiers has a tail");
    assert_eq!(tail_of_tail.name(), "gemini");
    assert!(
        tail_of_tail.fallback_tail().is_none(),
        "the last tier ends the chain"
    );
}

#[test]
fn a_two_tier_chain_has_a_solo_tail() {
    let chain = EmbacleProvider::chain(vec![copilot_like(), cohere_like()]).unwrap();
    let tail = chain.fallback_tail().expect("a chain has a tail");
    assert_eq!(tail.name(), "cohere");
    assert!(tail.fallback_tail().is_none());
}

#[test]
fn every_tier_built_assembles_three_deep() {
    let chain = ChatProvider::assemble_runtime_chain(
        ChainTiers {
            primary: Ok(copilot_like()),
            accounts: Vec::new(),
            secondary: Ok(cohere_like()),
            tertiary: Some(Ok(gemini_like())),
        },
        LlmProviderType::CopilotHeadless,
        LlmProviderType::Cohere,
    )
    .unwrap();

    assert_eq!(chain.name(), "copilot_headless");
    let tail = chain.fallback_tail().unwrap();
    assert_eq!(tail.name(), "cohere");
    assert_eq!(tail.fallback_tail().unwrap().name(), "gemini");
}

#[test]
fn a_failed_secondary_promotes_the_tertiary() {
    let chain = ChatProvider::assemble_runtime_chain(
        ChainTiers {
            primary: Ok(copilot_like()),
            accounts: Vec::new(),
            secondary: Err(AppError::config("COHERE_API_KEY unset")),
            tertiary: Some(Ok(gemini_like())),
        },
        LlmProviderType::CopilotHeadless,
        LlmProviderType::Cohere,
    )
    .unwrap();

    assert_eq!(chain.name(), "copilot_headless");
    let tail = chain
        .fallback_tail()
        .expect("the tertiary takes the secondary's place");
    assert_eq!(tail.name(), "gemini");
    assert!(tail.fallback_tail().is_none(), "two tiers, not three");
}

#[test]
fn a_failed_tertiary_leaves_the_two_tier_chain() {
    let chain = ChatProvider::assemble_runtime_chain(
        ChainTiers {
            primary: Ok(copilot_like()),
            accounts: Vec::new(),
            secondary: Ok(cohere_like()),
            tertiary: Some(Err(AppError::config("GEMINI_API_KEY unset"))),
        },
        LlmProviderType::CopilotHeadless,
        LlmProviderType::Cohere,
    )
    .unwrap();

    let tail = chain.fallback_tail().unwrap();
    assert_eq!(tail.name(), "cohere");
    assert!(tail.fallback_tail().is_none());
}

#[test]
fn a_failed_tail_runs_the_primary_alone() {
    let chain = ChatProvider::assemble_runtime_chain(
        ChainTiers {
            primary: Ok(copilot_like()),
            accounts: Vec::new(),
            secondary: Err(AppError::config("COHERE_API_KEY unset")),
            tertiary: Some(Err(AppError::config("GEMINI_API_KEY unset"))),
        },
        LlmProviderType::CopilotHeadless,
        LlmProviderType::Cohere,
    )
    .unwrap();

    assert_eq!(chain.name(), "copilot_headless");
    assert!(
        chain.fallback_tail().is_none(),
        "with no tail tier the primary is solo, not a chain of one"
    );
}

#[test]
fn a_failed_primary_runs_the_tail_alone() {
    let chain = ChatProvider::assemble_runtime_chain(
        ChainTiers {
            primary: Err(AppError::config("copilot binary missing")),
            accounts: Vec::new(),
            secondary: Ok(cohere_like()),
            tertiary: Some(Ok(gemini_like())),
        },
        LlmProviderType::CopilotHeadless,
        LlmProviderType::Cohere,
    )
    .unwrap();

    assert_eq!(
        chain.name(),
        "cohere",
        "the secondary leads when the primary is absent"
    );
    assert_eq!(chain.fallback_tail().unwrap().name(), "gemini");
}

#[test]
fn every_tier_failing_is_a_config_error() {
    let err = ChatProvider::assemble_runtime_chain(
        ChainTiers {
            primary: Err(AppError::config("copilot binary missing")),
            accounts: Vec::new(),
            secondary: Err(AppError::config("COHERE_API_KEY unset")),
            tertiary: None,
        },
        LlmProviderType::CopilotHeadless,
        LlmProviderType::Cohere,
    )
    .expect_err("nothing built, nothing to serve");

    assert_eq!(err.code, ErrorCode::ConfigError);
    assert!(
        err.message.contains("copilot binary missing") && err.message.contains("COHERE_API_KEY"),
        "both reasons reach the operator: {}",
        err.message
    );
}

// ---------------------------------------------------------------------------
// A pooled primary: its further accounts sit right behind it
// ---------------------------------------------------------------------------

#[test]
fn further_accounts_sit_between_the_primary_and_the_fallback() {
    let chain = ChatProvider::assemble_runtime_chain(
        ChainTiers {
            primary: Ok(copilot_like()),
            accounts: vec![cohere_like()],
            secondary: Ok(gemini_like()),
            tertiary: None,
        },
        LlmProviderType::CopilotHeadless,
        LlmProviderType::Gemini,
    )
    .unwrap();

    assert_eq!(chain.name(), "copilot_headless");
    let second = chain.fallback_tail().expect("the next account");
    assert_eq!(
        second.name(),
        "cohere",
        "account 2 is asked before the fallback"
    );
    assert_eq!(
        second.fallback_tail().expect("then the fallback").name(),
        "gemini"
    );
}

#[test]
fn accounts_alone_are_a_tail_when_no_fallback_built() {
    let chain = ChatProvider::assemble_runtime_chain(
        ChainTiers {
            primary: Ok(copilot_like()),
            accounts: vec![cohere_like()],
            secondary: Err(AppError::config("GEMINI_API_KEY unset")),
            tertiary: None,
        },
        LlmProviderType::CopilotHeadless,
        LlmProviderType::Gemini,
    )
    .unwrap();

    let tail = chain
        .fallback_tail()
        .expect("the second account is still a tier");
    assert_eq!(tail.name(), "cohere");
    assert!(tail.fallback_tail().is_none());
}
