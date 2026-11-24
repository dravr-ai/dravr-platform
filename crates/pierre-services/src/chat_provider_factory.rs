// ABOUTME: Service-layer resolution of the `ChatProvider` singleton, tenant BYO providers and the health probe
// ABOUTME: Lives in services so the routes layer depends on it, never the other way around
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Chat provider factory.
//!
//! Resolves the boot-built [`ChatProvider`] singleton for callers, builds a
//! tenant's own provider from stored credentials, and runs the LLM health
//! probe. Lives in the service layer so downstream services (chat pipeline,
//! memory extraction) can use it without importing from `crate::routes`, and
//! so the routes layer can depend on the services layer — not the other way
//! around.

use secrecy::ExposeSecret;
use std::env;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::sync::Arc;
use std::time::Duration;

use embacle::quota_http::{GithubHeadroomChecker, GithubRateLimit};
use embacle::types::RunnerError;
use pierre_auth::tenant::llm_manager::{LlmCredentials, LlmProvider as TenantLlmProvider};
use pierre_core::errors::AppError;
use pierre_llm::chain_guard::{copilot_headroom_checker, RateLimitTransition, CHAIN_GUARD};
use pierre_llm::config::LlmChain;
use pierre_llm::health::{LlmHealthState, LlmHealthStatus, TierProbe};
use pierre_llm::tier_bench::{probe_request, BENCH_REPROBE_INTERVAL};
use pierre_llm::{http_env, ChatProvider, LlmProvider};
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, error, info, warn};

