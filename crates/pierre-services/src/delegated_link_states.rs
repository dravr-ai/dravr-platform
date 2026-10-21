// ABOUTME: Where each athlete on a coach's coaching-platform roster stands with the coach's Dravr groups and links
// ABOUTME: Linked, proposed, proposable to one coached member, or why not; members are found by id and bound by email
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Roster link states
//!
//! A coach's coaching-platform roster lists athletes; the coach's Dravr groups
//! list members. [`roster_link_states`] tells, for every roster athlete, where
//! the two meet: a link of the coach's that holds the athlete, the one coached
//! member the athlete's roster email binds and whether a link to them could be
//! proposed, or why no member stands behind the athlete.
//!
//! The members are the live members of every active group the coach is
//! attached to as its human coach, each across tenants, the coach excluded:
//! the set a roster scope reads elsewhere. Their accounts are read by member
//! id, and a roster email is compared with their primary emails in memory;
//! no account is ever looked up by a roster email, so the states never tell
//! whether someone outside the coach's groups holds a Dravr account. Only the
//! member whose email a roster athlete names has their verification read, and
//! an unverified email binds no one: the one email rule every binding applies.
//!
//! A state carries the roster athlete as the platform lists them, name and
//! email included: text about third parties, for the coach and the athlete a
//! link concerns, never for a notice or a model.

use std::collections::{BTreeMap, HashMap};

use pierre_core::errors::AppResult;
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::{
    DelegatedConnection, DelegationStatus, RosterAccess, RosterAthlete, TenantId, User,
};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

use crate::coach_platform::CoachPlatform;
use crate::delegated_connections::{coach_roster, unbound_link_reason, DelegationServices};
use crate::delegated_read::{holds_own_connection, OwnLogins};
use crate::trainingpeaks_accounts::{
    bind_email, link_binding, provider_email_key, same_email, EmailBinding,
};

/// A live member of a group the coach coaches, in the group a link to them
/// is made in.
#[derive(Debug, Clone)]
pub struct CoachedMember {
    /// The member.
    pub user_id: Uuid,
    /// The group: the one asked for when the member is in it, else one where
    /// they share with the coach, oldest first, then the oldest of the rest.
    pub group: CoachingGroup,
}

/// Where one roster athlete stands with the coach's groups and links.
#[derive(Debug, Clone)]
pub enum LinkState {
    /// A confirmed link of the coach's reads the athlete as its member.
    Linked {
        /// The link.
        link: DelegatedConnection,
    },
    /// A confirmed link of the coach's names the athlete, but reads nothing:
    /// its athlete is no longer its member by email.
    LinkedNotReading {
        /// The link.
        link: DelegatedConnection,
        /// Why, as a link's `read_refused` names it
        /// ([`unbound_link_reason`]).
        reason: &'static str,
    },
    /// The coach proposed a link for the athlete; the member has not answered.
    Proposed {
        /// The proposal.
        link: DelegatedConnection,
    },
    /// The member the athlete's email binds reads the platform through a
    /// confirmed link of another coach's, so a second one would be refused.
    LinkedThroughAnotherCoach {
        /// The member.
        member: CoachedMember,
    },
    /// The member the athlete's email binds already holds a live link of the
    /// coach's on the platform, for another roster entry.
    MemberHoldsAnotherLink {
        /// The member.
        member: CoachedMember,
    },
    /// The member the athlete's email binds can be proposed a link.
    Proposable {
        /// The member, in the group the link would be made in.
        member: CoachedMember,
    },
    /// The athlete's roster email is a coached member's, but the member has
    /// not verified it, so it binds no one yet.
    MemberEmailUnverified {
        /// The member.
        member: CoachedMember,
    },
    /// The member the athlete's email binds connects the platform with an
    /// account of their own, in some tenant: no link is needed (one would be
    /// refused), since the coach reaches their calendar through the coach's
    /// own account.
    MemberOwnConnection {
        /// The member.
        member: CoachedMember,
    },
    /// The athlete's roster email is a coached member's who stopped sharing
    /// with the coach in every group the coach coaches them in. Nothing about
    /// the member's platform account or their other links is read.
    MemberNotSharing {
        /// The member.
        member: CoachedMember,
    },
    /// The member the athlete's email binds is on the roster with no access
    /// for the coach ([`RosterAccess::NoAccess`]): a link would read nothing,
    /// so none is proposed.
    NoAccess {
        /// The member.
        member: CoachedMember,
    },
    /// The platform shares no email for the athlete, so no member can be
    /// bound to them.
    RosterEmailMissing,
    /// The athlete's roster email is no live member's of any group the coach
    /// coaches.
    NotInYourGroups,
}

