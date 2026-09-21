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
//! Set `PIERRE_LLM_PROVIDER` environment variable:
//! - `gemini` (default): Google Gemini for full-featured capabilities
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
//! [`EmbacleProvider::from_provider_type`]. With
//! `PIERRE_LLM_RUNTIME_FALLBACK=true` the primary is chained in front of
//! `PIERRE_LLM_FALLBACK_PROVIDER` and, when set, `PIERRE_LLM_TERTIARY_PROVIDER`
//! by [`EmbacleProvider::chain`].

use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use super::{
    ChatRequest, ChatResponse, ChatResponseWithTools, ChatStream, EmbacleProvider, LlmCapabilities,
    LlmProvider, Tool,
};
use crate::config::LlmProviderType;
use crate::errors::AppError;
use crate::health::TierProbe;
use crate::model_check::validate_model_for_provider;
use crate::tool_bridge::{with_tool_defs, with_tools_response};

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

/// The tiers a runtime chain is assembled from, each as it came out of
/// construction: `Ok` is a tier, `Err` is the reason a configured tier is
/// missing (logged, never fatal while another tier built).
pub struct ChainTiers {
    /// `PIERRE_LLM_PROVIDER`'s provider.
    pub primary: Result<EmbacleProvider, AppError>,
    /// `PIERRE_LLM_FALLBACK_PROVIDER`'s provider.
    pub secondary: Result<EmbacleProvider, AppError>,
    /// `PIERRE_LLM_TERTIARY_PROVIDER`'s provider, when one is configured that
    /// differs from both other tiers.
    pub tertiary: Option<Result<EmbacleProvider, AppError>>,
}

impl ChatProvider {
    /// Create a provider from environment configuration
    ///
    /// Reads `PIERRE_LLM_PROVIDER` to determine which provider to use (see
    /// the module docs for the values).
    ///
    /// When `PIERRE_LLM_FALLBACK_ENABLED=true`, if the primary provider fails,
    /// attempts to use the fallback provider specified by `PIERRE_LLM_FALLBACK_PROVIDER`.
    ///
    /// # Errors
    ///
    /// Returns an error if the required API key environment variable is missing
    /// (for cloud providers) or if the local server cannot be reached, and
    /// fallback is disabled or also fails.
    pub async fn from_env() -> Result<Self, AppError> {
        let provider_type = LlmProviderType::from_env();

        info!(
            "Initializing LLM provider: {} (set {} to change)",
            provider_type,
            LlmProviderType::ENV_VAR
        );

        let primary_result = EmbacleProvider::from_provider_type(provider_type, None).await;

        if LlmProviderType::is_runtime_fallback_enabled() {
            return Self::build_runtime_chain(primary_result, provider_type).await;
        }

        Self::finalize_or_fallback(primary_result, provider_type).await
    }

    /// Build the runtime chain when `PIERRE_LLM_RUNTIME_FALLBACK=true`.
    ///
    /// Requires `PIERRE_LLM_FALLBACK_PROVIDER` to be set. If the primary
    /// fails to initialize, the chain collapses to a fallback-only provider
    /// (same effective result as the init-time fallback path). If the
    /// fallback is missing or fails to initialize, the chain collapses to
    /// primary-only so callers always get a usable provider when at least
    /// one side worked.
    async fn build_runtime_chain(
        primary_result: Result<EmbacleProvider, AppError>,
        primary_type: LlmProviderType,
    ) -> Result<Self, AppError> {
        let fallback_type = LlmProviderType::fallback_provider_from_env();

        let Some(fallback_type) = fallback_type else {
            warn!(
                "{} is true but {} is unset — runtime fallback disabled",
                LlmProviderType::RUNTIME_FALLBACK_ENV_VAR,
                LlmProviderType::FALLBACK_PROVIDER_ENV_VAR,
            );
            return Self::finalize_or_fallback(primary_result, primary_type).await;
        };

        if fallback_type == primary_type {
            warn!(
                "{} matches {} ({primary_type}) — runtime fallback disabled",
                LlmProviderType::FALLBACK_PROVIDER_ENV_VAR,
                LlmProviderType::ENV_VAR,
            );
            return Self::finalize_or_fallback(primary_result, primary_type).await;
        }

        // The secondary and tertiary take their own model overrides so each
        // tier can target a different model namespace (Copilot's
        // `claude-opus-4.8`, Cohere's `command-a-03-2025`, Gemini's
        // `gemini-flash-lite-latest`); the primary's `PIERRE_LLM_MODEL` would
        // 404 on the other vendors' APIs.
        let secondary_result = EmbacleProvider::from_provider_type(
            fallback_type,
            LlmProviderType::fallback_provider_model_from_env().as_deref(),
        )
        .await;

        let tertiary = match Self::tertiary_type(primary_type, fallback_type) {
            Some(tertiary_type) => Some(
                EmbacleProvider::from_provider_type(
                    tertiary_type,
                    LlmProviderType::tertiary_provider_model_from_env().as_deref(),
                )
                .await,
            ),
            None => None,
        };

        Self::assemble_runtime_chain(
            ChainTiers {
                primary: primary_result,
                secondary: secondary_result,
                tertiary,
            },
            primary_type,
            fallback_type,
        )
    }

