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
        "Your FTP is 265 watts.",
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
        "Your FTP is 265 watts (Coggan).",
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
            "Your ACWR sits at 1.15 per Banister.",
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

#[test]
fn measurements_dates_and_windows_are_not_model_claims() {
    // Production 2026-10-05: every one of these was reported on a strict coach
    // turn and sent to the style editor, which can only satisfy the rule by
    // stapling a framework onto a date.
    for reply in [
        "Start: 2026-04-29T18:00 UTC",
        "Voici ce que disent les mesures de ta sortie du samedi 3 octobre (départ à 5 h 40).",
        "Une fois qu'il est accessible, je regarde ses 12 dernières semaines.",
        "Distance : 42,1 km",
        "Given your time constraint of 45 minutes, keep it easy.",
    ] {
        let found = rules(CITE, CoachingPersona::PowerAthlete, reply, None);
        assert!(
            found.is_empty(),
            "no model-derived metric in {reply:?}, got {found:?}"
        );
    }
}

#[test]
fn a_model_metric_named_in_words_needs_a_citation_in_any_locale() {
    let found = rules(
        CITE,
        CoachingPersona::PowerAthlete,
        "Ta monotonie est à 2,1 cette semaine.",
        None,
    );
    assert!(
        found.contains(&"require_framework_citation_per_numeric".to_owned()),
        "Foster's monotony in French is still a model claim, got {found:?}"
    );
}

#[test]
fn the_conjunction_if_is_not_the_intensity_factor() {
    assert!(rules(
        CITE,
        CoachingPersona::PowerAthlete,
        "Ride 40 minutes, and stop if your legs fade.",
        None
    )
    .is_empty());
}

// ------------------------------------------------------- required label:value block

const REQUIRE_BLOCK: &str = r"
version: 2
personas:
  power_athlete:
    require_line_by_line_block: true
";

#[test]
fn localized_and_markdown_blocks_satisfy_the_required_block() {
    for reply in [
        "Distance : 42,1 km\nDurée : 1 h 12 min\nFréquence cardiaque : 142 bpm",
        "- **Distance:** 42.1 km\n- **Duration:** 1h12",
        "1. Distance: 42.1 km\n2. Duration: 1h12",
        "- **« Marche le matin » :** elle date du 11 sept\n- **« Sortie longue » :** 32 km",
        "Activity: VirtualRide\nStart: 2026-04-29T18:00 UTC",
    ] {
        let found = rules(REQUIRE_BLOCK, CoachingPersona::PowerAthlete, reply, None);
        assert!(found.is_empty(), "{reply:?} is a block, got {found:?}");
    }
}

#[test]
fn prose_still_misses_the_required_block() {
    let found = rules(
        REQUIRE_BLOCK,
        CoachingPersona::PowerAthlete,
        "Ta sortie de samedi était régulière : 42 km en 3 h 30 à 142 bpm de moyenne.\nOn garde le plan.",
        None,
    );
    assert!(
        found.contains(&"require_line_by_line_block".to_owned()),
        "a report in prose, one sentence with a colon, still asks for a block, got {found:?}"
    );
}

#[test]
fn one_figure_in_passing_is_not_a_report() {
    let found = rules(
        REQUIRE_BLOCK,
        CoachingPersona::PowerAthlete,
        "Tu as couru 42 km samedi ; je regarde le reste dès que c'est synchronisé.",
        None,
    );
    assert!(
        found.is_empty(),
        "a single measured value is prose, got {found:?}"
    );
}

#[test]
fn a_reply_with_no_measured_data_needs_no_block() {
    // Production 2026-10-05 (carnet#795): nothing was reachable yet, so there
    // was nothing to put in a block; the date, the window and the ordinal are
    // not measurements.
    for reply in [
        "Une fois qu'il est accessible, je regarde ses 12 dernières semaines.",
        "Je regarde ta sortie du samedi 3 octobre dès qu'elle est synchronisée.",
        "Je regarde ta séance demain entre 7 h et 9 h.",
        "Ta CTL des 12 dernières semaines et ton ATL des 7 derniers jours arrivent.",
        "Je regarde ta 2e sortie de la semaine dès qu'elle est synchronisée.",
    ] {
        let found = rules(REQUIRE_BLOCK, CoachingPersona::PowerAthlete, reply, None);
        assert!(found.is_empty(), "{reply:?} reports no data, got {found:?}");
    }
}

#[test]
fn model_metrics_count_as_reported_data() {
    let found = rules(
        REQUIRE_BLOCK,
        CoachingPersona::PowerAthlete,
        "Ta CTL est à 62 et ta TSB à -8 ce matin.",
        None,
    );
    assert!(
        found.contains(&"require_line_by_line_block".to_owned()),
        "two model metrics in prose are a report, got {found:?}"
    );
}

#[test]
fn trimp_needs_a_banister_citation() {
    // JF directive on carnet#795: TRIMP maps to Banister.
    let found = rules(
        CITE,
        CoachingPersona::PowerAthlete,
        "Ton TRIMP de la semaine est 412.",
        None,
    );
    assert!(
        found.contains(&"require_framework_citation_per_numeric".to_owned()),
        "an uncited TRIMP must fire, got {found:?}"
    );
    assert!(rules(
        CITE,
        CoachingPersona::PowerAthlete,
        "Ton TRIMP de la semaine est 412 (Banister).",
        None
    )
    .is_empty());
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
fn tenant_isolation_fails_closed_without_a_roster() {
    // A citation the roster cannot vouch for is treated as foreign: the
    // 2026-09-01 audit flipped this from fail-open (skip the check) to
    // fail-closed, because a skipped check let an unlucky lookup ship a
    // cross-athlete leak unexamined. The violation routes to deterministic
    // redaction, never to the fact-preserving style rewrite.
    let reply = "Mallory · dead\nDistance: 42 km\nTime: 3h30";
    let found = rules(COACH, CoachingPersona::Coach, reply, None);
    assert!(
        found.contains(&"require_tenant_isolation".to_owned()),
        "an unresolved roster must flag unverifiable citations, got {found:?}"
    );
}

#[test]
fn tenant_isolation_needs_a_citation_to_fire_without_a_roster() {
    // Fail-closed applies to citations only — a reply with no athlete
    // citation has nothing to verify and must pass even roster-less.
    let reply = "Solid week overall.\nDistance: 42 km";
    let found = rules(COACH, CoachingPersona::Coach, reply, None);
    assert!(
        !found.contains(&"require_tenant_isolation".to_owned()),
        "no citation, nothing to verify, got {found:?}"
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
