// ABOUTME: One group member's data a stored row drew on, with how it was read (peer or coach way) and its class and tier
// ABOUTME: The one record every chat row, room entry and A2A task stores in its link table, and the key its readers decide by

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A carried read
//!
//! A row the platform stores from a turn (a chat row, a room transcript entry,
//! an A2A task) records every other group member whose data it drew on, each
//! as one [`CarriedRead`] (carnet#1511 design call 19): **how** it was read
//! ([`ReadWay`]: through the room's peer view, or through the coach's view of
//! the member by one coach), **where** (the group), and **what** (the class
//! and the sensitivity tier). Nothing else is a carrier: a row's
//! `about_user_id` is written by no new row and kept only for the rows
//! written before (call 19).
//!
//! The class and the tier reuse the taxonomies that already exist, by their
//! stable names: [`CLASS_TRAINING`] and [`CLASS_WELLNESS`] name
//! `read_subject::DataClass` ([`CLASS_BEYOND_ACTIVITIES`] a training read of
//! more than the member's activities, carnet#1511 call 23), and [`TIER_T2`] and [`TIER_T3`] name
//! `dravr_barrage::Tier` as it serializes. T0 and T1 are not recorded: the
//! class already gates them.
//!
//! Who may read a row carrying a read is decided by one rule,
//! `pierre_services::member_reads`, from the read's [`key`](CarriedRead::key):
//! the same text the database builds from a link row's columns
//! (`carried_read_key!`), so a statement tests only whether a link row's key
//! is one the rule admitted, never the rule itself.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The stable name of `read_subject::DataClass::Training`, as read through
/// a member's activities: the only data their peer sharing covers
/// (carnet#1511 call 23).
pub const CLASS_TRAINING: &str = "training";
/// A member's training beyond their activities, read the coach way alone.
///
/// Their plan, a session or a draft of it, their physiology (thresholds,
/// zones, profile) and anything a training read declares no activity stream
/// for: no member's peer sharing covers it (carnet#1511 call 23).
pub const CLASS_BEYOND_ACTIVITIES: &str = "beyond_activities";
/// The stable name of `read_subject::DataClass::Wellness`.
pub const CLASS_WELLNESS: &str = "wellness";
/// `dravr_barrage::Tier::T2` as it serializes: health records.
pub const TIER_T2: &str = "T2";
/// `dravr_barrage::Tier::T3` as it serializes: intimate health records.
pub const TIER_T3: &str = "T3";

/// The key prefix of a member a row written before carried reads existed
/// names in its `about_user_id`: read by the per-member rule, never by a way.
pub const LEGACY_ABOUT_PREFIX: &str = "about|";

/// How a member's data was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadWay {
    /// Through a group's peer view: what the member shares with the group's
    /// other members (a peer's group tool, a member card in a member's turn,
    /// a quoted room entry).
    Peer,
    /// Through the coach's view of the member, by one coach: a roster, a
    /// coach's card, `athlete=`, a plan or session edit. Read again only by
    /// that coach under the member's live coach consent for the tier, or by
    /// a peer where the member's peer sharing covers it.
    Coach,
    /// Another participant's own words, said where every reader of the same
    /// thread or room reads them: a participant's question in a shared
    /// thread, the coach's words to the room. Read by whoever reads that
    /// thread, or, with a group, that room.
    Thread,
    /// A coach's own write onto the member's plan or calendar, through the
    /// coach's view of them: the write's record, and the stored data the
    /// write admitted for its own computation (carnet#1511 call 35). Read
    /// again exactly as a [`Coach`](Self::Coach) read is, so the reply about
    /// the member's plan reaches no one the member's coach view keeps out,
    /// but never a read the turn or a replay of it drew on: a session for
    /// another athlete after it is still taken. Stored as `coach` with the
    /// link row's `written` flag set.
    Written,
}

