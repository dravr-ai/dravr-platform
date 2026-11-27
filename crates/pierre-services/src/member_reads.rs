// ABOUTME: Whether a group member's data may be read by another person now: the group's coach by the coaching gates, a peer by peer consent
// ABOUTME: The one rule every reader of a stored row's carried reads asks (carnet#1511 call 19), and the peer rule group tools gate on

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Reading a group member
//!
//! A group's members read each other, and its human coach reads them, only
//! under the member's own consent:
//!
//! - **the group's human coach** (`coaching_groups.coach_user_id`) reads a
//!   member only through the coaching gates,
//!   [`coached_athlete_access`]: the coach rule (coach access and the
//!   attachment), a live membership, the member's `coach_sharing_consent`
//!   and one home tenant. Every coach read asks those gates, so no second
//!   rule here can drift from them (carnet#1425).
//! - **any other member** reads a live peer whose `peer_sharing_consent` is
//!   on, and only while the group's `peer_data_sharing` switch is on
//!   ([`peer_read_allowed`]).
//!
//! ## A stored row's reads: the one rule
//!
//! Every row the platform stores from a turn records each member read it
//! drew on as a [`CarriedRead`] (carnet#1511 call 19), and every reader of a
//! stored row — the transcript, the room page and its single entry, the
//! ambient block, the coach's review page, the conversation list's preview,
//! the replay into a turn, the export — asks [`readable_keys`] which of the
//! reads it holds the reader may read now, by key. The statements then only
//! test a link row's key against that answer. The rule:
//!
//! - **The member's own data** is theirs to read.
//! - **Another participant's words in the same thread**, or the coach's words
//!   to a room, are read by whoever reads that thread or room.
//! - **Through the room's peer view** ([`ReadWay::Peer`], and any read of a
//!   member's activities below T2, which is all their peer sharing covers:
//!   their plan and physiology are read the coach way alone, call 23): the
//!   reader holds a place in the read's group — a live member, or its coach
//!   now — and the member shares with that group now ([`peer_read_allowed`]).
//!   A later joiner and a replacement coach read room history this way
//!   (call 18).
//! - **Through a coach's view** ([`ReadWay::Coach`]): the reader is the coach
//!   who read it, still passing [`coached_athlete_access`] for the member in
//!   that group (coach access, the attachment, a live membership, coach
//!   consent, one home tenant: design Z4), and the member's consent for what
//!   was read: `wellness_read` for wellness; that and a standing
//!   `sensitive_t2_read` and their AI consent to T2 records for T2; their AI
//!   consent to T3 records for T3. A coach read stays the coach's unless the
//!   member's peer sharing covers it (call 19), and a coach who left the seat
//!   or lost access no longer reads through it (call 11).
//!
//! A row written before carried reads existed names one member in its
//! `about_user_id`, read by the per-member rule ([`readable_members`]); nothing in it
//! is reinterpreted (call 17).
//!
//! A reply posted where the whole room reads it ([`Reader::Room`]) may carry
//! only what every member reads: a member's data their peer sharing covers,
//! shared with that room now.

use std::collections::{BTreeSet, HashMap};

use dravr_tronc::dates::Timestamp;
use pierre_core::constants::oauth_providers::{SENSITIVE_T2, SENSITIVE_T3};
use pierre_core::errors::AppResult;
use pierre_core::models::carried_read::{
    CLASS_BEYOND_ACTIVITIES, CLASS_TRAINING, CLASS_WELLNESS, TIER_T2, TIER_T3,
};
use pierre_core::models::groups::GroupMember;
use pierre_core::models::{CarriedRead, ReadWay};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

use crate::coached_athlete::{coached_athlete_access, CoachedAthleteAccess};
use crate::provider_notice;

/// The standing a reader holds in the group whose member they read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupReader {
    /// The group's human coach (`coaching_groups.coach_user_id`): reads
    /// through the coaching gates.
    Coach,
    /// Any other member: reads by the member's peer consent, under the
    /// group's `peer_data_sharing` switch.
    Peer,
}

