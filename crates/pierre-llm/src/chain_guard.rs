// ABOUTME: Process-wide guard state protecting the runtime fallback chain's primary
// ABOUTME: GitHub rate-limit headroom + circuit breaker for preemptive fallback

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Chain Guard
//!
//! Shared state for two preemptive-fallback signals consulted by the
//! `EmbacleProvider` chain's observer (`chain_observer`):
//!
//! 1. **GitHub rate-limit headroom (Strategy A).** The periodic probe in
//!    pierre-services reads the core budget of the token Copilot spends
//!    through embacle's [`GithubHeadroomChecker::rate_limit`] (the checker
//!    [`copilot_headroom_checker`] builds, the same one the quota router
//!    meters Copilot with) and records each reading here via
//!    [`ChainGuard::record_github_headroom`]. Before asking a tier that
//!    spends the Copilot token, Chain reads
//!    [`ChainGuard::is_github_budget_low`] and passes that tier over when
//!    the budget is below threshold — Copilot's session-token exchange
//!    shares this 5000/hr pool, so a near-exhausted budget is a strong
//!    predictor that the *next* Copilot call will fail with
//!    `Authentication required`. A read that fails counts as low: a budget
//!    that cannot be read is not headroom. A tier that spends another
//!    credential is never passed over for this budget, first or not.
//!
//! 2. **Circuit breaker on primary auth/rate-limit failures (Strategy B).**
//!    [`ChainGuard::record_primary_failure`] tracks consecutive
//!    auth-shaped errors from the primary. After
//!    [`CIRCUIT_FAILURE_THRESHOLD`] consecutive failures the circuit
//!    *opens* — every subsequent request skips primary for
//!    [`CIRCUIT_COOLDOWN`]. The first request after cooldown is the
//!    half-open probe: a primary success on it closes the circuit; a
//!    failure resets the cooldown.
//!
//! State is per-process and lives behind a single [`LazyLock<ChainGuard>`]
//! so both the probe (which writes A) and the chain wrapper (which
//! reads A and writes B) hit the same instance without explicit
//! plumbing through `ServerContext`. The values are `AtomicU64`/`AtomicUsize`
//! so reads on the request hot path don't take a lock.

use std::env;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use embacle::quota_http::{GithubHeadroomChecker, GithubRateLimit};
use embacle::types::RunnerError;

/// GitHub core budget threshold for preemptive fallback.
///
/// Below this remaining count, Chain passes over every tier that spends the
/// Copilot GitHub token.
/// Pierre's Copilot session token exchange spends 1 call per refresh
/// (~every few minutes), plus the periodic probe spends 1, plus any
/// tool the LLM invokes that hits api.github.com. 200 leaves enough
/// headroom for a handful of refresh cycles before the limit window
/// resets.
pub const GITHUB_BUDGET_THRESHOLD: u64 = 200;

/// Consecutive primary failures before the circuit opens.
///
/// 3 is the standard Hystrix/resilience4j default — high enough to
/// absorb a single transient blip without flipping, low enough that a
/// stuck-broken primary doesn't waste many requests.
pub const CIRCUIT_FAILURE_THRESHOLD: usize = 3;

/// Open-state cooldown for the chain circuit breaker.
///
/// 60s strikes a balance: long enough to outlast typical Copilot rate
/// windows (which reset every minute), short enough that we re-probe
/// primary often when it's only briefly degraded.
pub const CIRCUIT_COOLDOWN_SECS: u64 = 60;

/// Sentinel for "GitHub rate limit hasn't been probed yet" — chain
/// treats this as fail-open (use primary): with no Copilot token there is
/// no budget to read, and before the first read there is nothing to act
/// on. `u64::MAX` is unambiguous: no GitHub API quota can legitimately be
/// this high.
const RATE_LIMIT_UNKNOWN: u64 = u64::MAX;

/// Sentinel for "the last read of the GitHub budget failed" — chain treats
/// this as fail-closed (skip the primary) until a read succeeds. As
/// unambiguous as [`RATE_LIMIT_UNKNOWN`], one below it.
const RATE_LIMIT_UNREADABLE: u64 = u64::MAX - 1;