impl LinkState {
    /// The coach's link the state is about, when it is one.
    #[must_use]
    pub const fn link(&self) -> Option<&DelegatedConnection> {
        match self {
            Self::Linked { link }
            | Self::LinkedNotReading { link, .. }
            | Self::Proposed { link } => Some(link),
            _ => None,
        }
    }

    /// The coached member the athlete's roster email names, when one does
    /// and no link of the coach's holds the athlete.
    #[must_use]
    pub const fn member(&self) -> Option<&CoachedMember> {
        match self {
            Self::LinkedThroughAnotherCoach { member }
            | Self::MemberHoldsAnotherLink { member }
            | Self::Proposable { member }
            | Self::MemberEmailUnverified { member }
            | Self::MemberOwnConnection { member }
            | Self::MemberNotSharing { member }
            | Self::NoAccess { member } => Some(member),
            _ => None,
        }
    }
}

/// One roster athlete and where they stand.
#[derive(Debug, Clone)]
pub struct RosterLinkState {
    /// The athlete as the coach's platform roster lists them, with the
    /// coach's access to them.
    pub athlete: RosterAthlete,
    /// Where they stand.
    pub state: LinkState,
}

/// Every athlete on the coach's `platform` roster, with where they stand.
///
/// The roster is read through the coach's own credential in `coach_tenant`
/// ([`coach_roster`]), served from the cache when it holds the roster.
///
/// # Errors
///
/// Returns the errors of [`coach_roster`], or a repository error.
pub async fn roster_link_states(
    services: DelegationServices<'_>,
    platform: &dyn CoachPlatform,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
) -> AppResult<Vec<RosterLinkState>> {
    Ok(
        assess(services, platform, coach_user_id, coach_tenant, None, false)
            .await?
            .into_iter()
            .map(|assessed| assessed.view)
            .collect(),
    )
}

/// A roster athlete's state, and whether the athlete's roster email binds
/// the member the state names: their verified email.
pub(crate) struct Assessed {
    /// The athlete and their state.
    pub(crate) view: RosterLinkState,
    /// Whether the member the state names is bound by email.
    pub(crate) bound: bool,
}

/// Every roster athlete's state, a member's group being `prefer_group` when
/// they are in it; `refresh` reads the roster live.
pub(crate) async fn assess(
    services: DelegationServices<'_>,
    platform: &dyn CoachPlatform,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    prefer_group: Option<Uuid>,
    refresh: bool,
) -> AppResult<Vec<Assessed>> {
    let repos = services.repos;
    let athletes = coach_roster(services, platform, coach_user_id, coach_tenant, refresh).await?;
    let members = coached_members(repos, coach_user_id, prefer_group).await?;
    // A link whose member is no longer coached speaks for no one the coach
    // still coaches.
    let links: Vec<DelegatedConnection> = repos
        .delegated_connections
        .list_live_for_coach(coach_user_id, coach_tenant, platform.backend())
        .await?
        .into_iter()
        .filter(|link| members.contains_key(&link.member_user_id))
        .collect();
    let ids: Vec<Uuid> = members.keys().copied().collect();
    let users = repos.users.get_global_many(&ids).await?;
    let context = Assessment {
        repos,
        backend: platform.backend(),
        coach_user_id,
        links: &links,
        members: &members,
        users: &users,
    };

    let mut assessed = Vec::with_capacity(athletes.len());
    for athlete in athletes {
        let (state, bound) = context.state_of(&athlete).await?;
        assessed.push(Assessed {
            view: RosterLinkState { athlete, state },
            bound,
        });
    }
    Ok(assessed)
}

/// One group a coached member is in, and whether they share with the coach
/// there.
#[derive(Debug, Clone)]
struct Membership {
    group: CoachingGroup,
    shares: bool,
}

