// ABOUTME: Pins the shape of an EmbacleProvider chain: head-tier identity, tail depth, and how missing tiers assemble
// ABOUTME: Replaces the in-source compose_* tests that drove the nested two-provider chain
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A chain reports its head tier — `name()` keys `llm_usage.provider` and the
//! price table, `capabilities()` picks the tool loop — and exposes the rest
//! as its tail for the tool loops' re-run. `assemble_runtime_chain` turns
//! whatever `PIERRE_LLM_CHAIN` tiers built at boot into that chain, in order,
//! each followed by its pooled accounts, moving later tiers up over one that
//! failed and collapsing to a solo provider when only one built.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::time::Duration;

use async_trait::async_trait;
use embacle::types::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider as EmbacleLlmProvider,
    RunnerError,
};
use pierre_llm::config::LlmProviderType;
use pierre_llm::errors::{AppError, ErrorCode};
use pierre_llm::{ChainTier, ChatProvider, EmbacleProvider, LlmProvider, QuotaCooldowns};

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
    let err =
        EmbacleProvider::chain(vec![], QuotaCooldowns::default()).expect_err("no tiers, no chain");
    assert_eq!(err.code, ErrorCode::ConfigError);
}

#[test]
fn one_tier_is_that_tier_unchained() {
    let solo = EmbacleProvider::chain(vec![cohere_like()], QuotaCooldowns::default()).unwrap();
    assert_eq!(solo.name(), "cohere");
    assert_eq!(solo.display_name(), "Cohere (scripted)");
    assert!(
        solo.fallback_tail().is_none(),
        "a single tier has nothing to fall back to"
    );
}

#[test]
fn a_chain_reports_its_head_tier_not_the_union() {
    let chain = EmbacleProvider::chain(
        vec![copilot_like(), cohere_like()],
        QuotaCooldowns::default(),
    )
    .unwrap();

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
    let chain = EmbacleProvider::chain(
        vec![copilot_like(), cohere_like(), gemini_like()],
        QuotaCooldowns::default(),
    )
    .unwrap();

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
    let chain = EmbacleProvider::chain(
        vec![copilot_like(), cohere_like()],
        QuotaCooldowns::default(),
    )
    .unwrap();
    let tail = chain.fallback_tail().expect("a chain has a tail");
    assert_eq!(tail.name(), "cohere");
    assert!(tail.fallback_tail().is_none());
}

/// A built tier with no further accounts.
fn ok(kind: LlmProviderType, tier: EmbacleProvider) -> ChainTier {
    ChainTier {
        kind,
        built: Ok(tier),
        accounts: Vec::new(),
    }
}

/// A built tier followed by its further accounts.
fn pooled(
    kind: LlmProviderType,
    tier: EmbacleProvider,
    accounts: Vec<EmbacleProvider>,
) -> ChainTier {
    ChainTier {
        kind,
        built: Ok(tier),
        accounts,
    }
}

/// A configured tier that did not build.
fn failed(kind: LlmProviderType, reason: &str) -> ChainTier {
    ChainTier {
        kind,
        built: Err(AppError::config(reason)),
        accounts: Vec::new(),
    }
}

/// An Anthropic-API-shaped tier or account, by name: native function calling.
fn anthropic_like(name: &'static str) -> EmbacleProvider {
    EmbacleProvider::from_runner(
        Box::new(Scripted::new(
            name,
            LlmCapabilities::STREAMING | LlmCapabilities::FUNCTION_CALLING,
        )),
        "Anthropic API (scripted)",
    )
}

/// A Claude-Code-shaped tier or account, by name.
fn claude_like(name: &'static str) -> EmbacleProvider {
    EmbacleProvider::from_runner(
        Box::new(Scripted::new(name, LlmCapabilities::STREAMING)),
        "Claude Code (scripted)",
    )
}

/// The names of a chain's tiers, head first.
fn tier_names(chain: &ChatProvider) -> Vec<&'static str> {
    let mut names = vec![chain.name()];
    let mut tail = chain.fallback_tail();
    while let Some(tier) = tail {
        names.push(tier.name());
        tail = tier.fallback_tail();
    }
    names
}

#[test]
fn every_tier_built_assembles_in_order() {
    let chain = ChatProvider::assemble_runtime_chain(vec![
        ok(LlmProviderType::CopilotHeadless, copilot_like()),
        ok(LlmProviderType::Cohere, cohere_like()),
        ok(LlmProviderType::Gemini, gemini_like()),
    ])
    .unwrap();

    assert_eq!(tier_names(&chain), ["copilot_headless", "cohere", "gemini"]);
}

