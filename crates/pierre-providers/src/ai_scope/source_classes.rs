// ABOUTME: Where served content came from, recorded on the running derivation for a result's trust label (owl phase 1)
// ABOUTME: Provider items, the agent's own writing, stored physiology, and the reads a claim was judged on

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The source classes a derivation served ([`SourceClass`]).
//!
//! A filter point records the class of every provider item it serves
//! ([`note_served`](super::note_served)). The readers of stored content no
//! filter point sees record theirs here, each at the gate that admits it: the
//! agent's own writing ([`admit_agent_written`]), the athlete's physiology
//! ([`admit_profile`], [`note_sport_anchors`]) and a provider's profile of the
//! athlete ([`note_served_from`]). Nothing is recorded under
//! [`unfiltered`](super::unfiltered): a cache write serves no one.
//!
//! A read that only judges, and serves nothing unless its verdict rests on
//! what it read, runs under [`judging`], which keeps its classes apart.

use std::collections::BTreeSet;
use std::future::Future;

use pierre_core::ai_policy::SourceClass;
use pierre_core::constants::oauth_providers::source_class_of;
use pierre_core::models::{SportAnchor, UserPhysiologicalProfile};
use pierre_core::transport::TransportPolicy;

use super::{admit_derived, current_provenance, flag, Provenance, PROVENANCE, UNFILTERED};

/// Record that an item of `class` was served, on the running derivation.
pub(super) fn note_source_class(class: SourceClass) {
    if !flag(&UNFILTERED) {
        let _ = PROVENANCE.try_with(|provenance| provenance.note_source_class(class));
    }
}

/// Record that the running read served items of each of `classes`.
pub fn note_source_classes(classes: impl IntoIterator<Item = SourceClass>) {
    for class in classes {
        note_source_class(class);
    }
}

/// Record that the running read served content Dravr's own agent wrote.
///
/// A remembered fact, a stored plan: the result's trust label says so
/// ([`SourceClass::AgentWritten`]).
pub fn note_agent_written() {
    note_source_class(SourceClass::AgentWritten);
}

/// Record that the running read served what the provider behind the key
/// `provider` reports of the athlete, read outside every list filter (their
/// profile).
pub fn note_served_from(provider: &str) {
    note_source_class(source_class_of(provider));
}

/// Whether stored content Dravr's own agent wrote, stamped `policy`, is
/// served here.
///
/// It is admitted as [`admit_derived`] admits it; when it is, the running read
/// is recorded as having served the agent's writing.
#[must_use]
pub fn admit_agent_written(policy: TransportPolicy) -> bool {
    let admitted = admit_derived(policy);
    if admitted {
        note_agent_written();
    }
    admitted
}

/// Whether the athlete's stored physiology `profile` is served here.
///
/// It is admitted as [`admit_derived`] admits it; when it is, the running read
/// is recorded as having served where each value it holds came from
/// ([`UserPhysiologicalProfile::source_classes`]).
#[must_use]
pub fn admit_profile(profile: &UserPhysiologicalProfile) -> bool {
    let admitted = admit_derived(profile.transport_policy);
    if admitted {
        note_source_classes(profile.source_classes());
    }
    admitted
}

/// Record that the running read served the athlete's stored per-sport
/// `anchors` ([`SportAnchor::source_class`]).
pub fn note_sport_anchors(anchors: &[SportAnchor]) {
    note_source_classes(anchors.iter().map(SportAnchor::source_class));
}