/// Whether a fellow member may read `member`'s data in a group whose
/// `peer_data_sharing` switch is `peer_data_sharing`: the member's peer
/// consent while the switch is on. A member who left is read by no one.
///
/// A group's human coach never reads through this rule: their read is the
/// coaching gates' to answer ([`coached_athlete_access`]).
#[must_use]
pub const fn peer_read_allowed(peer_data_sharing: bool, member: &GroupMember) -> bool {
    member.left_at.is_none() && peer_data_sharing && member.peer_sharing_consent
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
/// The per-member rule, which decides the one member a row written before carried
/// reads existed names in its `about_user_id` (carnet#1511 call 17), and
/// nothing else.
///
/// The group is `within` alone when given (the conversation's own group),
/// else any group they belong to or coach. A member no such group lets them
/// read — who withdrew their consent, left, or no longer has an account, or
/// whom the coaching gates refuse their coach for any other reason — is not
/// returned, so a caller that keeps only the returned members fails closed.
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
            let readable_here = match group.reader {
                GroupReader::Coach => coached_athlete_access(repos, reader, member, group.id)
                    .await?
                    .is_ok(),
                GroupReader::Peer => repos
                    .groups
                    .get_member(&group.id.to_string(), member)
                    .await?
                    .is_some_and(|membership| {
                        peer_read_allowed(group.peer_data_sharing, &membership)
                    }),
            };
            if readable_here {
                readable.push(member);
                break;
            }
        }
    }
    Ok(readable)
}

/// The read the per-member rule ([`readable_members`]) lets `reader` make of `member`.
///
/// `member` is the one member a row written before carried reads existed
/// names: read through the coach's view in a group they coach the member in, else
/// through the peer view of a group they share. `None` when no group lets
/// them. What a turn that replays such a row reads (carnet#1511 calls 17 and
/// 19), so the reply drawn from it carries a read its reader may read again.
///
/// # Errors
///
/// Returns the repository error from the group or membership lookups.
pub async fn legacy_read(
    repos: &RepositoryRegistry,
    reader: Uuid,
    member: Uuid,
) -> AppResult<Option<CarriedRead>> {
    if member == reader {
        return Ok(Some(CarriedRead::peer(member, None)));
    }
    // The coach's view first: a reader who coaches the member in one group
    // and shares another with them reads the row as their coach.
    let (coached, shared): (Vec<ReaderGroup>, Vec<ReaderGroup>) =
        reader_groups(repos, reader, None)
            .await?
            .into_iter()
            .partition(|group| matches!(group.reader, GroupReader::Coach));
    let groups = coached.into_iter().chain(shared);
    for group in groups {
        let read = match group.reader {
            GroupReader::Coach => coached_athlete_access(repos, reader, member, group.id)
                .await?
                .is_ok()
                .then(|| CarriedRead::coach(member, group.id, reader)),
            GroupReader::Peer => repos
                .groups
                .get_member(&group.id.to_string(), member)
                .await?
                .is_some_and(|membership| peer_read_allowed(group.peer_data_sharing, &membership))
                .then(|| CarriedRead::peer(member, Some(group.id))),
        };
        if read.is_some() {
            return Ok(read);
        }
    }
    Ok(None)
}

/// Who reads a stored row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reader {
    /// One person, on their own screen, in their export, or as a turn's
    /// sender replaying their own thread.
    Person(Uuid),
    /// Every member of group `group` at once, `sender` aside: a reply posted
    /// into a chat the whole room reads (carnet#1511 design calls 6 and 8).
    Room {
        /// The room's group.
        group: Uuid,
        /// Whether the group's peer switch is on, read with the group: off,
        /// the room reads no member's data.
        peer_switch: bool,
        /// The sender, whose own data the room's reply may carry.
        sender: Uuid,
    },
}

/// Of `keys`, those `reader` may read now.
///
/// The keys are the reads stored rows carry, as [`CarriedRead::key`] writes
/// them, and the `about` keys of rows written before. This is the one rule
/// every reader of a stored row asks (module doc,
/// carnet#1511 call 19). A key that names no read is readable by no one.
///
/// # Errors
///
/// Returns the repository error from a group, membership, coaching-gate or
/// consent lookup: the caller then serves nothing rather than everything.
pub async fn readable_keys(
    repos: &RepositoryRegistry,
    reader: Reader,
    keys: &[String],
) -> AppResult<Vec<String>> {
    let mut decider = KeyDecider::new(repos, reader);
    let mut readable = Vec::with_capacity(keys.len());
    for key in keys {
        if decider.readable(key).await? {
            readable.push(key.clone());
        }
    }
    Ok(readable)
}

/// Whether `reader` may read `read` now, under [`readable_keys`]'s rule.
///
/// # Errors
///
/// As [`readable_keys`].
pub(crate) async fn read_readable(
    repos: &RepositoryRegistry,
    reader: Reader,
    read: &CarriedRead,
) -> AppResult<bool> {
    KeyDecider::new(repos, reader).read(read).await
}