#[test]
fn four_tiers_assemble_four_deep() {
    let chain = ChatProvider::assemble_runtime_chain(vec![
        ok(LlmProviderType::Anthropic, anthropic_like("anthropic")),
        ok(LlmProviderType::CopilotHeadless, copilot_like()),
        ok(LlmProviderType::ClaudeCode, claude_like("claude-code")),
        ok(LlmProviderType::Gemini, gemini_like()),
    ])
    .unwrap();

    assert_eq!(
        tier_names(&chain),
        ["anthropic", "copilot_headless", "claude-code", "gemini"],
        "the ordered list has no slot limit"
    );
    assert!(
        chain.capabilities().supports_function_calling(),
        "an Anthropic head routes the turn to the API loop"
    );
}

#[test]
fn a_failed_middle_tier_lets_the_next_one_move_up() {
    let chain = ChatProvider::assemble_runtime_chain(vec![
        ok(LlmProviderType::CopilotHeadless, copilot_like()),
        failed(LlmProviderType::Cohere, "COHERE_API_KEY unset"),
        ok(LlmProviderType::Gemini, gemini_like()),
    ])
    .unwrap();

    assert_eq!(tier_names(&chain), ["copilot_headless", "gemini"]);
}

#[test]
fn a_failed_tail_runs_the_primary_alone() {
    let chain = ChatProvider::assemble_runtime_chain(vec![
        ok(LlmProviderType::CopilotHeadless, copilot_like()),
        failed(LlmProviderType::Cohere, "COHERE_API_KEY unset"),
        failed(LlmProviderType::Gemini, "GEMINI_API_KEY unset"),
    ])
    .unwrap();

    assert_eq!(chain.name(), "copilot_headless");
    assert!(
        chain.fallback_tail().is_none(),
        "with no tail tier the primary is solo, not a chain of one"
    );
}

#[test]
fn a_failed_primary_runs_the_tail_alone() {
    let chain = ChatProvider::assemble_runtime_chain(vec![
        failed(LlmProviderType::CopilotHeadless, "copilot binary missing"),
        ok(LlmProviderType::Cohere, cohere_like()),
        ok(LlmProviderType::Gemini, gemini_like()),
    ])
    .unwrap();

    assert_eq!(
        tier_names(&chain),
        ["cohere", "gemini"],
        "the second tier leads when the primary is absent"
    );
}

#[test]
fn every_tier_failing_is_a_config_error_naming_each() {
    let err = ChatProvider::assemble_runtime_chain(vec![
        failed(LlmProviderType::CopilotHeadless, "copilot binary missing"),
        failed(LlmProviderType::Cohere, "COHERE_API_KEY unset"),
    ])
    .expect_err("nothing built, nothing to serve");

    assert_eq!(err.code, ErrorCode::ConfigError);
    assert!(
        err.message.contains("copilot binary missing") && err.message.contains("COHERE_API_KEY"),
        "both reasons reach the operator: {}",
        err.message
    );
}

#[test]
fn a_single_failing_tier_returns_its_own_error() {
    let err = ChatProvider::assemble_runtime_chain(vec![ChainTier {
        kind: LlmProviderType::Gemini,
        built: Err(AppError::auth_invalid("GEMINI_API_KEY rejected")),
        accounts: Vec::new(),
    }])
    .expect_err("the only tier failed");
    assert_eq!(
        err.code,
        ErrorCode::AuthInvalid,
        "an unset chain must fail exactly as a lone provider always did"
    );
}

#[test]
fn a_single_entry_is_that_provider_unchained() {
    let chain =
        ChatProvider::assemble_runtime_chain(vec![ok(LlmProviderType::Cohere, cohere_like())])
            .unwrap();
    assert_eq!(chain.name(), "cohere");
    assert!(chain.fallback_tail().is_none());
}

// ---------------------------------------------------------------------------
// Pooled tiers: the accounts follow their provider wherever it sits
// ---------------------------------------------------------------------------

#[test]
fn a_pooled_primarys_accounts_sit_between_it_and_the_fallback() {
    let chain = ChatProvider::assemble_runtime_chain(vec![
        pooled(
            LlmProviderType::Anthropic,
            anthropic_like("anthropic"),
            vec![anthropic_like("anthropic#2")],
        ),
        ok(LlmProviderType::CopilotHeadless, copilot_like()),
    ])
    .unwrap();

    assert_eq!(
        tier_names(&chain),
        ["anthropic", "anthropic#2", "copilot_headless"],
        "account 2 is asked before the chain leaves Anthropic"
    );
}