impl ReadWay {
    /// The word a read's key carries. The link tables store every way by it
    /// but [`Written`](Self::Written), stored as `coach` with `written` set.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Peer => "peer",
            Self::Coach => "coach",
            Self::Thread => "thread",
            Self::Written => "written",
        }
    }

    /// The way a stored word names, `None` for any other word.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "peer" => Some(Self::Peer),
            "coach" => Some(Self::Coach),
            "thread" => Some(Self::Thread),
            "written" => Some(Self::Written),
            _ => None,
        }
    }

    /// Whether a read this way is read again through the coach's view of
    /// the member: a coach's read, or a coach's write onto them.
    #[must_use]
    pub const fn through_coach_view(self) -> bool {
        matches!(self, Self::Coach | Self::Written)
    }

    /// How strict a read this way is, the strictest first when merged. A
    /// write is read as strictly as a coach's read, and gives way to one
    /// ([`CarriedRead::strictest_per_member`]).
    const fn strictness(self) -> u8 {
        match self {
            Self::Coach => 3,
            Self::Written => 2,
            Self::Peer => 1,
            Self::Thread => 0,
        }
    }
}

/// One group member's data a stored row drew on.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CarriedRead {
    /// The member whose data it is.
    pub member: Uuid,
    /// How it was read.
    pub way: ReadWay,
    /// The group it was read under: the room's group for a peer read, the
    /// group whose coach read it for a coach read. `None` for a thread read,
    /// and for a read no group governed, which no one but the member reads
    /// again.
    pub group: Option<Uuid>,
    /// The coach who read it, for a coach read.
    pub coach: Option<Uuid>,
    /// What was read: [`CLASS_TRAINING`], [`CLASS_BEYOND_ACTIVITIES`] or
    /// [`CLASS_WELLNESS`].
    pub class: String,
    /// The most sensitive tier above T1 it reached: [`TIER_T2`] or
    /// [`TIER_T3`], `None` for T0 and T1.
    pub tier: Option<String>,
}

// Ids only, which is all a read holds.
impl fmt::Debug for CarriedRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key())
    }
}

impl CarriedRead {
    /// A member's training read through group `group`'s peer view.
    #[must_use]
    pub fn peer(member: Uuid, group: Option<Uuid>) -> Self {
        Self {
            member,
            way: ReadWay::Peer,
            group,
            coach: None,
            class: CLASS_TRAINING.to_owned(),
            tier: None,
        }
    }

    /// A member's training read by `coach` through the coach's view of them
    /// in group `group`.
    #[must_use]
    pub fn coach(member: Uuid, group: Uuid, coach: Uuid) -> Self {
        Self {
            member,
            way: ReadWay::Coach,
            group: Some(group),
            coach: Some(coach),
            class: CLASS_TRAINING.to_owned(),
            tier: None,
        }
    }

    /// Another participant's own words spoken to group `group`'s room, which
    /// every reader of that room reads: the coach's words there.
    #[must_use]
    pub fn room_words(member: Uuid, group: Uuid) -> Self {
        Self {
            group: Some(group),
            ..Self::thread(member)
        }
    }

    /// Another participant's own words read in the same thread.
    #[must_use]
    pub fn thread(member: Uuid) -> Self {
        Self {
            member,
            way: ReadWay::Thread,
            group: None,
            coach: None,
            class: CLASS_TRAINING.to_owned(),
            tier: None,
        }
    }

    /// This read as a write onto the member's account
    /// ([`ReadWay::Written`]): read again as a coach's read is, never drawn
    /// on (carnet#1511 call 35).
    #[must_use]
    pub fn written(mut self) -> Self {
        self.way = ReadWay::Written;
        self
    }

    /// This read of the member's wellness, not only their training.
    #[must_use]
    pub fn wellness(mut self) -> Self {
        CLASS_WELLNESS.clone_into(&mut self.class);
        self
    }

    /// This read of the member's training beyond their activities (their
    /// plan, their physiology), which only the coach way reads
    /// ([`CLASS_BEYOND_ACTIVITIES`], carnet#1511 call 23). A wellness read
    /// stays one: it is wider still.
    #[must_use]
    pub fn beyond_activities(mut self) -> Self {
        if self.class == CLASS_TRAINING {
            CLASS_BEYOND_ACTIVITIES.clone_into(&mut self.class);
        }
        self
    }

