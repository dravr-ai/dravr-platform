// ABOUTME: The accumulator a derivation stamps its rows from: what it served, what that owes, and whose data it was
// ABOUTME: Records whose data each read was, so a row or reply derived from one member owes only theirs (carnet#1174, #1464)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The [`Provenance`] accumulator (carnet#769) and the group members it
//! records (carnet#1174). The parent module scopes it ([`tracking`]) and
//! re-exports the public surface.
//!
//! [`tracking`]: super::tracking

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pierre_core::ai_policy::SourceClass;
use pierre_core::models::carried_read::ReadWay;
use pierre_core::models::CarriedRead;
use pierre_core::transport::TransportPolicy;
use uuid::Uuid;

use super::PROVENANCE;

tokio::task_local! {
    /// The member reads made inside the innermost [`reading_members`]
    /// scope: whose data one tool round served, and how (carnet#1174,
    /// carnet#1511 call 19). The `Arc` is what lets [`reading_members`] read
    /// the set back once its scope ends.
    static MEMBERS_READ: Arc<Mutex<BTreeSet<CarriedRead>>>;
    /// Whose data the reads inside the innermost [`reading_about`] scope
    /// serve: the person an attribution noted there is owed on (carnet#1464).
    static READ_ABOUT: Uuid;
    /// Set once a read inside the innermost [`reading_member_about`] scope
    /// serves the stored physiology of the person that scope reads
    /// (carnet#1511 call 27).
    static PHYSIOLOGY_SERVED: Arc<AtomicBool>;
}