    /// The tertiary provider type, when [`LlmProviderType::TERTIARY_PROVIDER_ENV_VAR`]
    /// is set and resolves to a different type than both primary and
    /// secondary (so the chain does not degenerate). `None` otherwise — the
    /// chain stays two-tier.
    fn tertiary_type(
        primary_type: LlmProviderType,
        fallback_type: LlmProviderType,
    ) -> Option<LlmProviderType> {
        let tertiary_type = LlmProviderType::tertiary_provider_from_env()?;

        if tertiary_type == primary_type || tertiary_type == fallback_type {
            warn!(
                "{} ({tertiary_type}) matches an existing tier — tertiary fallback disabled",
                LlmProviderType::TERTIARY_PROVIDER_ENV_VAR,
            );
            return None;
        }

        Some(tertiary_type)
    }

    /// Chain whichever tiers built, in order, and log every tier that did not.
    ///
    /// A missing tier is never fatal while another built: the secondary
    /// failing promotes the tertiary, the tertiary failing leaves the two-tier
    /// chain, the primary failing runs fallback-only. Only every tier failing
    /// is an error. The warnings are the operator's only signal that a
    /// configured tier is absent.
    ///
    /// Public so the assembly can be driven with scripted tiers; production
    /// reaches it through [`Self::from_env`].
    ///
    /// # Errors
    ///
    /// Returns a config error when no tier initialized.
    pub fn assemble_runtime_chain(
        tiers: ChainTiers,
        primary_type: LlmProviderType,
        fallback_type: LlmProviderType,
    ) -> Result<Self, AppError> {
        let ChainTiers {
            primary,
            secondary,
            tertiary,
        } = tiers;

        let tail = Self::tail_tiers(secondary, tertiary, fallback_type);
        let tiers = match (primary, tail) {
            (Ok(primary), Ok(tail)) => {
                info!(
                    primary = %primary_type,
                    secondary = %fallback_type,
                    tiers = tail.len() + 1,
                    "Runtime LLM fallback chain initialized"
                );
                let mut tiers = Vec::with_capacity(tail.len() + 1);
                tiers.push(primary);
                tiers.extend(tail);
                tiers
            }
            (Ok(primary), Err(secondary_err)) => {
                warn!(
                    primary = %primary_type,
                    secondary = %fallback_type,
                    error = %secondary_err,
                    "Fallback provider failed to initialize; running primary-only"
                );
                vec![primary]
            }
            (Err(primary_err), Ok(tail)) => {
                warn!(
                    primary = %primary_type,
                    secondary = %fallback_type,
                    error = %primary_err,
                    "Primary provider failed to initialize; running fallback-only"
                );
                tail
            }
            (Err(primary_err), Err(secondary_err)) => {
                return Err(AppError::config(format!(
                    "Both runtime-chain providers failed. Primary ({primary_type}): {primary_err}. \
                     Secondary ({fallback_type}): {secondary_err}"
                )))
            }
        };

        for tier in &tiers {
            validate_model_for_provider(tier);
        }
        Ok(Self::Embacle(EmbacleProvider::chain(tiers)?))
    }

    /// The tiers behind the primary: the secondary, then the tertiary when it
    /// built. `Err` when neither built (the secondary's error, as the one the
    /// operator configured first).
    fn tail_tiers(
        secondary: Result<EmbacleProvider, AppError>,
        tertiary: Option<Result<EmbacleProvider, AppError>>,
        fallback_type: LlmProviderType,
    ) -> Result<Vec<EmbacleProvider>, AppError> {
        match tertiary {
            None => secondary.map(|secondary| vec![secondary]),
            Some(Ok(tertiary)) => Ok(Self::tail_with_tertiary(secondary, tertiary, fallback_type)),
            Some(Err(tertiary_err)) => {
                Self::tail_without_tertiary(secondary, &tertiary_err, fallback_type)
            }
        }
    }

