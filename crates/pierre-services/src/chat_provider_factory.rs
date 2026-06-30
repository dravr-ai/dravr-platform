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

use pierre_auth::tenant::llm_manager::{LlmCredentials, LlmProvider as TenantLlmProvider};
use pierre_core::errors::AppError;
use pierre_llm::chain_guard::{RateLimitTransition, CHAIN_GUARD};
use pierre_llm::config::LlmProviderType;
use pierre_llm::health::{LlmHealthState, LlmHealthStatus};
use pierre_llm::{
    ChatMessage, ChatProvider, ChatRequest, CohereProvider, GeminiProvider, GroqProvider,
    LlmCapabilities, LlmProvider, OpenAiCompatibleConfig, OpenAiCompatibleProvider,
};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

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
/// `ServerContext::llm_provider`.
///
/// Production code leaves `llm_provider` set to `None` and this function
/// falls back to [`create_chat_provider`]. Integration tests (for example
/// the conversation-turn E2E) set the field to a deterministic mock via
/// `ServerContext::with_llm_provider` so the pipeline runs without
/// touching a real provider.
///
/// # Errors
///
/// Returns [`AppError`] from the fallback path ([`create_chat_provider`])
/// when no override is present and the environment-configured provider
/// cannot be initialized.
pub async fn create_chat_provider_from_resources(
    llm_provider: Option<&Arc<dyn LlmProvider>>,
) -> Result<ChatProvider, AppError> {
    if let Some(custom) = llm_provider {
        return Ok(ChatProvider::Custom(Arc::clone(custom)));
    }
    create_chat_provider().await
}

/// Return the [`ChatProvider`] singleton resolved from the two provider
/// handles a `ServerContext` carries.
///
/// This is the preferred accessor for any handler / service / background
/// task that needs to issue an LLM call. Cloning an `Arc` is cheap and
/// keeps every consumer pointed at the same provider instance so the
/// embacle `CopilotHeadlessRunner`'s long-lived `copilot --acp`
/// subprocess + cached GitHub→Copilot OAuth token are reused instead of
/// torn down and rebuilt per call.
///
/// Resolution order — **NEVER falls through to per-call
/// [`ChatProvider::from_env`]**:
///
/// 1. `chat_provider` singleton (production path)
/// 2. `llm_provider` wrapped in [`ChatProvider::Custom`]
///    (test path — fixtures inject a mock via
///    `ServerContextBuilder::with_llm_provider`)
/// 3. Otherwise: return [`AppError::internal`] — no silent copilot spawn
///
/// The per-call `from_env()` fallback was a footgun: under load (`cargo
/// test` running 30+ chat-touching tests with `--test-threads=4`, Cloud
/// Run's 5-min probe firing N times) each fallback spawned `copilot
/// --acp` and ran the GitHub→Copilot OAuth token exchange, which burned
/// `ChefFamille`'s shared 5000/hr GitHub REST budget in minutes and tripped
/// Copilot-internal per-PAT throttling that takes ~1 hour to recover.
/// Erroring fast pushes the failure to test setup / wiring instead.
///
/// # Errors
///
/// Returns [`AppError::internal`] when both `chat_provider` and
/// `llm_provider` are `None`. Callers should treat this as a wiring bug,
/// not a transient failure.
pub fn chat_provider_from_resources_arc(
    chat_provider: Option<&Arc<ChatProvider>>,
    llm_provider: Option<&Arc<dyn LlmProvider>>,
) -> Result<Arc<ChatProvider>, AppError> {
    if let Some(cp) = chat_provider {
        return Ok(Arc::clone(cp));
    }
    if let Some(llm) = llm_provider {
        return Ok(Arc::new(ChatProvider::Custom(Arc::clone(llm))));
    }
    Err(AppError::internal(
        "No ChatProvider configured on ServerContext — \
         wire one via ServerContextBuilder::with_chat_provider (production) \
         or ::with_llm_provider (tests). Per-call ChatProvider::from_env() \
         is intentionally disabled here to prevent copilot --acp spawn storms.",
    ))
}

/// Environment variable controlling the periodic LLM probe interval.
///
/// Defaults to 1800s (30 minutes). Set to `0` to disable periodic probing
/// (the startup probe still runs once). Reads at boot time only.
pub const LLM_PROBE_INTERVAL_ENV_VAR: &str = "PIERRE_LLM_HEALTH_PROBE_INTERVAL_SECS";