/// The checker whose readings feed [`ChainGuard::record_github_headroom`].
///
/// It is embacle's GitHub headroom checker on `COPILOT_GITHUB_TOKEN`, the
/// token the Copilot providers pass to their runtime, so the budget measured
/// is the one Copilot's session-token exchange spends. The quota router
/// meters its Copilot backend with a checker built here too.
///
/// `None` when the token is unset or empty: there is then no Copilot budget
/// to read, and the guard stays in its fail-open unknown state.
#[must_use]
pub fn copilot_headroom_checker() -> Option<GithubHeadroomChecker> {
    env::var("COPILOT_GITHUB_TOKEN")
        .ok()
        .filter(|token| !token.is_empty())
        .map(GithubHeadroomChecker::new)
}

/// Process-wide [`ChainGuard`] instance.
///
/// Shared by the GitHub rate-limit probe (pierre-services) and the
/// chain observer on the request path (pierre-llm). [`LazyLock`] for
/// zero-config plumbing — first access on either side initialises
/// the same instance.
pub static CHAIN_GUARD: LazyLock<ChainGuard> = LazyLock::new(ChainGuard::new);

/// Outcome of recording a GitHub rate-limit probe result.
///
/// Callers (the probe in pierre-services) compare the previous and
/// current budget tiers to decide whether to emit
/// `llm.rate_limit_low` / `llm.rate_limit_recovered` notify events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitTransition {
    /// Was below threshold and stayed below.
    StillLow,
    /// Was at or above threshold and dropped below.
    EnteredLow,
    /// Was below threshold and rose to or above.
    ExitedLow,
    /// Was at or above and stayed at or above.
    StillOk,
}

/// Outcome of recording a primary outcome on the circuit breaker.
/// Callers emit `llm.circuit_opened` / `llm.circuit_closed` notify
/// events on the transition variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitTransition {
    /// Circuit was closed and stayed closed.
    StillClosed,
    /// Circuit was open and stayed open.
    StillOpen,
    /// Circuit just opened on this event (failure threshold crossed).
    Opened,
    /// Circuit just closed on this event (primary success on half-open).
    Closed,
}

/// Shared guard state. Cheap to read from the hot path: every field
/// is an atomic primitive.
pub struct ChainGuard {
    /// `core.remaining` from the most recent github.com `/rate_limit` read.
    /// `RATE_LIMIT_UNKNOWN` until the probe has run once;
    /// `RATE_LIMIT_UNREADABLE` while the last read failed.
    github_remaining: AtomicU64,
    /// Unix-seconds when GitHub's core rate-limit window resets. Stored
    /// for telemetry; not used in the skip decision.
    github_reset_at: AtomicU64,
    /// Consecutive auth/rate-limit failures from the primary since the
    /// last success.
    consecutive_primary_failures: AtomicUsize,
    /// Unix-milliseconds when the circuit opened. `0` means closed.
    circuit_opened_at_ms: AtomicU64,
}

impl Default for ChainGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainGuard {
    /// Build a fresh guard in the fail-open initial state: rate limit
    /// unknown, circuit closed, no recorded failures.
    #[must_use]
    pub fn new() -> Self {
        Self {
            github_remaining: AtomicU64::new(RATE_LIMIT_UNKNOWN),
            github_reset_at: AtomicU64::new(0),
            consecutive_primary_failures: AtomicUsize::new(0),
            circuit_opened_at_ms: AtomicU64::new(0),
        }
    }