/// One attribution a derivation owes, and whose data owed it: `None` when the
/// read that served it ran in no [`reading_about`] scope, so whose data it was
/// is not known.
type OwedAttribution = (Option<Uuid>, &'static str);

/// Whether a turn, or any other derivation, has served data whose terms keep
/// it first-party (carnet#769).
///
/// A positive accumulator: it starts clear and only ever gets set. Cloning
/// shares the flag — the `Arc` is what lets an executor built inside a turn
/// mark the turn's own flag from the task its calls run on.
#[derive(Debug, Clone, Default)]
pub struct Provenance {
    /// Set once anything first-party-only was served. Shared between the turn
    /// and every executor and nested derivation it hands a clone to.
    served_first_party_only: Arc<AtomicBool>,
    /// Set once anything kept to Dravr's members was served (carnet#1170):
    /// nothing derived from it may be posted into a messaging group chat.
    /// Shared the same way as the flag above.
    served_members_only: Arc<AtomicBool>,
    /// The attributions of what was served (`"Garmin"` for Garmin
    /// device-sourced data): what a reply derived from it must say it drew on
    /// (carnet#521), each with the person whose data owed it (carnet#1464), so
    /// a reply about one athlete owes only what their data owes. Shared the
    /// same way as the flag.
    attributions: Arc<Mutex<BTreeSet<OwedAttribution>>>,
    /// Set once a tool call served the athlete's provider items to the model:
    /// their own data was put in front of whatever this derivation produces.
    /// Read by the chat turn as `chat.answer_delivered`'s `grounded`
    /// (carnet#828). Set by the dispatch chokepoint when a call's
    /// [`serving_items`] scope served an item, and handed up to the enclosing
    /// derivation.
    ///
    /// [`serving_items`]: super::serving_items
    served_provider_data: Arc<AtomicBool>,
    /// Every read of a group member's data, with how it was read and its
    /// class and tier (carnet#1174, carnet#1511 call 19): what a row derived
    /// from it carries. Shared the same way as the flag.
    members_read: Arc<Mutex<BTreeSet<CarriedRead>>>,
    /// The reads that draw on a member: every read but a write onto a
    /// member ([`note_member_written`], [`ReadWay::Written`]), whether made
    /// on this turn or replayed from the topic's rows since its newest
    /// divider. What refuses a free-text write and fact extraction
    /// (carnet#1511 calls 24, 26 and 35).
    /// Shared the same way as the flag.
    drawn_on: Arc<Mutex<BTreeSet<CarriedRead>>>,
    /// Set once the derivation replayed a legacy row whose reads no row
    /// names (carnet#1511 A8): it drew on members no one can tell, so a
    /// free-text write onto any account but the caller's own is refused
    /// until a divider. Shared the same way as the flag.
    drew_on_unknown: Arc<AtomicBool>,
    /// Where what was served came from (owl T02 §3.1): the class of every
    /// provider whose items a filter point served, and
    /// [`SourceClass::AgentWritten`] once the agent's own memory or plan was.
    /// Read by the trust label on a result whose caller writes its own reply.
    /// Shared the same way as the flag.
    source_classes: Arc<Mutex<BTreeSet<SourceClass>>>,
    /// The room the derivation runs in, when it runs in a group's room
    /// ([`reading_in_room`]): what every reader of it may already read. A
    /// read within it is still recorded and stamped, but refuses no write to
    /// the caller's own account (carnet#1511 call 12, as narrowed). `None`
    /// outside a room. Shared the same way as the flag.
    room_view: Arc<Mutex<Option<RoomView>>>,
    /// The AI-consent keys of the provider data served (carnet#1157): what a
    /// row derived from it names, so the athlete's withdrawal of one reaches
    /// the row. Handed up to the enclosing derivation like the attributions,
    /// and shared the same way as the flag.
    consent_sources: Arc<Mutex<BTreeSet<String>>>,
}

impl Provenance {
    /// A clear accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that first-party-only data was served.
    pub fn mark(&self) {
        self.served_first_party_only.store(true, Ordering::Release);
    }

    /// Record that data kept to Dravr's members was served (carnet#1170).
    pub fn mark_members_only(&self) {
        self.served_members_only.store(true, Ordering::Release);
    }

    /// Whether data kept to Dravr's members was served so far.
    #[must_use]
    pub fn members_only(&self) -> bool {
        self.served_members_only.load(Ordering::Acquire)
    }

    /// Record that data carrying `attribution` was served, `owner`'s data
    /// when whose it was is known.
    pub fn attribute(&self, owner: Option<Uuid>, attribution: &'static str) {
        if let Ok(mut attributions) = self.attributions.lock() {
            attributions.insert((owner, attribution));
        }
    }

    /// Record that a provider-backed read was served.
    pub fn mark_provider_data(&self) {
        self.served_provider_data.store(true, Ordering::Release);
    }

    /// Whether a tool call served provider items to the model so far.
    #[must_use]
    pub fn served_provider_data(&self) -> bool {
        self.served_provider_data.load(Ordering::Acquire)
    }

    /// The attributions of what was served so far about the people `owed_for`
    /// accepts, asked with `None` for data whose owner is not known.
    #[must_use]
    pub fn attributions_owed(
        &self,
        owed_for: impl Fn(Option<Uuid>) -> bool,
    ) -> BTreeSet<&'static str> {
        self.owed()
            .into_iter()
            .filter(|(owner, _)| owed_for(*owner))
            .map(|(_, attribution)| attribution)
            .collect()
    }

    /// Every attribution served so far, with whose data owed it.
    fn owed(&self) -> BTreeSet<OwedAttribution> {
        self.attributions
            .lock()
            .map_or_default(|attributions| attributions.clone())
    }

    /// Hand every attribution this derivation owes to `enclosing`, each on
    /// the person whose data owed it.
    pub(super) fn attribute_to(&self, enclosing: &Self) {
        for (owner, attribution) in self.owed() {
            enclosing.attribute(owner, attribution);
        }
    }

    /// Hand this derivation's first-party-only and grounding marks, its
    /// attributions, the members it read and the consent keys of what it
    /// served to `enclosing`, the one it ran inside.
    ///
    /// Content derived inside derived content taints both, and a tool call's
    /// grounding mark reaches the call that enclosed it. Where what it served
    /// came from (its source classes) is not handed on here: the caller hands
    /// it on or keeps it ([`super::tracking`], [`super::judging`]).
    pub(super) fn pass_to(&self, enclosing: &Self) {
        if self.policy().is_first_party_only() {
            enclosing.mark();
        }
        if self.members_only() {
            enclosing.mark_members_only();
        }
        if self.served_provider_data() {
            enclosing.mark_provider_data();
        }
        if self.drew_on_unknown_member() {
            enclosing.note_unknown_member();
        }
        self.attribute_to(enclosing);
        let drawn_on = self.drawn_on();
        for read in self.carried_reads() {
            if drawn_on.contains(&read) {
                enclosing.note_member(read);
            } else {
                enclosing.note_written(read);
            }
        }
        for key in self.consent_sources() {
            enclosing.note_consent_source(&key);
        }
    }

    /// Record that data whose AI consent is `key` was served.
    pub fn note_consent_source(&self, key: &str) {
        if let Ok(mut sources) = self.consent_sources.lock() {
            sources.insert(key.to_ascii_lowercase());
        }
    }

    /// The AI-consent keys of everything served so far, sorted.
    #[must_use]
    pub fn consent_sources(&self) -> Vec<String> {
        self.consent_sources
            .lock()
            .map_or_default(|sources| sources.iter().cloned().collect())
    }

    /// Record that an item of `class` was served.
    pub fn note_source_class(&self, class: SourceClass) {
        if let Ok(mut classes) = self.source_classes.lock() {
            classes.insert(class);
        }
    }

    /// The class of everything served so far.
    #[must_use]
    pub fn source_classes(&self) -> BTreeSet<SourceClass> {
        self.source_classes
            .lock()
            .map_or_default(|classes| classes.clone())
    }

    /// The policy a row derived from what was served so far is stamped with.
    #[must_use]
    pub fn policy(&self) -> TransportPolicy {
        TransportPolicy::from_first_party_only(self.served_first_party_only.load(Ordering::Acquire))
    }

    /// Record that a group member's data was read, how and of what class
    /// and tier, whoever else may read it: a stamp names every member read a
    /// row drew on (carnet#1511 calls 12, as narrowed by JF 7106050048, and
    /// 19), and the read draws on them ([`Self::members_drawn_on`]). A
    /// write onto a member ([`ReadWay::Written`]), made on this turn or
    /// replayed from a row that carries it, draws on no one (carnet#1511
    /// call 35).
    pub(super) fn note_member(&self, read: CarriedRead) {
        if read.way != ReadWay::Written {
            if let Ok(mut members) = self.drawn_on.lock() {
                members.insert(read.clone());
            }
        }
        self.note_written(read);
    }

    /// Record that a write acted on a member's account: carried and stamped
    /// as a read is, since the call's result is theirs, but drawing on no
    /// one (carnet#1511 call 26), so the next write onto them, or onto
    /// another athlete, is still taken.
    fn note_written(&self, read: CarriedRead) {
        if let Ok(mut members) = self.members_read.lock() {
            members.insert(read);
        }
    }

    /// Record that the derivation runs in `view`, a group's room.
    fn read_in_room(&self, view: RoomView) {
        if let Ok(mut room_view) = self.room_view.lock() {
            *room_view = Some(view);
        }
    }

    /// Whether every reader of the room the derivation runs in may already
    /// read what `read` read ([`RoomView::admits`]). Outside a room, or
    /// where the view cannot be read, nothing is.
    #[must_use]
    fn room_admits(&self, read: &CarriedRead) -> bool {
        self.room_view
            .lock()
            .is_ok_and(|view| view.as_ref().is_some_and(|view| view.admits(read)))
    }

    /// The members drawn on so far ([`Self::members_drawn_on`]), by a read
    /// wider than what every reader of the room the derivation runs in may
    /// already read: every member drawn on outside a room. What refuses a write to the caller's own account
    /// (carnet#1511 call 12: room-visible data relaxes only own-account
    /// writes). Decided per read, by how it was read and under which group
    /// (design §2.1), never by the member alone.
    #[must_use]
    pub fn members_read_beyond_room(&self) -> BTreeSet<Uuid> {
        self.drawn_on()
            .into_iter()
            .filter(|read| !self.room_admits(read))
            .map(|read| read.member)
            .collect()
    }

    /// Every group member whose data was read so far, however it was read.
    #[must_use]
    pub fn members_read(&self) -> BTreeSet<Uuid> {
        self.members_read
            .lock()
            .map_or_default(|members| members.iter().map(|read| read.member).collect())
    }

    /// Every member the derivation drew on so far: read on this turn or
    /// replayed from the topic since its newest divider, never only written
    /// to (carnet#1511 calls 24 and 26).
    #[must_use]
    pub fn members_drawn_on(&self) -> BTreeSet<Uuid> {
        self.drawn_on()
            .into_iter()
            .map(|read| read.member)
            .collect()
    }

    /// Record that the derivation drew on members no row names: a replayed
    /// legacy row that may carry several (carnet#1511 A8).
    fn note_unknown_member(&self) {
        self.drew_on_unknown.store(true, Ordering::Release);
    }

    /// Whether the derivation drew on members no row names
    /// ([`Self::note_unknown_member`]).
    #[must_use]
    pub fn drew_on_unknown_member(&self) -> bool {
        self.drew_on_unknown.load(Ordering::Acquire)
    }

    /// The members the derivation drew on so far through `coach`'s coach view
    /// of them: an athlete `coach` held coach access to when it read them.
    #[must_use]
    pub fn members_drawn_on_as_coach(&self, coach: Uuid) -> BTreeSet<Uuid> {
        self.drawn_on()
            .into_iter()
            .filter(|read| read.way == ReadWay::Coach && read.coach == Some(coach))
            .map(|read| read.member)
            .collect()
    }

    /// The reads the derivation drew on so far, with how each was read.
    fn drawn_on(&self) -> BTreeSet<CarriedRead> {
        self.drawn_on
            .lock()
            .map_or_default(|members| members.clone())
    }

    /// Every member read so far, with how it was read: what a row derived
    /// from it carries (carnet#1511 call 19).
    #[must_use]
    pub fn carried_reads(&self) -> BTreeSet<CarriedRead> {
        self.members_read
            .lock()
            .map_or_default(|members| members.clone())
    }
}

