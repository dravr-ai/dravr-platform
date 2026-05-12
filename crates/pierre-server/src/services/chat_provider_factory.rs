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

use std::env;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::sync::Arc;
use std::time::Duration;

use pierre_llm::config::LlmProviderType;
use pierre_llm::{ChatMessage, ChatProvider, ChatRequest};
use tokio::time::interval;
use tracing::{error, info, warn};

use crate::errors::AppError;
use crate::health::{LlmHealthState, LlmHealthStatus};
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

/// Environment variable controlling the periodic LLM probe interval.
///
/// Defaults to 300s (5 minutes). Set to `0` to disable periodic probing
/// (the startup probe still runs once). Reads at boot time only.
pub const LLM_PROBE_INTERVAL_ENV_VAR: &str = "PIERRE_LLM_HEALTH_PROBE_INTERVAL_SECS";

/// Default re-probe interval when [`LLM_PROBE_INTERVAL_ENV_VAR`] is unset.
const DEFAULT_LLM_PROBE_INTERVAL_SECS: u64 = 300;

/// Spawn the LLM probe task — runs once at startup, then re-probes on a
/// configurable interval (default 5 minutes).
///
/// On every probe:
///
/// * Records the outcome onto [`LlmHealthState`] so `/ready` and
///   `/health/llm` reflect the latest round-trip.
/// * On `Healthy -> Unhealthy` (or `Unknown -> Unhealthy`) transitions,
///   emits an `error!` line — the [`pierre_mcp_server::logging::ErrorNotificationLayer`]
///   wired in `init_from_env` auto-routes that line to the
///   `SLACK_ERROR_CHANNEL` so an operator pages without extra plumbing.
/// * On `Unhealthy -> Healthy` transitions, emits an `info!` recovery line.
///
/// The task is fire-and-forget; failures only surface via the dedicated
/// readiness route + Slack alert. Restoring health after a transient
/// failure is the request-time fallback chain's responsibility, not this
/// probe's.
pub fn spawn_llm_health_probe(health_state: Arc<LlmHealthState>) {
    let provider_name = LlmProviderType::from_env().to_string();
    let interval_secs = env::var(LLM_PROBE_INTERVAL_ENV_VAR)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_LLM_PROBE_INTERVAL_SECS);

    tokio::spawn(async move {
        info!(
            provider = %provider_name,
            interval_secs,
            "Starting LLM health probe task"
        );

        // First probe runs immediately so /ready reflects boot-time state
        // within the first round-trip; subsequent probes follow the
        // interval. When interval_secs is 0, we run a single probe and
        // exit.
        run_one_probe(&provider_name, &health_state, ProbeKind::Startup).await;

        if interval_secs == 0 {
            info!(
                provider = %provider_name,
                "Periodic LLM probe disabled ({LLM_PROBE_INTERVAL_ENV_VAR}=0); ran startup probe only"
            );
            return;
        }

        let mut ticker = interval(Duration::from_secs(interval_secs));
        ticker.tick().await; // consume the immediate first tick
        loop {
            ticker.tick().await;
            run_one_probe(&provider_name, &health_state, ProbeKind::Periodic).await;
        }
    });
}

#[derive(Debug, Clone, Copy)]
enum ProbeKind {
    Startup,
    Periodic,
}

impl Display for ProbeKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Startup => write!(f, "startup"),
            Self::Periodic => write!(f, "periodic"),
        }
    }
}