    /// Record one read of the Copilot token's GitHub core budget — the
    /// counts [`GithubHeadroomChecker::rate_limit`] returned, or the error
    /// that stopped it. Returns the budget-tier transition for the caller
    /// to optionally page on.
    ///
    /// Fewer than [`GITHUB_BUDGET_THRESHOLD`] remaining is low. A failed
    /// read is low too (fail closed) and keeps the last known reset time:
    /// an unreachable endpoint, a non-2xx answer or a changed shape says
    /// nothing about headroom, and must never read as plenty of it.
    pub fn record_github_headroom(
        &self,
        reading: &Result<GithubRateLimit, RunnerError>,
    ) -> RateLimitTransition {
        let (remaining, reset_at) = reading.as_ref().map_or_else(
            |_| {
                (
                    RATE_LIMIT_UNREADABLE,
                    self.github_reset_at.load(Ordering::Relaxed),
                )
            },
            |counts| {
                (
                    counts.remaining,
                    counts
                        .resets_at
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |since| since.as_secs()),
                )
            },
        );
        let was_low = budget_is_low(self.github_remaining.load(Ordering::Relaxed));
        let is_low = budget_is_low(remaining);
        self.github_remaining.store(remaining, Ordering::Relaxed);
        self.github_reset_at.store(reset_at, Ordering::Relaxed);
        match (was_low, is_low) {
            (true, true) => RateLimitTransition::StillLow,
            (false, true) => RateLimitTransition::EnteredLow,
            (true, false) => RateLimitTransition::ExitedLow,
            (false, false) => RateLimitTransition::StillOk,
        }
    }

    /// Latest known GitHub `core.remaining`. `None` when no read has
    /// succeeded since the last attempt: the probe hasn't run yet, or its
    /// last read failed.
    #[must_use]
    pub fn github_remaining(&self) -> Option<u64> {
        let raw = self.github_remaining.load(Ordering::Relaxed);
        (raw != RATE_LIMIT_UNKNOWN && raw != RATE_LIMIT_UNREADABLE).then_some(raw)
    }

    /// Latest known GitHub rate-limit reset epoch (unix seconds).
    #[must_use]
    pub fn github_reset_at(&self) -> u64 {
        self.github_reset_at.load(Ordering::Relaxed)
    }

    /// True when the last read of the GitHub budget found fewer than
    /// [`GITHUB_BUDGET_THRESHOLD`] remaining, or failed. Hot-path check
    /// used by Chain to decide whether to pass over a tier that spends the
    /// Copilot GitHub token.
    #[must_use]
    pub fn is_github_budget_low(&self) -> bool {
        budget_is_low(self.github_remaining.load(Ordering::Relaxed))
    }

    /// Record an auth-shaped primary failure. Returns the circuit
    /// transition for the caller to optionally page on. Non-auth
    /// failures should NOT call this — they shouldn't count against
    /// the circuit (we want to react to systemic rate-limit /
    /// expired-token issues, not to transient request-specific 5xxs).
    pub fn record_primary_failure(&self) -> CircuitTransition {
        let was_open = self.is_circuit_open();
        let count = self
            .consecutive_primary_failures
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        if count >= CIRCUIT_FAILURE_THRESHOLD && !was_open {
            self.circuit_opened_at_ms.store(now_ms(), Ordering::Relaxed);
            CircuitTransition::Opened
        } else if was_open {
            // Half-open probe failed — reset the cooldown clock so the
            // next probe doesn't fire until another full window passes.
            self.circuit_opened_at_ms.store(now_ms(), Ordering::Relaxed);
            CircuitTransition::StillOpen
        } else {
            CircuitTransition::StillClosed
        }
    }

    /// Record a successful primary call. Resets the consecutive-failure
    /// counter and, if the circuit was open, closes it.
    pub fn record_primary_success(&self) -> CircuitTransition {
        let was_open = self.is_circuit_open();
        self.consecutive_primary_failures
            .store(0, Ordering::Relaxed);
        if was_open {
            self.circuit_opened_at_ms.store(0, Ordering::Relaxed);
            CircuitTransition::Closed
        } else {
            CircuitTransition::StillClosed
        }
    }

    /// True when the circuit is currently open (cooldown still
    /// active). `record_primary_success` and the cooldown-elapsed
    /// check are the only ways to flip back to closed.
    #[must_use]
    pub fn is_circuit_open(&self) -> bool {
        let opened_at = self.circuit_opened_at_ms.load(Ordering::Relaxed);
        if opened_at == 0 {
            return false;
        }
        let elapsed_ms = now_ms().saturating_sub(opened_at);
        elapsed_ms < CIRCUIT_COOLDOWN_SECS * 1_000
    }
}

/// Whether a stored budget figure is a reason to pass over a Copilot tier:
/// a read below the threshold, or a failed read. Never-read is not.
const fn budget_is_low(stored: u64) -> bool {
    match stored {
        RATE_LIMIT_UNKNOWN => false,
        RATE_LIMIT_UNREADABLE => true,
        remaining => remaining < GITHUB_BUDGET_THRESHOLD,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
