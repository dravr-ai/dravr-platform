// ABOUTME: Records what kind of account a TrainingPeaks connection signed in with, as TrainingPeaks reports it
// ABOUTME: A coach account is granted manages_roster (never revoked here) and its misfiled cached activities go
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `TrainingPeaks` account role
//!
//! A `TrainingPeaks` account either trains or coaches. A coach account keeps no
//! calendar of its own: its session reads the athletes on its roster, each by
//! id. The platform needs to know which it holds for three reasons, all served
//! by [`record_trainingpeaks_role`]:
//!
//! - a read of a coach account's own workouts has nothing to return, so the read
//!   path refuses it in words before any scrape once the role is recorded on the
//!   connection;
//! - a `TrainingPeaks` coach is a coach, so the account is granted
//!   `manages_roster`, the permission to coach a group. The grant is never
//!   revoked here: a coach who later signs in with an athlete account keeps a
//!   permission they may already be using;
//! - before sciotte 0.14 a coach account's list read fell through to whichever
//!   athlete TrainingPeaks served, and the platform filed those workouts as the
//!   coach's own. They are deleted when the account is found to be a coach's.
//!
//! The role is read by [`probe_trainingpeaks_role`], which runs after every
//! `TrainingPeaks` login and once for a connection made before roles were
//! recorded, and is also learned from the scraper's refusal of a coach
//! account's own read, which the read path records.

use pierre_core::constants::oauth_providers::SCIOTTE_TRAININGPEAKS;
use pierre_core::errors::AppResult;
use pierre_core::models::{ProviderAccountRole, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_providers::sciotte_provider::SciotteTarget;
use pierre_providers::sciotte_remote::{
    AccountRole, AthleteProfile, AuthSession, RemoteSciotteClient,
};
use tracing::info;
use uuid::Uuid;

/// Record that `user_id`'s `TrainingPeaks` connection in `tenant` signed in
/// with an account of `role`.
///
/// The role lands on the connection row, where the read path and the
/// providers card read it. A [`ProviderAccountRole::Coach`] account is also
/// granted `manages_roster` when it does not hold it — written only ever to
/// `true` — and loses the cached `TrainingPeaks` activities filed under it,
/// which can only be another athlete's workouts misfiled before sciotte 0.14.
///
/// # Errors
///
/// Returns the repository error when the role, the grant or the purge cannot
/// be written.
pub async fn record_trainingpeaks_role(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    role: ProviderAccountRole,
) -> AppResult<()> {
    // The role is written last: a recorded role is what tells the next read
    // (and the probe that skips an unchanged role) that the grant and the
    // purge are done, so a failure before it leaves the role unread and the
    // whole step runs again.
    if role == ProviderAccountRole::Coach {
        grant_roster(repos, user_id).await?;
        purge_misfiled_activities(repos, user_id, tenant).await?;
    }
    let recorded = repos
        .provider_connections
        .set_account_role(user_id, tenant, SCIOTTE_TRAININGPEAKS, role)
        .await?;
    info!(
        user_id = %user_id,
        tenant_id = %tenant,
        role = %role,
        recorded,
        "TrainingPeaks account role read"
    );
    Ok(())
}

/// Grant `manages_roster` to a `TrainingPeaks` coach account that does not
/// hold it. Only ever writes `true`: nothing in this module takes it back.
async fn grant_roster(repos: &RepositoryRegistry, user_id: Uuid) -> AppResult<()> {
    let holds_grant = repos
        .users
        .get_global(user_id)
        .await?
        .is_some_and(|user| user.manages_roster);
    if !holds_grant {
        repos.users.set_manages_roster(user_id, true).await?;
        info!(user_id = %user_id, "manages_roster granted: TrainingPeaks coach account");
    }
    Ok(())
}

/// Delete the cached `TrainingPeaks` activities filed under a coach account:
/// it has no calendar of its own, so each one is another athlete's workout a
/// read before sciotte 0.14 misfiled.
async fn purge_misfiled_activities(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
) -> AppResult<()> {
    let removed = repos
        .activity_cache
        .delete_provider_activities(user_id, &tenant, SCIOTTE_TRAININGPEAKS)
        .await?;
    if removed > 0 {
        info!(
            user_id = %user_id,
            tenant_id = %tenant,
            removed,
            "Deleted cached TrainingPeaks activities filed under a coach account"
        );
    }
    Ok(())
}

/// The profile of the `TrainingPeaks` account `session` signed in with, read
/// on the scraper service: its names, whether it trains or coaches, and a
/// coach account's roster.
///
/// # Errors
///
/// Returns an error when the scraper service is not configured, or when it
/// cannot import the session or read the profile.
pub async fn trainingpeaks_profile(session: &AuthSession) -> AppResult<AthleteProfile> {
    let remote = RemoteSciotteClient::require_from_env()?;
    remote
        .import_session(
            session,
            SciotteTarget::TrainingPeaks.scraper_provider_name(),
        )
        .await?;
    remote.get_athlete(&session.session_id).await
}

/// The platform's account role for a `TrainingPeaks` profile: a coach when
/// `TrainingPeaks` says so, and otherwise an athlete, whose own calendar the
/// read path goes on reading.
#[must_use]
pub fn account_role(profile: &AthleteProfile) -> ProviderAccountRole {
    match profile.role {
        Some(AccountRole::Coach) => ProviderAccountRole::Coach,
        Some(AccountRole::Athlete) | None => ProviderAccountRole::Athlete,
    }
}

/// Read what kind of account `session` signed in with and record it
/// ([`record_trainingpeaks_role`]).
///
/// The probe costs one profile read on the scraper service, so callers run it
/// off the request that stored the session.
///
/// # Errors
///
/// Returns the scraper error when the profile cannot be read (nothing is
/// recorded then), or the repository error when recording fails.
pub async fn probe_trainingpeaks_role(
    repos: &RepositoryRegistry,
    session: &AuthSession,
    user_id: Uuid,
    tenant: TenantId,
) -> AppResult<ProviderAccountRole> {
    let profile = trainingpeaks_profile(session).await?;
    let role = account_role(&profile);
    record_trainingpeaks_role(repos, user_id, tenant, role).await?;
    Ok(role)
}
