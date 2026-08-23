// ABOUTME: The fabrication gate's deterministic pieces — peer-name matching and verdict parsing
// ABOUTME: "Phil", "Phile" and "Philippe" must reach the same member; garbage verdicts fail open

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Deterministic halves of the peer fabrication gate (live incident
//! 2026-08-22: the coach invented «4h30» and «pas de distance» about a peer
//! whose true record — 53 min, 6.1 km — sat in its own context).
//!
//! `mentioned_peers` decides which turns get platform-side peer grounding
//! and which replies face the claim verifier; `parse_unsupported_verdict`
//! turns the verifier model's reply into the unsupported-claim list. Both
//! must behave exactly, because a false name-match grounds the wrong
//! person's data and a mis-parsed verdict either blocks a legitimate reply
//! or waves a fabricated one through.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashMap;

use chrono::Utc;
use pierre_chat_pipeline::stages::capability_recovery::parse_unsupported_verdict;
use pierre_chat_pipeline::stages::peer_grounding::mentioned_peers;
use pierre_core::models::groups::OvertrainingRiskLevel;
use pierre_core::models::MemberFitnessSnapshot;
use uuid::Uuid;

fn member(name: &str) -> MemberFitnessSnapshot {
    MemberFitnessSnapshot {
        user_id: Uuid::new_v4(),
        display_name: name.to_owned(),
        ctl: None,
        atl: None,
        tsb: None,
        weekly_volume_km: 0.0,
        previous_week_volume_km: None,
        weekly_activity_count: 0,
        weekly_duration_seconds: 0,
        primary_sport: None,
        vdot: None,
        overtraining_risk: OvertrainingRiskLevel::Low,
        days_since_last_activity: None,
        last_activity_per_provider: HashMap::new(),
        recent_activities: Vec::new(),
        needs_reauth_providers: Vec::new(),
        served_stale: false,
        computed_at: Utc::now(),
    }
}

/// The live incident's three spellings — the short form, the typo, and the
/// full name — must all reach the same roster member.
#[test]
fn phil_phile_and_philippe_all_reach_philippe() {
    let roster = vec![member("Philippe Tremblay")];
    let requester = Uuid::new_v4();

    for text in [
        "Peux-tu comparer mes heures avec Phil?",
        "le tsb de Phile et de moi pour comparaison",
        "regarde la sortie de Philippe",
    ] {
        let hits = mentioned_peers(text, &roster, requester);
        assert_eq!(
            hits.len(),
            1,
            "{text:?} must match the one roster member, got {hits:?}"
        );
        assert_eq!(hits[0].display_name, "Philippe Tremblay");
    }
}

/// The requester never matches themself — a self-reference is `get_activities`
/// territory, not peer grounding.
#[test]
fn the_requester_is_never_a_peer_mention() {
    let me = member("Jean Francois Arcand");
    let me_id = me.user_id;
    let roster = vec![me, member("Philippe Tremblay")];

    let hits = mentioned_peers("compare Jean Francois et Philippe", &roster, me_id);
    assert_eq!(hits.len(), 1, "only the peer may match, got {hits:?}");
    assert_eq!(hits[0].display_name, "Philippe Tremblay");
}

/// Short tokens and unrelated names never match — a missed grounding is
/// recoverable (the model can still fetch), a wrong one is not.
#[test]
fn short_or_unrelated_tokens_match_nobody() {
    let roster = vec![member("Philippe Tremblay"), member("Raphael Couturier")];
    let requester = Uuid::new_v4();

    assert!(mentioned_peers("on y va?", &roster, requester).is_empty());
    assert!(mentioned_peers("mon vélo est prêt", &roster, requester).is_empty());
    // "Ph" is below both the exact and prefix thresholds.
    assert!(mentioned_peers("Ph a roulé", &roster, requester).is_empty());
}

/// A clean verdict, a verdict wrapped in prose, and an empty verdict all
/// parse; the wrapped case is the live shape (models narrate around JSON).
#[test]
fn verdicts_parse_with_and_without_surrounding_prose() {
    assert_eq!(
        parse_unsupported_verdict(r#"{"unsupported": ["une course de 4h30"]}"#),
        vec!["une course de 4h30".to_owned()]
    );
    assert_eq!(
        parse_unsupported_verdict(
            "Here is my analysis:\n{\"unsupported\": [\"4h30\", \"pas de distance\"]}\nDone."
        ),
        vec!["4h30".to_owned(), "pas de distance".to_owned()]
    );
    assert!(parse_unsupported_verdict(r#"{"unsupported": []}"#).is_empty());
}

/// Garbage fails OPEN — a flaky verifier must never cost the athlete a
/// legitimate reply, so an unparseable verdict reads as "supported".
#[test]
fn unparseable_verdicts_fail_open() {
    assert!(parse_unsupported_verdict("").is_empty());
    assert!(parse_unsupported_verdict("I could not check that.").is_empty());
    assert!(parse_unsupported_verdict("{not json at all]").is_empty());
    assert!(parse_unsupported_verdict(r#"{"wrong_key": ["x"]}"#).is_empty());
}
