// ABOUTME: Whether a group's human coach may act for one athlete, by id: coach of the active group, live member, consent, one home tenant
// ABOUTME: The one check the coach's athlete thread route and the plan tools' scope both run, with a typed refusal each words its own way

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Acting for a coached athlete
//!
//! A group's human coach — the Dravr user attached through a coach invite,
//! `coaching_groups.coach_user_id` — may read and write a roster athlete's
//! plan and calendar. Four gates decide it, all required, run here by id so
//! that every path that acts for an athlete asks them the same way: the plan
//! tools after they resolve an `athlete=` name, the coach's thread bound to
//! one athlete on every call made in it, and the route that opens that thread
//! (carnet#1070).
//!
//! 1. **The coach attachment.** The group is active and the caller is its
//!    `coach_user_id`. Owner and admin roles do not qualify — on a
//!    channel-bound group the owner is whoever spoke first.
//! 2. **A live member.** The athlete belongs to the group and has not left
//!    it. A coach is never their own athlete.
//! 3. **The athlete's consent.** The member's own `coach_sharing_consent`.
//!    Joining grants it (ADR-002: membership is consent to share with the
//!    coach) and `/group consent coach no` revokes it.
//! 4. **One home tenant.** The athlete's plan lives under their own tenant.
//!    An athlete who belongs to several would see a plan on one surface and
//!    not another, so that case is refused rather than guessed.
//!
//! A refusal before the membership gate passes names nobody: the caller has
//! not shown they coach this athlete. Once it passes, the refusal carries the
//! athlete's name and the group's, which the caller may tell the coach.
//!
//! Beside the gates sits the one rule for a room shaped for a coach and a
//! single athlete, [`solo_athlete`] (carnet#1249). It replaces none of them.

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use serde_json::json;
use uuid::Uuid;

use crate::athlete_display_name::fetch_user_display_name;

/// The coach may act for the athlete: where their plan lives, how they are
/// named, and the group that lets the coach.
#[derive(Debug, Clone)]
pub struct CoachedAthleteAccess {
    /// The athlete.
    pub athlete_user_id: Uuid,
    /// The athlete's one home tenant, where their plan and calendar live.
    pub tenant: TenantId,
    /// The athlete's roster name
    /// ([`roster_display_name`](pierre_core::models::roster_display_name)).
    pub display_name: String,
    /// The group the coach coaches the athlete in.
    pub group: CoachingGroup,
}

/// Why the coach may not act for the athlete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoachedAthleteRefusal {
    /// The group is missing or archived, or the caller is not its human
    /// coach.
    NotGroupCoach,
    /// The athlete is not a live member of the group, or is the coach.
    NotAMember,
    /// The athlete has stopped sharing their data with the group's coach.
    CoachSharingRevoked {
        /// The athlete's roster name.
        athlete: String,
        /// The group's name.
        group: String,
    },
    /// The athlete belongs to no tenant, so their plan has nowhere to live.
    NoTenant {
        /// The athlete's roster name.
        athlete: String,
    },
    /// The athlete belongs to several tenants.
    SeveralTenants {
        /// The athlete's roster name.
        athlete: String,
    },
}

impl CoachedAthleteRefusal {
    /// The reason a client branches on, carried as `details.reason`.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::NotGroupCoach => "not_group_coach",
            Self::NotAMember => "not_a_member",
            Self::CoachSharingRevoked { .. } => "coach_sharing_revoked",
            Self::NoTenant { .. } => "no_tenant",
            Self::SeveralTenants { .. } => "several_tenants",
        }
    }

    /// The refusal as an HTTP error: 403 for a caller who does not coach the
    /// athlete in the group or whose athlete withdrew consent, 404 for a
    /// person who is not a live member of it, 409 for an athlete whose
    /// tenancy leaves no one place for their plan. The message is English and
    /// addressed to an API caller; `details.reason` is what a client words.
    #[must_use]
    pub fn into_error(self) -> AppError {
        let reason = self.reason();
        let mut error = match self {
            Self::NotGroupCoach => AppError::new(
                ErrorCode::PermissionDenied,
                "Only the group's coach can act for its athletes",
            ),
            Self::NotAMember => AppError::new(
                ErrorCode::ResourceNotFound,
                "This athlete is not a member of the group",
            ),
            Self::CoachSharingRevoked { athlete, group } => AppError::new(
                ErrorCode::PermissionDenied,
                format!("{athlete} has stopped sharing their data with the coach of '{group}'"),
            ),
            Self::NoTenant { athlete } => AppError::new(
                ErrorCode::ResourceLocked,
                format!("{athlete} belongs to no tenant, so there is nowhere to keep their plan"),
            ),
            Self::SeveralTenants { athlete } => AppError::new(
                ErrorCode::ResourceLocked,
                format!(
                    "{athlete} belongs to several tenants, so their plan would show on one of \
                     their surfaces and be missing on another"
                ),
            ),
        };
        error.details = Some(Box::new(json!({ "reason": reason })));
        error
    }
}

