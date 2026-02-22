// ABOUTME: Thin re-export module for the pierre-llm crate
// ABOUTME: Re-exports pierre-llm types and adds tenant-aware factory functions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM Provider Integration
//!
//! This module re-exports the `pierre-llm` crate and adds tenant-aware
//! factory methods that integrate with the main crate's database and
//! credential management systems.

pub use pierre_llm::*;

// Re-export submodules explicitly so `crate::llm::pricing::calculate_cost` etc. work
pub use pierre_llm::config;
pub use pierre_llm::pricing;
pub use pierre_llm::prompts;
pub use pierre_llm::sse_parser;

use pierre_core::models::TenantId;
use tracing::info;
use uuid::Uuid;

use crate::database_plugins::factory::Database;
use crate::errors::AppError;
use crate::tenant::llm_manager::{
    LlmCredentials, LlmProvider as TenantLlmProvider, TenantLlmManager,
};

/// Create a `ChatProvider` for a specific tenant and user
///
/// Resolution order for API keys:
/// 1. User-specific credentials (from `user_llm_credentials` table)
/// 2. Tenant-level default (from `user_llm_credentials` table with `user_id = NULL`)
/// 3. Environment variable fallback (`GEMINI_API_KEY`, `GROQ_API_KEY`, etc.)
///
/// # Errors
///
/// Returns an error if no credentials are found for the provider.
pub async fn chat_provider_from_tenant(
    user_id: Option<Uuid>,
    tenant_id: TenantId,
    provider: TenantLlmProvider,
    database: &Database,
) -> Result<ChatProvider, AppError> {
    let credentials =
        TenantLlmManager::get_credentials(user_id, tenant_id, provider, database).await?;

    chat_provider_from_credentials(credentials)
}

/// Create a `ChatProvider` from pre-fetched credentials
///
/// # Errors
///
/// Returns an error if the provider type is not supported.
pub fn chat_provider_from_credentials(
    credentials: LlmCredentials,
) -> Result<ChatProvider, AppError> {
    info!(
        "Creating {} provider from {} credentials",
        credentials.provider, credentials.source
    );

    match credentials.provider {
        TenantLlmProvider::Gemini => {
            let mut provider = GeminiProvider::new(&credentials.api_key)?;
            if let Some(model) = credentials.default_model {
                provider = provider.with_default_model(model);
            }
            Ok(ChatProvider::Gemini(provider))
        }
        TenantLlmProvider::Groq => Ok(ChatProvider::Groq(GroqProvider::new(credentials.api_key))),
        TenantLlmProvider::Local => {
            let base_url = credentials
                .base_url
                .unwrap_or_else(|| "http://localhost:11434/v1".to_owned());
            let model = credentials
                .default_model
                .unwrap_or_else(|| "qwen2.5:14b-instruct".to_owned());
            let config = OpenAiCompatibleConfig {
                base_url,
                api_key: if credentials.api_key.is_empty() {
                    None
                } else {
                    Some(credentials.api_key)
                },
                default_model: model.clone(),
                fallback_model: model,
                provider_name: "local".to_owned(),
                display_name: "Local LLM".to_owned(),
                capabilities: LlmCapabilities::STREAMING
                    | LlmCapabilities::FUNCTION_CALLING
                    | LlmCapabilities::SYSTEM_MESSAGES,
            };
            let provider = OpenAiCompatibleProvider::new(config)?;
            Ok(ChatProvider::Local(provider))
        }
        TenantLlmProvider::OpenAi | TenantLlmProvider::Anthropic => Err(AppError::config(format!(
            "{} provider is not yet supported. Use Gemini, Groq, or Local.",
            credentials.provider
        ))),
    }
}
