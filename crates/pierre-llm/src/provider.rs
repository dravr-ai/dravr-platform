// ABOUTME: Unified LLM provider selector for runtime provider switching
// ABOUTME: Every production provider is an embacle runner behind EmbacleProvider; Custom injects a test double
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM Provider Selector
//!
//! This module provides a unified interface for LLM providers that can be
//! configured at runtime via environment variables.
//!
//! ## Configuration
//!
//! `PIERRE_LLM_CHAIN` lists the providers in order, each `provider` or
//! `provider=model` (see [`LlmChain`] for the grammar); the first is the
//! primary. The provider names:
//! - `gemini` (default): Google Gemini for full-featured capabilities
//! - `anthropic`: the Anthropic Messages API (Claude Haiku / Sonnet / Opus)
//! - `groq`: Groq for cost-effective open-source models
//! - `cohere`: Cohere Command A / Command R
//! - `openrouter`: `OpenRouter` as a unified gateway to 200+ models
//! - `local`/`ollama`/`vllm`/`localai`: a local `OpenAI`-compatible endpoint
//! - `claude_code`/`copilot`/`cursor_agent`/`opencode`/`warp_cli`/…: a CLI subprocess runner
//! - `copilot_headless`/`copilot_sdk`: a Copilot turn provider (ACP or the Rust SDK)
//! - `openai_api`/`openai`: an `OpenAI`-compatible HTTP API
//! - `router`: the quota router (Claude Code in front, Copilot Headless behind)
//!
//! Every one of them is an embacle runner, constructed by
//! [`EmbacleProvider::from_provider_type`], and the tiers that built are
//! chained in order by [`EmbacleProvider::chain`].

use embacle::QuotaCooldowns;
use std::fmt;
use std::sync::Arc;
use tracing::{error, info, warn};

use super::{
    ChatRequest, ChatResponse, ChatResponseWithTools, ChatStream, EmbacleProvider, LlmCapabilities,
    LlmProvider, Tool,
};
use crate::config::{LlmChain, LlmProviderType};
use crate::errors::AppError;
use crate::health::TierProbe;
use crate::model_check::validate_model_for_provider;
use crate::stage::{LlmStage, StageModels};
use crate::tool_bridge::{with_tool_defs, with_tools_response};

