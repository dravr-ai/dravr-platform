// ABOUTME: Builds embacle's HTTP providers (Gemini, Cohere, Groq, OpenRouter, OpenAI-compatible) for the platform
// ABOUTME: From the platform's env names at boot, or from a tenant's stored credentials for the BYO path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Construction of embacle's `http-api` providers.
//!
//! The vendor quirks, retries and SSE plumbing live in embacle; what stays
//! here is the platform's contract with its environment. Every provider gets
//! the shared LLM `reqwest::Client` (one pool, one set of timeouts), goes
//! through [`EmbacleProvider::from_runner`] like every other tier — a vendor
//! 429 reaches the platform as `ExternalRateLimited`, the upstream fault every
//! provider's rate limit or spent quota carries — and the model comes from
//! the explicit override a chain tier was configured with
//! (`PIERRE_LLM_FALLBACK_PROVIDER_MODEL`, `PIERRE_LLM_TERTIARY_PROVIDER_MODEL`)
//! before any vendor default.
//!
//! Gemini's boot-time model comes from `PIERRE_LLM_DEFAULT_MODEL` — the
//! platform's unified model variable — so a Gemini primary keeps the
//! configuration it always had. The other four read their own `*_DEFAULT_MODEL`
//! through embacle's `from_env()`.

use embacle::{
    CohereConfig, CohereProvider, GeminiConfig, GeminiProvider, GroqConfig, GroqProvider,
    OpenAiCompatibleConfig, OpenAiCompatibleProvider, OpenRouterConfig, OpenRouterProvider,
};
use pierre_core::http_client::llm_inner_client;
use tracing::info;

use crate::config::{HttpProvider, LlmModelConfig};
use crate::embacle_provider::EmbacleProvider;
use crate::errors::AppError;
use crate::LlmCapabilities;

/// Build the HTTP provider `kind` names from the process environment.
///
/// `model_override` wins over every env var; it is how a fallback tier gets a
/// model in its own namespace instead of the primary's.
///
/// # Errors
///
/// Returns `AppError` when the provider's API key (or, for Gemini,
/// `PIERRE_LLM_DEFAULT_MODEL`) is unset.
pub(crate) fn build(
    kind: HttpProvider,
    model_override: Option<&str>,
) -> Result<EmbacleProvider, AppError> {
    match kind {
        HttpProvider::Gemini => gemini_from_env(model_override),
        HttpProvider::Groq => groq_from_env(model_override),
        HttpProvider::Cohere => cohere_from_env(model_override),
        HttpProvider::OpenRouter => openrouter_from_env(model_override),
        HttpProvider::Local => Ok(local_from_env(model_override)),
    }
}