/// Run `fut`, returning the member reads it made.
///
/// The tool loop runs each round's calls in this, so the rows that round
/// writes carry the reads it made and no other round's (carnet#1174). The
/// reads still mark the turn's own provenance, which is what the reply drawn
/// from every round carries.
pub async fn reading_members<F: Future>(fut: F) -> (F::Output, BTreeSet<CarriedRead>) {
    let members = Arc::new(Mutex::new(BTreeSet::new()));
    let output = MEMBERS_READ.scope(Arc::clone(&members), fut).await;
    let read = members.lock().map_or_default(|m| m.clone());
    (output, read)
}

/// Record that the running call read a group member's data, as `read` says.
///
/// The read names whose data it is, through which view (the room's peer
/// view, or a coach's view by one coach), and of what class and tier
/// (carnet#1174, carnet#1511 call 19).
///
/// Called where a tool resolves the member and their consent lets the read
/// through, before their data is fetched, where a turn puts a member's card
/// in its prompt, and where a turn stores or replays the room words of theirs
/// its message quotes: whatever the turn derives from here on may restate
/// them.
pub fn note_member_read(read: CarriedRead) {
    // Recorded whoever else may read it, for the turn and the round alike: a
    // stamp names every member a row drew on (carnet#1511 call 12, narrowed).
    let _ = PROVENANCE.try_with(|provenance| provenance.note_member(read.clone()));
    let _ = MEMBERS_READ.try_with(|members| {
        if let Ok(mut members) = members.lock() {
            members.insert(read);
        }
    });
}