/// Run one probe round and log the outcome.
///
/// Splits the logging policy by probe kind:
///
/// * Startup probes don't escalate: the readiness route is the operator
///   signal — a boot-time failure is expected to be transient if the
///   fallback chain is configured, and we don't want to page on every
///   container start.
/// * Periodic probes escalate on `Healthy -> Unhealthy` transitions
///   (paging-worthy: the provider became unavailable while traffic was
///   landing) and clear with an `info!` recovery line on
///   `Unhealthy -> Healthy`.
async fn run_one_probe(provider_name: &str, health_state: &LlmHealthState, kind: ProbeKind) {
    let provider = match create_chat_provider().await {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("provider construction failed: {e}");
            let previous = health_state
                .record_unhealthy(provider_name.to_owned(), msg.clone())
                .await;
            log_probe_outcome(provider_name, kind, previous, false, Some(&msg));
            return;
        }
    };
    match provider.health_check().await {
        Ok(true) => match roundtrip_probe(&provider).await {
            Ok(()) => {
                let previous = health_state.record_healthy(provider_name.to_owned()).await;
                log_probe_outcome(provider_name, kind, previous, true, None);
            }
            Err(e) => {
                let msg = format!("roundtrip probe failed: {e}");
                let previous = health_state
                    .record_unhealthy(provider_name.to_owned(), msg.clone())
                    .await;
                log_probe_outcome(provider_name, kind, previous, false, Some(&msg));
            }
        },
        Ok(false) => {
            let msg = "provider reported unhealthy";
            let previous = health_state
                .record_unhealthy(provider_name.to_owned(), msg)
                .await;
            log_probe_outcome(provider_name, kind, previous, false, Some(msg));
        }
        Err(e) => {
            let msg = format!("health_check round-trip failed: {e}");
            let previous = health_state
                .record_unhealthy(provider_name.to_owned(), msg.clone())
                .await;
            log_probe_outcome(provider_name, kind, previous, false, Some(&msg));
        }
    }
}

/// Cheapest possible end-to-end check that the configured LLM provider is
/// actually able to serve a request right now. Sends a one-token "ping"
/// prompt and accepts ANY non-error response as proof of life.
///
/// The base [`ChatProvider::health_check`] only verifies provider-specific
/// readiness (binary present, API key shape, etc.). For Copilot CLI in
/// particular that probe stays green even when the session token has
/// silently expired against `api.github.com`, so the first real chat
/// breaks for users. This roundtrip catches that class of failure before
/// it reaches a user-facing turn, which lets `/ready` flip to 503 and
/// gives the runtime fallback chain a chance to log a transition.
async fn roundtrip_probe(provider: &ChatProvider) -> Result<(), AppError> {
    let request = ChatRequest::new(vec![ChatMessage::user("ping".to_owned())])
        .with_temperature(0.0)
        .with_max_tokens(1);
    provider.complete(&request).await.map(|_| ())
}

fn log_probe_outcome(
    provider: &str,
    kind: ProbeKind,
    previous: LlmHealthStatus,
    now_healthy: bool,
    error: Option<&str>,
) {
    let now = if now_healthy {
        LlmHealthStatus::Healthy
    } else {
        LlmHealthStatus::Unhealthy
    };

    if previous == now {
        log_steady_state(provider, kind, now_healthy, error);
    } else {
        log_transition(provider, kind, previous, now_healthy, error);
    }
}

fn log_steady_state(provider: &str, kind: ProbeKind, now_healthy: bool, error: Option<&str>) {
    if now_healthy {
        info!(provider, %kind, "LLM probe healthy");
    } else {
        warn!(
            provider,
            %kind,
            error = error.unwrap_or(""),
            "LLM probe still unhealthy"
        );
    }
}

/// Escalate a status transition. The `Healthy -> Unhealthy` and
/// `Unknown -> Unhealthy` (periodic only) paths emit `error!` so the
/// tronc [`ErrorNotificationLayer`] auto-routes the line to Slack;
/// recoveries log at `info!`.
fn log_transition(
    provider: &str,
    kind: ProbeKind,
    previous: LlmHealthStatus,
    now_healthy: bool,
    error: Option<&str>,
) {
    if now_healthy {
        log_recovery_transition(provider, kind, previous);
    } else {
        log_failure_transition(provider, kind, previous, error.unwrap_or(""));
    }
}

fn log_failure_transition(provider: &str, kind: ProbeKind, previous: LlmHealthStatus, error: &str) {
    let pageable = matches!(previous, LlmHealthStatus::Healthy)
        || (matches!(previous, LlmHealthStatus::Unknown) && matches!(kind, ProbeKind::Periodic));
    if pageable {
        error!(
            provider,
            %kind,
            previous = %previous,
            error,
            "LLM probe transitioned to Unhealthy; chat traffic at risk"
        );
    } else {
        warn!(
            provider,
            %kind,
            error,
            "LLM startup probe failed; relying on runtime fallback if configured"
        );
    }
}

fn log_recovery_transition(provider: &str, kind: ProbeKind, previous: LlmHealthStatus) {
    if matches!(previous, LlmHealthStatus::Unhealthy) {
        info!(
            provider,
            %kind,
            "LLM probe Unhealthy -> Healthy; chat traffic recovered"
        );
    } else {
        info!(provider, %kind, "LLM probe healthy");
    }
}
