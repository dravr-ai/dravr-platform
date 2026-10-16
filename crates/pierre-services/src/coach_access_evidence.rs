// ABOUTME: An Intervals.icu key whose live roster coaches an active athlete files its owner's coach-access request
// ABOUTME: Never a grant (ADR-018): filed once, for a bound key, never over coach access already held or an earlier request

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach access from an Intervals.icu roster
//!
//! Any Intervals.icu account can give another one edit access, so a key whose
//! roster lists athletes it coaches is evidence that its owner coaches, never
//! proof: two free accounts of one person's show the same. `manages_roster`
//! therefore stays a super-admin's grant (ADR-018), and the evidence saves the
//! coach only the asking: [`file_request_from_roster`] opens their
//! coach-access request ([`request_access`], naming no group), which a
//! super-admin grants or declines in one tap. The group the coach then makes
//! to coach, or asks from, is named on it
//! ([`crate::coach_access::name_requested_group`]), so a grant attaches them
//! to it.
//!
//! A request is filed once per account. A user who holds coach access gets
//! none, and neither does one who ever had a request, whatever its outcome:
//! a declined request, or a grant one led to and an operator later revoked,
//! is never asked again by a roster read.
//!
//! Two reads file it: every live read of the key's roster
//! ([`crate::coach_platform`], never a cached roster), and the probe run right
//! after a key is linked ([`probe_intervals_icu_roster`]).

use pierre_core::constants::oauth_providers::INTERVALS_ICU;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CoachAccessRequest, RosterAccess, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_email::templates::CoachAccessOrigin;
use pierre_providers::delegation::CoachRoster;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::request_budget::in_background;
use tracing::{info, warn};
use uuid::Uuid;

use crate::coach_access::{request_access, RequestOutcome};
use crate::coach_platform::{coach_platform, live_api_roster, RosterRead};
use crate::trainingpeaks_accounts::EmailBinding;

/// How many athletes on `roster` the account coaches and the platform lists
/// as active: access `Some(Coach)` and standing `Some(true)`. An access or a
/// standing the platform did not report counts as neither. The account's own
/// entry is never among the athletes.
fn coached_active_athletes(roster: &CoachRoster) -> usize {
    roster
        .athletes
        .iter()
        .filter(|athlete| {
            athlete.access == Some(RosterAccess::Coach) && athlete.active == Some(true)
        })
        .count()
}

/// A coach-access request a roster read filed, and the evidence it was filed
/// on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiledRequest {
    /// The request opened.
    pub request: CoachAccessRequest,
    /// How many athletes the roster's account coaches and the platform lists
    /// as active; callers read it through [`Self::origin`].
    pub(crate) coached_active_athletes: usize,
}

impl FiledRequest {
    /// How the operator email names where the request came from: filed from
    /// the roster, never asked for.
    #[must_use]
    pub const fn origin(&self) -> CoachAccessOrigin {
        CoachAccessOrigin::IntervalsIcuRoster {
            coached_active_athletes: self.coached_active_athletes,
        }
    }
}

/// Whether `user_id` is someone a request could be filed for: an account that
/// holds no coach access (`manages_roster`, or platform admin, who may coach
/// a group already) and never had a coach-access request.
///
/// # Errors
///
/// Returns not-found for an unknown user, or the repository error.
async fn may_file_for(repos: &RepositoryRegistry, user_id: Uuid) -> AppResult<bool> {
    let user = repos
        .users
        .get_global(user_id)
        .await?
        .ok_or_else(|| AppError::not_found("User"))?;
    if user.manages_roster || user.is_admin {
        return Ok(false);
    }
    Ok(repos
        .coach_access_requests
        .latest_for_user(user_id)
        .await?
        .is_none())
}

/// File `user_id`'s coach-access request from their Intervals.icu `roster`,
/// when the roster is evidence they coach.
///
/// It files only when `binding` is [`EmailBinding::Bound`] (the key's account
/// is theirs, by their verified email), the roster coaches at least one
/// active athlete, and [`may_file_for`] holds. It never grants and never
/// revokes anything.
///
/// Returns the request it opened, with the evidence; `None` when it filed
/// none, a request opened meanwhile by another read included.
///
/// # Errors
///
/// Returns not-found for an unknown user, or the repository error when the
/// user, their requests or the new request cannot be read or written.
pub(crate) async fn file_request_from_roster(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    roster: &CoachRoster,
    binding: EmailBinding,
) -> AppResult<Option<FiledRequest>> {
    let coached = coached_active_athletes(roster);
    if binding != EmailBinding::Bound || coached == 0 || !may_file_for(repos, user_id).await? {
        return Ok(None);
    }
    match request_access(repos, user_id, None).await? {
        RequestOutcome::Opened(request) => {
            info!(
                user_id = %user_id,
                request_id = %request.id,
                coached_active_athletes = coached,
                "Coach access request filed from an Intervals.icu roster"
            );
            Ok(Some(FiledRequest {
                request,
                coached_active_athletes: coached,
            }))
        }
        RequestOutcome::AlreadyPending(_) => Ok(None),
    }
}