/// Record that the turn replays a row carrying `read`.
///
/// The turn's provenance carries it as it carries any read, so whatever the
/// turn derives is stamped and withheld by it, and it draws on the member
/// as a read made on this turn does (carnet#1511 call 24). It names no
/// round's rows: the round made no read.
pub fn note_replayed_member_read(read: CarriedRead) {
    let _ = PROVENANCE.try_with(|provenance| provenance.note_member(read));
}

/// Record that the turn replays a legacy row whose reads no row names
/// (carnet#1511 A8).
///
/// A legacy row named its member only when its turn read exactly one, so a
/// legacy row naming no one, in a conversation where another legacy row
/// names someone else, may have drawn on several: the turn draws on members
/// no one can tell, and nothing it writes as free text onto another
/// account is taken until a divider. Nothing is stamped on a row: no
/// member can be named.
pub fn note_replayed_unknown_member() {
    let _ = PROVENANCE.try_with(Provenance::note_unknown_member);
}

/// Record that the running call writes onto `read`'s member's account.
///
/// The call's result and every row derived from it are stamped about them,
/// the round's included, as a write ([`ReadWay::Written`]): read again as a
/// coach's read of them is, but drawing on no one, on this turn or on a
/// replay of its rows, so it refuses no later write (carnet#1511 calls 26
/// and 35). A coach prescribing Alex two sessions in one turn writes both,
/// and may still write for Bea, in this turn or the next. The stored
/// physiology the write admits for its own computation is the write's, and
/// draws on no one either.
pub fn note_member_written(read: CarriedRead) {
    let read = read.written();
    let _ = PROVENANCE.try_with(|provenance| provenance.note_written(read.clone()));
    let _ = MEMBERS_READ.try_with(|members| {
        if let Ok(mut members) = members.lock() {
            members.insert(read);
        }
    });
}