/// Run `fut`, a read that judges with what it reads, returning the classes of
/// what it read instead of recording them on the running derivation.
///
/// Everything else it served reaches the enclosing derivation as under
/// [`tracking`](super::tracking): a row the caller writes is still stamped
/// from it. Its caller records the classes ([`note_source_classes`]) only when
/// what it answers rests on what was read.
pub async fn judging<F: Future>(fut: F) -> (F::Output, BTreeSet<SourceClass>) {
    let enclosing = current_provenance();
    let judged = Provenance::new();
    let output = PROVENANCE.scope(judged.clone(), fut).await;
    if let Some(enclosing) = enclosing {
        judged.pass_to(&enclosing);
    }
    (output, judged.source_classes())
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Timestamp;
    use pierre_core::models::{ActivityBuilder, SportType};
    use uuid::Uuid;

    use super::*;
    use crate::ai_scope::tests::{first_party, NolioOnly, Relay};
    use crate::ai_scope::{ai_read, derived, filter_activities, tracking, unfiltered};

    fn profile() -> UserPhysiologicalProfile {
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Run);
        profile.ftp_watts = Some(285);
        profile
    }

    #[tokio::test]
    async fn each_stored_read_records_where_it_came_from_when_admitted() {
        let call = Provenance::new();
        tracking(call.clone(), async {
            assert!(admit_profile(&profile()), "an unstamped profile is served");
            assert!(
                admit_agent_written(TransportPolicy::AnyTransport),
                "content any transport may carry is served"
            );
            note_served_from("wahoo");
        })
        .await;
        assert_eq!(
            call.source_classes(),
            BTreeSet::from([
                SourceClass::Device,
                SourceClass::PatientEntered,
                SourceClass::AgentWritten
            ]),
            "the profile, the agent's writing and Wahoo's profile each name their class"
        );
    }

    #[tokio::test]
    async fn a_cache_write_records_no_class() {
        let call = Provenance::new();
        tracking(call.clone(), async {
            unfiltered(async {
                let _ = admit_profile(&profile());
                let _ = admit_agent_written(TransportPolicy::AnyTransport);
                note_served_from("wahoo");
            })
            .await;
        })
        .await;
        assert!(
            call.source_classes().is_empty(),
            "a read under unfiltered serves no one, so it records no class"
        );
    }

    #[tokio::test]
    async fn a_judging_read_keeps_its_classes_and_hands_on_its_marks() {
        let call = Provenance::new();
        let ((), read) = tracking(call.clone(), async {
            judging(async {
                note_served_from("wahoo");
                if let Some(judged) = current_provenance() {
                    judged.mark();
                    judged.mark_provider_data();
                }
            })
            .await
        })
        .await;
        assert_eq!(
            read,
            BTreeSet::from([SourceClass::Device]),
            "the judge hands what it read back"
        );
        assert!(
            call.source_classes().is_empty(),
            "and the call is not recorded as having served it"
        );
        assert!(
            call.policy().is_first_party_only() && call.served_provider_data(),
            "while a row the call writes is still stamped from what was read"
        );
    }

    // owl phase 1: the class of what a nested read served reaches the call
    // that encloses it, the agent's own content is marked as such, and a
    // cache write records nothing.
    #[tokio::test]
    async fn source_classes_reach_the_enclosing_call_and_a_cache_write_marks_none() {
        let provider = Relay::governed();
        let call = Provenance::new();
        tracking(call.clone(), async {
            let ((read, _), _) =
                derived(first_party(ai_read(provider.get_activities(None, None)))).await;
            assert!(
                read.is_ok_and(|served| !served.is_empty()),
                "the relay served sessions"
            );
            note_agent_written();
        })
        .await;
        assert_eq!(
            call.source_classes(),
            BTreeSet::from([SourceClass::ThirdPartyApp, SourceClass::AgentWritten]),
            "Nolio's relayed sessions are another app's, and the memory read is the agent's"
        );

        let cache_write = Provenance::new();
        tracking(cache_write.clone(), async {
            let written = unfiltered(provider.get_activities(None, None)).await;
            assert!(written.is_ok(), "the cache write read the relay");
            unfiltered(async { note_agent_written() }).await;
        })
        .await;
        assert!(
            cache_write.source_classes().is_empty(),
            "a read under unfiltered serves no one, so it names no source"
        );

        let strava = Provenance::new();
        tracking(strava.clone(), async {
            let start = Timestamp::parse_from_rfc3339("2026-09-30T06:00:00Z").unwrap();
            let served = filter_activities(
                &NolioOnly,
                vec![
                    ActivityBuilder::new("s", "Easy", SportType::Run, start, 1800, "strava")
                        .build(),
                ],
            );
            assert_eq!(
                served.len(),
                1,
                "a Strava session is served unfiltered here"
            );
        })
        .await;
        assert_eq!(
            strava.source_classes(),
            BTreeSet::from([SourceClass::Device]),
            "a device's own connection is the device's record"
        );
    }
}
