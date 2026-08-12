// ABOUTME: Pins every persona-contract rule that had no check before 2026-08-12
// ABOUTME: Each rule gets a firing case and a passing case so a stubbed check fails here

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use pierre_chat_pipeline::stages::persona_conformance::{check_reply_conformance, RosterScope};
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::models::CoachingPersona;

/// Build a registry from a YAML overlay so tests exercise the same parse path
/// production uses, rather than hand-constructing a contract struct.
fn registry(yaml: &str) -> Arc<PersonaContractRegistry> {
    let registry = Arc::new(PersonaContractRegistry::new());
    registry.apply_overlay(yaml).expect("overlay applies");
    registry
}

fn rules(
    yaml: &str,
    persona: CoachingPersona,
    reply: &str,
    roster: Option<&RosterScope>,
) -> Vec<String> {
    check_reply_conformance(&registry(yaml), persona, reply, roster)
        .into_iter()
        .map(|v| v.rule.to_owned())
        .collect()
}

// ---------------------------------------------------------------- round numbers

const ROUND: &str = r"
version: 2
personas:
  casual:
    round_numbers_required: true
";

#[test]
fn unrounded_decimal_violates_round_numbers() {
    let found = rules(ROUND, CoachingPersona::Casual, "Your TSS was 312.47.", None);
    assert!(
        found.contains(&"round_numbers_required".to_owned()),
        "4+ significant digits must fire, got {found:?}"
    );
}

#[test]
fn rounded_decimal_and_bare_integer_pass() {
    assert!(rules(ROUND, CoachingPersona::Casual, "About 5.2 hours.", None).is_empty());
    assert!(
        rules(
            ROUND,
            CoachingPersona::Casual,
            "You walked 4200 steps.",
            None
        )
        .is_empty(),
        "integers carry no fractional precision and must not fire"
    );
}

// ----------------------------------------------------------------- exact numbers

const EXACT: &str = r"
version: 2
personas:
  power_athlete:
    require_exact_numbers: true
";

#[test]
fn hedge_next_to_a_number_violates_exact_numbers() {
    let found = rules(
        EXACT,
        CoachingPersona::PowerAthlete,
        "Ride around 250 watts.",
        None,
    );
    assert!(
        found.contains(&"require_exact_numbers".to_owned()),
        "a hedge beside a digit must fire, got {found:?}"
    );
}

#[test]
fn committed_number_passes_exact_numbers() {
    assert!(rules(
        EXACT,
        CoachingPersona::PowerAthlete,
        "Ride 250 watts for 40 minutes.",
        None
    )
    .is_empty());
}

#[test]
fn hedge_far_from_any_number_passes_exact_numbers() {
    assert!(
        rules(
            EXACT,
            CoachingPersona::PowerAthlete,
            "Roughly the same session structure as last block, holding steady effort.",
            None
        )
        .is_empty(),
        "a hedge with no nearby digit is prose, not an imprecise prescription"
    );
}

// -------------------------------------------------------------------- P0-P3 ladder

const LADDER: &str = r"
version: 2
personas:
  power_athlete:
    require_p0_p3_ladder: true
";

#[test]
fn verdict_without_ladder_anchor_violates() {
    let found = rules(
        LADDER,
        CoachingPersona::PowerAthlete,
        "Modify today's session and keep the volume down.",
        None,
    );
    assert!(
        found.contains(&"require_p0_p3_ladder".to_owned()),
        "a verdict with no severity anchor must fire, got {found:?}"
    );
}

#[test]
fn verdict_with_ladder_anchor_passes() {
    assert!(rules(
        LADDER,
        CoachingPersona::PowerAthlete,
        "Modify today's session. P2 — reduce volume, keep intensity.",
        None
    )
    .is_empty());
}

#[test]
fn lowercase_prose_is_not_a_verdict() {
    assert!(
        rules(
            LADDER,
            CoachingPersona::PowerAthlete,
            "Just go easy today and enjoy the ride.",
            None
        )
        .is_empty(),
        "lowercase 'go' is prose; only the capitalised verdict token binds"
    );
}

// ------------------------------------------------- framework citation per numeric

const CITE: &str = r"
version: 2
personas:
  power_athlete:
    require_framework_citation_per_numeric: true
    framework_allowlist:
      - Coggan
      - Banister
";

#[test]
fn numeric_claim_without_framework_violates() {
    let found = rules(
        CITE,
        CoachingPersona::PowerAthlete,
        "Your threshold is 265 watts.",
        None,
    );
    assert!(
        found.contains(&"require_framework_citation_per_numeric".to_owned()),
        "an uncited numeric claim must fire, got {found:?}"
    );
}

#[test]
fn numeric_claim_with_allowlisted_framework_passes() {
    assert!(rules(
        CITE,
        CoachingPersona::PowerAthlete,
        "Your threshold is 265 watts (Coggan).",
        None
    )
    .is_empty());
}