    /// This read reaching `tier` (`"T0"` to `"T3"`, as `dravr_barrage::Tier`
    /// serializes): kept when it is T2 or T3 and above what it already reached.
    #[must_use]
    pub fn reaching(mut self, tier: &str) -> Self {
        let tier = match tier {
            TIER_T2 => TIER_T2,
            TIER_T3 => TIER_T3,
            _ => return self,
        };
        if self.tier.as_deref() != Some(TIER_T3) {
            self.tier = Some(tier.to_owned());
        }
        self
    }

    /// Whether this read is of the member's activities alone, below T2: the
    /// only data a member's peer sharing covers (carnet#1511 call 23). Their
    /// plan and physiology ([`CLASS_BEYOND_ACTIVITIES`]) and their wellness
    /// are read the coach way alone.
    #[must_use]
    pub fn peer_view_covers(&self) -> bool {
        self.class == CLASS_TRAINING && self.tier.is_none()
    }

    /// The key a reader decides the read by: the same text the database
    /// builds from a link row's columns (`carried_read_key!`).
    #[must_use]
    pub fn key(&self) -> String {
        let id = |id: Option<Uuid>| id.map_or_default(|id| id.to_string());
        format!(
            "{}|{}|{}|{}|{}|{}",
            self.way.as_str(),
            self.member,
            id(self.group),
            id(self.coach),
            self.class,
            self.tier.as_deref().unwrap_or_default(),
        )
    }

    /// The read a [`key`](Self::key) names, `None` for a key that names none
    /// (a legacy `about` key, or text no read writes).
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        let mut parts = key.split('|');
        let way = ReadWay::parse(parts.next()?)?;
        let member = Uuid::parse_str(parts.next()?).ok()?;
        let id = |part: &str| -> Option<Option<Uuid>> {
            if part.is_empty() {
                Some(None)
            } else {
                Uuid::parse_str(part).ok().map(Some)
            }
        };
        let group = id(parts.next()?)?;
        let coach = id(parts.next()?)?;
        let class = parts.next()?.to_owned();
        let tier = match parts.next()? {
            "" => None,
            tier => Some(tier.to_owned()),
        };
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            member,
            way,
            group,
            coach,
            class,
            tier,
        })
    }

    /// The key of a member a legacy row names in its `about_user_id`.
    #[must_use]
    pub fn legacy_about_key(member: Uuid) -> String {
        format!("{LEGACY_ABOUT_PREFIX}{member}")
    }

    /// The member a legacy `about` key names, `None` for any other key.
    #[must_use]
    pub fn legacy_about_member(key: &str) -> Option<Uuid> {
        Uuid::parse_str(key.strip_prefix(LEGACY_ABOUT_PREFIX)?).ok()
    }

    /// One read per member of `reads`, the strictest of each member's: a
    /// coach read over a write over a peer read over a thread read, the
    /// widest class
    /// (wellness over training beyond activities over activities) and
    /// the higher tier, and, between two reads the same way under different
    /// groups or coaches, no group or coach at all. A link table holds one
    /// row per row and member, and a merged read is read by no one the
    /// stricter one keeps out. A write merged with any read of the same
    /// member is a coach's read, under the write's group and coach unless a
    /// coach's read names its own: one row cannot say both, and a write
    /// that hid a real read would let a replay of it draw on no one
    /// (carnet#1511 call 35).
    #[must_use]
    pub fn strictest_per_member<'a>(reads: impl IntoIterator<Item = &'a Self>) -> Vec<Self> {
        let mut merged: Vec<Self> = Vec::new();
        for read in reads {
            match merged.iter_mut().find(|kept| kept.member == read.member) {
                None => merged.push(read.clone()),
                Some(kept) => {
                    let mixes_a_write =
                        (kept.way == ReadWay::Written) != (read.way == ReadWay::Written);
                    let class = class_rank(&kept.class).max(class_rank(&read.class));
                    let tier = match (kept.tier.as_deref(), read.tier.as_deref()) {
                        (Some(TIER_T3), _) | (_, Some(TIER_T3)) => Some(TIER_T3),
                        (Some(TIER_T2), _) | (_, Some(TIER_T2)) => Some(TIER_T2),
                        _ => None,
                    };
                    if read.way.strictness() > kept.way.strictness() {
                        kept.way = read.way;
                        kept.group = read.group;
                        kept.coach = read.coach;
                    } else if read.way == kept.way {
                        // Two reads the same way under different groups or
                        // coaches: one row cannot name both, so it names
                        // neither, which no reader admits more than either
                        // (a read no group governs is read by the member
                        // alone, a coach read by no coach by no coach).
                        if kept.group != read.group {
                            kept.group = None;
                        }
                        if kept.coach != read.coach {
                            kept.coach = None;
                        }
                    }
                    if mixes_a_write {
                        kept.way = ReadWay::Coach;
                    }
                    class_of_rank(class).clone_into(&mut kept.class);
                    kept.tier = tier.map(str::to_owned);
                }
            }
        }
        merged
    }
}