/// Name every retired chain variable the environment still sets.
///
/// With `PIERRE_LLM_CHAIN` unset they are an operator's chain the process is
/// ignoring — a binary deployed before its environment moved to the chain, or
/// a stale `.envrc` — and the platform default (Gemini alone) serves every
/// turn, so it is an error. With the chain set they are only inert.
fn report_retired_variables(chain_configured: bool, retired: &[&'static str]) {
    if retired.is_empty() {
        return;
    }
    if chain_configured {
        warn!(
            retired = ?retired,
            "Retired LLM chain variables are set and ignored; {} decides the chain. Remove them.",
            LlmChain::ENV_VAR
        );
    } else {
        error!(
            retired = ?retired,
            "Retired LLM chain variables are set but {} is not: they are ignored, and the \
             platform default (Gemini alone, no tier behind it) serves every turn. Set {}.",
            LlmChain::ENV_VAR,
            LlmChain::ENV_VAR
        );
    }
}

/// Unified chat provider.
///
/// This enum provides a consistent interface regardless of which
/// underlying provider is configured.
pub enum ChatProvider {
    /// Any embacle runner: HTTP provider, CLI runner, Copilot turn provider,
    /// quota router, or a fallback chain over them.
    Embacle(EmbacleProvider),
    /// Custom provider supplied by the caller (used by tests to inject a
    /// deterministic mock). Never constructed in production code paths —
    /// production providers are resolved via [`ChatProvider::from_env`] or
    /// the per-tenant credential factory.
    Custom(Arc<dyn LlmProvider>),
}

/// One configured tier of the chain as it came out of construction: `built`
/// is `Ok` with the provider, or `Err` with the reason it is missing (logged,
/// never fatal while another tier built).
pub struct ChainTier {
    /// The provider the chain entry names.
    pub kind: LlmProviderType,
    /// The provider, or why it did not build.
    pub built: Result<EmbacleProvider, AppError>,
    /// The provider's further accounts, in order (`CLAUDE_CODE_OAUTH_TOKEN_2`,
    /// `ANTHROPIC_API_KEY_2`, …), each its own tier right behind it — a spent
    /// account moves the turn to the next account before the chain leaves
    /// the provider, wherever the provider sits. They go where the tier goes:
    /// a tier that did not build takes its accounts with it, the binary or
    /// the shared settings being the same ones.
    pub accounts: Vec<EmbacleProvider>,
}

impl ChatProvider {
    /// Create the provider `PIERRE_LLM_CHAIN` configures.
    ///
    /// Every entry is built in order, with the model it names and its pooled
    /// accounts right behind it, and the tiers that built are chained
    /// ([`Self::assemble_runtime_chain`]). Unset, the chain is one Gemini
    /// entry on `PIERRE_LLM_DEFAULT_MODEL`.
    ///
    /// The provider it returns is the platform's head, so it is also the one
    /// that carries the background stages' models ([`crate::stage`]), resolved
    /// here once for whichever kind the head turned out to be.
    ///
    /// # Errors
    ///
    /// Returns an error when no tier initialized: the one tier's own error
    /// for a single-entry chain, a config error naming every tier otherwise.
    pub async fn from_env() -> Result<Self, AppError> {
        let chain = LlmChain::from_env();
        for issue in chain.issues() {
            warn!(
                variable = LlmChain::ENV_VAR,
                "LLM chain configuration: {issue}"
            );
        }
        report_retired_variables(
            LlmChain::is_configured(),
            &LlmChain::retired_variables_set(),
        );
        info!(
            chain = %chain,
            "Initializing LLM provider chain (set {} to change)",
            LlmChain::ENV_VAR
        );

        let mut tiers = Vec::with_capacity(chain.entries().len());
        for entry in chain.entries() {
            let model = entry.model.as_deref();
            tiers.push(ChainTier {
                kind: entry.provider,
                built: EmbacleProvider::from_provider_type(entry.provider, model).await,
                accounts: EmbacleProvider::pooled_accounts(entry.provider, model),
            });
        }
        Ok(Self::assemble_runtime_chain(tiers)?.with_stage_models_from_env())
    }

    /// This provider with its background stages routed per [`StageModels::from_env`],
    /// for the kind its head was built as and held to the models that head
    /// publishes. A primary that failed to build leaves a fallback at the head,
    /// and the stages follow that kind.
    fn with_stage_models_from_env(self) -> Self {
        match self {
            Self::Embacle(provider) => {
                let stage_models = StageModels::from_env(provider.kind());
                Self::Embacle(provider.with_stage_models(stage_models))
            }
            Self::Custom(provider) => Self::Custom(provider),
        }
    }

    /// The model `stage` runs on through this provider, or `None` for its own.
    ///
    /// Only the platform's head carries stage models; a tenant's own provider
    /// and a test double answer `None` for every stage.
    #[must_use]
    pub fn stage_model(&self, stage: LlmStage) -> Option<&str> {
        match self {
            Self::Embacle(provider) => provider.stage_model(stage),
            Self::Custom(_) => None,
        }
    }

    /// `request`, routed to `stage`'s model when this provider routes it, and
    /// untouched otherwise.
    #[must_use]
    pub fn routed(&self, stage: LlmStage, mut request: ChatRequest) -> ChatRequest {
        if let Some(model) = self.stage_model(stage) {
            request.model = Some(model.to_owned());
        }
        request
    }

    /// Chain whichever tiers built, in order, each followed by its accounts,
    /// and log every tier that did not.
    ///
    /// A missing tier is never fatal while another built: the tiers behind it
    /// move up. Only every tier failing is an error. The warnings are the
    /// operator's only signal that a configured tier is absent.
    ///
    /// Public so the assembly can be driven with scripted tiers; production
    /// reaches it through [`Self::from_env`].
    ///
    /// # Errors
    ///
    /// Returns an error when no tier initialized: a single tier's own error
    /// unchanged, else a config error carrying every tier's reason.
    pub fn assemble_runtime_chain(tiers: Vec<ChainTier>) -> Result<Self, AppError> {
        let configured = tiers.len();
        let mut built = Vec::new();
        let mut failures = Vec::new();
        for ChainTier {
            kind,
            built: result,
            accounts,
        } in tiers
        {
            match result {
                Ok(tier) => {
                    built.push(tier);
                    built.extend(accounts);
                }
                Err(err) => {
                    warn!(
                        provider = %kind,
                        error = %err,
                        "LLM chain tier failed to initialize; the tiers behind it move up"
                    );
                    failures.push((kind, err));
                }
            }
        }

        if built.is_empty() {
            if failures.len() == 1 {
                if let Some((_, only)) = failures.pop() {
                    return Err(only);
                }
            }
            let reasons = failures
                .iter()
                .map(|(kind, err)| format!("{kind}: {err}"))
                .collect::<Vec<_>>()
                .join(". ");
            return Err(AppError::config(format!(
                "Every LLM chain tier failed to initialize. {reasons}"
            )));
        }

        for tier in &built {
            validate_model_for_provider(tier);
        }
        if built.len() > 1 {
            info!(
                tiers = ?built.iter().map(LlmProvider::name).collect::<Vec<_>>(),
                configured,
                missing = failures.len(),
                "Runtime LLM fallback chain initialized"
            );
        }
        Ok(Self::Embacle(EmbacleProvider::chain(
            built,
            QuotaCooldowns::default(),
        )?))
    }

    /// The inner [`EmbacleProvider`], when this is one.
    ///
    /// The headless tool loop reaches through it to the Copilot turn provider
    /// (the head of a chain), which it converses with directly.
    #[must_use]
    pub const fn as_embacle_provider(&self) -> Option<&EmbacleProvider> {
        match self {
            Self::Embacle(p) => Some(p),
            Self::Custom(_) => None,
        }
    }

    /// The runtime-fallback chain minus its head, when this is a chain.
    ///
    /// The tool loops bypass the chain to converse with its head (see
    /// [`Self::complete_with_tools_on_head`]), so the chain's own fallback
    /// never fires for tool-calling turns. Exposing the tail lets a tool loop
    /// re-run the turn against it when the head fails with a provider fault.
    #[must_use]
    pub fn fallback_tail(&self) -> Option<&Self> {
        match self {
            Self::Embacle(p) => p.fallback_tail(),
            Self::Custom(_) => None,
        }
    }

    /// The quota cooldown book the runtime-fallback chain shares, when this
    /// is one (see [`EmbacleProvider::chain`]).
    #[must_use]
    pub const fn quota_cooldowns(&self) -> Option<&QuotaCooldowns> {
        match self {
            Self::Embacle(p) => p.quota_cooldowns(),
            Self::Custom(_) => None,
        }
    }

    /// The chain's head on its own, when this is a chain; `None` when it is
    /// not, the provider being its own head already.
    ///
    /// A turn is routed by the head's capabilities, so the tool loop it takes
    /// is the head's. Sending that loop's request through the whole chain
    /// would let a fault hand it to a tier that cannot honour it — a
    /// tool-carrying request reaching a Copilot runtime with no MCP servers
    /// comes back as an ungrounded reply. The tool loops converse with the
    /// head alone and, on a provider fault, re-run the turn against
    /// [`Self::fallback_tail`], dispatched on the tail's own capabilities.
    #[must_use]
    fn chain_head(&self) -> Option<Self> {
        match self {
            Self::Embacle(p) if p.fallback_tail().is_some() => Some(Self::Embacle(p.head_alone())),
            Self::Embacle(_) | Self::Custom(_) => None,
        }
    }

    /// Every tier of the runtime-fallback chain as a standalone provider, in
    /// chain order. Empty when this is not a chain: a solo provider is already
    /// what a call through it exercises.
    #[must_use]
    pub fn chain_tiers(&self) -> Vec<Self> {
        let mut tiers = Vec::new();
        if self.fallback_tail().is_none() {
            return tiers;
        }
        let mut current = Some(self);
        while let Some(Self::Embacle(chain)) = current {
            tiers.push(Self::Embacle(chain.head_alone()));
            current = chain.fallback_tail();
        }
        tiers
    }

    /// Whether some tier that may serve a tool turn takes its tools over MCP
    /// rather than as structured declarations on the request.
    ///
    /// A turn is routed by the head's capabilities, but when the head faults
    /// the tool loops re-run it on [`Self::fallback_tail`], each tier routed
    /// by its own: a Copilot runtime then calls the turn's tools only through
    /// the MCP servers the turn opened, and a CLI runner prefers them to its
    /// text catalogue. Asked of the head alone, an Anthropic head in front of
    /// a Copilot tier would answer no, the turn would open no servers, and the
    /// Copilot tier would answer with no tools at all.
    #[must_use]
    pub fn needs_mcp_servers(&self) -> bool {
        let tiers = self.chain_tiers();
        if tiers.is_empty() {
            return !self.capabilities().supports_function_calling();
        }
        tiers
            .iter()
            .any(|tier| !tier.capabilities().supports_function_calling())
    }

    /// Ask every tier of the chain, alone and in order, for one completion.
    ///
    /// One billed round-trip per tier, so this belongs at startup and nowhere
    /// periodic. An empty completion counts as a failure, as it does inside the
    /// chain, where the strict policy passes such a tier over. Empty when this
    /// is not a chain.
    pub async fn probe_chain_tiers(&self, request: &ChatRequest) -> Vec<TierProbe> {
        let mut probes = Vec::new();
        for (position, tier) in self.chain_tiers().iter().enumerate() {
            let outcome = match tier.complete(request).await {
                Ok(response) if response.content.trim().is_empty() => {
                    Err("empty completion".to_owned())
                }
                Ok(response) => Ok(response.model),
                Err(e) => Err(e.to_string()),
            };
            probes.push(TierProbe {
                provider: tier.name(),
                position,
                outcome,
            });
        }
        probes
    }

    /// Perform a chat completion with tool/function calling support
    ///
    /// Every provider takes its tools on the request and reports calls on the
    /// response (see `tool_bridge`); capability is the provider's answer, not
    /// this enum's, so a provider that advertises `FUNCTION_CALLING` is asked
    /// and expected to answer.
    ///
    /// Tool-calling always uses non-streaming mode. Streaming tool-call accumulation
    /// is complex and provides negligible UX benefit for short tool-call payloads.
    /// If `request.stream` is true, it is ignored for tool-calling requests.
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails.
    pub async fn complete_with_tools(
        &self,
        request: &ChatRequest,
        tools: Option<Vec<Tool>>,
    ) -> Result<ChatResponseWithTools, AppError> {
        let forwarded = with_tool_defs(request, tools);
        let response = self.inner().complete(&forwarded).await?;
        Ok(with_tools_response(response))
    }

    /// [`Self::complete_with_tools`] against the chain's head alone, outside
    /// the chain's fallback: the head's own error comes back instead of
    /// another tier's answer to a request it could not serve. A turn is
    /// routed by the head's capabilities, and a tool-carrying request a fault
    /// handed to a Copilot runtime with no MCP servers would come back as an
    /// ungrounded reply; the tool loops re-run a faulted turn against
    /// [`Self::fallback_tail`] instead, dispatched on the tail's own
    /// capabilities. The same call as [`Self::complete_with_tools`] when this
    /// is not a chain.
    ///
    /// # Errors
    ///
    /// Returns the head's error when its call fails.
    pub async fn complete_with_tools_on_head(
        &self,
        request: &ChatRequest,
        tools: Option<Vec<Tool>>,
    ) -> Result<ChatResponseWithTools, AppError> {
        match self.chain_head() {
            Some(head) => head.complete_with_tools(request, tools).await,
            None => self.complete_with_tools(request, tools).await,
        }
    }

    /// The provider behind either variant.
    fn inner(&self) -> &dyn LlmProvider {
        match self {
            Self::Embacle(p) => p,
            Self::Custom(p) => p.as_ref(),
        }
    }
}

// Delegate LlmProvider trait methods to the underlying provider.
// The canonical delegation lives in the LlmProvider trait impl below.
// These inherent methods delegate to it so callers don't need to import the trait.
impl ChatProvider {
    /// Get provider name
    #[must_use]
    pub fn name(&self) -> &'static str {
        LlmProvider::name(self)
    }

    /// Get provider display name
    #[must_use]
    pub fn display_name(&self) -> &'static str {
        LlmProvider::display_name(self)
    }

    /// Get provider capabilities
    #[must_use]
    pub fn capabilities(&self) -> LlmCapabilities {
        LlmProvider::capabilities(self)
    }

    /// Get default model
    #[must_use]
    pub fn default_model(&self) -> &str {
        LlmProvider::default_model(self)
    }

    /// Get available models
    #[must_use]
    pub fn available_models(&self) -> &[String] {
        LlmProvider::available_models(self)
    }

    /// Perform a chat completion
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails.
    pub async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        LlmProvider::complete(self, request).await
    }

    /// Perform a streaming chat completion
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails.
    pub async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        LlmProvider::complete_stream(self, request).await
    }

    /// Check provider health
    ///
    /// # Errors
    ///
    /// Returns an error if the health check fails.
    pub async fn health_check(&self) -> Result<bool, AppError> {
        LlmProvider::health_check(self).await
    }
}