/// Default re-probe interval when [`LLM_PROBE_INTERVAL_ENV_VAR`] is unset.
///
/// Raised from 5 to 30 minutes (2026-06-29) to cut the standing cost of the
/// synthetic round-trip: each periodic probe is a billed `copilot --acp`
/// premium request, so a 5-minute cadence cost ~288 requests/day even with
/// zero user traffic. Combined with the real-traffic piggyback in
/// [`spawn_llm_health_probe`], an idle service now probes 48×/day and a busy
/// one effectively never probes synthetically.
const DEFAULT_LLM_PROBE_INTERVAL_SECS: u64 = 1800;

/// Spawn the LLM probe task — runs once at startup, then re-probes on a
/// configurable interval (default 30 minutes).
///
/// On every periodic tick, the task first checks whether a real chat turn
/// already proved the provider live within the interval (via
/// [`LlmHealthState::since_last_success`], stamped by the chat pipeline). If
/// so it skips the synthetic round-trip — a real served turn is stronger
/// proof of life than the one-token ping and, crucially, the ping is a
/// *billed* `copilot --acp` request, so piggybacking on real traffic avoids
/// paying for redundant probes on a busy service. The free GitHub
/// rate-limit probe still runs every tick.
///
/// When the probe does run:
///
/// * Records the outcome onto [`LlmHealthState`] so `/ready` and
///   `/health/llm` reflect the latest round-trip.
/// * On `Healthy -> Unhealthy` (or `Unknown -> Unhealthy`) transitions,
///   emits an `error!` line — the [`pierre_logging::ErrorNotificationLayer`]
///   wired in `init_from_env` auto-routes that line to the
///   `SLACK_ERROR_CHANNEL` so an operator pages without extra plumbing.
/// * On `Unhealthy -> Healthy` transitions, emits an `info!` recovery line.
///
/// The task is fire-and-forget; failures only surface via the dedicated
/// readiness route + Slack alert. Restoring health after a transient
/// failure is the request-time fallback chain's responsibility, not this
/// probe's.
pub fn spawn_llm_health_probe(
    health_state: Arc<LlmHealthState>,
    chat_provider: Option<Arc<ChatProvider>>,
) {
    let provider_name = LlmProviderType::from_env().to_string();
    let interval_secs = env::var(LLM_PROBE_INTERVAL_ENV_VAR)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_LLM_PROBE_INTERVAL_SECS);

    tokio::spawn(async move {
        info!(
            provider = %provider_name,
            interval_secs,
            chat_provider_singleton = chat_provider.is_some(),
            "Starting LLM health probe task"
        );

        // First probe runs immediately so /ready reflects boot-time state
        // within the first round-trip; subsequent probes follow the
        // interval. When interval_secs is 0, we run a single probe and
        // exit.
        run_one_probe(
            &provider_name,
            &health_state,
            chat_provider.as_ref(),
            ProbeKind::Startup,
        )
        .await;

        if interval_secs == 0 {
            info!(
                provider = %provider_name,
                "Periodic LLM probe disabled ({LLM_PROBE_INTERVAL_ENV_VAR}=0); ran startup probe only"
            );
            return;
        }

        // Boot-time GitHub rate-limit probe so CHAIN_GUARD has a value
        // before any chat request lands; without this, the very first
        // call would fail-open even when GitHub's budget is already
        // exhausted.
        run_github_rate_limit_probe().await;

        let probe_interval = Duration::from_secs(interval_secs);
        let mut ticker = interval(probe_interval);
        ticker.tick().await; // consume the immediate first tick
        loop {
            ticker.tick().await;
            if should_skip_probe(health_state.since_last_success(), probe_interval) {
                refresh_health_from_real_traffic(&provider_name, &health_state).await;
            } else {
                run_one_probe(
                    &provider_name,
                    &health_state,
                    chat_provider.as_ref(),
                    ProbeKind::Periodic,
                )
                .await;
            }
            run_github_rate_limit_probe().await;
        }
    });
}