#[test]
fn decimal_does_not_split_the_sentence() {
    assert!(
        rules(
            CITE,
            CoachingPersona::PowerAthlete,
            "Your ratio sits at 1.15 per Banister.",
            None
        )
        .is_empty(),
        "splitting on the decimal point would strand 'per Banister' in its own sentence"
    );
}

#[test]
fn empty_allowlist_disables_the_citation_rule() {
    let yaml = r"
version: 2
personas:
  power_athlete:
    require_framework_citation_per_numeric: true
";
    assert!(
        rules(
            yaml,
            CoachingPersona::PowerAthlete,
            "Your threshold is 265 watts.",
            None
        )
        .is_empty(),
        "with nothing allowed every sentence would fail; the field documents this as disabled"
    );
}

// --------------------------------------------------------- structured block size

const BLOCK: &str = r"
version: 2
personas:
  enthusiast:
    structured_block_max_lines: 5
";

#[test]
fn oversized_structured_block_violates() {
    let reply = "Distance: 42 km\nTime: 3h30\nPace: 5:00\nHR: 152\nTSS: 210\nElevation: 800 m";
    let found = rules(BLOCK, CoachingPersona::Enthusiast, reply, None);
    assert!(
        found.contains(&"structured_block_max_lines".to_owned()),
        "a 6-line block over a 5-line cap must fire, got {found:?}"
    );
}

#[test]
fn block_within_cap_passes() {
    let reply = "Distance: 42 km\nTime: 3h30\nPace: 5:00";
    assert!(rules(BLOCK, CoachingPersona::Enthusiast, reply, None).is_empty());
}

// ------------------------------------------------------- acronym first-use gloss

const FIRST_USE: &str = r"
version: 2
glossary:
  CTL:
    en: chronic training load
personas:
  enthusiast:
    forbid_acronyms_first_use_unglossed: true
";

#[test]
fn unglossed_first_use_violates() {
    let found = rules(
        FIRST_USE,
        CoachingPersona::Enthusiast,
        "Your CTL is climbing steadily this block.",
        None,
    );
    assert!(
        found.contains(&"forbid_acronyms_first_use_unglossed".to_owned()),
        "an unglossed first use must fire, got {found:?}"
    );
}

#[test]
fn glossed_first_use_then_bare_reuse_passes() {
    assert!(
        rules(
            FIRST_USE,
            CoachingPersona::Enthusiast,
            "Your CTL (chronic training load) is climbing. That CTL trend is healthy.",
            None
        )
        .is_empty(),
        "first use is glossed; later bare uses are explicitly allowed by this rule"
    );
}

// ------------------------------------------------------------ athlete id prefix

const COACH: &str = r"
version: 2
personas:
  coach:
    require_athlete_id_prefix: true
    require_tenant_isolation: true
";

#[test]
fn data_block_without_athlete_prefix_violates() {
    let reply = "Distance: 42 km\nTime: 3h30";
    let found = rules(COACH, CoachingPersona::Coach, reply, None);
    assert!(
        found.contains(&"require_athlete_id_prefix".to_owned()),
        "an unattributed data block must fire, got {found:?}"
    );
}

#[test]
fn prefixed_data_block_passes() {
    let scope = RosterScope::from_athlete_ids(["11111111-2222-3333-4444-555566667a1b"]);
    let reply = "Alice · 7a1b\nDistance: 42 km\nTime: 3h30";
    let found = rules(COACH, CoachingPersona::Coach, reply, Some(&scope));
    assert!(
        found.is_empty(),
        "an attributed block from a rostered athlete is clean, got {found:?}"
    );
}

// ------------------------------------------------------------- tenant isolation

#[test]
fn citing_an_athlete_outside_the_roster_violates() {
    let scope = RosterScope::from_athlete_ids(["11111111-2222-3333-4444-555566667a1b"]);
    let reply = "Mallory · dead\nDistance: 42 km\nTime: 3h30";
    let found = rules(COACH, CoachingPersona::Coach, reply, Some(&scope));
    assert!(
        found.contains(&"require_tenant_isolation".to_owned()),
        "a citation outside the roster must fire, got {found:?}"
    );
}

#[test]
fn tenant_isolation_fails_open_without_a_roster() {
    let reply = "Mallory · dead\nDistance: 42 km\nTime: 3h30";
    let found = rules(COACH, CoachingPersona::Coach, reply, None);
    assert!(
        !found.contains(&"require_tenant_isolation".to_owned()),
        "an unresolved roster must not flag every citation, got {found:?}"
    );
}

#[test]
fn roster_scope_matches_on_the_uuid_suffix_case_insensitively() {
    let scope = RosterScope::from_athlete_ids(["11111111-2222-3333-4444-5555666677AB"]);
    assert!(scope.allows("77ab"));
    assert!(scope.allows("77AB"));
    assert!(!scope.allows("dead"));
    assert!(!scope.is_empty());
}

#[test]
fn short_or_malformed_ids_do_not_widen_the_roster() {
    let scope = RosterScope::from_athlete_ids(["ab", ""]);
    assert!(
        scope.is_empty(),
        "an id too short to carry a suffix must be dropped, not padded into the allowed set"
    );
}
