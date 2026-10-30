// ABOUTME: Whether a group member's data may be read by another person now: the group's coach by the member's coach consent, a peer by theirs
// ABOUTME: The rule the group tools gate a member read on, and the one a turn's history replays a stamped row under (carnet#1174)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Reading a group member
//!
//! A group's members read each other, and its human coach reads them, only
//! under the member's own consent:
//!
//! - **the group's human coach** (`coaching_groups.coach_user_id`) reads a
//!   live member whose `coach_sharing_consent` is on. Joining grants it
//!   (ADR-002) and the member revokes it; the group's peer switch does not
//!   govern it.
//! - **any other member** reads a live peer whose `peer_sharing_consent` is
//!   on, and only while the group's `peer_data_sharing` switch is on.
//!
//! [`member_read_allowed`] is that rule for one membership; the group tools
//! gate every fresh read on it. [`readable_members`] asks it of a set of
//! members by id, for the turn that replays rows stamped with them
//! (`about_user_id`, carnet#1174): a member who withdrew their consent, left
//! the group, or deleted their account is no longer replayed into a prompt.

use pierre_core::errors::AppResult;
use pierre_core::models::groups::GroupMember;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// The standing a reader holds in the group whose member they read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupReader {
    /// The group's human coach (`coaching_groups.coach_user_id`): reads by
    /// the member's coach consent.
    Coach,
    /// Any other member: reads by the member's peer consent, under the
    /// group's `peer_data_sharing` switch.
    Peer,
}

/// Whether `reader` may read `member`'s data in a group.
///
/// The group's coach reads by the member's coach consent; a peer by their
/// peer consent while the group's `peer_data_sharing` switch is on. A member
/// who left is read by no one.
#[must_use]
pub fn member_read_allowed(
    reader: GroupReader,
    peer_data_sharing: bool,
    member: &GroupMember,
) -> bool {
    if member.left_at.is_some() {
        return false;
    }
    match reader {
        GroupReader::Coach => member.coach_sharing_consent,
        GroupReader::Peer => peer_data_sharing && member.peer_sharing_consent,
    }
}

/// A group the reader reads members of, with their standing in it.
struct ReaderGroup {
    id: Uuid,
    peer_data_sharing: bool,
    reader: GroupReader,
}

/// Every active group `reader` belongs to or is the human coach of, narrowed
/// to `within` when given: the room a conversation is bound to.
async fn reader_groups(
    repos: &RepositoryRegistry,
    reader: Uuid,
    within: Option<Uuid>,
) -> AppResult<Vec<ReaderGroup>> {
    let mut groups: Vec<ReaderGroup> = repos
        .groups
        .list_groups_for_user(reader)
        .await?
        .into_iter()
        .map(|group| ReaderGroup {
            id: group.id,
            peer_data_sharing: group.peer_data_sharing,
            reader: GroupReader::Peer,
        })
        .collect();
    for coached in repos.groups.list_groups_coached_by(reader).await? {
        if !coached.is_active {
            continue;
        }
        // A coach who is also a member of the group they coach reads it as
        // its coach.
        if let Some(existing) = groups.iter_mut().find(|group| group.id == coached.id) {
            existing.reader = GroupReader::Coach;
        } else {
            groups.push(ReaderGroup {
                id: coached.id,
                peer_data_sharing: coached.peer_data_sharing,
                reader: GroupReader::Coach,
            });
        }
    }
    if let Some(room) = within {
        groups.retain(|group| group.id == room);
    }
    Ok(groups)
}

/// Of `members`, those `reader` may read now through a group they share.
///
/// The group is `within` alone when given (the conversation's own group),
/// else any group they belong to or coach. A member no such group lets them
/// read — who withdrew their consent, left, or no longer has an account — is
/// not returned, so a caller that keeps only the returned members fails
/// closed.
///
/// # Errors
///
/// Returns the repository error from the group or membership lookups.
pub async fn readable_members(
    repos: &RepositoryRegistry,
    reader: Uuid,
    within: Option<Uuid>,
    members: &[Uuid],
) -> AppResult<Vec<Uuid>> {
    if members.is_empty() {
        return Ok(Vec::new());
    }
    let groups = reader_groups(repos, reader, within).await?;
    let mut readable = Vec::with_capacity(members.len());
    for &member in members {
        // No one reads themselves through a group: the coaching gates refuse
        // a coach as their own athlete.
        if member == reader {
            continue;
        }
        for group in &groups {
            let Some(membership) = repos
                .groups
                .get_member(&group.id.to_string(), member)
                .await?
            else {
                continue;
            };
            if member_read_allowed(group.reader, group.peer_data_sharing, &membership) {
                readable.push(member);
                break;
            }
        }
    }
    Ok(readable)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pierre_core::models::groups::{CoachConsents, GroupRole};

    use super::*;

    fn member(peer: bool, coach: bool, left: bool) -> GroupMember {
        GroupMember {
            id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            tenant_id: String::new(),
            role: GroupRole::Member,
            peer_sharing_consent: peer,
            coach_sharing_consent: coach,
            coach_sharing_consent_at: None,
            coach_consents: CoachConsents::default(),
            consent_given_at: Utc::now(),
            joined_at: Utc::now(),
            left_at: left.then(Utc::now),
            display_name: None,
        }
    }

    #[test]
    fn the_coach_reads_by_coach_consent_alone() {
        assert!(
            member_read_allowed(GroupReader::Coach, false, &member(false, true, false)),
            "the coach reads a consenting member whatever the peer switch says"
        );
        assert!(
            !member_read_allowed(GroupReader::Coach, true, &member(true, false, false)),
            "a member who revoked the coach's read is not read by the coach"
        );
    }

    #[test]
    fn a_peer_reads_by_peer_consent_under_the_group_switch() {
        assert!(
            member_read_allowed(GroupReader::Peer, true, &member(true, false, false)),
            "a peer reads a member who shares with the group while its switch is on"
        );
        assert!(
            !member_read_allowed(GroupReader::Peer, false, &member(true, true, false)),
            "the group's peer switch off stops every peer read"
        );
        assert!(
            !member_read_allowed(GroupReader::Peer, true, &member(false, true, false)),
            "coach consent grants no peer read"
        );
    }

    #[test]
    fn a_member_who_left_is_read_by_no_one() {
        assert!(
            !member_read_allowed(GroupReader::Coach, true, &member(true, true, true)),
            "the coach reads no member who left, whatever their consents say"
        );
        assert!(
            !member_read_allowed(GroupReader::Peer, true, &member(true, true, true)),
            "a peer reads no member who left, whatever their consents say"
        );
    }
}
