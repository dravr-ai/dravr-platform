// ABOUTME: Pins the peer-access-denial register — the coach denying it can read ANOTHER athlete's data
// ABOUTME: Positive pins are the 2026-08-30 Telegram incident sentences; negatives are honest DM coaching

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The own-access register (`contains_capability_failure`) fires the outbound
//! verification trigger on every surface, so a peer denial must never widen
//! it. The peer register is a separate predicate the pipeline consults only
//! with a group roster in hand; on replay both registers scrub alike.

use pierre_core::narration::{
    contains_capability_failure, contains_peer_access_denial, scrub_internal_narration,
    scrub_replayed_narration,
};

/// The replacement the Guardian's repair path manufactured on 2026-08-30 in a
/// Telegram group: every factual claim in it was false.
const INCIDENT_2026_08_30: &str = "Confiance : 1/10 — je n'ai jamais eu accès à l'historique de \
     Jean-Daniel. Ce que tu vois là, c'est encore mes propres activités. Je n'ai aucune donnée \
     sur lui tant que JD n'a pas envoyé un message ici.";

#[test]
fn the_incident_sentences_are_peer_denials_not_own_access_claims() {
    assert!(contains_peer_access_denial(INCIDENT_2026_08_30));
    assert!(
        !contains_capability_failure(INCIDENT_2026_08_30),
        "a third-person denial must not read as the coach's own tools being broken"
    );
    for sentence in [
        "Je n'ai jamais eu accès à l'historique de Jean-Daniel.",
        "Je n'ai aucune donnée sur lui.",
        "Je n'ai pas accès aux données de Phil tant qu'il n'a pas consenti.",
        "Je ne peux pas récupérer les activités de Marc pour le moment.",
    ] {
        assert!(
            contains_peer_access_denial(sentence),
            "peer register must fire on: {sentence}"
        );
    }
}

#[test]
fn peer_denials_are_detected_in_all_five_locales() {
    let fr = "Je n'ai pas accès à ses activités Strava.";
    let en = "I don't have access to his activities, only the roster summary.";
    let es = "No tengo acceso a los datos de Marta esta semana.";
    let de = "Ich habe keinen Zugriff auf seine Daten.";
    let pt = "Não tenho acesso aos dados dele.";
    for reply in [fr, en, es, de, pt] {
        assert!(
            contains_peer_access_denial(reply),
            "peer register must fire on: {reply}"
        );
        assert!(
            !contains_capability_failure(reply),
            "peer register must stay out of the own-access register: {reply}"
        );
    }
}

#[test]
fn honest_coaching_about_someone_elses_credentials_or_gear_is_not_a_denial() {
    for reply in [
        // Credentials and privacy reassurance — subject is the coach, the
        // object is not fitness data.
        "I don't have access to his Strava password — he logs in on Strava's own page.",
        // The athlete's own missing data point: first-person, but the object is
        // the athlete's ("ta"), which is neither register's business.
        "Je n'ai aucune donnée sur ta FC max — ajoute-la dans ton profil.",
        // Provider limitation talk with no first-person subject.
        "Impossible de récupérer les activités de Strava antérieures à 2020 — leur API s'arrête là.",
        // Ordinary peer coaching content.
        "Comme Phil hier, pars sur 45 min tranquilles.",
        "Marc a roulé 3h dimanche, tu peux viser 2h30.",
    ] {
        assert!(
            !contains_peer_access_denial(reply),
            "peer register must not fire on: {reply}"
        );
        assert!(
            !contains_capability_failure(reply),
            "own register must not fire on: {reply}"
        );
    }
}

#[test]
fn a_missing_stream_on_the_athletes_own_activity_is_not_an_own_access_claim() {
    // The one FR peer form that a DM reply can also produce: «aux données de»
    // followed by a stream name instead of a person. It must never start a
    // verification fetch — the pipeline gates the peer register on a roster
    // AND a named roster member — and it is not the coach's own tools failing.
    let reply = "Je n'ai pas accès aux données de fréquence cardiaque de cette sortie.";
    assert!(!contains_capability_failure(reply));
}

#[test]
fn peer_denials_are_scrubbed_on_replay_but_delivered_outbound() {
    let reply = "Je n'ai pas accès aux données de Jean-Daniel tant qu'il n'a pas consenti. \
                 Pour toi: sortie facile de 45 min ce soir.";
    let scrubbed = scrub_replayed_narration(reply);
    assert_eq!(
        scrubbed.removed, 1,
        "exactly the denial sentence is dropped"
    );
    assert!(scrubbed.cleaned.contains("45 min"));
    assert!(!scrubbed.cleaned.contains("Jean-Daniel"));

    let outbound = scrub_internal_narration(reply);
    assert!(
        !outbound.fired(),
        "an honest consent denial still reaches the athlete"
    );
}