/// How wide a read's class is, the narrowest first: a merged read keeps the
/// widest. A class no read writes ranks as wellness, which no reader admits
/// more than.
fn class_rank(class: &str) -> u8 {
    match class {
        CLASS_TRAINING => 0,
        CLASS_BEYOND_ACTIVITIES => 1,
        _ => 2,
    }
}

/// The class of rank `rank` ([`class_rank`]).
const fn class_of_rank(rank: u8) -> &'static str {
    match rank {
        0 => CLASS_TRAINING,
        1 => CLASS_BEYOND_ACTIVITIES,
        _ => CLASS_WELLNESS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two reads of one member the same way keep neither
    /// group (nor coach) when they differ, so the merged row is read by no
    /// one either read keeps out: a read no group governs stays the
    /// member's alone, and a read under G2 is not read as G1's.
    #[test]
    fn a_merge_of_two_reads_the_same_way_keeps_no_group_either_keeps_out() {
        let (alex, g1, g2, c1, c2) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let merged = CarriedRead::strictest_per_member(&[
            CarriedRead::peer(alex, Some(g1)),
            CarriedRead::peer(alex, None),
        ]);
        assert_eq!(
            merged,
            vec![CarriedRead::peer(alex, None)],
            "a read no group governs keeps the merge out of every group"
        );
        let merged = CarriedRead::strictest_per_member(&[
            CarriedRead::peer(alex, Some(g1)),
            CarriedRead::peer(alex, Some(g2)),
        ]);
        assert_eq!(
            merged,
            vec![CarriedRead::peer(alex, None)],
            "reads under two groups merge into a read of neither"
        );
        let merged = CarriedRead::strictest_per_member(&[
            CarriedRead::coach(alex, g1, c1),
            CarriedRead::coach(alex, g1, c2),
        ]);
        let neither = CarriedRead {
            coach: None,
            ..CarriedRead::coach(alex, g1, c1)
        };
        assert_eq!(
            merged,
            vec![neither],
            "two coaches' reads are read by neither"
        );
        let same = CarriedRead::strictest_per_member(&[
            CarriedRead::peer(alex, Some(g1)),
            CarriedRead::peer(alex, Some(g1)),
        ]);
        assert_eq!(
            same,
            vec![CarriedRead::peer(alex, Some(g1))],
            "two reads under one group keep that group"
        );
    }

    /// carnet#1511 call 23: a member's peer sharing covers their
    /// activities alone. A read of their plan or physiology is no peer
    /// view's, and a merge keeps the widest class.
    #[test]
    fn peer_sharing_covers_activities_alone() {
        let (alex, squad, casey) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let activities = CarriedRead::coach(alex, squad, casey);
        let plan = CarriedRead::coach(alex, squad, casey).beyond_activities();
        assert!(
            activities.peer_view_covers(),
            "a peer view covers an activities read"
        );
        assert!(
            !plan.peer_view_covers(),
            "a plan read is the coach way's alone"
        );
        assert_eq!(
            CarriedRead::coach(alex, squad, casey)
                .wellness()
                .beyond_activities()
                .class,
            CLASS_WELLNESS,
            "a wellness read stays one"
        );
        let merged = CarriedRead::strictest_per_member(&[activities.clone(), plan.clone()]);
        assert_eq!(
            merged,
            vec![plan.clone()],
            "an activities read merged with a plan read keeps the plan's class"
        );
        let merged = CarriedRead::strictest_per_member(&[plan, activities.wellness()]);
        assert_eq!(
            merged.first().map(|read| read.class.as_str()),
            Some(CLASS_WELLNESS),
            "a merge keeps the widest class, wellness: {merged:?}"
        );
        let key = CarriedRead::coach(alex, squad, casey)
            .beyond_activities()
            .key();
        assert_eq!(
            CarriedRead::from_key(&key).map(|read| read.class),
            Some(CLASS_BEYOND_ACTIVITIES.to_owned()),
            "a key carries the read's class: {key}"
        );
    }

    #[test]
    fn a_key_names_the_read_it_was_built_from() {
        let (alex, squad, casey) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        for read in [
            CarriedRead::peer(alex, Some(squad)),
            CarriedRead::peer(alex, None),
            CarriedRead::coach(alex, squad, casey)
                .wellness()
                .reaching(TIER_T2),
            CarriedRead::thread(alex),
            CarriedRead::coach(alex, squad, casey)
                .beyond_activities()
                .written(),
        ] {
            let key = read.key();
            assert_eq!(
                CarriedRead::from_key(&key),
                Some(read),
                "a key parses back to the read it was built from: {key}"
            );
        }
        assert_eq!(
            CarriedRead::from_key(&CarriedRead::legacy_about_key(alex)),
            None,
            "a legacy about-key is no carried read"
        );
        assert_eq!(
            CarriedRead::legacy_about_member(&CarriedRead::legacy_about_key(alex)),
            Some(alex),
            "a legacy about-key still names its member"
        );
    }

    /// carnet#1511 call 35: a write onto a member merged with a real read
    /// of them is a coach's read, so a replay of the row still draws on
    /// them; two writes stay a write.
    #[test]
    fn a_write_gives_way_to_any_read_of_its_member() {
        let (alex, squad, casey) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let write = CarriedRead::coach(alex, squad, casey)
            .beyond_activities()
            .written();
        for read in [
            CarriedRead::peer(alex, Some(squad)),
            CarriedRead::thread(alex),
            CarriedRead::coach(alex, squad, casey),
        ] {
            for reads in [[write.clone(), read.clone()], [read.clone(), write.clone()]] {
                let merged = CarriedRead::strictest_per_member(&reads);
                assert_eq!(
                    merged,
                    vec![CarriedRead::coach(alex, squad, casey).beyond_activities()],
                    "bug: a write hid a read of its member ({read:?})"
                );
            }
        }
        assert_eq!(
            CarriedRead::strictest_per_member(&[write.clone(), write.clone()]),
            vec![write.clone()],
            "two writes of one member stay a write"
        );
        assert!(
            write.way.through_coach_view() && ReadWay::Coach.through_coach_view(),
            "a write and a coach read both go through the coach view"
        );
        assert!(
            !ReadWay::Peer.through_coach_view() && !ReadWay::Thread.through_coach_view(),
            "a peer or thread read never goes through the coach view"
        );
    }

    #[test]
    fn a_merged_read_is_the_strictest_of_a_members_reads() {
        let (alex, bea, squad, casey) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let reads = [
            CarriedRead::peer(alex, Some(squad)),
            CarriedRead::coach(alex, squad, casey).wellness(),
            CarriedRead::peer(alex, Some(squad)).reaching(TIER_T3),
            CarriedRead::thread(bea),
        ];
        let merged = CarriedRead::strictest_per_member(&reads);
        assert_eq!(
            merged.len(),
            2,
            "a merge keeps one read per member: {merged:?}"
        );
        let alex_read = merged.iter().find(|read| read.member == alex);
        assert_eq!(
            alex_read.map(|read| read.way),
            Some(ReadWay::Coach),
            "bug: a coach read outranks a peer read"
        );
        assert_eq!(
            alex_read.and_then(|read| read.coach),
            Some(casey),
            "the coach read keeps the coach who made it"
        );
        assert_eq!(
            alex_read.map(|read| read.class.as_str()),
            Some(CLASS_WELLNESS),
            "bug: the wider class is kept"
        );
        assert_eq!(
            alex_read.and_then(|read| read.tier.as_deref()),
            Some(TIER_T3),
            "bug: the higher tier is kept"
        );
        assert!(
            alex_read.is_some_and(|read| !read.peer_view_covers()),
            "a merge that kept a wellness read is no peer view's"
        );
        assert!(CarriedRead::peer(alex, Some(squad)).peer_view_covers());
    }
}
