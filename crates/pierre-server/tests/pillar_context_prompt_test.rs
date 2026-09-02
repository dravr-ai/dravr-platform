// ABOUTME: Tests the coach-proposal pillar-context prompt block built from a user's Dossier
// ABOUTME: Covers the empty dossier, the north-star + covered-pillar render, and stale-fact exclusion

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::models::{Dossier, DossierFact, Pillar};
use pierre_routes_coaches::coaches::proposal_profile::pillar_context_prompt;
use uuid::Uuid;

fn fact(object: &str, stale: bool) -> DossierFact {
    DossierFact {
        kind: "north_star".to_owned(),
        predicate_code: "states".to_owned(),
        object: object.to_owned(),
        confidence: 0.9,
        source: "onboarding".to_owned(),
        updated_at: chrono::Utc::now(),
        valid_until: None,
        stale,
    }
}

#[test]
fn empty_dossier_yields_no_context() {
    let d = Dossier::empty(Uuid::nil(), Uuid::nil());
    assert_eq!(pillar_context_prompt(&d), None);
}

#[test]
fn north_star_and_covered_pillars_surface() {
    let mut d = Dossier::empty(Uuid::nil(), Uuid::nil());
    d.north_star = vec![fact("be present for my kids", false)];
    d.pillars
        .insert(Pillar::Fuelling, vec![fact("avoid dairy", false)]);
    let ctx = pillar_context_prompt(&d).expect("a dossier with fresh facts renders a context line");
    assert!(
        ctx.contains("be present for my kids"),
        "north star must reach the prompt: {ctx}"
    );
    assert!(
        ctx.contains("Fuelling"),
        "covered pillar must be labelled: {ctx}"
    );
}

#[test]
fn stale_only_context_is_ignored() {
    let mut d = Dossier::empty(Uuid::nil(), Uuid::nil());
    d.north_star = vec![fact("old motivation", true)];
    assert_eq!(pillar_context_prompt(&d), None);
}