/// Gemini on `GEMINI_API_KEY`, on `model_override` else `PIERRE_LLM_DEFAULT_MODEL`.
fn gemini_from_env(model_override: Option<&str>) -> Result<EmbacleProvider, AppError> {
    let model = match model_override {
        Some(model) => model.to_owned(),
        None => {
            LlmModelConfig::from_env()
                .map_err(AppError::config)?
                .default_model
        }
    };
    let config = GeminiConfig::from_env()?.with_model(model);
    info!(model = %config.model, "Creating Gemini provider");
    Ok(EmbacleProvider::from_runner(
        Box::new(GeminiProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        GEMINI_DISPLAY_NAME,
    ))
}

/// Groq on `GROQ_API_KEY` and `GROQ_DEFAULT_MODEL`, overridden by `model_override`.
fn groq_from_env(model_override: Option<&str>) -> Result<EmbacleProvider, AppError> {
    let mut config = GroqConfig::from_env()?;
    if let Some(model) = model_override {
        config = config.with_model(model);
    }
    info!(model = %config.model, "Creating Groq provider");
    Ok(EmbacleProvider::from_runner(
        Box::new(GroqProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        GROQ_DISPLAY_NAME,
    ))
}

/// Cohere on `COHERE_API_KEY` and `COHERE_DEFAULT_MODEL`, overridden by `model_override`.
fn cohere_from_env(model_override: Option<&str>) -> Result<EmbacleProvider, AppError> {
    let mut config = CohereConfig::from_env()?;
    if let Some(model) = model_override {
        config = config.with_model(model);
    }
    info!(model = %config.model, "Creating Cohere provider");
    Ok(EmbacleProvider::from_runner(
        Box::new(CohereProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        COHERE_DISPLAY_NAME,
    ))
}

/// `OpenRouter` on `OPENROUTER_API_KEY` and `OPENROUTER_DEFAULT_MODEL`,
/// overridden by `model_override`.
fn openrouter_from_env(model_override: Option<&str>) -> Result<EmbacleProvider, AppError> {
    let mut config = OpenRouterConfig::from_env()?;
    if let Some(model) = model_override {
        config = config.with_model(model);
    }
    info!(model = %config.model, "Creating OpenRouter provider");
    Ok(EmbacleProvider::from_runner(
        Box::new(OpenRouterProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        OPENROUTER_DISPLAY_NAME,
    ))
}

/// A local `OpenAI`-compatible endpoint on `LOCAL_LLM_BASE_URL` /
/// `LOCAL_LLM_MODEL` / `LOCAL_LLM_API_KEY`, overridden by `model_override`.
fn local_from_env(model_override: Option<&str>) -> EmbacleProvider {
    let mut config = OpenAiCompatibleConfig::from_env();
    if let Some(model) = model_override {
        model.clone_into(&mut config.default_model);
    }
    info!(
        provider = %config.provider_name,
        base_url = %config.base_url,
        model = %config.default_model,
        "Creating local OpenAI-compatible provider"
    );
    local_provider(config)
}

/// Gemini from a tenant's own key (the BYO path), on the tenant's default
/// model when it stored one and on `PIERRE_LLM_DEFAULT_MODEL` otherwise.
///
/// # Errors
///
/// Returns `AppError` when no model can be resolved.
pub fn gemini_with_key(
    api_key: &str,
    default_model: Option<String>,
) -> Result<EmbacleProvider, AppError> {
    let model = match default_model {
        Some(model) => model,
        None => {
            LlmModelConfig::from_env()
                .map_err(AppError::config)?
                .default_model
        }
    };
    let config = GeminiConfig::new(api_key).with_model(model);
    Ok(EmbacleProvider::from_runner(
        Box::new(GeminiProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        GEMINI_DISPLAY_NAME,
    ))
}

/// Groq from a tenant's own key (the BYO path).
#[must_use]
pub fn groq_with_key(api_key: String, default_model: Option<String>) -> EmbacleProvider {
    let mut config = GroqConfig::new(api_key);
    if let Some(model) = default_model {
        config = config.with_model(model);
    }
    EmbacleProvider::from_runner(
        Box::new(GroqProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        GROQ_DISPLAY_NAME,
    )
}

/// Cohere from a tenant's own key (the BYO path).
#[must_use]
pub fn cohere_with_key(api_key: String, default_model: Option<String>) -> EmbacleProvider {
    let mut config = CohereConfig::new(api_key);
    if let Some(model) = default_model {
        config = config.with_model(model);
    }
    EmbacleProvider::from_runner(
        Box::new(CohereProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        COHERE_DISPLAY_NAME,
    )
}

/// A local OpenAI-compatible endpoint from a tenant's stored base URL, key
/// and model (the BYO path). Reports itself as `local`.
#[must_use]
pub fn local_from_credentials(
    base_url: String,
    api_key: Option<String>,
    model: String,
) -> EmbacleProvider {
    let config = OpenAiCompatibleConfig {
        base_url,
        api_key,
        default_model: model,
        provider_name: "local".to_owned(),
        display_name: "Local LLM".to_owned(),
        capabilities: LlmCapabilities::STREAMING
            | LlmCapabilities::FUNCTION_CALLING
            | LlmCapabilities::SYSTEM_MESSAGES,
    };
    local_provider(config)
}

/// An `OpenAI`-compatible endpoint from an explicit config: the local
/// integration and scenario drivers, and the evals' Ollama candidates.
///
/// Every platform construction goes through here rather than
/// `OpenAiCompatibleProvider::new`, which builds a client of its own on
/// embacle's 120 s default: the shared LLM client carries the platform's 300 s
/// request timeout, and a CPU-bound Ollama takes more than two minutes to
/// process a full coaching prompt. The display name is the one the config's
/// `provider_name` has always carried on the platform.
#[must_use]
pub fn local_provider(config: OpenAiCompatibleConfig) -> EmbacleProvider {
    let display_name = local_display_name(&config.provider_name);
    EmbacleProvider::from_runner(
        Box::new(OpenAiCompatibleProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        display_name,
    )
}

const GEMINI_DISPLAY_NAME: &str = "Google Gemini";
const GROQ_DISPLAY_NAME: &str = "Groq (Llama/Mixtral)";
const COHERE_DISPLAY_NAME: &str = "Cohere (Command)";
const OPENROUTER_DISPLAY_NAME: &str = "OpenRouter";

/// The static display name for each `name()` an OpenAI-compatible provider
/// reports; the platform trait wants a `&'static str`.
fn local_display_name(provider_name: &str) -> &'static str {
    match provider_name {
        "ollama" => "Ollama (Local)",
        "vllm" => "vLLM (Local)",
        "localai" => "LocalAI",
        _ => "Local LLM",
    }
}
