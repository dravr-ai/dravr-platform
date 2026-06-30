// ABOUTME: LLM startup probe health state for the /ready and /health/llm endpoints
// ABOUTME: Pure value types — no pierre-server coupling — consumed by the readiness gate
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM Health Probe Types
//!
//! Shared types for the LLM startup probe and runtime health snapshot. The
//! `/ready` and `/health/llm` Axum handlers in pierre-server consume
//! [`LlmHealthState`]; the chat provider factory writes to it after every
//! probe round-trip.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Monotonic base for the lock-free last-success clock. `Instant` cannot be
/// stored in an atomic, so successes are recorded as milliseconds elapsed
/// since this process-wide base.
static PROCESS_START: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Milliseconds elapsed since [`PROCESS_START`], saturating instead of
/// truncating (the value would only exceed `u64` after ~584 million years).
fn elapsed_ms() -> u64 {
    u64::try_from(PROCESS_START.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Status reported by the LLM startup probe.
///
/// `Unknown` is the boot default — the `/ready` route returns 200 in that
/// window so a slow LLM round-trip does not delay Cloud Run's first traffic
/// shift. Once the probe finishes, the status flips to `Healthy` or
/// `Unhealthy` and stays there for the lifetime of the process (re-probing
/// is the fallback chain's and the alert layer's job, not the readiness
/// gate's).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LlmHealthStatus {
    /// Probe has not finished yet (boot grace period).
    #[default]
    Unknown,
    /// Probe round-trip succeeded.
    Healthy,
    /// Probe round-trip failed; the recorded error is in [`LlmHealthSnapshot`].
    Unhealthy,
}

impl fmt::Display for LlmHealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => write!(f, "unknown"),
            Self::Healthy => write!(f, "healthy"),
            Self::Unhealthy => write!(f, "unhealthy"),
        }
    }
}

/// Snapshot of the most recent LLM probe result, exposed via `/health/llm`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmHealthSnapshot {
    /// Probe status — defaults to [`LlmHealthStatus::Unknown`] until the
    /// boot probe has reported.
    pub status: LlmHealthStatus,
    /// Provider name from `PIERRE_LLM_PROVIDER` at boot, or `None` when
    /// the probe has not yet run.
    pub provider: Option<String>,
    /// First-line error reason when `status == Unhealthy`; `None` otherwise.
    pub error: Option<String>,
    /// Timestamp of the last probe result, or `None` before the first probe.
    pub checked_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Default for LlmHealthSnapshot {
    fn default() -> Self {
        Self {
            status: LlmHealthStatus::Unknown,
            provider: None,
            error: None,
            checked_at: None,
        }
    }
}

/// Shared LLM health state populated by the startup probe and consumed by
/// the `/ready` and `/health/llm` Axum handlers.
///
/// Cloud Run's startup probe defaults to `/health` (matched by the Docker
/// `HEALTHCHECK`). Operators who want a hard gate on the boot-time LLM
/// round-trip can point the startup probe at `/ready` instead — that
/// handler returns 503 while `status == Unhealthy`.
pub struct LlmHealthState {
    inner: RwLock<LlmHealthSnapshot>,
    /// Milliseconds (since [`PROCESS_START`]) of the last successful LLM
    /// round-trip recorded by a real chat turn; `0` means none yet. Kept
    /// lock-free so the chat hot path can stamp it without contending on the
    /// snapshot `RwLock`. Read by the periodic health probe to skip its own
    /// (billed) round-trip when real traffic has already proven liveness.
    last_success_ms: AtomicU64,
}

impl Default for LlmHealthState {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmHealthState {
    /// Build a fresh state with `Unknown` status.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(LlmHealthSnapshot::default()),
            last_success_ms: AtomicU64::new(0),
        }
    }

    /// Record that a real chat turn just completed a live LLM round-trip.
    ///
    /// The periodic health probe reads [`Self::since_last_success`] and skips
    /// its own billed probe when a real turn has proven the provider live
    /// within the probe interval. Lock-free and cheap — safe on the hot path.
    pub fn note_success(&self) {
        self.last_success_ms
            .store(elapsed_ms().max(1), Ordering::Relaxed);
    }

    /// Time since the last real successful round-trip, or `None` if none has
    /// been recorded yet this process.
    #[must_use]
    pub fn since_last_success(&self) -> Option<Duration> {
        match self.last_success_ms.load(Ordering::Relaxed) {
            0 => None,
            recorded => Some(Duration::from_millis(elapsed_ms().saturating_sub(recorded))),
        }
    }

    /// Record a successful probe result. Returns the previous status so
    /// callers can detect transitions and log accordingly (e.g. emit
    /// `error!` on `Healthy -> Unhealthy` so the tronc Slack layer pages).
    pub async fn record_healthy(&self, provider: impl Into<String>) -> LlmHealthStatus {
        let mut guard = self.inner.write().await;
        let previous = guard.status;
        *guard = LlmHealthSnapshot {
            status: LlmHealthStatus::Healthy,
            provider: Some(provider.into()),
            error: None,
            checked_at: Some(chrono::Utc::now()),
        };
        previous
    }

    /// Record a failed probe result. The error message is truncated to a
    /// single line so it stays grep-able in the readiness JSON payload.
    /// Returns the previous status so callers can detect transitions.
    pub async fn record_unhealthy(
        &self,
        provider: impl Into<String>,
        error: impl Into<String>,
    ) -> LlmHealthStatus {
        let provider = provider.into();
        let raw = error.into();
        let first_line = raw.lines().next().unwrap_or(raw.as_str()).to_owned();
        let mut guard = self.inner.write().await;
        let previous = guard.status;
        *guard = LlmHealthSnapshot {
            status: LlmHealthStatus::Unhealthy,
            provider: Some(provider),
            error: Some(first_line),
            checked_at: Some(chrono::Utc::now()),
        };
        previous
    }

    /// Copy the current snapshot for a readiness/JSON response.
    pub async fn snapshot(&self) -> LlmHealthSnapshot {
        self.inner.read().await.clone()
    }
}
