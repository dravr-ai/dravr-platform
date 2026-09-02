// ABOUTME: The persona push-policy gate for dispatched notifications — persona, contract, arming flag
// ABOUTME: Implements the pierre-notifications PersonaPolicyGate SPI beside the messaging sink

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persona notification-policy resolution.
//!
//! The persona contracts synced from contremaitre carry a
//! `notification: { tier_floor, digest }` block per persona. This gate turns
//! that block plus the user's stored persona and the
//! `persona_notification_policy` feature flag into the [`PushPolicy`] the
//! dispatch facade consults on every [`PushTier`]-carrying dispatch.
//!
//! It hangs off the [`PersonaPolicyGate`] SPI (like
//! [`crate::notification_channel_sink::MessagingChannelSink`] hangs off the
//! channel-sink SPI) because the user repository, contract registry and
//! feature-flag store all live above `pierre-notifications`.
//!
//! [`PushTier`]: pierre_notifications::PushTier

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::feature_flags::FeatureKey;
use pierre_database::RepositoryRegistry;
use pierre_notifications::{DigestCadence, PersonaPolicyGate, PushPolicy, PushTier, TenantId};
use tracing::debug;
use uuid::Uuid;

/// Resolves a user's persona push policy from the live contract registry.
pub struct PersonaNotificationPolicyGate {
    /// Repository registry — supplies the user row (persona) and the
    /// feature-flag resolution (arming).
    repos: Arc<RepositoryRegistry>,
    /// Hot-reloaded persona contracts carrying each persona's
    /// `notification` policy block.
    contracts: Arc<PersonaContractRegistry>,
}

impl PersonaNotificationPolicyGate {
    /// Build the gate from the assembled repositories and contract registry.
    #[must_use]
    pub const fn new(
        repos: Arc<RepositoryRegistry>,
        contracts: Arc<PersonaContractRegistry>,
    ) -> Self {
        Self { repos, contracts }
    }
}

#[async_trait]
impl PersonaPolicyGate for PersonaNotificationPolicyGate {
    /// Resolve the policy, or `None` when none applies.
    ///
    /// A contract applies only when the hydrated snapshot **explicitly
    /// contains the user's persona** — this deliberately bypasses the
    /// snapshot's Casual fallback lookup. Gating is suppressive, so an
    /// unknown or not-yet-synced persona must resolve to "no gate", never to
    /// Casual's P0 floor, which would near-mute the user on a registry gap.
    /// The same permissive rule applies inside the labels: an unrecognized
    /// `tier_floor` or `digest` value resolves to `None`.
    ///
    /// Lookup failures (user gone, database error) also answer `None`; the
    /// gate is best-effort and a failure must degrade to pre-gate delivery,
    /// not to suppression.
    async fn policy_for(&self, user_id: Uuid, tenant_id: TenantId) -> Option<PushPolicy> {
        let user = match self.repos.users.get_global(user_id).await {
            Ok(user) => user?,
            Err(e) => {
                debug!(%user_id, error = %e, "persona policy gate: user lookup failed; no gate");
                return None;
            }
        };
        let persona = user.coaching_persona;

        let snapshot = self.contracts.snapshot();
        let contract = snapshot.by_slug.get(persona.as_str())?;
        let floor = contract
            .notification
            .tier_floor
            .as_deref()
            .and_then(|label| PushTier::from_str(label).ok());
        let digest = contract
            .notification
            .digest
            .as_deref()
            .and_then(|label| DigestCadence::from_str(label).ok());

        let armed = match self
            .repos
            .feature_flags
            .resolve_for_user(tenant_id.0, user_id)
            .await
        {
            Ok(flags) => flags
                .get(&FeatureKey::PersonaNotificationPolicy)
                .copied()
                .unwrap_or(false),
            Err(e) => {
                debug!(%user_id, error = %e, "persona policy gate: flag resolution failed; shadow");
                false
            }
        };

        Some(PushPolicy {
            persona: persona.as_str().to_owned(),
            floor,
            digest,
            armed,
        })
    }
}
