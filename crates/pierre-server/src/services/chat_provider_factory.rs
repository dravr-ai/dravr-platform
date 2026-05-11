// ABOUTME: Service-layer factory for `ChatProvider` instances built from environment config
// ABOUTME: Canonical home for create_chat_provider() — services must not depend on the routes layer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Chat provider factory.
//!
//! Thin wrapper around [`pierre_llm::ChatProvider::from_env`]. Lives in the
//! service layer so downstream services (chat pipeline, memory extraction)
//! can call it without importing from `crate::routes`, and so the routes
//! layer can depend on the services layer — not the other way around.

use std::sync::Arc;

use pierre_llm::config::LlmProviderType;
use pierre_llm::ChatProvider;
use tracing::{info, warn};

use crate::errors::AppError;
use crate::health::LlmHealthState;
use crate::mcp::resources::ServerContext;

/// Build a [`ChatProvider`] from the current process environment.
///
/// All provider backends (Gemini, Groq, Local, CLI runners, Copilot SDK) are
/// resolved inside `pierre-llm`; this function exists purely to give service
/// and route callers a single entry point that keeps provider construction
/// out of individual handlers.
///
/// # Errors
///
/// Returns [`AppError`] if the configured provider cannot be initialized
/// (missing API key, invalid endpoint, etc.).
pub async fn create_chat_provider() -> Result<ChatProvider, AppError> {
    ChatProvider::from_env().await
}

/// Build a [`ChatProvider`] honoring any override injected on
/// [`ServerContext::llm_provider`].
///
/// Production code leaves `llm_provider` set to `None` and this function
/// falls back to [`create_chat_provider`]. Integration tests (for example
/// the conversation-turn E2E) set the field to a deterministic mock via
/// [`ServerContext::with_llm_provider`] so the pipeline runs without
/// touching a real provider.
///
/// # Errors
///
/// Returns [`AppError`] from the fallback path ([`create_chat_provider`])
/// when no override is present and the environment-configured provider
/// cannot be initialized.
pub async fn create_chat_provider_from_resources(
    resources: &Arc<ServerContext>,
) -> Result<ChatProvider, AppError> {
    if let Some(custom) = resources.llm_provider.clone() {
        return Ok(ChatProvider::Custom(custom));
    }
    create_chat_provider().await
}

/// Spawn the boot-time LLM probe task.
///
/// Runs once shortly after the server starts: builds a provider via
/// [`create_chat_provider`], calls `LlmProvider::health_check`, and records
/// the outcome on [`LlmHealthState`]. The state defaults to
/// `LlmHealthStatus::Unknown` so the readiness gate stays open during the
/// probe round-trip — the operator-facing fast-fail path is the dedicated
/// `/ready` route (returns 503 once status flips to `Unhealthy`).
///
/// The task is fire-and-forget; failures are logged but do not affect
/// process state directly. Restoring health after a transient failure is
/// the request-time fallback chain's responsibility, not this probe's.
pub fn spawn_llm_health_probe(health_state: Arc<LlmHealthState>) {
    let provider_name = LlmProviderType::from_env().to_string();
    tokio::spawn(async move {
        info!(provider = %provider_name, "Starting LLM startup health probe");
        let provider = match create_chat_provider().await {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("provider construction failed: {e}");
                warn!(provider = %provider_name, error = %e, "LLM startup probe failed");
                health_state.record_unhealthy(provider_name, msg).await;
                return;
            }
        };
        match provider.health_check().await {
            Ok(true) => {
                info!(provider = %provider_name, "LLM startup probe succeeded");
                health_state.record_healthy(provider_name).await;
            }
            Ok(false) => {
                warn!(
                    provider = %provider_name,
                    "LLM startup probe reported unhealthy"
                );
                health_state
                    .record_unhealthy(provider_name, "provider reported unhealthy")
                    .await;
            }
            Err(e) => {
                let msg = format!("health_check round-trip failed: {e}");
                warn!(provider = %provider_name, error = %e, "LLM startup probe round-trip failed");
                health_state.record_unhealthy(provider_name, msg).await;
            }
        }
    });
}