    /// The tertiary built: behind the secondary when that built too, in its
    /// place otherwise.
    fn tail_with_tertiary(
        secondary: Result<EmbacleProvider, AppError>,
        tertiary: EmbacleProvider,
        fallback_type: LlmProviderType,
    ) -> Vec<EmbacleProvider> {
        match secondary {
            Ok(secondary) => {
                info!(
                    secondary = %fallback_type,
                    "Tertiary LLM fallback initialized; chain is primary -> secondary -> tertiary"
                );
                vec![secondary, tertiary]
            }
            Err(secondary_err) => {
                warn!(
                    secondary = %fallback_type,
                    error = %secondary_err,
                    "Secondary provider failed to initialize; promoting tertiary"
                );
                vec![tertiary]
            }
        }
    }

    /// The tertiary did not build: the secondary alone, or nothing.
    fn tail_without_tertiary(
        secondary: Result<EmbacleProvider, AppError>,
        tertiary_err: &AppError,
        fallback_type: LlmProviderType,
    ) -> Result<Vec<EmbacleProvider>, AppError> {
        match secondary {
            Ok(secondary) => {
                warn!(
                    secondary = %fallback_type,
                    error = %tertiary_err,
                    "Tertiary provider failed to initialize; running two-tier chain"
                );
                Ok(vec![secondary])
            }
            Err(secondary_err) => {
                warn!(
                    secondary = %fallback_type,
                    secondary_error = %secondary_err,
                    tertiary_error = %tertiary_err,
                    "Both secondary and tertiary failed; falling back to primary-only"
                );
                Err(secondary_err)
            }
        }
    }

    /// Finalize provider initialization or attempt fallback on failure
    async fn finalize_or_fallback(
        result: Result<EmbacleProvider, AppError>,
        provider_type: LlmProviderType,
    ) -> Result<Self, AppError> {
        match result {
            Ok(provider) => {
                debug!(
                    "Provider {} initialized with model: {}",
                    provider.display_name(),
                    provider.default_model()
                );
                validate_model_for_provider(&provider);
                Ok(Self::Embacle(provider))
            }
            Err(primary_error) => Self::try_fallback(provider_type, primary_error).await,
        }
    }

    /// Attempt to initialize a fallback provider after primary fails
    async fn try_fallback(
        primary_type: LlmProviderType,
        primary_error: AppError,
    ) -> Result<Self, AppError> {
        let fallback_enabled = LlmProviderType::is_fallback_enabled();
        let fallback_provider = LlmProviderType::fallback_provider_from_env();

        let Some(fallback) = fallback_provider else {
            return Err(primary_error);
        };

        if !fallback_enabled || fallback == primary_type {
            return Err(primary_error);
        }

        let wait_secs = LlmProviderType::fallback_wait_secs();
        info!(
            "Primary provider {} failed, waiting {}s before fallback to {}",
            primary_type, wait_secs, fallback
        );

        sleep(Duration::from_secs(wait_secs)).await;

        match EmbacleProvider::from_provider_type(fallback, None).await {
            Ok(provider) => {
                info!(
                    "Fallback provider {} initialized with model: {}",
                    provider.display_name(),
                    provider.default_model()
                );
                Ok(Self::Embacle(provider))
            }
            Err(fallback_error) => Err(AppError::config(format!(
                "Both primary ({primary_type}) and fallback ({fallback}) providers failed. \
                Primary: {primary_error}. Fallback: {fallback_error}"
            ))),
        }
    }

    /// Check if this provider supports tool calling
    #[must_use]
    pub fn supports_tool_calling(&self) -> bool {
        self.capabilities().supports_function_calling()
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
    /// The headless tool loop bypasses the chain to converse with the head's
    /// turn provider, so the chain's own fallback never fires for SDK
    /// tool-calling turns. Exposing the tail lets the tool loop re-run the
    /// turn against it when the head fails with a provider fault.
    #[must_use]
    pub fn fallback_tail(&self) -> Option<&Self> {
        match self {
            Self::Embacle(p) => p.fallback_tail(),
            Self::Custom(_) => None,
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
