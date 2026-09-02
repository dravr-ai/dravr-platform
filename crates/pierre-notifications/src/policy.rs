// ABOUTME: Push-tier ladder + per-persona notification policy consumed by the dispatch facade
// ABOUTME: Owns the PersonaPolicyGate SPI so persona resolution stays above this crate

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Persona notification policy
//!
//! The persona contracts in contremaitre promise a notification cadence per
//! persona — Casual is "P0-only unsolicited push, weekly digest", Power-athlete
//! is "the full P0/P1/P2 ladder". This module gives those promises a typed
//! shape the dispatch facade can enforce:
//!
//! - [`PushTier`] — the P0–P3 urgency ladder every dispatched event declares.
//! - [`PushPolicy`] — one user's resolved policy: tier floor, digest cadence,
//!   and whether enforcement is armed.
//! - [`PersonaPolicyGate`] — the SPI through which the facade asks for a
//!   policy. Implemented above this crate (persona + contract + feature-flag
//!   lookups live in `pierre-services`), mirroring how
//!   [`crate::NotificationChannelSink`] keeps messaging above this crate.
//!
//! ## Floor semantics
//!
//! A floor of `Pn` means **only tiers ≤ `Pn` deliver unsolicited** — the floor
//! names the *highest* tier number the user still receives. Casual's `P0`
//! floor therefore gates P1, P2 and P3 while P0 always delivers. `None` (no
//! floor configured, or an unrecognized label) is permissive: every tier
//! delivers.

use std::error::Error;
use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use uuid::Uuid;

use crate::TenantId;

/// Urgency tier of a dispatched notification, `P0` most urgent.
///
/// Ordered so a tier comparison reads as urgency: `P0 < P1 < P2 < P3`, and a
/// notification is persona-gated exactly when `tier > floor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PushTier {
    /// Break-glass: delivered to every persona, always.
    P0,
    /// High-signal events (coach messages, reauth prompts).
    P1,
    /// Advisory alerts (training load, recovery, verdicts).
    P2,
    /// Ambient noise (sync confirmations, celebrations, digest fodder).
    P3,
}

impl PushTier {
    /// Canonical label, matching the `tier_floor` values in
    /// contremaitre's `persona_contracts.yaml`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::P0 => "P0",
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
        }
    }
}

impl fmt::Display for PushTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by [`PushTier::from_str`] for a label outside the P0–P3
/// ladder.
///
/// Callers resolving a persona contract treat this as **no floor** (see
/// [`PushPolicy::floor`]): an unknown label must never collapse to some
/// persona's floor — falling back to Casual's `P0` would near-mute the user on
/// a contremaitre typo, so the failure mode is deliberately "deliver
/// everything", the pre-gate behaviour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPushTier(pub String);

impl fmt::Display for UnknownPushTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown push tier '{}'", self.0)
    }
}

impl Error for UnknownPushTier {}

impl FromStr for PushTier {
    type Err = UnknownPushTier;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "P0" | "p0" => Ok(Self::P0),
            "P1" | "p1" => Ok(Self::P1),
            "P2" | "p2" => Ok(Self::P2),
            "P3" | "p3" => Ok(Self::P3),
            other => Err(UnknownPushTier(other.to_owned())),
        }
    }
}

/// Digest cadence a persona contract prescribes for gated notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DigestCadence {
    /// One digest per day.
    Daily,
    /// One digest per week — the only cadence the digest scheduler batches
    /// today; see `pierre_services::notification_digest_scheduler`.
    Weekly,
    /// Digest attached to each training session (Power-athlete request).
    PerSession,
    /// Digest rolled up per coached athlete (Coach request).
    PerAthlete,
}

impl DigestCadence {
    /// Canonical label, matching the `digest` values in contremaitre's
    /// `persona_contracts.yaml`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::PerSession => "per_session",
            Self::PerAthlete => "per_athlete",
        }
    }
}

impl fmt::Display for DigestCadence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by [`DigestCadence::from_str`] for an uncatalogued label.
/// Treated as **no digest** by policy resolution — same permissive rationale
/// as [`UnknownPushTier`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownDigestCadence(pub String);

impl fmt::Display for UnknownDigestCadence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown digest cadence '{}'", self.0)
    }
}

impl Error for UnknownDigestCadence {}

impl FromStr for DigestCadence {
    type Err = UnknownDigestCadence;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "daily" => Ok(Self::Daily),
            "weekly" => Ok(Self::Weekly),
            "per_session" => Ok(Self::PerSession),
            "per_athlete" => Ok(Self::PerAthlete),
            other => Err(UnknownDigestCadence(other.to_owned())),
        }
    }
}

/// One user's resolved notification policy, as answered by a
/// [`PersonaPolicyGate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushPolicy {
    /// Persona slug the policy was resolved from (`casual`, `enthusiast`,
    /// `power_athlete`, `coach`). Carried so shadow-verdict logs can be
    /// analysed per persona before arming.
    pub persona: String,
    /// Highest tier number delivered unsolicited; `None` = no gate. An
    /// unrecognized `tier_floor` label resolves to `None` too — permissive,
    /// never someone else's floor (see [`UnknownPushTier`]).
    pub floor: Option<PushTier>,
    /// Digest cadence for gated notifications; `None` when unset or the
    /// label is uncatalogued.
    pub digest: Option<DigestCadence>,
    /// Whether enforcement is armed for this user
    /// (`FeatureKey::PersonaNotificationPolicy`). Unarmed policies produce
    /// shadow-verdict logs only and every notification still delivers.
    pub armed: bool,
}

impl PushPolicy {
    /// Whether a notification at `tier` falls above this policy's floor.
    ///
    /// Floor `Pn` delivers tiers ≤ `Pn` only, so `tier > floor` is gated;
    /// no floor gates nothing. This is the *would-gate* verdict independent
    /// of [`Self::armed`] — the facade decides whether to enforce or only
    /// log it.
    #[must_use]
    pub fn gates(&self, tier: PushTier) -> bool {
        self.floor.is_some_and(|floor| tier > floor)
    }
}

/// Resolves the persona notification policy for a user.
///
/// Implemented once, by `pierre_services::persona_notification_policy_gate`,
/// and consumed by [`crate::NotificationService::dispatch_with_tier`]. An SPI
/// rather than a direct dependency for the same reason as
/// [`crate::NotificationChannelSink`]: the user repository, persona-contract
/// registry and feature-flag store all live above this crate.
#[async_trait]
pub trait PersonaPolicyGate: Send + Sync {
    /// The resolved policy for `user_id` in `tenant_id`, or `None` when no
    /// policy applies — unknown user, contract registry not hydrated, or the
    /// user's persona absent from the hydrated snapshot. `None` means the
    /// dispatch proceeds exactly as it did before persona gating existed.
    async fn policy_for(&self, user_id: Uuid, tenant_id: TenantId) -> Option<PushPolicy>;
}
