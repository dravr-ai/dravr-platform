// ABOUTME: Operator reset of one user's onboarding, so the wizard and the guided walks run again from the start
// ABOUTME: Disconnects providers through the chokepoint, keeps coaching platforms (groups ride on them), clears only the user's own rows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Returning a user to the start of onboarding, from an operator surface.
//!
//! Whether onboarding runs is derived, not flagged: the wizard's provider
//! steps show while the user holds no real provider connection, each step
//! shows until its `user_onboarding` row exists, and the chat opens the
//! intake or the pillar walk while those rows and the pillar facts are
//! missing. A reset therefore has two halves.
//!
//! The provider half disconnects every held provider through the same
//! chokepoint [`crate::user_removal`] uses, so each grant is revoked at the
//! provider rather than orphaned there. A coaching platform
//! ([`coach_platform`]: a provider declaring a coach roster, such as
//! `TrainingPeaks` or Intervals.icu) is the exception and
//! is kept: its disconnect ends every delegated coach and member link — and
//! for `TrainingPeaks` revokes an earned `manages_roster` — which is group
//! state a reset must not touch. A provider this build cannot revoke is kept too, since only the
//! chokepoint may clear a provider's rows. Either one kept leaves the user
//! connected, so the wizard's provider steps stay hidden; the report names it.
//!
//! The data half is [`UserOnboardingRepository::reset_onboarding`], one
//! transaction over the user's own rows, optionally widened to everything the
//! agents remember about them.
//!
//! [`UserOnboardingRepository::reset_onboarding`]: pierre_database::repositories::UserOnboardingRepository::reset_onboarding

use std::collections::BTreeMap;

use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::OnboardingResetScope;
use pierre_database::RepositoryRegistry;
use pierre_providers::registry::global_registry;
use serde::Serialize;
use tracing::info;
use uuid::Uuid;

use crate::coach_platform::coach_platform;
use crate::provider_revocation::DisconnectReason;
use crate::user_removal::{
    disconnect_each, held_providers, DisconnectedProvider, HeldProvider, Interruption,
    ProviderDisconnector,
};

/// Everything an onboarding reset did, for the operator's response.
#[derive(Debug, Clone, Default, Serialize)]
pub struct OnboardingResetReport {
    /// Providers disconnected through the chokepoint, each with what the
    /// provider said about its grant.
    pub disconnected: Vec<DisconnectedProvider>,
    /// Coaching-platform connections left in place because disconnecting
    /// them changes group state (delegated links, the earned roster grant).
    pub kept_for_groups: Vec<HeldProvider>,
    /// Providers this build cannot revoke, left in place because only the
    /// disconnect chokepoint may clear a provider's rows.
    pub not_revocable: Vec<HeldProvider>,
    /// Rows the data half deleted or updated, by label.
    pub rows_changed: BTreeMap<String, u64>,
}

impl OnboardingResetReport {
    /// Whether the user still holds a provider after the reset, which keeps
    /// the wizard's provider steps (and the steps ahead of them) hidden.
    #[must_use]
    pub fn still_connected(&self) -> bool {
        !self.kept_for_groups.is_empty() || !self.not_revocable.is_empty()
    }
}

/// The outcome of an onboarding reset.
#[derive(Debug, Clone)]
pub enum OnboardingResetOutcome {
    /// Both halves ran.
    Reset(OnboardingResetReport),
    /// A disconnect or the data reset failed; what was done first stands, and
    /// no onboarding row was changed (the data half is one transaction and
    /// runs last).
    Interrupted(Interruption),
}

/// What an interrupted reset already did and what it did not, as an operator
/// reads it; `cause` is the failure as the caller may show it.
#[must_use]
pub fn describe_interruption(interruption: &Interruption, cause: &str) -> String {
    let done = interruption.describe_disconnected();
    interruption.failed.as_ref().map_or_else(
        || {
            format!(
                "The onboarding reset failed: {cause}. {done}. No onboarding record was changed."
            )
        },
        |failed| {
            format!(
                "Disconnecting {} failed: {cause}. Its grant may already have been revoked at the provider. {done}. No onboarding record was changed.",
                failed.describe()
            )
        },
    )
}

/// Whether an onboarding reset keeps `provider` connected because groups
/// rely on it: it is a coaching platform ([`coach_platform`]).
///
/// `provider` may be the user-facing name a held provider carries, which a
/// coaching platform is found by as well as by its backend. The build's
/// provider descriptors, which the global registry holds as every server's
/// registry does, say which providers are coaching platforms.
#[must_use]
pub fn kept_for_groups(provider: &str) -> bool {
    coach_platform(&global_registry(), provider).is_some()
}

/// Reset a user's onboarding.
///
/// Disconnects every held provider except `TrainingPeaks` and any this build
/// cannot revoke, then runs the data reset for `scope` in one transaction.
/// The disconnects go first, as they do for a user delete, so a failure there
/// leaves every onboarding row in place.
///
/// # Errors
///
/// Returns a database error from the reads that precede any change, and a
/// `ResourceUnavailable` error when the user holds a provider to disconnect
/// and no disconnector is wired; nothing was touched then. A failed
/// disconnect or data reset is [`OnboardingResetOutcome::Interrupted`], not an
/// error, since a disconnect may have revoked its grant before failing.
pub async fn reset_onboarding(
    repos: &RepositoryRegistry,
    disconnector: Option<&dyn ProviderDisconnector>,
    user_id: Uuid,
    scope: OnboardingResetScope,
) -> AppResult<OnboardingResetOutcome> {
    let (kept_for_groups, held): (Vec<HeldProvider>, Vec<HeldProvider>) =
        held_providers(repos, user_id)
            .await?
            .into_iter()
            .partition(|target| kept_for_groups(&target.provider));

    let mut report = OnboardingResetReport {
        kept_for_groups,
        ..OnboardingResetReport::default()
    };
    if !held.is_empty() {
        let Some(disconnector) = disconnector else {
            return Err(AppError::resource_unavailable(
                "Provider disconnect is not wired on this server; resetting would leave the user's grants authorized at the provider",
            ));
        };
        let (revocable, not_revocable): (Vec<HeldProvider>, Vec<HeldProvider>) = held
            .into_iter()
            .partition(|target| disconnector.supports(&target.provider));
        report.not_revocable = not_revocable;
        report.disconnected =
            match disconnect_each(disconnector, user_id, revocable, DisconnectReason::Operator)
                .await
            {
                Ok(disconnected) => disconnected,
                Err(interruption) => return Ok(OnboardingResetOutcome::Interrupted(*interruption)),
            };
    }

    let reset = match repos
        .user_onboarding
        .reset_onboarding(&user_id.to_string(), scope)
        .await
    {
        Ok(reset) => reset,
        Err(error) => {
            return Ok(OnboardingResetOutcome::Interrupted(Interruption {
                disconnected: report.disconnected,
                failed: None,
                error,
            }))
        }
    };
    report.rows_changed = reset.rows_changed;

    info!(
        user_id = %user_id,
        with_memory = scope == OnboardingResetScope::WithMemory,
        disconnected = report.disconnected.len(),
        kept_for_groups = report.kept_for_groups.len(),
        not_revocable = report.not_revocable.len(),
        tables_changed = report.rows_changed.len(),
        "Onboarding reset by operator"
    );
    Ok(OnboardingResetOutcome::Reset(report))
}
