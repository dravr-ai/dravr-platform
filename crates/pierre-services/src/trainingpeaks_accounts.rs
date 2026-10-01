// ABOUTME: Records what kind of account a TrainingPeaks connection signed in with, as TrainingPeaks reports it
// ABOUTME: A coach account whose email is the user's verified email holds manages_roster while it stays connected
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
//!   `manages_roster`, the permission to coach a group — but only when the
//!   coach account is the user's own: its `TrainingPeaks` email must be the
//!   user's verified Dravr email, case aside ([`email_binding`]). A coach
//!   account that shares no email, or another person's, earns nothing, and
//!   the user's Dravr email proves nothing until it is verified. The grant
//!   lasts as long as the reason for it: when the connection that earned it
//!   signs in again as an athlete or as a coach account that is not the
//!   user's, or is disconnected ([`revoke_roster_for_coach_connection`]), the
//!   grant goes unless another `TrainingPeaks` coach connection still holds it
//!   or an operator made it;
//! - before sciotte 0.14 a coach account's list read fell through to whichever
//!   athlete TrainingPeaks served, and the platform filed those workouts as the
//!   coach's own. They are deleted when the account is found to be a coach's.
//!
//! The role is read by [`probe_trainingpeaks_role`], which runs after every
//! `TrainingPeaks` login and once for a connection made before roles were
//! recorded, and is also learned from the scraper's refusal of a coach
//! account's own read, which the read path records. That refusal carries no
//! email, so it grants nothing: the grant waits for a profile read.
//!
//! A coach link binds a roster athlete to a member the same way, and every
//! read through it checks that binding again ([`link_binding`]).

use dravr_sciotte::client::SciotteClient;
use dravr_sciotte::models::{AccountRole, AthleteProfile, AuthSession};
use pierre_core::constants::oauth_providers::SCIOTTE_TRAININGPEAKS;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{normalize_email, DelegatedConnection, ProviderAccountRole, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_providers::sciotte_error::to_app_error;
use pierre_providers::sciotte_provider::SciotteTarget;
use tracing::info;
use uuid::Uuid;

/// Whether a `TrainingPeaks` account's email binds it to a Dravr account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailBinding {
    /// The `TrainingPeaks` email is the Dravr account's verified email, case
    /// aside.
    Bound,
    /// `TrainingPeaks` shared no email for the account.
    ProviderEmailMissing,
    /// The Dravr account's email was never verified, so it proves no one.
    DravrEmailUnverified,
    /// The two emails differ.
    Mismatch,
}

impl EmailBinding {
    /// The binding's name, for the logs: never the emails themselves.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bound => "bound",
            Self::ProviderEmailMissing => "provider_email_missing",
            Self::DravrEmailUnverified => "dravr_email_unverified",
            Self::Mismatch => "mismatch",
        }
    }
}

/// Whether the `TrainingPeaks` account whose email is `provider_email` is
/// `user_id`'s: that email must be the user's verified Dravr email.
///
/// Both are compared in their normalized form ([`normalize_email`]), so case
/// and surrounding whitespace do not matter.
///
/// # Errors
///
/// Returns a not-found error when the user does not exist, or the repository
/// error when the user or their verification cannot be read.
pub async fn email_binding(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    provider_email: Option<&str>,
) -> AppResult<EmailBinding> {
    let Some(provider_email) = provider_email
        .map(normalize_email)
        .filter(|email| !email.is_empty())
    else {
        return Ok(EmailBinding::ProviderEmailMissing);
    };
    let user = repos
        .users
        .get_global(user_id)
        .await?
        .ok_or_else(|| AppError::not_found("User"))?;
    if !repos.email_verification.is_verified(user_id).await? {
        return Ok(EmailBinding::DravrEmailUnverified);
    }
    Ok(if provider_email == normalize_email(&user.email) {
        EmailBinding::Bound
    } else {
        EmailBinding::Mismatch
    })
}

/// Whether the roster athlete a confirmed `link` names is its member by email.
///
/// Only then does the link serve reads: the email it stored at the proposal
/// must be the member's verified Dravr email. A link stored before links kept
/// that email binds as [`EmailBinding::ProviderEmailMissing`]. Checked where a
/// read is authorized and a link is shown, never migrated, so the link stands
/// until the coach proposes it anew and the member confirms.
///
/// # Errors
///
/// Returns the error of reading the member or their verification.
pub async fn link_binding(
    repos: &RepositoryRegistry,
    link: &DelegatedConnection,
) -> AppResult<EmailBinding> {
    let email = link.provider_athlete_email.as_deref();
    email_binding(repos, link.member_user_id, email).await
}

/// What a `TrainingPeaks` profile read found for the user signed in with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountReading {
    /// Whether the account trains or coaches.
    pub role: ProviderAccountRole,
    /// For a coach account, whether it is the user's own; `None` for an
    /// athlete account, which earns no coach powers to bind.
    pub binding: Option<EmailBinding>,
}

/// Record what a `TrainingPeaks` profile read found for `user_id`'s
/// connection in `tenant`.
///
/// That is the account's role ([`record_trainingpeaks_role`]) and, for a
/// coach account, the `manages_roster` grant, which it earns only when its
/// email binds it to the user ([`email_binding`]). A coach account that does
/// not bind gives back a grant the connection held.
///
/// # Errors
///
/// Returns the repository error when the user cannot be read, or the role,
/// the grant or the purge cannot be written.
pub async fn record_trainingpeaks_profile(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    profile: &AthleteProfile,
) -> AppResult<AccountReading> {
    let role = account_role(profile);
    let binding = if role == ProviderAccountRole::Coach {
        let binding = coach_binding(repos, user_id, tenant, profile).await?;
        if binding == EmailBinding::Bound {
            grant_roster(repos, user_id).await?;
        }
        Some(binding)
    } else {
        None
    };
    record_trainingpeaks_role(repos, user_id, tenant, role).await?;
    Ok(AccountReading { role, binding })
}