/// Hand a read a nested [`reading_members`] scope collected to the scope
/// enclosing it, so a chat round collecting its own calls' members still
/// names it.
///
/// The turn's provenance is not touched: the read already marked it, as a
/// read or as a write's target, and noting it again as a read would make a
/// write's target draw on its member (carnet#1511 call 26).
pub fn note_round_read(read: CarriedRead) {
    let _ = MEMBERS_READ.try_with(|members| {
        if let Ok(mut members) = members.lock() {
            members.insert(read);
        }
    });
}

/// What every reader of a group's room may already read: the words spoken
/// to it, and the training below T2 of each member who shares with it,
/// read through its own peer view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomView {
    /// The room's group.
    pub group: Uuid,
    /// The members who share their training with the room.
    pub sharers: BTreeSet<Uuid>,
}

impl RoomView {
    /// Whether every reader of this room may already read what `read` read:
    ///
    /// - words said to this room, or in the thread the derivation runs in
    ///   ([`ReadWay::Thread`] under this group or none), which every reader
    ///   of it reads, the coach's words to the room among them;
    /// - a member's training below T2 ([`CarriedRead::peer_view_covers`])
    ///   read through this room's own peer view, of a member who shares with
    ///   it.
    ///
    /// A coach-way read or write is wider than any peer view (the coach's consents),
    /// and a read under another group is no read of this room's, so neither
    /// is (carnet#1511 call 19).
    #[must_use]
    pub fn admits(&self, read: &CarriedRead) -> bool {
        match read.way {
            ReadWay::Thread => read.group.is_none_or(|group| group == self.group),
            ReadWay::Peer => {
                read.group == Some(self.group)
                    && read.peer_view_covers()
                    && self.sharers.contains(&read.member)
            }
            ReadWay::Coach | ReadWay::Written => false,
        }
    }
}