/// The reads of the rooms of `groups` that `reader` may read now.
///
/// Every read their transcript entries carry or their authors' words are read through
/// ([`CoachingGroupRepository::list_group_read_keys`]), decided by the one
/// rule ([`readable_keys`]). What every read of a room's transcript binds
/// (carnet#1511 call 19).
///
/// [`CoachingGroupRepository::list_group_read_keys`]: pierre_database::repositories::CoachingGroupRepository::list_group_read_keys
///
/// # Errors
///
/// As [`readable_keys`], and the repository error listing a room's reads.
pub async fn readable_room_keys(
    repos: &RepositoryRegistry,
    reader: Reader,
    groups: &[Uuid],
) -> AppResult<Vec<String>> {
    let mut keys: Vec<String> = Vec::new();
    for group in groups {
        for key in repos
            .groups
            .list_group_read_keys(&group.to_string())
            .await?
        {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    readable_keys(repos, reader, &keys).await
}

/// [`readable_keys`]'s rule, with every group, membership and consent it
/// reads looked up once.
struct KeyDecider<'repos> {
    repos: &'repos RepositoryRegistry,
    reader: Reader,
    /// The groups the person reader holds a place in, with the group's peer
    /// switch: loaded on first use.
    places: Option<HashMap<Uuid, bool>>,
    /// Live memberships looked up, by group and member.
    memberships: HashMap<(Uuid, Uuid), Option<GroupMember>>,
    /// Coaching-gate answers, by group and member.
    coached: HashMap<(Uuid, Uuid), Option<CoachedAthleteAccess>>,
    /// Members' AI consents to sensitive records, by member, each read in
    /// their home tenant.
    withheld: HashMap<Uuid, BTreeSet<String>>,
}

impl<'repos> KeyDecider<'repos> {
    fn new(repos: &'repos RepositoryRegistry, reader: Reader) -> Self {
        Self {
            repos,
            reader,
            places: None,
            memberships: HashMap::new(),
            coached: HashMap::new(),
            withheld: HashMap::new(),
        }
    }

    async fn readable(&mut self, key: &str) -> AppResult<bool> {
        if let Some(member) = CarriedRead::legacy_about_member(key) {
            return self.legacy(member).await;
        }
        match CarriedRead::from_key(key) {
            Some(read) => self.read(&read).await,
            None => Ok(false),
        }
    }

    /// A legacy row's `about` member, by the per-member rule.
    async fn legacy(&mut self, member: Uuid) -> AppResult<bool> {
        match self.reader {
            Reader::Person(reader) => Ok(member == reader
                || !readable_members(self.repos, reader, None, &[member])
                    .await?
                    .is_empty()),
            Reader::Room { group, sender, .. } => {
                Ok(member == sender || self.shares_with(group, member).await?)
            }
        }
    }

    async fn read(&mut self, read: &CarriedRead) -> AppResult<bool> {
        match self.reader {
            Reader::Person(reader) => self.person_reads(reader, read).await,
            Reader::Room { group, sender, .. } => {
                if read.member == sender {
                    return Ok(true);
                }
                // The whole room reads words spoken to it, and of a member's
                // data only what their peer sharing covers, shared with this
                // room now.
                if read.way == ReadWay::Thread {
                    return Ok(read.group == Some(group));
                }
                Ok(read.peer_view_covers() && self.shares_with(group, read.member).await?)
            }
        }
    }

    async fn person_reads(&mut self, reader: Uuid, read: &CarriedRead) -> AppResult<bool> {
        if read.member == reader || read.way == ReadWay::Thread {
            return Ok(true);
        }
        let Some(group) = read.group else {
            return Ok(false);
        };
        if read.peer_view_covers()
            && self.holds_place(reader, group).await?
            && self.shares_with(group, read.member).await?
        {
            return Ok(true);
        }
        if !read.way.through_coach_view() || read.coach != Some(reader) {
            return Ok(false);
        }
        let Some(access) = self.coached(reader, read.member, group).await? else {
            return Ok(false);
        };
        let consents = &access.consents;
        let class_read = match read.class.as_str() {
            CLASS_TRAINING | CLASS_BEYOND_ACTIVITIES => true,
            CLASS_WELLNESS => consents.wellness_read,
            _ => false,
        };
        if !class_read {
            return Ok(false);
        }
        match read.tier.as_deref() {
            None => Ok(true),
            Some(TIER_T2) => Ok(consents.wellness_read
                && consents.sensitive_t2_standing(Timestamp::now())
                && !self.ai_withheld(&access).await.contains(SENSITIVE_T2)),
            Some(TIER_T3) => Ok(!self.ai_withheld(&access).await.contains(SENSITIVE_T3)),
            Some(_) => Ok(false),
        }
    }

    /// Whether the person reader holds a place in `group` now: a live member,
    /// or its human coach.
    async fn holds_place(&mut self, reader: Uuid, group: Uuid) -> AppResult<bool> {
        if self.places.is_none() {
            self.places = Some(repos_places(self.repos, reader).await?);
        }
        Ok(self
            .places
            .as_ref()
            .is_some_and(|places| places.contains_key(&group)))
    }

    /// Whether `member` shares with `group`'s room now: live, peer consent
    /// on, the group's peer switch on.
    async fn shares_with(&mut self, group: Uuid, member: Uuid) -> AppResult<bool> {
        let switch = match self.reader {
            Reader::Room {
                group: room,
                peer_switch,
                ..
            } if room == group => peer_switch,
            Reader::Person(reader) => {
                // A person reads through a group they hold a place in, whose
                // switch that lookup carried.
                if !self.holds_place(reader, group).await? {
                    return Ok(false);
                }
                self.places
                    .as_ref()
                    .and_then(|places| places.get(&group).copied())
                    .unwrap_or(false)
            }
            Reader::Room { .. } => false,
        };
        if !switch {
            return Ok(false);
        }
        Ok(self
            .membership(group, member)
            .await?
            .is_some_and(|membership| peer_read_allowed(true, &membership)))
    }

    async fn membership(&mut self, group: Uuid, member: Uuid) -> AppResult<Option<GroupMember>> {
        if let Some(found) = self.memberships.get(&(group, member)) {
            return Ok(found.clone());
        }
        let found = self
            .repos
            .groups
            .get_member(&group.to_string(), member)
            .await?;
        self.memberships.insert((group, member), found.clone());
        Ok(found)
    }

    async fn coached(
        &mut self,
        coach: Uuid,
        member: Uuid,
        group: Uuid,
    ) -> AppResult<Option<CoachedAthleteAccess>> {
        if let Some(found) = self.coached.get(&(group, member)) {
            return Ok(found.clone());
        }
        let found = coached_athlete_access(self.repos, coach, member, group)
            .await?
            .ok();
        self.coached.insert((group, member), found.clone());
        Ok(found)
    }

    /// The member's AI consents withheld, in their home tenant: what a coach's
    /// read of a T2 or T3 record was served under.
    async fn ai_withheld(&mut self, access: &CoachedAthleteAccess) -> BTreeSet<String> {
        if let Some(found) = self.withheld.get(&access.athlete_user_id) {
            return found.clone();
        }
        let found = provider_notice::ai_consent_withheld(
            self.repos,
            access.tenant.as_uuid(),
            access.athlete_user_id,
        )
        .await;
        self.withheld.insert(access.athlete_user_id, found.clone());
        found
    }
}

/// The groups `reader` holds a place in now, each with its peer switch:
/// every active group they belong to, and every active group they coach.
async fn repos_places(repos: &RepositoryRegistry, reader: Uuid) -> AppResult<HashMap<Uuid, bool>> {
    Ok(reader_groups(repos, reader, None)
        .await?
        .into_iter()
        .map(|group| (group.id, group.peer_data_sharing))
        .collect())
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Timestamp;
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
            consent_given_at: Timestamp::now(),
            joined_at: Timestamp::now(),
            left_at: left.then(Timestamp::now),
            display_name: None,
        }
    }

    #[test]
    fn a_peer_reads_by_peer_consent_under_the_group_switch() {
        assert!(
            peer_read_allowed(true, &member(true, false, false)),
            "bug: a peer reads a member who shares with the group while its switch is on"
        );
        assert!(
            !peer_read_allowed(false, &member(true, true, false)),
            "bug: the group's peer switch off stops every peer read"
        );
        assert!(
            !peer_read_allowed(true, &member(false, true, false)),
            "bug: coach consent grants no peer read"
        );
    }

    #[test]
    fn a_member_who_left_is_read_by_no_peer() {
        assert!(
            !peer_read_allowed(true, &member(true, true, true)),
            "bug: a peer reads no member who left, whatever their consents say"
        );
    }
}