#[test]
fn a_pooled_primary_alone_still_pools() {
    let chain = ChatProvider::assemble_runtime_chain(vec![pooled(
        LlmProviderType::Anthropic,
        anthropic_like("anthropic"),
        vec![anthropic_like("anthropic#2")],
    )])
    .unwrap();

    assert_eq!(tier_names(&chain), ["anthropic", "anthropic#2"]);
}

#[test]
fn a_fallback_tiers_accounts_sit_right_behind_it() {
    let chain = ChatProvider::assemble_runtime_chain(vec![
        ok(LlmProviderType::CopilotSdk, copilot_like()),
        pooled(
            LlmProviderType::ClaudeCode,
            claude_like("claude-code"),
            vec![claude_like("claude-code#2"), claude_like("claude-code#3")],
        ),
        ok(LlmProviderType::Gemini, gemini_like()),
    ])
    .unwrap();

    assert_eq!(
        tier_names(&chain),
        [
            "copilot_headless",
            "claude-code",
            "claude-code#2",
            "claude-code#3",
            "gemini"
        ],
        "every Claude account is asked before the chain leaves Claude"
    );
}

#[test]
fn the_last_tiers_accounts_end_the_chain() {
    let chain = ChatProvider::assemble_runtime_chain(vec![
        ok(LlmProviderType::CopilotSdk, copilot_like()),
        ok(LlmProviderType::Gemini, gemini_like()),
        pooled(
            LlmProviderType::ClaudeCode,
            claude_like("claude-code"),
            vec![claude_like("claude-code#2")],
        ),
    ])
    .unwrap();

    assert_eq!(
        tier_names(&chain),
        ["copilot_headless", "gemini", "claude-code", "claude-code#2"]
    );
}

#[test]
fn a_tier_that_did_not_build_takes_its_accounts_with_it() {
    let chain = ChatProvider::assemble_runtime_chain(vec![
        ok(LlmProviderType::CopilotSdk, copilot_like()),
        ChainTier {
            kind: LlmProviderType::ClaudeCode,
            built: Err(AppError::config("claude binary missing")),
            accounts: vec![claude_like("claude-code#2")],
        },
        ok(LlmProviderType::Gemini, gemini_like()),
    ])
    .unwrap();

    assert_eq!(tier_names(&chain), ["copilot_headless", "gemini"]);
}

/// A tier whose account is out of quota: every call is refused.
struct OutOfQuota {
    name: &'static str,
    models: Vec<String>,
}

#[async_trait]
impl EmbacleLlmProvider for OutOfQuota {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &str {
        self.name
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::STREAMING | LlmCapabilities::FUNCTION_CALLING
    }

    fn default_model(&self) -> &str {
        &self.models[0]
    }

    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
        Err(RunnerError::rate_limit(
            self.name,
            "credit balance exhausted: Your credit balance is too low",
        ))
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, RunnerError> {
        Err(RunnerError::internal("scripted runners do not stream"))
    }

    async fn health_check(&self) -> Result<bool, RunnerError> {
        Ok(true)
    }
}

fn out_of_quota(name: &'static str) -> EmbacleProvider {
    EmbacleProvider::from_runner(
        Box::new(OutOfQuota {
            name,
            models: vec![format!("{name}-model")],
        }),
        "Out of quota (scripted)",
    )
}

fn ping() -> ChatRequest {
    ChatRequest::new(vec![pierre_llm::ChatMessage::user("ping")])
}

/// The full chain, every tail chain and the tool loops' head-only calls keep
/// their quota cooldowns in the one book the chain was built with: a refusal
/// recorded by any of them is honoured by all of them.
#[tokio::test]
async fn every_chain_over_the_tiers_shares_the_one_cooldown_book() {
    let book = QuotaCooldowns::new(Duration::from_secs(60));
    let chain = EmbacleProvider::chain(
        vec![cohere_like(), out_of_quota("anthropic"), gemini_like()],
        book.clone(),
    )
    .unwrap();
    let tail = chain.fallback_tail().expect("a chain has a tail");
    assert!(
        tail.quota_cooldowns().is_some(),
        "the tail carries the chain's book for the tool loops to read"
    );

    // The tail chain's own walk records the refusal in the shared book…
    let served = tail.complete(&ping()).await.unwrap();
    assert_eq!(served.content, "from gemini");
    assert!(
        book.is_cooling("anthropic"),
        "a refusal inside the tail chain lands in the book the chain was built with"
    );

    // …and a window started outside any chain is honoured by the full chain,
    // which then passes over both cooling tiers.
    book.start("cohere");
    let served = chain.complete(&ping()).await.unwrap();
    assert_eq!(
        served.content, "from gemini",
        "the full chain skips the head cooled from outside and the tier the tail cooled"
    );
}