/// Record that the derivation running here runs in `view`, a group's room.
///
/// What every reader of it may already read is still recorded and stamped;
/// it only does not refuse a write to the caller's own account
/// ([`Provenance::members_read_beyond_room`], carnet#1511 call 12 as
/// narrowed). Outside any derivation it records nothing.
pub fn reading_in_room(view: RoomView) {
    let _ = PROVENANCE.try_with(|provenance| provenance.read_in_room(view));
}

/// Run `fut` reading `person`'s data: an attribution what it serves owes is
/// owed on `person` (carnet#1464).
///
/// Set where a read's owner is known: the dispatch chokepoint around a read
/// of a coached athlete, and a group snapshot around each member's. A read in
/// no such scope owes its attribution on no one in particular, which every
/// reply derived from it then carries.
pub async fn reading_about<F: Future>(person: Uuid, fut: F) -> F::Output {
    READ_ABOUT.scope(person, fut).await
}

/// [`reading_about`] for a coach's read of a coached athlete, returning
/// whether it served the athlete's stored physiology.
///
/// Their saved thresholds, zones, maximum or resting heart rate, age: what
/// the tool returned, not only what it declared, decides that the read went
/// beyond their activities (carnet#1511 call 27).
pub async fn reading_member_about<F: Future>(person: Uuid, fut: F) -> (F::Output, bool) {
    let served = Arc::new(AtomicBool::new(false));
    let output = PHYSIOLOGY_SERVED
        .scope(Arc::clone(&served), READ_ABOUT.scope(person, fut))
        .await;
    (output, served.load(Ordering::Acquire))
}

/// Record that the running read served `owner`'s stored physiology: marks
/// the innermost [`reading_member_about`] scope when it reads `owner`, so a
/// coach's read tool that served it went beyond their activities
/// (carnet#1511 call 27). A write onto `owner` that admits it for its own
/// computation runs in no such scope, and reads no one (call 35).
pub(super) fn note_physiology_served(owner: Uuid) {
    if read_about() == Some(owner) {
        let _ = PHYSIOLOGY_SERVED.try_with(|served| served.store(true, Ordering::Release));
    }
}

