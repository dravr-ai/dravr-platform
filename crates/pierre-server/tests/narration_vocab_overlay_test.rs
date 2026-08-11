// ABOUTME: Pins the narration-vocabulary overlay chain: YAML parse → version gate → validation →
// ABOUTME: atomic install into GLOBAL_NARRATION_VOCAB → matcher semantics per class (replay vs outbound)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The narration vocabulary is runtime-extensible via the contremaitre
//! overlay `config/narration.yaml` so a phrasing mutation observed live
//! («je ne suis pas capable…» escaping the compiled «je ne peux pas»
//! family for 18 days, 2026-07-24 → 08-11) starts matching on the next
//! sync tick instead of the next deploy. These tests pin the semantics the
//! sync engine and the scrubs rely on:
//!
//! - `capability_failure` entries: replay scrub + outbound boundary
//!   detector, never the outbound scrub.
//! - `internal_narration` entries: outbound scrub too.
//! - `identity` entries: replay scrub ONLY — the outbound withhold
//!   (`contains_identity_leak`) keeps its compiled-in table and negation
//!   semantics.
//! - last-good-wins: version gate, over-match rejection, and parse errors
//!   all leave the previous snapshot live.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_contremaitre::manifest::parse_manifest;
use pierre_contremaitre::narration_vocab::{current_sha256, reload_narration_vocab};
use pierre_core::narration::{
    contains_capability_failure, contains_identity_leak, scrub_internal_narration,
    scrub_replayed_narration,
};
use serial_test::serial;

#[test]
#[serial]
fn capability_entries_extend_detection_and_replay_scrub_only() {
    let phrase = "Je ne suis plus en mesure de récupérer tes activités ce matin.";
    assert!(
        !contains_capability_failure(phrase),
        "phrase must not be in the compiled-in table, or this test proves nothing"
    );

    let yaml =
        "version: 1\ncapability_failure:\n  - \"plus en mesure de récupérer tes activités\"\n";
    let counts = reload_narration_vocab(yaml, "sha-cap-1".to_owned()).expect("overlay applies");
    assert_eq!(counts.capability_failure, 1);

    assert!(
        contains_capability_failure(phrase),
        "the boundary detector must pick up the overlay entry"
    );
    assert!(
        scrub_replayed_narration(phrase).fired(),
        "the replay scrub must drop the overlay-matched sentence"
    );
    assert!(
        !scrub_internal_narration(phrase).fired(),
        "capability vocabulary must stay replay-only for the outbound scrub — an honest \
         outage report still reaches the athlete"
    );
}

#[test]
#[serial]
fn internal_entries_are_scrubbed_outbound_too() {
    let phrase = "Je respecte le protocole des blocs invisibles du système.";
    let yaml = "version: 1\ninternal_narration:\n  - \"protocole des blocs invisibles\"\n";
    reload_narration_vocab(yaml, "sha-int-1".to_owned()).expect("overlay applies");

    assert!(scrub_internal_narration(phrase).fired());
    assert!(scrub_replayed_narration(phrase).fired());
}

#[test]
#[serial]
fn identity_entries_extend_replay_scrub_but_never_the_withhold() {
    let phrase = "Je suis un assistant de terminal générique, pas ton coach.";
    let yaml = "version: 1\nidentity:\n  - \"assistant de terminal générique\"\n";
    reload_narration_vocab(yaml, "sha-id-1".to_owned()).expect("overlay applies");

    assert!(
        scrub_replayed_narration(phrase).fired(),
        "a poisoned identity row must be dropped on replay"
    );
    assert!(
        !contains_identity_leak(phrase),
        "the outbound withhold keeps its compiled-in table — plain overlay strings carry \
         no negation/class semantics and must never withhold a reply"
    );
}

#[test]
#[serial]
fn unsupported_version_is_rejected_and_previous_snapshot_stays() {
    let good = "version: 1\ncapability_failure:\n  - \"incapable d'atteindre tes données\"\n";
    reload_narration_vocab(good, "sha-keep".to_owned()).expect("v1 applies");
    assert!(contains_capability_failure(
        "Je suis incapable d'atteindre tes données ce soir."
    ));

    let bad = "version: 2\ncapability_failure:\n  - \"whatever phrasing here\"\n";
    let err = reload_narration_vocab(bad, "sha-reject".to_owned())
        .expect_err("an unknown version must be rejected");
    assert!(
        err.contains("version"),
        "error names the version gate: {err}"
    );

    assert!(
        contains_capability_failure("Je suis incapable d'atteindre tes données ce soir."),
        "the previous snapshot must stay live after a rejected apply"
    );
    assert_eq!(
        current_sha256().as_deref(),
        Some("sha-keep"),
        "the rejected document must not advance the skip-check sha"
    );
}

#[test]
#[serial]
fn overmatching_entry_rejects_the_whole_overlay() {
    let good = "version: 1\ncapability_failure:\n  - \"impossible d'interroger tes plateformes\"\n";
    reload_narration_vocab(good, "sha-before-overmatch".to_owned()).expect("v1 applies");

    // «de mon» folds to 6 characters — it would match half of every French
    // reply, so the WHOLE document is rejected, not just the entry.
    let bad = "version: 1\ncapability_failure:\n  - \"de mon\"\n  - \"une entrée parfaitement valide ici\"\n";
    let err = reload_narration_vocab(bad, "sha-overmatch".to_owned())
        .expect_err("an over-matching entry must reject the apply");
    assert!(err.contains("de mon"), "error names the entry: {err}");

    assert!(
        contains_capability_failure("C'est impossible d'interroger tes plateformes aujourd'hui."),
        "the previous snapshot must stay live"
    );
    assert_eq!(current_sha256().as_deref(), Some("sha-before-overmatch"));
}

#[test]
#[serial]
fn malformed_yaml_is_rejected() {
    let err = reload_narration_vocab("version: [not, a, number", "sha-bad-yaml".to_owned())
        .expect_err("malformed YAML must be rejected");
    assert!(err.contains("parse"), "error names the parse stage: {err}");
}

#[test]
#[serial]
fn empty_overlay_installs_and_matches_nothing() {
    let counts = reload_narration_vocab("version: 1\n", "sha-empty".to_owned())
        .expect("an empty overlay is the steady state and must apply");
    assert_eq!(counts.capability_failure, 0);
    assert_eq!(counts.internal_narration, 0);
    assert_eq!(counts.identity, 0);
    assert_eq!(current_sha256().as_deref(), Some("sha-empty"));

    // Compiled-in vocabulary keeps working with an empty overlay installed.
    assert!(contains_capability_failure(
        "Je ne suis pas capable de récupérer tes activités en ce moment."
    ));
}

#[test]
fn manifest_without_narration_entry_still_parses() {
    // Mirrors the older-manifest compatibility pin for the tools section:
    // a manifest predating `config.narration` must deserialize cleanly and
    // leave the field `None` (compiled-in vocabulary only).
    let manifest_json = r#"{
        "version": 5,
        "config": {
            "cageux": { "path": "config/cageux.yaml", "sha256": "abc" }
        },
        "prompts": { "system": {}, "coaches": {}, "personas": {} },
        "tools": {},
        "evidence": {},
        "strings": {}
    }"#;
    let manifest = parse_manifest(manifest_json).expect("older manifest parses");
    assert!(manifest.config.narration.is_none());
    assert!(manifest.config.cageux.is_some());
}