/// Decide whether the periodic probe can skip its billed round-trip because
/// a real chat turn already proved the provider live within the interval.
///
/// Returns `true` only when a real success has been recorded AND it landed
/// less than `interval` ago. No real traffic yet (`None`) always probes, so
/// an idle service keeps its synthetic liveness signal.
#[must_use]
pub fn should_skip_probe(since_last_success: Option<Duration>, interval: Duration) -> bool {
    matches!(since_last_success, Some(elapsed) if elapsed < interval)
}

/// Refresh the readiness snapshot from real chat traffic instead of paying
/// for a synthetic probe.
///
/// A real served turn is proof of life, so we mark the state `Healthy` —
/// this also clears any stale `Unhealthy` left by an earlier transient probe
/// and emits an `info!` recovery line when it actually flips status.
async fn refresh_health_from_real_traffic(provider_name: &str, health_state: &LlmHealthState) {
    let previous = health_state.record_healthy(provider_name.to_owned()).await;
    if previous == LlmHealthStatus::Healthy {
        debug!(
            provider = provider_name,
            "LLM probe skipped; real chat traffic proved liveness within interval"
        );
    } else {
        info!(
            provider = provider_name,
            ?previous,
            "LLM probe skipped; real chat traffic proved liveness (snapshot -> healthy)"
        );
    }
}

/// Probe GitHub's `/rate_limit` endpoint with the PAT we use for
/// Copilot session-token exchange. The endpoint itself does NOT count
/// against the core rate budget — it's intentionally free so callers
/// can self-throttle.
///
/// Pushes the latest `core.remaining` / `core.reset` into
/// [`pierre_llm::chain_guard::CHAIN_GUARD`] so the runtime fallback
/// chain can short-circuit to the secondary when the budget is too low
/// for Copilot's next session refresh to succeed. Emits notify events
/// in `#dravr-signal` on threshold transitions so operators see the
/// degradation before users do.
///
/// Fail-open: any network/parse/auth issue logs at `warn!` and leaves
/// `CHAIN_GUARD` in whatever state the previous probe left it (or
/// `RATE_LIMIT_UNKNOWN` if no probe has succeeded yet).
async fn run_github_rate_limit_probe() {
    let Some((remaining, reset_at)) = fetch_github_rate_limit().await else {
        return;
    };
    let transition = CHAIN_GUARD.record_github_rate_limit(remaining, reset_at);
    log_rate_limit_transition(transition, remaining, reset_at);
}

async fn fetch_github_rate_limit() -> Option<(u64, u64)> {
    let token = env::var("GITHUB_PERSONAL_ACCESS_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())?;
    let client = build_probe_client()?;
    let response = send_rate_limit_request(&client, &token).await?;
    let body = parse_rate_limit_body(response).await?;
    let remaining = body["resources"]["core"]["remaining"]
        .as_u64()
        .unwrap_or(u64::MAX);
    let reset_at = body["resources"]["core"]["reset"].as_u64().unwrap_or(0);
    Some((remaining, reset_at))
}

fn build_probe_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(
            |e| warn!(error = %e, "Failed to build reqwest client for GitHub rate-limit probe"),
        )
        .ok()
}

async fn send_rate_limit_request(
    client: &reqwest::Client,
    token: &str,
) -> Option<reqwest::Response> {
    let response = client
        .get("https://api.github.com/rate_limit")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "dravr-platform-probe")
        .send()
        .await
        .map_err(|e| warn!(error = %e, "GitHub rate-limit probe HTTP request failed"))
        .ok()?;
    if !response.status().is_success() {
        warn!(
            status = %response.status(),
            "GitHub rate-limit probe returned non-2xx; leaving CHAIN_GUARD unchanged"
        );
        return None;
    }
    Some(response)
}

async fn parse_rate_limit_body(response: reqwest::Response) -> Option<serde_json::Value> {
    response
        .json()
        .await
        .map_err(|e| warn!(error = %e, "GitHub rate-limit probe response was not valid JSON"))
        .ok()
}

fn log_rate_limit_transition(transition: RateLimitTransition, remaining: u64, reset_at: u64) {
    match transition {
        RateLimitTransition::EnteredLow => log_entered_low(remaining, reset_at),
        RateLimitTransition::ExitedLow => log_exited_low(remaining, reset_at),
        RateLimitTransition::StillLow | RateLimitTransition::StillOk => {
            debug!(remaining, reset_at, "GitHub rate-limit probe steady state");
        }
    }
}