/// Return the [`ChatProvider`] singleton resolved from the two provider
/// handles a `ServerContext` carries.
///
/// This is the preferred accessor for any handler / service / background
/// task that needs to issue an LLM call. Cloning an `Arc` is cheap and
/// keeps every consumer pointed at the same provider instance so the
/// embacle Copilot runner's long-lived runtime child (`copilot --acp` or
/// `copilot-runtime`) + cached GitHub→Copilot OAuth token are reused instead
/// of torn down and rebuilt per call.
///
/// Resolution order — **NEVER falls through to per-call
/// [`ChatProvider::from_chain`]**:
///
/// 1. `chat_provider` singleton (production path)
/// 2. `llm_provider` wrapped in [`ChatProvider::Custom`]
///    (test path — fixtures inject a mock via
///    `ServerContextOptions::with_llm_provider`)
/// 3. Otherwise: return [`AppError::internal`] — no silent copilot spawn
///
/// A per-call provider build was a footgun: under load (`cargo
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
         wire one via ServerContextOptions::with_chat_provider (production) \
         or ::with_llm_provider (tests). Per-call ChatProvider::from_chain() \
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
/// Startup asks every tier alone first and benches the ones that cannot
/// answer ([`pierre_llm::tier_bench`]), then asks the chain, which passes the
/// benched tiers over. `Unhealthy` therefore means no tier answers, never
/// that the head is dead in front of tiers that would serve. A second task
/// ([`recover_while_degraded`]) re-asks benched tiers, and the chain while it
/// is `Unhealthy`, so an instance recovers without a new revision; it makes
/// no call while nothing is benched and the chain is healthy, and it runs
/// whatever the interval is.
///
/// On every periodic tick, the task first checks whether a real chat turn
/// already proved the provider live within the interval (via
/// [`LlmHealthState::since_last_success`], stamped by the chat pipeline). If
/// so it skips the synthetic round-trip — a real served turn is stronger
/// proof of life than the synthetic ping and, crucially, the ping is a
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
///   `SLACK_ERROR_CHANNEL` so an admin pages without extra plumbing. A
///   failure at startup pages like any other: the chat pipeline refuses
///   every turn while the status reads `Unhealthy`.
/// * On `Unhealthy -> Healthy` transitions, emits an `info!` recovery line.
///
/// The task is fire-and-forget; failures only surface via the dedicated
/// readiness route + Slack alert.
pub fn spawn_llm_health_probe(
    llm_chain: &LlmChain,
    health_state: Arc<LlmHealthState>,
    chat_provider: Option<Arc<ChatProvider>>,
) {
    let provider_name = llm_chain.head().provider.to_string();
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

        run_startup_probe(&provider_name, &health_state, chat_provider.as_ref()).await;

        tokio::spawn(recover_while_degraded(
            provider_name.clone(),
            Arc::clone(&health_state),
            chat_provider.clone(),
        ));

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
        // exhausted. Without a Copilot token there is no budget to read.
        let headroom = copilot_headroom_checker();
        if let Some(checker) = &headroom {
            run_github_rate_limit_probe(checker).await;
        }

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
            if let Some(checker) = &headroom {
                run_github_rate_limit_probe(checker).await;
            }
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
/// this also clears any stale `Unhealthy` left by an earlier transient probe.
///
/// Both branches log at `info!`: skipping the synthetic round-trip is the
/// whole point of the cost fix, so it must be observable at the prod INFO
/// level (a `debug!` here made the piggyback invisible in Cloud Run, where
/// only startup probes showed up). Volume is bounded by the probe interval
/// (≤48 lines/day/instance), so the steady-state skip is cheap to surface.
async fn refresh_health_from_real_traffic(provider_name: &str, health_state: &LlmHealthState) {
    // Stamp `checked_at` with the real last-success time, not `now`: the skip
    // fires precisely because a real turn proved liveness up to a full interval
    // ago, so pretending the snapshot was just checked would overstate its
    // freshness on `/health/llm`.
    let observed_ago = health_state.since_last_success().unwrap_or_default();
    let previous = health_state
        .record_healthy_observed(provider_name.to_owned(), observed_ago)
        .await;
    if previous == LlmHealthStatus::Healthy {
        info!(
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

/// Read the GitHub core budget of the token Copilot spends, through
/// embacle's [`GithubHeadroomChecker`], and record it on
/// [`pierre_llm::chain_guard::CHAIN_GUARD`] so the runtime fallback chain
/// passes over every tier that spends the Copilot token when fewer than
/// [`pierre_llm::chain_guard::GITHUB_BUDGET_THRESHOLD`] requests remain
/// for Copilot's next session refresh; a tier on another credential is
/// never passed over for it. The endpoint itself does NOT count
/// against the core budget. Emits notify events in `#dravr-signal` on
/// threshold transitions so admins see the degradation before users do.
///
/// Fail-closed: a read that fails is recorded as a low budget, so the chain
/// skips the Copilot tiers until a read succeeds. Each skip already raises
/// `embacle.fallback_triggered` (`reason = preemptive_guard`), so the
/// failed read is logged here without an event of its own.
async fn run_github_rate_limit_probe(checker: &GithubHeadroomChecker) {
    let reading = checker.rate_limit().await;
    let transition = CHAIN_GUARD.record_github_headroom(&reading);
    log_rate_limit_transition(transition, &reading);
}

fn log_rate_limit_transition(
    transition: RateLimitTransition,
    reading: &Result<GithubRateLimit, RunnerError>,
) {
    let counts = match reading {
        Ok(counts) => counts,
        Err(error) => {
            warn!(
                error = %error,
                ?transition,
                "GitHub rate-limit read failed; chain skips the Copilot tiers until a read succeeds"
            );
            return;
        }
    };
    let remaining = counts.remaining;
    let reset_at = CHAIN_GUARD.github_reset_at();
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
         Chain will skip the Copilot tiers until recovered"
    );
    info!(
        target: "notify",
        event = "llm.rate_limit_low",
        remaining = remaining,
        reset_at = reset_at,
        "GitHub rate-limit budget low; chain skipping the Copilot tiers preemptively"
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
        "GitHub rate-limit budget recovered; chain Copilot tiers re-enabled"
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

/// The boot-time probe: every tier alone, then the chain.
///
/// The tiers go first. Each is asked on its own, once — a call through the
/// chain proves only that somebody answers, so a dead tier behind a live one
/// stays invisible until it is the only one left — and each one that rejects
/// the probe is benched (`ChatProvider::probe_and_bench_chain_tiers`). The
/// chain is asked second, so its verdict is the verdict of the tiers that can
/// serve: a dead head in front of a live tier reads `Healthy`, served by that
/// tier, and turns are admitted.
///
/// Startup only for the per-tier pass: it costs one billed completion per
/// tier, which a periodic tick must not.
async fn run_startup_probe(
    provider_name: &str,
    health_state: &LlmHealthState,
    chat_provider: Option<&Arc<ChatProvider>>,
) {
    if let Some(provider) = chat_provider {
        for probe in provider.probe_and_bench_chain_tiers().await {
            log_tier_probe(&probe, provider.is_tier_benched(probe.provider));
        }
    }
    run_one_probe(
        provider_name,
        health_state,
        chat_provider,
        ProbeKind::Startup,
    )
    .await;
}

/// Re-ask what is out of service, every [`BENCH_REPROBE_INTERVAL`], for the
/// life of the process.
///
/// Without it a bad boot is permanent wherever the periodic probe is disabled
/// (`PIERRE_LLM_HEALTH_PROBE_INTERVAL_SECS=0`, dev's setting): nothing else
/// writes the status again, the gate refuses every turn, and so no turn can
/// prove the instance healthy. On 2026-11-21 that kept every instance that
/// booted `Unhealthy` refusing turns for close to 14 hours.
async fn recover_while_degraded(
    provider_name: String,
    health_state: Arc<LlmHealthState>,
    chat_provider: Option<Arc<ChatProvider>>,
) {
    let mut ticker = interval(BENCH_REPROBE_INTERVAL);
    // A pass that outlasts the interval (several benched tiers each running
    // into their own timeout) must not be followed by the ticks it missed,
    // back to back: the next pass waits a full interval.
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await; // consume the immediate first tick
    loop {
        ticker.tick().await;
        run_recovery_tick(&provider_name, &health_state, chat_provider.as_ref()).await;
    }
}

/// One recovery pass: the benched tiers are asked again, and the chain is
/// asked again while its status reads `Unhealthy`.
///
/// A healthy chain with nothing benched makes no call here, which is what
/// keeps this inside the 2026-09-21 "startup only" decision: that decision is
/// about a billed ping on an account that works, and the only calls made here
/// go to a tier that last refused, or to a chain no tier of which answered.
async fn run_recovery_tick(
    provider_name: &str,
    health_state: &LlmHealthState,
    chat_provider: Option<&Arc<ChatProvider>>,
) {
    let Some(provider) = chat_provider else {
        return;
    };
    for probe in provider.reprobe_benched_tiers().await {
        log_tier_reprobe(&probe);
    }
    if health_state.snapshot().await.status == LlmHealthStatus::Unhealthy {
        run_one_probe(
            provider_name,
            health_state,
            chat_provider,
            ProbeKind::Periodic,
        )
        .await;
    }
}

/// A benched tier that answers again is an `info!`; one that stays dead is a
/// `debug!`, since its bench was already reported once.
fn log_tier_reprobe(probe: &TierProbe) {
    match &probe.outcome {
        Ok(served_by) => info!(
            tier = probe.provider,
            position = probe.position,
            served_by,
            "LLM chain tier answers the probe again; back in the chain"
        ),
        Err(error) => debug!(
            tier = probe.provider,
            position = probe.position,
            error,
            "LLM chain tier still cannot answer the probe; stays benched"
        ),
    }
}

/// A healthy tier is an `info!` naming who answered. One that failed its
/// probe is reported with the `llm.tier_unhealthy` notify event and a log
/// line that says whether it was `benched` (see [`log_failed_tier`]).
fn log_tier_probe(probe: &TierProbe, benched: bool) {
    match &probe.outcome {
        Ok(served_by) => info!(
            tier = probe.provider,
            position = probe.position,
            served_by,
            "LLM chain tier healthy"
        ),
        Err(error) => {
            log_failed_tier(probe, error, benched);
            info!(
                target: "notify",
                event = "llm.tier_unhealthy",
                provider = probe.provider,
                position = probe.position,
                error = error,
                "LLM chain tier failed its startup probe"
            );
        }
    }
}

/// The log line for a tier that failed its startup probe.
///
/// A tier that failed with a provider fault stays in the chain, which falls
/// through it on that fault: a `warn!`, as a slow cold start must not page on
/// every instance start. A benched tier rejected the probe itself and is
/// passed over until it answers: a `warn!` behind the head, and an `error!`,
/// which pages, for the head, since every turn is then served by a fallback.
fn log_failed_tier(probe: &TierProbe, error: &str, benched: bool) {
    if !benched {
        warn!(
            tier = probe.provider,
            position = probe.position,
            error,
            "LLM chain tier failed its startup probe; the chain is shallower than configured"
        );
        return;
    }
    log_benched_tier(probe, error);
}

fn log_benched_tier(probe: &TierProbe, error: &str) {
    if probe.position == 0 {
        error!(
            tier = probe.provider,
            position = probe.position,
            error,
            "LLM chain tier failed its startup probe; it is the head, benched: later tiers serve every turn until it answers"
        );
    } else {
        warn!(
            tier = probe.provider,
            position = probe.position,
            error,
            "LLM chain tier failed its startup probe; benched, the chain is shallower than configured"
        );
    }
}

/// Run one probe round and log the outcome.
///
/// Every transition into `Unhealthy` escalates, at startup included: the
/// chain has already passed its benched tiers over, so a failure here means
/// no tier answers and the chat pipeline is refusing every turn.
/// `Unhealthy -> Healthy` clears with an `info!` recovery line.
async fn run_one_probe(
    provider_name: &str,
    health_state: &LlmHealthState,
    chat_provider: Option<&Arc<ChatProvider>>,
    kind: ProbeKind,
) {
    // Singleton-only: reuse the warm provider so the underlying Copilot
    // Headless subprocess + cached GitHub→Copilot OAuth token stay alive
    // across probe ticks. No per-tick provider build —
    // see `chat_provider_from_resources_arc` for the full rationale.
    let Some(cp) = chat_provider else {
        let msg = "no chat_provider singleton wired; probe skipped";
        let previous = health_state
            .record_unhealthy(provider_name.to_owned(), msg)
            .await;
        log_probe_outcome(provider_name, kind, previous, None, Some(msg));
        return;
    };
    let provider: &ChatProvider = cp.as_ref();
    match provider.health_check().await {
        Ok(true) => match roundtrip_probe(provider).await {
            Ok(served_by) => {
                let previous = health_state
                    .record_healthy_served(provider_name.to_owned(), served_by.clone())
                    .await;
                log_probe_outcome(provider_name, kind, previous, Some(&served_by), None);
            }
            Err(e) => {
                let msg = format!("roundtrip probe failed: {e}");
                let previous = health_state
                    .record_unhealthy(provider_name.to_owned(), msg.clone())
                    .await;
                log_probe_outcome(provider_name, kind, previous, None, Some(&msg));
            }
        },
        Ok(false) => {
            let msg = "provider reported unhealthy";
            let previous = health_state
                .record_unhealthy(provider_name.to_owned(), msg)
                .await;
            log_probe_outcome(provider_name, kind, previous, None, Some(msg));
        }
        Err(e) => {
            let msg = format!("health_check round-trip failed: {e}");
            let previous = health_state
                .record_unhealthy(provider_name.to_owned(), msg.clone())
                .await;
            log_probe_outcome(provider_name, kind, previous, None, Some(&msg));
        }
    }
}

/// End-to-end check that the configured LLM chain is actually able to serve
/// a request right now. Sends [`probe_request`] and accepts ANY non-error
/// response as proof of life, returning the tier that answered as that
/// runner names itself in the response's `model`.
///
/// The base [`ChatProvider::health_check`] only verifies provider-specific
/// readiness (binary present, API key shape, etc.). For Copilot CLI in
/// particular that probe stays green even when the session token has
/// silently expired against `api.github.com`, and for Claude Code it is a
/// `--version` that passes without a login, so the first real chat breaks
/// for users. This roundtrip catches that class of failure before it
/// reaches a user-facing turn, which lets `/ready` flip to 503 and gives
/// the runtime fallback chain a chance to log a transition.
async fn roundtrip_probe(provider: &ChatProvider) -> Result<String, AppError> {
    provider
        .complete(&probe_request())
        .await
        .map(|response| response.model)
}

/// `served_by` is `Some` exactly when the probe succeeded: the tier that
/// answered, so the healthy line says who is actually serving.
fn log_probe_outcome(
    provider: &str,
    kind: ProbeKind,
    previous: LlmHealthStatus,
    served_by: Option<&str>,
    error: Option<&str>,
) {
    let now_healthy = served_by.is_some();
    let now = if now_healthy {
        LlmHealthStatus::Healthy
    } else {
        LlmHealthStatus::Unhealthy
    };

    if previous == now {
        log_steady_state(provider, kind, served_by, error);
    } else {
        log_transition(provider, kind, previous, served_by, error);
    }
}

fn log_steady_state(provider: &str, kind: ProbeKind, served_by: Option<&str>, error: Option<&str>) {
    if let Some(served_by) = served_by {
        info!(provider, %kind, served_by, "LLM probe healthy");
    } else {
        warn!(
            provider,
            %kind,
            error = error.unwrap_or(""),
            "LLM probe still unhealthy"
        );
    }
}

/// Escalate a status transition. A transition into `Unhealthy` emits
/// `error!` so the tronc [`ErrorNotificationLayer`] auto-routes the line to
/// Slack; recoveries log at `info!`.
fn log_transition(
    provider: &str,
    kind: ProbeKind,
    previous: LlmHealthStatus,
    served_by: Option<&str>,
    error: Option<&str>,
) {
    if let Some(served_by) = served_by {
        log_recovery_transition(provider, kind, previous, served_by);
    } else {
        log_failure_transition(provider, kind, previous, error.unwrap_or(""));
    }
}

/// The startup line keeps the words "LLM startup probe failed": the
/// `dravr-llm-startup-probe-failed` log-based alert
/// (`infra/environments/dev/llm_chain_monitoring.tf`) filters on them.
fn log_failure_transition(provider: &str, kind: ProbeKind, previous: LlmHealthStatus, error: &str) {
    match kind {
        ProbeKind::Startup => error!(
            provider,
            %kind,
            previous = %previous,
            error,
            "LLM startup probe failed: no tier of the chain answers, every chat turn is refused until one does"
        ),
        ProbeKind::Periodic => error!(
            provider,
            %kind,
            previous = %previous,
            error,
            "LLM probe transitioned to Unhealthy; every chat turn is refused until a tier answers"
        ),
    }
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

/// `served_by` names the tier that answered. The first probe of a process is
/// always a transition (`Unknown -> Healthy`), so this is the line an
/// admin reads at every instance start.
fn log_recovery_transition(
    provider: &str,
    kind: ProbeKind,
    previous: LlmHealthStatus,
    served_by: &str,
) {
    if matches!(previous, LlmHealthStatus::Unhealthy) {
        info!(
            provider,
            %kind,
            served_by,
            "LLM probe Unhealthy -> Healthy; chat traffic recovered"
        );
        // Pair with llm.provider_unhealthy so admins can close the
        // incident loop in #dravr-signal without reading logs.
        info!(
            target: "notify",
            event = "llm.provider_recovered",
            provider = provider,
            previous_state = %previous,
            "LLM probe transitioned back to Healthy"
        );
    } else {
        info!(provider, %kind, served_by, "LLM probe healthy");
    }
}

/// Whether [`chat_provider_from_credentials`] builds a provider for a stored
/// credential of `provider`.
///
/// The BYO resolver asks this before it reads a tenant's stored key: a
/// credential no provider is built for would otherwise be decrypted and then
/// dropped on every turn. `OpenAi` and `Anthropic` keys are accepted by the
/// LLM settings, but pierre-llm builds neither from a tenant's key — the
/// platform's own Anthropic tier runs on its pooled organizations.
pub(crate) const fn builds_from_credentials(provider: TenantLlmProvider) -> bool {
    match provider {
        TenantLlmProvider::Gemini
        | TenantLlmProvider::Groq
        | TenantLlmProvider::Cohere
        | TenantLlmProvider::Local => true,
        TenantLlmProvider::OpenAi | TenantLlmProvider::Anthropic => false,
    }
}

/// Construct a [`ChatProvider`] from already-resolved tenant LLM credentials.
///
/// Used by the LLM-settings test endpoint after the caller has loaded
/// credentials via [`pierre_auth::tenant::llm_manager::TenantLlmManager`] —
/// this fn picks the right embacle HTTP provider for the credential's
/// `provider` field and applies any provider-specific knobs (default model,
/// local-LLM base URL, etc.) through [`pierre_llm::http_env`].
///
/// # Errors
///
/// Returns [`AppError::config`] when the credential references a provider
/// that pierre-llm does not build a `ChatProvider` for — every one for which
/// `builds_from_credentials` answers `false` (`OpenAi`, `Anthropic`).
pub fn chat_provider_from_credentials(
    credentials: LlmCredentials,
) -> Result<ChatProvider, AppError> {
    info!(
        "Creating {} provider from {} credentials",
        credentials.provider, credentials.source
    );

    match credentials.provider {
        TenantLlmProvider::Gemini => Ok(ChatProvider::Embacle(http_env::gemini_with_key(
            credentials.api_key.expose_secret(),
            credentials.default_model,
        )?)),
        TenantLlmProvider::Groq => Ok(ChatProvider::Embacle(http_env::groq_with_key(
            credentials.api_key.expose_secret().to_owned(),
            credentials.default_model,
        ))),
        TenantLlmProvider::Cohere => Ok(ChatProvider::Embacle(http_env::cohere_with_key(
            credentials.api_key.expose_secret().to_owned(),
            credentials.default_model,
        ))),
        TenantLlmProvider::Local => {
            let base_url = credentials
                .base_url
                .unwrap_or_else(|| "http://localhost:11434/v1".to_owned());
            let model = credentials
                .default_model
                .unwrap_or_else(|| "qwen2.5:14b-instruct".to_owned());
            let api_key = Some(credentials.api_key).filter(|key| !key.expose_secret().is_empty());
            Ok(ChatProvider::Embacle(http_env::local_from_credentials(
                base_url,
                api_key.as_ref().map(|s| s.expose_secret().to_owned()),
                model,
            )))
        }
        TenantLlmProvider::OpenAi | TenantLlmProvider::Anthropic => Err(AppError::config(format!(
            "{} provider is not yet supported. Use Gemini, Groq, Cohere, or Local.",
            credentials.provider
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use embacle::types::{
        ChatRequest as RunnerRequest, ChatResponse as RunnerResponse, ChatStream as RunnerStream,
        LlmCapabilities, LlmProvider as RunnerProvider, RunnerError,
    };
    use pierre_auth::tenant::llm_manager::{
        CredentialSource, LlmCredentials, LlmProvider as TenantLlmProvider,
    };
    use pierre_core::models::TenantId;
    use pierre_llm::chain_guard::CHAIN_GUARD;
    use pierre_llm::health::{LlmHealthState, LlmHealthStatus};
    use pierre_llm::{ChatProvider, EmbacleProvider, QuotaCooldowns};
    use secrecy::SecretString;
    use tokio::sync::{Mutex, MutexGuard};
    use uuid::Uuid;

    use super::{
        builds_from_credentials, chat_provider_from_credentials, run_recovery_tick,
        run_startup_probe,
    };

    /// A tier that rejects every request as invalid while `alive` is unset —
    /// the shape Anthropic gave a spent credit on 2026-11-20 — and answers
    /// once it is set.
    struct Account {
        label: &'static str,
        alive: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
        models: Vec<String>,
    }

    /// What a test reads and flips on an [`Account`] the chain owns.
    struct Handle {
        alive: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
    }

    impl Handle {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    fn account(label: &'static str, alive: bool) -> (EmbacleProvider, Handle) {
        let alive = Arc::new(AtomicBool::new(alive));
        let calls = Arc::new(AtomicUsize::new(0));
        let runner = Account {
            label,
            alive: Arc::clone(&alive),
            calls: Arc::clone(&calls),
            models: vec![format!("{label}-model")],
        };
        (
            EmbacleProvider::from_runner(Box::new(runner), label),
            Handle { alive, calls },
        )
    }

    #[async_trait]
    impl RunnerProvider for Account {
        fn name(&self) -> &'static str {
            self.label
        }

        fn display_name(&self) -> &str {
            self.label
        }

        fn capabilities(&self) -> LlmCapabilities {
            LlmCapabilities::SYSTEM_MESSAGES
        }

        fn default_model(&self) -> &str {
            &self.models[0]
        }

        fn available_models(&self) -> &[String] {
            &self.models
        }

        async fn complete(&self, _request: &RunnerRequest) -> Result<RunnerResponse, RunnerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if !self.alive.load(Ordering::SeqCst) {
                return Err(RunnerError::invalid_request(
                    "Your credit balance is too low to access the Anthropic API.",
                ));
            }
            Ok(RunnerResponse {
                replay_blocks: None,
                content: "pong".to_owned(),
                model: self.models[0].clone(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }

        async fn complete_stream(
            &self,
            _request: &RunnerRequest,
        ) -> Result<RunnerStream, RunnerError> {
            Err(RunnerError::internal("not streamed in this test"))
        }

        async fn health_check(&self) -> Result<bool, RunnerError> {
            Ok(true)
        }
    }

    /// `CHAIN_GUARD`'s circuit breaker is one per process and counts a
    /// primary's faults, so the tests that run a chain hold this for their
    /// whole body and start from a closed circuit.
    async fn fresh_chain_guard() -> MutexGuard<'static, ()> {
        static CHAIN_GUARD_USERS: Mutex<()> = Mutex::const_new(());
        let held = CHAIN_GUARD_USERS.lock().await;
        CHAIN_GUARD.record_primary_success();
        held
    }

    fn chain_of(tiers: Vec<EmbacleProvider>) -> Arc<ChatProvider> {
        Arc::new(ChatProvider::Embacle(
            EmbacleProvider::chain(tiers, QuotaCooldowns::default()).expect("tiers"),
        ))
    }

    /// Revisions 01830 to 01841 on 2026-11-21: the head was dead at every
    /// boot and six tiers behind it answered. The instance must come up
    /// `Healthy`, served by the first live tier, so the gate admits turns.
    #[tokio::test]
    async fn a_dead_head_at_boot_leaves_the_chain_healthy_and_benched() {
        let _chain_guard = fresh_chain_guard().await;
        let (head_tier, head) = account("anthropic", false);
        let (second_tier, _) = account("anthropic#2", true);
        let chain = chain_of(vec![head_tier, second_tier]);
        let health = LlmHealthState::new();

        run_startup_probe("anthropic", &health, Some(&chain)).await;

        let snapshot = health.snapshot().await;
        assert_eq!(snapshot.status, LlmHealthStatus::Healthy);
        assert_eq!(snapshot.served_by.as_deref(), Some("anthropic#2-model"));
        assert!(chain.is_tier_benched("anthropic"));
        assert_eq!(
            head.calls(),
            1,
            "the head is asked once, alone; the chain probe passes it over"
        );
    }

    /// A boot that finds no tier alive is `Unhealthy`, and stays so only
    /// until a tier answers again: the next recovery tick brings the instance
    /// back without a new revision.
    #[tokio::test]
    async fn an_instance_that_booted_unhealthy_recovers_when_a_tier_answers() {
        let _chain_guard = fresh_chain_guard().await;
        let (head_tier, head) = account("anthropic", false);
        let (last_tier, last) = account("copilot_sdk", false);
        let chain = chain_of(vec![head_tier, last_tier]);
        let health = LlmHealthState::new();

        run_startup_probe("anthropic", &health, Some(&chain)).await;
        assert_eq!(
            health.snapshot().await.status,
            LlmHealthStatus::Unhealthy,
            "no tier answers"
        );

        run_recovery_tick("anthropic", &health, Some(&chain)).await;
        assert_eq!(
            health.snapshot().await.status,
            LlmHealthStatus::Unhealthy,
            "still nobody"
        );

        last.alive.store(true, Ordering::SeqCst);
        run_recovery_tick("anthropic", &health, Some(&chain)).await;

        let snapshot = health.snapshot().await;
        assert_eq!(snapshot.status, LlmHealthStatus::Healthy);
        assert_eq!(snapshot.served_by.as_deref(), Some("copilot_sdk-model"));
        assert!(
            chain.is_tier_benched("anthropic"),
            "the head is still dead and stays benched"
        );
        assert!(!chain.is_tier_benched("copilot_sdk"));

        head.alive.store(true, Ordering::SeqCst);
        run_recovery_tick("anthropic", &health, Some(&chain)).await;
        assert!(
            !chain.is_tier_benched("anthropic"),
            "the head answers again and is reinstated"
        );
    }

    /// A provider that is not a chain has no tier to bench, and recovers the
    /// same way: the recovery tick asks it again while it reads `Unhealthy`,
    /// and stops asking once it answers.
    #[tokio::test]
    async fn a_solo_provider_that_booted_unhealthy_recovers_when_it_answers() {
        let (solo_tier, solo) = account("anthropic", false);
        let provider = Arc::new(ChatProvider::Embacle(solo_tier));
        let health = LlmHealthState::new();

        run_startup_probe("anthropic", &health, Some(&provider)).await;
        assert_eq!(
            health.snapshot().await.status,
            LlmHealthStatus::Unhealthy,
            "the only provider cannot answer"
        );

        solo.alive.store(true, Ordering::SeqCst);
        run_recovery_tick("anthropic", &health, Some(&provider)).await;

        let snapshot = health.snapshot().await;
        assert_eq!(snapshot.status, LlmHealthStatus::Healthy);
        assert_eq!(snapshot.served_by.as_deref(), Some("anthropic-model"));

        let calls_once_healthy = solo.calls();
        run_recovery_tick("anthropic", &health, Some(&provider)).await;
        assert_eq!(
            solo.calls(),
            calls_once_healthy,
            "a healthy provider is not asked again"
        );
    }

    /// The 2026-09-21 "startup only" decision: nothing is spent on a probe
    /// while the chain works. A recovery tick on a healthy chain with nothing
    /// benched asks nobody.
    #[tokio::test]
    async fn a_recovery_tick_on_a_healthy_chain_makes_no_call() {
        let _chain_guard = fresh_chain_guard().await;
        let (head_tier, head) = account("anthropic", true);
        let (second_tier, second) = account("copilot_sdk", true);
        let chain = chain_of(vec![head_tier, second_tier]);
        let health = LlmHealthState::new();

        run_startup_probe("anthropic", &health, Some(&chain)).await;
        let (head_at_boot, second_at_boot) = (head.calls(), second.calls());

        run_recovery_tick("anthropic", &health, Some(&chain)).await;

        assert_eq!(head.calls(), head_at_boot);
        assert_eq!(second.calls(), second_at_boot);
    }

    /// The predicate the BYO resolver asks is the factory's own answer, for
    /// every provider a tenant can store a key for.
    #[test]
    fn builds_from_credentials_agrees_with_the_factory() {
        for provider in [
            TenantLlmProvider::Gemini,
            TenantLlmProvider::Groq,
            TenantLlmProvider::OpenAi,
            TenantLlmProvider::Anthropic,
            TenantLlmProvider::Cohere,
            TenantLlmProvider::Local,
        ] {
            let built = chat_provider_from_credentials(LlmCredentials {
                tenant_id: TenantId::from_uuid(Uuid::new_v4()),
                user_id: None,
                provider,
                api_key: SecretString::from("test-key"),
                base_url: None,
                default_model: Some("test-model".to_owned()),
                source: CredentialSource::TenantDefault,
            });
            assert_eq!(
                builds_from_credentials(provider),
                built.is_ok(),
                "{provider}: the predicate and the factory disagree"
            );
        }
    }
}