/// Whose data the running read serves, when a [`reading_about`] scope says.
#[must_use]
pub(super) fn read_about() -> Option<Uuid> {
    READ_ABOUT.try_with(|person| *person).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_scope::{derived, note_attribution, tracking};

    fn peer(member: Uuid) -> CarriedRead {
        CarriedRead::peer(member, Some(Uuid::nil()))
    }

    // carnet#1511 call 12, narrowed by JF 7106050048: a stamp names every
    // member a row drew on, so the turn and the round record a member every
    // reader of the room already reads; only the beyond-room set, which
    // refuses own-account writes, leaves them out. Outside a room every
    // member is beyond it.
    #[tokio::test]
    async fn a_room_records_every_member_and_counts_only_wider_ones_beyond_it() {
        let (alice, carol) = (Uuid::new_v4(), Uuid::new_v4());
        let turn = Provenance::new();
        let round = tracking(turn.clone(), async {
            reading_in_room(RoomView {
                group: Uuid::nil(),
                sharers: BTreeSet::from([alice]),
            });
            let ((), round) = reading_members(async {
                note_member_read(peer(alice));
                note_member_read(peer(carol));
            })
            .await;
            round
        })
        .await;
        assert_eq!(
            round,
            BTreeSet::from([peer(alice), peer(carol)]),
            "the round names both members it read"
        );
        assert_eq!(
            turn.members_read(),
            BTreeSet::from([alice, carol]),
            "the turn names both members it read"
        );
        assert_eq!(
            turn.members_read_beyond_room(),
            BTreeSet::from([carol]),
            "only Carol is wider than the room's own view"
        );

        let outside = Provenance::new();
        tracking(outside.clone(), async { note_member_read(peer(alice)) }).await;
        assert_eq!(
            outside.members_read_beyond_room(),
            BTreeSet::from([alice]),
            "outside a room every member read is beyond it"
        );
    }

    // carnet#1511 call 12 and design §2.1: a room admits a read by how it
    // was read and under which group, never by the member alone. Words said
    // to the room (the coach's) and a sharer's training through this room's
    // peer view are within it; a coach-way read, a read under another group,
    // a non-sharer, wellness and a T2 read are beyond it.
    #[test]
    fn a_room_admits_a_read_by_how_and_where_it_was_read() {
        let (alex, coach, room, other) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let view = RoomView {
            group: room,
            sharers: BTreeSet::from([alex]),
        };
        assert!(
            view.admits(&CarriedRead::room_words(coach, room)),
            "words said to this room are within it"
        );
        assert!(
            view.admits(&CarriedRead::thread(coach)),
            "the coach's own thread read is within the room"
        );
        assert!(
            view.admits(&CarriedRead::peer(alex, Some(room))),
            "a sharer's training read through this room's peer view is within it"
        );
        for (beyond, why) in [
            (
                CarriedRead::room_words(coach, other),
                "words said to another room",
            ),
            (CarriedRead::coach(alex, room, coach), "a coach-way read"),
            (
                CarriedRead::peer(alex, Some(other)),
                "a read under another group",
            ),
            (CarriedRead::peer(alex, None), "a read no group governed"),
            (
                CarriedRead::peer(coach, Some(room)),
                "a member who does not share",
            ),
            (CarriedRead::peer(alex, Some(room)).wellness(), "wellness"),
            (
                CarriedRead::peer(alex, Some(room)).reaching("T2"),
                "a T2 read",
            ),
        ] {
            assert!(!view.admits(&beyond), "bug: the room admitted {why}");
        }
    }

    // carnet#1174, carnet#1511 call 19: a round's rows carry the reads its
    // own calls made, each as it was made, while the turn, which the reply
    // carries, keeps every read.
    #[tokio::test]
    async fn each_round_names_its_own_reads_and_the_turn_keeps_them_all() {
        let (alice, bruno, squad, casey) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let coach_wellness = CarriedRead::coach(bruno, squad, casey).wellness();
        let turn = Provenance::new();
        let (first, second) = tracking(turn.clone(), async {
            let ((), first) = reading_members(async { note_member_read(peer(alice)) }).await;
            let ((), second) = reading_members(async {
                note_member_read(coach_wellness.clone());
                note_member_read(coach_wellness.clone());
            })
            .await;
            (first, second)
        })
        .await;

        assert_eq!(
            first,
            BTreeSet::from([peer(alice)]),
            "the first round read Alice"
        );
        assert_eq!(
            second,
            BTreeSet::from([coach_wellness.clone()]),
            "the second round read Bruno's wellness as his coach, however often"
        );
        assert_eq!(
            turn.carried_reads(),
            BTreeSet::from([peer(alice), coach_wellness]),
            "the turn keeps every read its rounds made, each as it was made"
        );
        assert_eq!(
            turn.members_read(),
            BTreeSet::from([alice, bruno]),
            "the turn read every member its rounds read"
        );
    }

    #[tokio::test]
    async fn a_nested_derivation_hands_its_reads_to_the_enclosing_one() {
        let member = Uuid::new_v4();
        let turn = Provenance::new();
        tracking(turn.clone(), async {
            let ((), _) = derived(async { note_member_read(peer(member)) }).await;
        })
        .await;
        assert_eq!(
            turn.carried_reads(),
            BTreeSet::from([peer(member)]),
            "a read a nested derivation made is the turn's too"
        );
    }

    // carnet#1511 calls 24 and 26: a replayed read draws on its member as a
    // live one does; a write's own target is carried and stamped, through a
    // nested derivation too, but draws on no one.
    #[tokio::test]
    async fn a_replayed_read_draws_on_its_member_and_a_write_on_none() {
        let (replayed, live, written) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let turn = Provenance::new();
        let round = tracking(turn.clone(), async {
            note_replayed_member_read(peer(replayed));
            let ((), _) = derived(async { note_member_read(peer(live)) }).await;
            let ((), round) = reading_members(async {
                let ((), _) = derived(async { note_member_written(peer(written)) }).await;
            })
            .await;
            round
        })
        .await;
        assert_eq!(
            turn.carried_reads(),
            BTreeSet::from([peer(replayed), peer(live), peer(written).written()]),
            "every read and the write's target are carried, the write as one"
        );
        assert_eq!(
            round,
            BTreeSet::from([peer(written).written()]),
            "the write's round is stamped about its target"
        );
        assert_eq!(
            turn.members_drawn_on(),
            BTreeSet::from([replayed, live]),
            "bug: a replayed read drew on no one, or a write's target counted as drawn on"
        );
        assert_eq!(
            turn.members_read_beyond_room(),
            BTreeSet::from([replayed, live]),
            "outside a room the replayed and live reads are beyond it"
        );
    }

    // carnet#1511 call 35: a row carrying a write onto a member, replayed
    // on the next turn of the topic, is carried and stamped but draws on no
    // one; a real read on the same row still does.
    #[tokio::test]
    async fn a_replayed_write_draws_on_no_one() {
        let (alex, bea) = (Uuid::new_v4(), Uuid::new_v4());
        let turn = Provenance::new();
        tracking(turn.clone(), async {
            note_replayed_member_read(peer(alex).written());
            note_replayed_member_read(peer(bea));
        })
        .await;
        assert_eq!(
            turn.members_read(),
            BTreeSet::from([alex, bea]),
            "a replayed write and a replayed read both name their member"
        );
        assert_eq!(
            turn.members_drawn_on(),
            BTreeSet::from([bea]),
            "bug: a write onto Alex replayed from an earlier turn drew on her"
        );
    }

    // carnet#1464: an attribution is owed on the person whose data owed it,
    // through a nested derivation too; a read whose owner no scope names
    // owes it on no one in particular.
    #[tokio::test]
    async fn each_attribution_is_owed_on_whose_data_owed_it() {
        let (coach, athlete) = (Uuid::new_v4(), Uuid::new_v4());
        let turn = Provenance::new();
        tracking(turn.clone(), async {
            reading_about(coach, async { note_attribution(Some("Garmin")) }).await;
            reading_about(athlete, async { note_attribution(Some("Polar")) }).await;
            let ((), _) = derived(reading_about(athlete, async {
                note_attribution(Some("Coros"));
            }))
            .await;
            note_attribution(Some("Suunto"));
        })
        .await;

        assert_eq!(
            turn.attributions_owed(|_| true),
            BTreeSet::from(["Coros", "Garmin", "Polar", "Suunto"]),
            "the turn keeps every attribution, whoever's data owed it"
        );
        assert_eq!(
            turn.attributions_owed(|owner| owner == Some(athlete)),
            BTreeSet::from(["Coros", "Polar"]),
            "the athlete's reads owe theirs alone, a nested derivation's included"
        );
        assert_eq!(
            turn.attributions_owed(|owner| owner == Some(coach)),
            BTreeSet::from(["Garmin"]),
            "the coach's read owes its own"
        );
        assert_eq!(
            turn.attributions_owed(|owner| owner.is_none()),
            BTreeSet::from(["Suunto"]),
            "a read no scope names is owed on no one in particular"
        );
    }

    #[tokio::test]
    async fn a_member_read_outside_any_scope_marks_nothing() {
        note_member_read(peer(Uuid::new_v4()));
        let ((), read) = reading_members(async {}).await;
        assert!(read.is_empty(), "a round that read no member names none");
    }
}
