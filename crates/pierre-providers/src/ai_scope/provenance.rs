// ABOUTME: The accumulator a derivation stamps its rows from: what it served, what that owes, and whose data it was
// ABOUTME: Records the group member a read was about, so a row derived from one member is stamped with them (carnet#1174)

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

use pierre_core::transport::TransportPolicy;
use uuid::Uuid;

use super::{current_provenance, PROVENANCE};

tokio::task_local! {
    /// The group members read inside the innermost [`reading_members`]
    /// scope: whose data one tool round served (carnet#1174). The `Arc` is
    /// what lets [`reading_members`] read the set back once its scope ends.
    static MEMBERS_READ: Arc<Mutex<BTreeSet<Uuid>>>;
}

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
    /// The attributions of what was served (`"Garmin"` for Garmin
    /// device-sourced data): what a reply derived from it must say it drew on
    /// (carnet#521). Shared the same way as the flag.
    attributions: Arc<Mutex<BTreeSet<&'static str>>>,
    /// Set once a tool call served the athlete's provider items to the model:
    /// their own data was put in front of whatever this derivation produces.
    /// Read by the chat turn as `chat.answer_delivered`'s `grounded`
    /// (carnet#828). Set by the dispatch chokepoint when a call's
    /// [`serving_items`] scope served an item, and handed up to the enclosing
    /// derivation.
    ///
    /// [`serving_items`]: super::serving_items
    served_provider_data: Arc<AtomicBool>,
    /// The group members whose data was read under their consent
    /// (carnet#1174): what a row derived from it is about. Shared the same
    /// way as the flag.
    members_read: Arc<Mutex<BTreeSet<Uuid>>>,
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

    /// Record that data carrying `attribution` was served.
    pub fn attribute(&self, attribution: &'static str) {
        if let Ok(mut attributions) = self.attributions.lock() {
            attributions.insert(attribution);
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

    /// The attributions of everything served so far.
    #[must_use]
    pub fn attributions(&self) -> BTreeSet<&'static str> {
        self.attributions
            .lock()
            .map(|attributions| attributions.clone())
            .unwrap_or_default()
    }

    /// The policy a row derived from what was served so far is stamped with.
    #[must_use]
    pub fn policy(&self) -> TransportPolicy {
        TransportPolicy::from_first_party_only(self.served_first_party_only.load(Ordering::Acquire))
    }

    /// Record that a group member's data was read.
    pub(super) fn note_member(&self, member: Uuid) {
        if let Ok(mut members) = self.members_read.lock() {
            members.insert(member);
        }
    }

    /// Every group member whose data was read so far.
    #[must_use]
    pub(super) fn members_read(&self) -> BTreeSet<Uuid> {
        self.members_read
            .lock()
            .map(|members| members.clone())
            .unwrap_or_default()
    }

    /// The member a row derived from what was read so far is about: the one
    /// member read, or `None` when none was, or several were. A row about
    /// several members cannot name them all.
    #[must_use]
    fn sole_member(&self) -> Option<Uuid> {
        sole(&self.members_read())
    }
}

/// The one member of `members`, or `None` when it holds none or several.
#[must_use]
pub fn sole(members: &BTreeSet<Uuid>) -> Option<Uuid> {
    match members.len() {
        1 => members.first().copied(),
        _ => None,
    }
}

/// Run `fut`, returning the group members whose data it read.
///
/// The tool loop runs each round's calls in this, so the rows that round
/// writes are stamped with the members it read and no other round's
/// (carnet#1174). The reads still mark the turn's own provenance, which is
/// what the reply drawn from every round is stamped from.
pub async fn reading_members<F: Future>(fut: F) -> (F::Output, BTreeSet<Uuid>) {
    let members = Arc::new(Mutex::new(BTreeSet::new()));
    let output = MEMBERS_READ.scope(Arc::clone(&members), fut).await;
    let read = members.lock().map(|m| m.clone()).unwrap_or_default();
    (output, read)
}

/// Record that the running call read `member`'s data: a group member whose
/// consent let the caller read them (carnet#1174).
///
/// Called where a tool resolves the member and their consent lets the read
/// through, before their data is fetched, and where a turn stores the room
/// words of theirs its message quotes: whatever the turn derives from here on
/// may restate them.
pub fn note_member_read(member: Uuid) {
    let _ = PROVENANCE.try_with(|provenance| provenance.note_member(member));
    let _ = MEMBERS_READ.try_with(|members| {
        if let Ok(mut members) = members.lock() {
            members.insert(member);
        }
    });
}

/// The member content derived here so far is about: the one group member the
/// running derivation read, `None` outside one, or when it read none or
/// several (carnet#1174).
#[must_use]
pub fn derived_member() -> Option<Uuid> {
    current_provenance().and_then(|p| p.sole_member())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_scope::{derived, tracking};

    // carnet#1174: a round's rows name the members its own calls read, while
    // the turn, which the reply is stamped from, keeps every member read.
    #[tokio::test]
    async fn each_round_names_its_own_members_and_the_turn_keeps_them_all() {
        let (alice, bruno) = (Uuid::new_v4(), Uuid::new_v4());
        let turn = Provenance::new();
        let ((first, second), sole_after_first) = tracking(turn.clone(), async {
            let ((), first) = reading_members(async { note_member_read(alice) }).await;
            let sole_after_first = derived_member();
            let ((), second) = reading_members(async {
                note_member_read(bruno);
                note_member_read(bruno);
            })
            .await;
            ((first, second), sole_after_first)
        })
        .await;

        assert_eq!(first, BTreeSet::from([alice]), "the first round read Alice");
        assert_eq!(
            second,
            BTreeSet::from([bruno]),
            "the second round read Bruno alone, however often"
        );
        assert_eq!(
            sole_after_first,
            Some(alice),
            "a turn that read one member names them"
        );
        assert_eq!(
            turn.members_read(),
            BTreeSet::from([alice, bruno]),
            "the turn keeps every member its rounds read"
        );
        assert_eq!(
            turn.sole_member(),
            None,
            "a turn that read several members names none of them"
        );
    }

    #[tokio::test]
    async fn a_nested_derivation_hands_its_members_to_the_enclosing_one() {
        let member = Uuid::new_v4();
        let turn = Provenance::new();
        tracking(turn.clone(), async {
            let ((), _) = derived(async { note_member_read(member) }).await;
        })
        .await;
        assert_eq!(
            turn.sole_member(),
            Some(member),
            "a member a nested derivation read is the turn's too"
        );
    }

    #[tokio::test]
    async fn a_member_read_outside_any_scope_marks_nothing() {
        note_member_read(Uuid::new_v4());
        assert_eq!(
            derived_member(),
            None,
            "no derivation runs here, so none is about anyone"
        );
        let ((), read) = reading_members(async {}).await;
        assert!(read.is_empty(), "a round that read no member names none");
    }
}
