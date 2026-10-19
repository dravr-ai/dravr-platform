// ABOUTME: The coaching-platform link a member who joins a coached group is proposed as they join
// ABOUTME: The athlete the group's picker suggests for them, from a roster read live when a cached one binds no one

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The link a join proposes
//!
//! A member who joins a group whose human coach reads a coaching-platform
//! roster is proposed the link to the roster athlete their verified email
//! binds ([`propose_on_join`]), so the coach has nothing to propose by hand.
//! It is the link the group's picker suggests, proposed through the ordinary
//! [`propose`]: the same email binding, access, one-live-link and
//! still-sharing rules, every consent left to the member.

use pierre_core::errors::AppResult;
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::{DelegatedConnection, TenantId};
use tracing::info;
use uuid::Uuid;

use crate::coach_platform::{connected_coach_platforms, CoachPlatform};
use crate::delegated_connections::{
    propose, roster_cache_key, roster_for_group, DelegationServices, Proposal,
};

/// Propose the link of `member_user_id`, who just joined `group`.
///
/// The link is the one its coach would propose from the group's picker: to
/// the roster athlete whose email binds the member
/// ([`crate::delegated_connections::RosterEntry::suggested_member_user_id`]).
/// The coach is the group's human coach, reading their roster in
/// `group_tenant` — the tenant a coach of the group is in
/// ([`pierre_groups::creation_policy::may_coach_group`]) and the one the
/// picker reads it in. Each coaching platform the coach holds a credential of
/// their own for is tried in [`crate::coach_platform::coach_platforms`]
/// order, and the first whose roster binds the member is proposed: one card
/// for the member to answer, and the coach can still propose another
/// platform by hand. A cached roster that binds no one is read once more,
/// live, so an athlete the coach has just added is found; a platform whose
/// roster cannot be read is passed over.
///
/// Only the joining member is matched, by their verified email, so nothing
/// about a roster athlete who is no member reaches the notice. Nothing read
/// from the roster reaches the join's reply either, which is why the roster's
/// transport gate ([`crate::coach_platform::check_roster_transport`]) does not
/// apply, as it does not to [`propose`].
///
/// `None` when the group has no human coach, the member is that coach, the
/// coach holds no coaching-platform credential in `group_tenant`, or no roster
/// binds the member.
///
/// # Errors
///
/// Returns the errors of [`propose`], or the repository error when the
/// coach's credentials cannot be read.
pub async fn propose_on_join(
    services: DelegationServices<'_>,
    group: &CoachingGroup,
    group_tenant: TenantId,
    member_user_id: Uuid,
) -> AppResult<Option<DelegatedConnection>> {
    let Some(coach) = group.coach_user_id.filter(|coach| *coach != member_user_id) else {
        return Ok(None);
    };
    let platforms =
        connected_coach_platforms(services.repos, services.registry, coach, group_tenant).await?;
    for platform in &platforms {
        let platform = platform.as_ref();
        let Some(athlete) = joiner_on_roster(
            services,
            platform,
            group,
            (coach, group_tenant),
            member_user_id,
        )
        .await
        else {
            continue;
        };
        let link = propose(
            services,
            group,
            coach,
            group_tenant,
            Proposal {
                provider: platform.backend(),
                provider_athlete_id: &athlete,
                member_user_id,
            },
        )
        .await?;
        return Ok(Some(link));
    }
    Ok(None)
}

/// The roster athlete `group`'s picker suggests for `member_user_id` on
/// `platform`, read through the coach's credential in the coach's tenant;
/// `None` when no athlete binds the member or the roster cannot be read
/// (logged).
///
/// A roster served from the cache can predate an athlete the coach has added
/// on the platform since, so a cached roster that binds no one is read once
/// more, live. A roster the cache did not hold was read live already and is
/// not read again.
async fn joiner_on_roster(
    services: DelegationServices<'_>,
    platform: &dyn CoachPlatform,
    group: &CoachingGroup,
    (coach, coach_tenant): (Uuid, TenantId),
    member_user_id: Uuid,
) -> Option<String> {
    let key = roster_cache_key(coach, coach_tenant, platform.backend());
    let cached = matches!(services.cache.exists(&key).await, Ok(true));
    for refresh in [false, true] {
        if refresh && !cached {
            break;
        }
        match roster_for_group(services, platform, group, coach, coach_tenant, refresh).await {
            Ok(roster) => {
                if let Some(entry) = roster
                    .into_iter()
                    .find(|entry| entry.suggested_member_user_id == Some(member_user_id))
                {
                    return Some(entry.athlete.id);
                }
            }
            Err(e) => {
                info!(
                    group_id = %group.id,
                    coach_user_id = %coach,
                    provider = platform.backend(),
                    refresh,
                    error = %e,
                    "A join proposes no link on this platform: the coach's roster was not read"
                );
                return None;
            }
        }
    }
    None
}