impl fmt::Debug for ChatProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Embacle(p) => f
                .debug_tuple("ChatProvider::Embacle")
                .field(&p.name())
                .finish(),
            Self::Custom(p) => f
                .debug_tuple("ChatProvider::Custom")
                .field(&p.name())
                .finish(),
        }
    }
}

// Implement LlmProvider trait for ChatProvider to enable trait object usage
#[async_trait::async_trait]
impl LlmProvider for ChatProvider {
    fn name(&self) -> &'static str {
        self.inner().name()
    }

    fn display_name(&self) -> &'static str {
        self.inner().display_name()
    }

    fn capabilities(&self) -> LlmCapabilities {
        self.inner().capabilities()
    }

    fn default_model(&self) -> &str {
        self.inner().default_model()
    }

    fn available_models(&self) -> &[String] {
        self.inner().available_models()
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.inner().complete(request).await
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        self.inner().complete_stream(request).await
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        self.inner().health_check().await
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use embacle::types::{
        ChatRequest as RunnerRequest, ChatResponse as RunnerResponse, ChatStream as RunnerStream,
        LlmCapabilities as RunnerCapabilities, LlmProvider as RunnerProvider, RunnerError,
    };

    use super::{ChatProvider, EmbacleProvider, QuotaCooldowns};

    /// A tier that is only ever asked what it can do.
    struct Tier {
        name: &'static str,
        capabilities: RunnerCapabilities,
    }

    #[async_trait]
    impl RunnerProvider for Tier {
        fn name(&self) -> &'static str {
            self.name
        }

        fn display_name(&self) -> &str {
            self.name
        }

        fn capabilities(&self) -> RunnerCapabilities {
            self.capabilities
        }

        fn default_model(&self) -> &str {
            self.name
        }

        fn available_models(&self) -> &[String] {
            &[]
        }

        async fn complete(&self, _request: &RunnerRequest) -> Result<RunnerResponse, RunnerError> {
            Err(RunnerError::internal("not asked in this test"))
        }

        async fn complete_stream(
            &self,
            _request: &RunnerRequest,
        ) -> Result<RunnerStream, RunnerError> {
            Err(RunnerError::internal("not asked in this test"))
        }

        async fn health_check(&self) -> Result<bool, RunnerError> {
            Ok(true)
        }
    }

    /// An API tier: tools travel as structured declarations on the request.
    fn api(name: &'static str) -> EmbacleProvider {
        let capabilities =
            RunnerCapabilities::SYSTEM_MESSAGES | RunnerCapabilities::FUNCTION_CALLING;
        EmbacleProvider::from_runner(Box::new(Tier { name, capabilities }), name)
    }

    /// A Copilot-runtime tier: tools reach it only over the turn's MCP servers.
    fn copilot(name: &'static str) -> EmbacleProvider {
        let capabilities =
            RunnerCapabilities::SYSTEM_MESSAGES | RunnerCapabilities::SDK_TOOL_CALLING;
        EmbacleProvider::from_runner(Box::new(Tier { name, capabilities }), name)
    }

    fn chain(tiers: Vec<EmbacleProvider>) -> ChatProvider {
        ChatProvider::Embacle(EmbacleProvider::chain(tiers, QuotaCooldowns::default()).unwrap())
    }

    #[test]
    fn a_copilot_tier_behind_an_api_head_needs_the_mcp_servers() {
        let provider = chain(vec![
            api("anthropic"),
            api("anthropic#2"),
            copilot("copilot_sdk"),
        ]);
        assert!(
            provider.capabilities().supports_function_calling(),
            "the turn is routed by the head, which takes structured tools"
        );
        assert!(
            provider.needs_mcp_servers(),
            "the tail's Copilot tier serves a faulted turn only through the turn's MCP servers"
        );
    }

    #[test]
    fn an_all_api_chain_needs_no_mcp_servers() {
        let provider = chain(vec![api("anthropic"), api("gemini")]);
        assert!(!provider.needs_mcp_servers());
    }

    #[test]
    fn a_chain_offers_its_head_alone_and_a_solo_provider_is_its_own_head() {
        let provider = chain(vec![api("anthropic"), copilot("copilot_sdk")]);
        let head = provider
            .chain_head()
            .expect("a chain has a head to call alone");
        assert_eq!(head.name(), "anthropic");
        assert!(
            head.fallback_tail().is_none(),
            "the head alone has nothing behind it, so a fault comes back as the head's own"
        );

        assert!(ChatProvider::Embacle(api("gemini")).chain_head().is_none());
    }

    #[test]
    fn a_solo_provider_needs_them_only_without_function_calling() {
        assert!(!ChatProvider::Embacle(api("gemini")).needs_mcp_servers());
        assert!(ChatProvider::Embacle(copilot("copilot_sdk")).needs_mcp_servers());
    }
}