/// Every live member of every active group `coach_user_id` coaches, the
/// coach excluded, each with their groups in the order a link picks one
/// ([`rank_memberships`]).
async fn coached_members(
    repos: &RepositoryRegistry,
    coach_user_id: Uuid,
    prefer_group: Option<Uuid>,
) -> AppResult<BTreeMap<Uuid, Vec<Membership>>> {
    let mut members: BTreeMap<Uuid, Vec<Membership>> = BTreeMap::new();
    for group in repos.groups.list_groups_coached_by(coach_user_id).await? {
        if !group.is_active {
            continue;
        }
        for member in repos.groups.list_members(&group.id.to_string()).await? {
            if member.left_at.is_some() || member.user_id == coach_user_id {
                continue;
            }
            members.entry(member.user_id).or_default().push(Membership {
                group: group.clone(),
                shares: member.coach_sharing_consent,
            });
        }
    }
    for memberships in members.values_mut() {
        rank_memberships(memberships, prefer_group);
    }
    Ok(members)
}

/// Order a member's groups the way a link picks one: `prefer_group` first,
/// then the groups where they share with the coach, each kind oldest first,
/// ties by id.
fn rank_memberships(memberships: &mut [Membership], prefer_group: Option<Uuid>) {
    memberships.sort_by_key(|m| {
        (
            Some(m.group.id) != prefer_group,
            !m.shares,
            m.group.created_at,
            m.group.id,
        )
    });
}

/// The coached member whose primary email `email` reads as, with their
/// account and their groups; the first by id when several do.
fn member_named_by<'a>(
    email: Option<&str>,
    members: &'a BTreeMap<Uuid, Vec<Membership>>,
    users: &'a HashMap<Uuid, User>,
) -> Option<(Uuid, &'a User, &'a [Membership])> {
    members.iter().find_map(|(id, memberships)| {
        let user = users.get(id)?;
        same_email(email, &user.email).then_some((*id, user, memberships.as_slice()))
    })
}

/// What every athlete's state is read against.
struct Assessment<'a> {
    repos: &'a RepositoryRegistry,
    backend: &'a str,
    coach_user_id: Uuid,
    links: &'a [DelegatedConnection],
    members: &'a BTreeMap<Uuid, Vec<Membership>>,
    users: &'a HashMap<Uuid, User>,
}