/// [`file_request_from_roster`] after a live roster read, which it never
/// fails: an error is logged and the roster is still served.
pub(crate) async fn file_request_after_live_read(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    roster: &CoachRoster,
    binding: EmailBinding,
) {
    if let Err(e) = file_request_from_roster(repos, user_id, roster, binding).await {
        warn!(
            user_id = %user_id,
            error = %e,
            "Could not file a coach access request from an Intervals.icu roster"
        );
    }
}

/// Read `user_id`'s Intervals.icu roster live through the key they hold in
/// `tenant`, then file the coach-access request it is evidence for
/// ([`file_request_from_roster`]).
///
/// Run off the request right after a key is linked, as work nobody waits on
/// ([`in_background`]). Nothing is read for a user no request could be filed
/// for, and nothing is filed when the key is gone or replaced by an OAuth
/// grant before the probe runs (a grant lists no roster), or when this build
/// carries no Intervals.icu coaching platform.
///
/// Returns the request it opened, with the evidence the caller's
/// announcement states; `None` when it filed none.
///
/// # Errors
///
/// Returns the refusal or provider error of the roster read, or a repository
/// error.
pub async fn probe_intervals_icu_roster(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    user_id: Uuid,
    tenant: TenantId,
) -> AppResult<Option<FiledRequest>> {
    let Some(platform) = coach_platform(registry, INTERVALS_ICU) else {
        return Ok(None);
    };
    if !may_file_for(repos, user_id).await? {
        return Ok(None);
    }
    let Some(token) = repos
        .oauth_tokens
        .get_token(user_id, tenant, INTERVALS_ICU)
        .await?
    else {
        return Ok(None);
    };
    let Some(credentials) = platform.coach_credentials(&token) else {
        return Ok(None);
    };
    let read = RosterRead {
        repos,
        registry,
        coach_user_id: user_id,
        coach_tenant: tenant,
        token: &token,
        recorded_role: None,
    };
    let (roster, binding) =
        in_background(live_api_roster(platform.as_ref(), read, credentials)).await?;
    file_request_from_roster(repos, user_id, &roster, binding).await
}

#[cfg(test)]
mod tests {
    use pierre_core::models::RosterAthlete;

    use super::*;

    fn athlete(id: &str, access: Option<RosterAccess>, active: Option<bool>) -> RosterAthlete {
        RosterAthlete::new(id, None, None)
            .with_access(access)
            .with_active(active)
    }

    fn roster(athletes: Vec<RosterAthlete>) -> CoachRoster {
        CoachRoster {
            account_email: Some("coach@evidence.test".to_owned()),
            athletes,
        }
    }

    #[test]
    fn only_an_athlete_coached_and_active_counts_as_evidence() {
        let mixed = roster(vec![
            athlete("i1", Some(RosterAccess::Coach), Some(true)),
            athlete("i2", Some(RosterAccess::Coach), Some(true)),
            athlete("i3", Some(RosterAccess::Coach), Some(false)),
            athlete("i4", Some(RosterAccess::Follower), Some(true)),
            athlete("i5", Some(RosterAccess::NoAccess), Some(true)),
        ]);
        assert_eq!(coached_active_athletes(&mixed), 2);
    }

    #[test]
    fn an_access_or_standing_the_platform_did_not_report_counts_as_neither() {
        for unreported in [
            athlete("i1", None, Some(true)),
            athlete("i2", Some(RosterAccess::Coach), None),
            athlete("i3", None, None),
        ] {
            let id = unreported.id.clone();
            assert_eq!(
                coached_active_athletes(&roster(vec![unreported])),
                0,
                "{id}"
            );
        }
        assert_eq!(coached_active_athletes(&roster(Vec::new())), 0);
    }
}