/// Whether `coach` may act for `athlete` as the human coach of `group_id`.
///
/// Runs the four gates in order (see the module docs) and answers with the
/// first that fails. Nothing is cached: a caller that acts on every turn asks
/// again on every turn, so a consent revoked mid-thread refuses the next act.
///
/// The outer `Err` is a repository failure; the inner `Err` the refusal.
///
/// # Errors
///
/// Returns the repository error from the group, member or tenant lookups.
pub async fn coached_athlete_access(
    repos: &RepositoryRegistry,
    coach: Uuid,
    athlete: Uuid,
    group_id: Uuid,
) -> AppResult<Result<CoachedAthleteAccess, CoachedAthleteRefusal>> {
    // Walked from the active groups the caller is attached to as their human
    // coach — never from the groups they merely belong to or own.
    let Some(group) = repos
        .groups
        .list_groups_coached_by(coach)
        .await?
        .into_iter()
        .find(|group| group.id == group_id && group.is_active)
    else {
        return Ok(Err(CoachedAthleteRefusal::NotGroupCoach));
    };
    let group_key = group_id.to_string();

    if athlete == coach {
        return Ok(Err(CoachedAthleteRefusal::NotAMember));
    }
    // `get_member` answers for a live member only.
    let Some(member) = repos.groups.get_member(&group_key, athlete).await? else {
        return Ok(Err(CoachedAthleteRefusal::NotAMember));
    };

    let display_name = fetch_user_display_name(repos, athlete).await;
    if !member.coach_sharing_consent {
        return Ok(Err(CoachedAthleteRefusal::CoachSharingRevoked {
            athlete: display_name,
            group: group.name,
        }));
    }

    let tenants = repos.tenants.list_for_user(athlete).await?;
    let tenant = match tenants.as_slice() {
        [tenant] => tenant.id,
        [] => {
            return Ok(Err(CoachedAthleteRefusal::NoTenant {
                athlete: display_name,
            }))
        }
        _ => {
            return Ok(Err(CoachedAthleteRefusal::SeveralTenants {
                athlete: display_name,
            }))
        }
    };

    Ok(Ok(CoachedAthleteAccess {
        athlete_user_id: athlete,
        tenant,
        display_name,
        group,
    }))
}

/// The one athlete of a room its human coach shares with no one else.
///
/// That is `group`'s live members other than its human coach, when they
/// number exactly one (carnet#1249). `None` for a group with no human coach,
/// and for one with no athlete or several.
///
/// The coach holds no membership row unless they also own the group, and a
/// coach is never their own athlete, so they are left out of the count
/// either way. Every surface that treats a room as one coach's and one
/// athlete's asks here — the coach's view of the group and the line a plan
/// publish leaves in its room — so no two count a different room. Neither the
/// group's state nor the athlete's consent is asked: each caller runs its own
/// gates.
///
/// # Errors
///
/// Returns the repository error from the member listing.
pub async fn solo_athlete(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
) -> AppResult<Option<Uuid>> {
    let Some(coach) = group.coach_user_id else {
        return Ok(None);
    };
    let members = repos.groups.list_members(&group.id.to_string()).await?;
    let mut athletes = members.iter().filter(|member| member.user_id != coach);
    Ok(match (athletes.next(), athletes.next()) {
        (Some(athlete), None) => Some(athlete.user_id),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each refusal reaches an HTTP caller with its status and the reason a
    /// client words, and names the athlete only past the membership gate.
    #[test]
    fn every_refusal_carries_its_status_and_reason() {
        let athlete = "Mira Subject".to_owned();
        for (refusal, status, reason) in [
            (CoachedAthleteRefusal::NotGroupCoach, 403, "not_group_coach"),
            (CoachedAthleteRefusal::NotAMember, 404, "not_a_member"),
            (
                CoachedAthleteRefusal::CoachSharingRevoked {
                    athlete: athlete.clone(),
                    group: "Tempo Squad".to_owned(),
                },
                403,
                "coach_sharing_revoked",
            ),
            (
                CoachedAthleteRefusal::NoTenant {
                    athlete: athlete.clone(),
                },
                409,
                "no_tenant",
            ),
            (
                CoachedAthleteRefusal::SeveralTenants {
                    athlete: athlete.clone(),
                },
                409,
                "several_tenants",
            ),
        ] {
            let names_athlete = !matches!(
                refusal,
                CoachedAthleteRefusal::NotGroupCoach | CoachedAthleteRefusal::NotAMember
            );
            let error = refusal.into_error();
            assert_eq!(error.code.http_status(), status, "{reason}");
            assert_eq!(
                error.details.as_deref().and_then(|d| d["reason"].as_str()),
                Some(reason)
            );
            assert_eq!(error.message.contains(&athlete), names_athlete, "{reason}");
        }
    }
}