impl Assessment<'_> {
    /// Where `athlete` stands, and whether their roster email binds the member
    /// the state names.
    async fn state_of(&self, athlete: &RosterAthlete) -> AppResult<(LinkState, bool)> {
        if let Some(link) = self
            .links
            .iter()
            .find(|l| l.provider_athlete_id == athlete.id)
        {
            return Ok((self.link_state(link).await?, false));
        }
        if provider_email_key(athlete.email.as_deref()).is_none() {
            return Ok((LinkState::RosterEmailMissing, false));
        }
        let Some((member_id, user, memberships)) =
            member_named_by(athlete.email.as_deref(), self.members, self.users)
        else {
            return Ok((LinkState::NotInYourGroups, false));
        };
        // A member is listed under every group they are in, so they have one.
        let Some(chosen) = memberships.first() else {
            return Ok((LinkState::NotInYourGroups, false));
        };
        let member = CoachedMember {
            user_id: member_id,
            group: chosen.group.clone(),
        };
        let verified = self.repos.email_verification.is_verified(member_id).await?;
        let bound =
            bind_email(athlete.email.as_deref(), &user.email, verified) == EmailBinding::Bound;

        // A member who stopped sharing with the coach everywhere is reported
        // as that alone: nothing about their platform account or their other
        // links is read.
        if !memberships.iter().any(|m| m.shares) {
            return Ok((LinkState::MemberNotSharing { member }, bound));
        }
        if !bound {
            return Ok((LinkState::MemberEmailUnverified { member }, bound));
        }
        let state = self.bound_member_state(athlete, member).await?;
        Ok((state, bound))
    }

    /// The state of a coach's live `link` for the athlete.
    async fn link_state(&self, link: &DelegatedConnection) -> AppResult<LinkState> {
        if link.status != DelegationStatus::Confirmed {
            return Ok(LinkState::Proposed { link: link.clone() });
        }
        let reason = unbound_link_reason(link_binding(self.repos, link).await?);
        Ok(reason.map_or_else(
            || LinkState::Linked { link: link.clone() },
            |reason| LinkState::LinkedNotReading {
                link: link.clone(),
                reason,
            },
        ))
    }

    /// The state of `member`, a sharing member the athlete's roster email
    /// binds, when no link of the coach's holds the athlete.
    async fn bound_member_state(
        &self,
        athlete: &RosterAthlete,
        member: CoachedMember,
    ) -> AppResult<LinkState> {
        let member_links = self
            .repos
            .delegated_connections
            .list_live_for_member(member.user_id)
            .await?;
        let on_platform = || member_links.iter().filter(|l| l.provider == self.backend);
        if on_platform().any(|l| {
            l.status == DelegationStatus::Confirmed && l.coach_user_id != self.coach_user_id
        }) {
            return Ok(LinkState::LinkedThroughAnotherCoach { member });
        }
        if on_platform().any(|l| l.coach_user_id == self.coach_user_id) {
            return Ok(LinkState::MemberHoldsAnotherLink { member });
        }
        // Their own account in any tenant: the coach does not know which
        // tenant a confirmation would come from.
        let logins = OwnLogins::of(self.repos);
        if holds_own_connection(logins, member.user_id, None, &[self.backend]).await? {
            return Ok(LinkState::MemberOwnConnection { member });
        }
        if athlete.access == Some(RosterAccess::NoAccess) {
            return Ok(LinkState::NoAccess { member });
        }
        Ok(LinkState::Proposable { member })
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use pierre_core::models::groups::{GroupDigestMode, GroupRespondMode};

    use super::*;

    fn group(age_days: i64) -> CoachingGroup {
        let created_at = Utc::now() - Duration::days(age_days);
        CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4().to_string(),
            name: format!("Group {age_days}"),
            description: None,
            agent_id: Uuid::new_v4().to_string(),
            owner_id: Uuid::new_v4(),
            coach_user_id: None,
            peer_data_sharing: false,
            respond_mode: GroupRespondMode::default(),
            digest_mode: GroupDigestMode::Off,
            max_members: 10,
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            created_at,
            updated_at: created_at,
        }
    }

    fn ranked(memberships: &[Membership], prefer: Option<Uuid>) -> Vec<Uuid> {
        let mut memberships = memberships.to_vec();
        rank_memberships(&mut memberships, prefer);
        memberships.iter().map(|m| m.group.id).collect()
    }

    #[test]
    fn a_member_s_group_is_the_asked_one_then_a_sharing_one_then_the_oldest() {
        let (newest_sharing, oldest_sharing, oldest_withheld) = (group(1), group(30), group(90));
        let memberships = vec![
            Membership {
                group: newest_sharing.clone(),
                shares: true,
            },
            Membership {
                group: oldest_withheld.clone(),
                shares: false,
            },
            Membership {
                group: oldest_sharing.clone(),
                shares: true,
            },
        ];
        assert_eq!(
            ranked(&memberships, None),
            [oldest_sharing.id, newest_sharing.id, oldest_withheld.id]
        );
        assert_eq!(
            ranked(&memberships, Some(oldest_withheld.id)),
            [oldest_withheld.id, oldest_sharing.id, newest_sharing.id],
            "the asked-for group comes first, shared with or not"
        );
        assert_eq!(
            ranked(&memberships, Some(Uuid::new_v4())),
            [oldest_sharing.id, newest_sharing.id, oldest_withheld.id],
            "a group the member is not in changes nothing"
        );
    }

    fn user(email: &str) -> User {
        User::new(email.to_owned(), String::new(), None)
    }

    #[test]
    fn a_roster_email_names_the_coached_member_whose_email_it_is() {
        let (alex, sam) = (Uuid::new_v4(), Uuid::new_v4());
        let members: BTreeMap<Uuid, Vec<Membership>> = [alex, sam]
            .into_iter()
            .map(|id| {
                (
                    id,
                    vec![Membership {
                        group: group(1),
                        shares: true,
                    }],
                )
            })
            .collect();
        let users: HashMap<Uuid, User> = [
            (alex, user("alex@links.test")),
            (sam, user("sam@links.test")),
        ]
        .into_iter()
        .collect();
        let named = |email: Option<&str>| member_named_by(email, &members, &users).map(|m| m.0);
        assert_eq!(named(Some(" ALEX@links.test")), Some(alex));
        assert_eq!(named(Some("sam@links.test")), Some(sam));
        assert_eq!(
            named(Some("outsider@links.test")),
            None,
            "an email no coached member holds names no one"
        );
        assert_eq!(named(None), None);
    }
}