/// Whether the coach account `profile` describes is `user_id`'s
/// ([`email_binding`]).
///
/// When it is not, the `manages_roster` a connection in `tenant` earned is
/// taken back ([`revoke_roster_for_coach_connection`]). A bound account is
/// granted nothing here: only a newly read role grants.
///
/// # Errors
///
/// Returns the repository error when the user or the connections cannot be
/// read, or the grant cannot be written.
pub async fn coach_binding(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    profile: &AthleteProfile,
) -> AppResult<EmailBinding> {
    let binding = email_binding(repos, user_id, profile.email.as_deref()).await?;
    if binding != EmailBinding::Bound {
        info!(
            user_id = %user_id,
            tenant_id = %tenant,
            binding = binding.as_str(),
            "TrainingPeaks coach account is not bound to the user; no manages_roster"
        );
        revoke_roster_for_coach_connection(repos, user_id, tenant).await?;
    }
    Ok(binding)
}

/// Record that `user_id`'s `TrainingPeaks` connection in `tenant` signed in
/// with an account of `role`.
///
/// The role lands on the connection row, where the read path and the
/// providers card read it. A [`ProviderAccountRole::Coach`] account loses the
/// cached `TrainingPeaks` activities filed under it, which can only be another
/// athlete's workouts misfiled before sciotte 0.14. A connection recorded as a
/// coach that now reads as an athlete gives the `manages_roster` grant back
/// ([`revoke_roster_for_coach_connection`]). The grant itself is earned only
/// through a profile read ([`record_trainingpeaks_profile`]), which carries
/// the account's email.
///
/// # Errors
///
/// Returns the repository error when the role, the revoke or the purge cannot
/// be written.
pub async fn record_trainingpeaks_role(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    role: ProviderAccountRole,
) -> AppResult<()> {
    // The role is written last: a recorded role is what tells the next read
    // (and the probe that skips an unchanged role) that the purge is done, so
    // a failure before it leaves the role unread and the whole step runs
    // again.
    if role == ProviderAccountRole::Coach {
        purge_misfiled_activities(repos, user_id, tenant).await?;
    } else if recorded_role(repos, user_id, tenant).await? == Some(ProviderAccountRole::Coach) {
        // The connection that earned the grant now signs in as an athlete.
        revoke_roster_for_coach_connection(repos, user_id, tenant).await?;
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

/// The account role recorded on `user_id`'s `TrainingPeaks` connection in
/// `tenant`, `None` when there is no connection or it was never read.
async fn recorded_role(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
) -> AppResult<Option<ProviderAccountRole>> {
    Ok(repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await?
        .into_iter()
        .find(|connection| connection.provider == SCIOTTE_TRAININGPEAKS)
        .and_then(|connection| connection.account_role))
}

/// Take back the `manages_roster` a `TrainingPeaks` coach connection earned,
/// when the connection in `tenant` stops earning it.
///
/// It stops when it signs in as an athlete or as a coach account that is not
/// the user's, or when it is disconnected.
///
/// The grant stays while the user holds a `TrainingPeaks` coach connection in
/// any other tenant, since that connection earns it too, and whenever an
/// operator made it: a grant the connection did not earn is not the
/// connection's to take back.
///
/// # Errors
///
/// Returns the repository error when the connections cannot be read or the
/// grant cannot be written.
pub async fn revoke_roster_for_coach_connection(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
) -> AppResult<()> {
    let tenant_text = tenant.to_string();
    let coach_elsewhere = repos
        .provider_connections
        .get_for_user(user_id, None)
        .await?
        .iter()
        .any(|connection| {
            connection.provider == SCIOTTE_TRAININGPEAKS
                && connection.account_role == Some(ProviderAccountRole::Coach)
                && connection.tenant_id != tenant_text
        });
    if coach_elsewhere {
        return Ok(());
    }
    if repos.users.revoke_earned_manages_roster(user_id).await? {
        info!(user_id = %user_id, "manages_roster revoked: no TrainingPeaks coach connection left");
    }
    Ok(())
}

/// Grant `manages_roster` to the user of a bound `TrainingPeaks` coach
/// account who does not hold it.
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
/// The session is imported first, and imported once more when the read
/// reaches an instance the import did not: that instance holding no session
/// says nothing about the account's, so it must not read as a session to
/// reconnect.
///
/// # Errors
///
/// Returns an error when the scraper service is not configured, or when it
/// cannot import the session or read the profile.
pub async fn trainingpeaks_profile(session: &AuthSession) -> AppResult<AthleteProfile> {
    let remote = SciotteClient::require_from_env().map_err(to_app_error)?;
    remote
        .read_imported(
            session,
            SciotteTarget::TrainingPeaks.scraper_provider_name(),
            || remote.get_athlete(&session.session_id),
        )
        .await
        .map_err(to_app_error)
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

/// Read what kind of account `session` signed in with and record it, with
/// the grant a coach account bound to the user earns
/// ([`record_trainingpeaks_profile`]).
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
    Ok(
        record_trainingpeaks_profile(repos, user_id, tenant, &profile)
            .await?
            .role,
    )
}