fn log_entered_low(remaining: u64, reset_at: u64) {
    warn!(
        remaining,
        reset_at,
        "GitHub rate-limit headroom dropped below threshold; \
         Chain will skip primary until recovered"
    );
    info!(
        target: "notify",
        event = "llm.rate_limit_low",
        remaining = remaining,
        reset_at = reset_at,
        "GitHub rate-limit budget low; chain skipping primary preemptively"
    );
}

fn log_exited_low(remaining: u64, reset_at: u64) {
    info!(
        remaining,
        reset_at, "GitHub rate-limit headroom recovered above threshold"
    );
    info!(
        target: "notify",
        event = "llm.rate_limit_recovered",
        remaining = remaining,
        "GitHub rate-limit budget recovered; chain primary re-enabled"
    );
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
async fn run_one_probe(
    provider_name: &str,
    health_state: &LlmHealthState,
    chat_provider: Option<&Arc<ChatProvider>>,
    kind: ProbeKind,
) {
    // Singleton-only: reuse the warm provider so the underlying Copilot
    // Headless subprocess + cached GitHub→Copilot OAuth token stay alive
    // across probe ticks. No per-tick `create_chat_provider()` fallback —
    // see `chat_provider_from_resources_arc` for the full rationale.
    let Some(cp) = chat_provider else {
        let msg = "no chat_provider singleton wired; probe skipped";
        let previous = health_state
            .record_unhealthy(provider_name.to_owned(), msg)
            .await;
        log_probe_outcome(provider_name, kind, previous, false, Some(msg));
        return;
    };
    let provider: &ChatProvider = cp.as_ref();
    match provider.health_check().await {
        Ok(true) => match roundtrip_probe(provider).await {
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
        log_pageable_failure(provider, kind, previous, error);
    } else {
        log_startup_failure(provider, kind, error);
    }
}

fn log_pageable_failure(provider: &str, kind: ProbeKind, previous: LlmHealthStatus, error: &str) {
    error!(
        provider,
        %kind,
        previous = %previous,
        error,
        "LLM probe transitioned to Unhealthy; chat traffic at risk"
    );
    // Operationally distinct from the all-errors firehose: route the
    // transition through NotifyLayer so it lands in #dravr-signal with
    // a stable event name + structured fields. See ADR-014.
    info!(
        target: "notify",
        event = "llm.provider_unhealthy",
        provider = provider,
        previous_state = %previous,
        error = error,
        "LLM probe transitioned to Unhealthy"
    );
}

fn log_startup_failure(provider: &str, kind: ProbeKind, error: &str) {
    warn!(
        provider,
        %kind,
        error,
        "LLM startup probe failed; relying on runtime fallback if configured"
    );
}

fn log_recovery_transition(provider: &str, kind: ProbeKind, previous: LlmHealthStatus) {
    if matches!(previous, LlmHealthStatus::Unhealthy) {
        info!(
            provider,
            %kind,
            "LLM probe Unhealthy -> Healthy; chat traffic recovered"
        );
        // Pair with llm.provider_unhealthy so operators can close the
        // incident loop in #dravr-signal without reading logs.
        info!(
            target: "notify",
            event = "llm.provider_recovered",
            provider = provider,
            previous_state = %previous,
            "LLM probe transitioned back to Healthy"
        );
    } else {
        info!(provider, %kind, "LLM probe healthy");
    }
}

/// Construct a [`ChatProvider`] from already-resolved tenant LLM credentials.
///
/// Used by the LLM-settings test endpoint after the caller has loaded
/// credentials via [`pierre_auth::tenant::llm_manager::TenantLlmManager`] —
/// this fn picks the right concrete pierre-llm provider for the credential's
/// `provider` field and applies any provider-specific knobs (default model,
/// local-LLM base URL, etc.).
///
/// # Errors
///
/// Returns [`AppError::config`] when the credential references a provider
/// (currently `OpenAi`, `Anthropic`) that pierre-llm doesn't yet expose as a
/// `ChatProvider` variant.
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
        TenantLlmProvider::Cohere => {
            let mut provider = CohereProvider::new(credentials.api_key);
            if let Some(model) = credentials.default_model {
                provider = provider.with_default_model(model);
            }
            Ok(ChatProvider::Cohere(provider))
        }
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
            "{} provider is not yet supported. Use Gemini, Groq, Cohere, or Local.",
            credentials.provider
        ))),
    }
}
