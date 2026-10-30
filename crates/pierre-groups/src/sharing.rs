// ABOUTME: Who may see a group member's training: the group's coach by coach sharing, everyone else by peer sharing
// ABOUTME: The one rule the agent's group context, the analytics routes and each manager's digest read (carnet#1166)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Member visibility within a coaching group.
//!
//! A member's training reaches another member only by that member's own
//! choice: coach sharing for the group's human coach, peer sharing for anyone
//! else. Owning or administering the group grants nothing more.

use std::collections::HashSet;

use pierre_core::models::groups::{CoachingGroup, GroupMember};
use uuid::Uuid;

use crate::service::GroupService;

impl GroupService {
    /// The members whose training `viewer` may see in this group.
    ///
    /// The group's human coach reads each member's coach sharing
    /// (`coach_sharing_user_ids`); anyone else, the owner and admins
    /// included, reads their peer sharing ([`Self::peer_sharing_user_ids`]).
    /// Managing a group grants no view of its members' training: joining is
    /// consent to share with the coach, not with whoever runs the group
    /// (carnet#1166). The viewer is always in the set, since their own
    /// training is theirs to see.
    ///
    /// Every surface that shows one member's training to another reads this
    /// rule: the agent's group context, `/stats`, the weekly report, the
    /// health flags and each manager's copy of the weekly digest.
    #[must_use]
    pub fn visible_member_ids(
        group: &CoachingGroup,
        members: &[GroupMember],
        viewer: Uuid,
    ) -> HashSet<Uuid> {
        let mut visible = if group.coach_user_id == Some(viewer) {
            Self::coach_sharing_user_ids(members)
        } else {
            Self::peer_sharing_user_ids(group, members)
        };
        visible.insert(viewer);
        visible
    }

    /// The members whose training the group's human coach may see: every
    /// live member who has not revoked `coach_sharing_consent`.
    ///
    /// Joining a group grants it (ADR-002: membership is consent to share with
    /// the coach) and `/group consent coach no` revokes it. The group's
    /// `peer_data_sharing` switch does not apply: it governs what members see
    /// of each other, never what their coach sees.
    #[must_use]
    fn coach_sharing_user_ids(members: &[GroupMember]) -> HashSet<Uuid> {
        members
            .iter()
            .filter(|m| m.left_at.is_none() && m.coach_sharing_consent)
            .map(|m| m.user_id)
            .collect()
    }

    /// The members whose training the rest of the group may see.
    ///
    /// Per-member `peer_sharing_consent` is the source of truth between
    /// members, so a subset can share while the rest stay private.
    /// `peer_data_sharing` is the admin kill switch: when FALSE nobody
    /// shares, whatever they consented to. Auto-bound groups default it to
    /// TRUE so individual consent works without an extra step. The digest
    /// posted into the group's chat reads this rule, and so does every viewer
    /// but the group's human coach, owner and admins included
    /// ([`Self::visible_member_ids`]).
    #[must_use]
    pub fn peer_sharing_user_ids(group: &CoachingGroup, members: &[GroupMember]) -> HashSet<Uuid> {
        if !group.peer_data_sharing {
            return HashSet::new();
        }
        members
            .iter()
            .filter(|m| m.peer_sharing_consent)
            .map(|m| m.user_id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pierre_core::models::groups::{
        CoachConsents, GroupDigestMode, GroupRespondMode, GroupRole,
    };

    use super::*;

    fn group(owner_id: Uuid, coach_user_id: Option<Uuid>) -> CoachingGroup {
        let now = Utc::now();
        CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4().to_string(),
            name: "Les Rouleurs".to_owned(),
            description: None,
            agent_id: Uuid::new_v4().to_string(),
            owner_id,
            coach_user_id,
            peer_data_sharing: true,
            respond_mode: GroupRespondMode::default(),
            digest_mode: GroupDigestMode::default(),
            max_members: 20,
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn member(group: &CoachingGroup, role: GroupRole, peer: bool, coach: bool) -> GroupMember {
        let now = Utc::now();
        GroupMember {
            id: Uuid::new_v4(),
            group_id: group.id,
            user_id: Uuid::new_v4(),
            tenant_id: group.tenant_id.clone(),
            role,
            peer_sharing_consent: peer,
            coach_sharing_consent: coach,
            coach_sharing_consent_at: Some(now),
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        }
    }

    /// A coach-owner, an admin who does not share, and three athletes: one
    /// sharing with everyone, one with their peers only, one with the coach
    /// only.
    fn roster() -> (CoachingGroup, Vec<GroupMember>) {
        let owner = Uuid::new_v4();
        let group = group(owner, Some(owner));
        let mut members = vec![
            member(&group, GroupRole::Owner, false, true),
            member(&group, GroupRole::Admin, false, true),
            member(&group, GroupRole::Member, true, true),
            member(&group, GroupRole::Member, true, false),
            member(&group, GroupRole::Member, false, true),
        ];
        members[0].user_id = owner;
        (group, members)
    }

    fn ids(members: &[GroupMember], picks: &[usize]) -> HashSet<Uuid> {
        picks.iter().map(|&i| members[i].user_id).collect()
    }

    #[test]
    fn the_coach_sees_the_members_who_share_with_the_coach() {
        let (group, members) = roster();
        let coach = members[0].user_id;
        assert_eq!(
            GroupService::visible_member_ids(&group, &members, coach),
            ids(&members, &[0, 1, 2, 4]),
            "the athlete who ran /group consent coach no is left out"
        );
    }

    #[test]
    fn an_admin_who_is_not_the_coach_sees_the_peer_sharers_and_themself() {
        let (group, members) = roster();
        let admin = members[1].user_id;
        assert_eq!(
            GroupService::visible_member_ids(&group, &members, admin),
            ids(&members, &[1, 2, 3]),
            "managing the group grants no view of a member who does not share"
        );
    }

    #[test]
    fn an_owner_who_is_not_the_coach_reads_the_peer_rule() {
        let (mut group, members) = roster();
        group.coach_user_id = None;
        let owner = members[0].user_id;
        assert_eq!(
            GroupService::visible_member_ids(&group, &members, owner),
            ids(&members, &[0, 2, 3]),
            "a group with no coach leaves its owner the peer sharers and themself"
        );
    }

    #[test]
    fn the_peer_switch_off_leaves_a_manager_only_themself_but_not_the_coach() {
        let (mut group, members) = roster();
        group.peer_data_sharing = false;
        assert_eq!(
            GroupService::visible_member_ids(&group, &members, members[1].user_id),
            ids(&members, &[1]),
            "with the peer switch off nobody shares with an admin who is not the coach"
        );
        assert_eq!(
            GroupService::visible_member_ids(&group, &members, members[0].user_id),
            ids(&members, &[0, 1, 2, 4]),
            "the peer switch governs what members see of each other, never the coach"
        );
    }
}
