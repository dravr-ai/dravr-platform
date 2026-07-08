// ABOUTME: Repository trait for durable per-user onboarding step state (profile-type, connect, coach, messaging)
// ABOUTME: Server-driven completion that survives device changes, replacing the web flow's localStorage-only flags
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

/// One persisted onboarding step state for a user.
///
/// A record exists only for a step the user has reached; a step with no record is
/// still pending. `chosen_channel` is set only on the messaging-channel step (the
/// messaging app the user picked); it is `None` for every other step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingStepRecord {
    /// Step identifier (`profile_type`, `connect_provider`, `coach_proposal`,
    /// `messaging_channel`, `messaging_configure`).
    pub step_id: String,
    /// `complete` or `skipped`.
    pub status: String,
    /// The messaging channel chosen at the `messaging_channel` step, if any.
    pub chosen_channel: Option<String>,
}

/// Durable onboarding progress, keyed by `(user_id, step_id)`.
///
/// Scoped by `user_id` — the auth-token-derived isolation boundary, mirroring the
/// provider-connection onboarding gate, which also reads by `user_id` alone.
/// `tenant_id` is a best-effort audit column (the onboarding JWT may carry no
/// active tenant), never a filter.
#[async_trait]
pub trait UserOnboardingRepository: Send + Sync {
    /// Upsert a step's `status` (and, for the messaging-channel step, the chosen
    /// channel) for a user. Re-marking an existing step overwrites its status.
    async fn set_onboarding_step(
        &self,
        user_id: &str,
        step_id: &str,
        status: &str,
        chosen_channel: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<()>;

    /// Every recorded step state for a user (rows exist only for reached steps).
    async fn get_onboarding_steps(&self, user_id: &str) -> AppResult<Vec<OnboardingStepRecord>>;
}
