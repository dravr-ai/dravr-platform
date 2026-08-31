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
use pierre_chat_pipeline::stages::capability_recovery::{
    parse_unsupported_verdict, peer_repair_prompt,
};
use pierre_chat_pipeline::stages::capability_subject::{
    resolve_ask_subject, subject_reask_instruction, AskSubject, SUBJECT_DECLINED_MARKER,
    SUBJECT_FETCHED_MARKER,
};
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

/// The repair prompt is chat-shaped: no tool-result framing (which routed
/// the CLI into silent task-completion, 2026-08-23), claims named, payload
/// present, and an explicit reply-with-text-only ask.
#[test]
fn repair_prompt_is_chat_shaped() {
    let prompt = peer_repair_prompt(
        "Philippe Tremblay",
        &[
            "une course de 4h30".to_owned(),
            "pas de distance".to_owned(),
        ],
        r#"{"activities":[{"id":"a1"}]}"#,
        "## Langue de la réponse\n\nRédige ta réponse en français.",
    );
    assert!(
        !prompt.contains("[Tool Result for"),
        "no tool-result framing"
    );
    assert!(prompt.contains("une course de 4h30; pas de distance"));
    assert!(prompt.contains(r#"{"activities":[{"id":"a1"}]}"#));
    assert!(prompt.contains("Reply with the message text only"));
    // A repair is the last message in the request — the strongest position in
    // the prompt — so it names the turn's language outright instead of asking
    // for "their language" and hoping (carnet#159).
    assert!(
        !prompt.contains("in their language"),
        "the repair prompt must not ask the model to infer the language"
    );
    assert!(
        prompt.contains("Rédige ta réponse en français"),
        "the caller's locale directive must ride at the end of the repair prompt"
    );
    assert!(
        prompt.contains("dravr-viz"),
        "the chart-preservation ask survives the reshape"
    );
}

// ════════════════════════════════════════════════════════════════════════
// Subject routing (live incident 2026-08-30: the Guardian fetched the
// REQUESTER's activities for a question about a peer, then told the model
// that data answered it)
// ════════════════════════════════════════════════════════════════════════

/// The message names the peer: the message is the authority, whatever the
/// reply says.
#[test]
fn a_message_naming_a_peer_makes_that_peer_the_subject() {
    let roster = vec![member("Jean-Daniel Tremblay"), member("Marc Dubois")];
    let requester = Uuid::new_v4();
    let subject = resolve_ask_subject(
        "Pour le plan — as-tu bien regardé l'historique Strava de Jean-Daniel ?",
        "Voici le plan de la semaine.",
        false,
        &roster,
        requester,
    );
    let AskSubject::Peers(peers) = subject else {
        panic!("a named roster member must be the subject");
    };
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].display_name, "Jean-Daniel Tremblay");
}

/// A pronoun ask whose REPLY denies access to a named peer: the denial is
/// the claim being adjudicated, so its subject is that peer — but only on a
/// capability-claim trigger.
#[test]
fn a_peer_denial_in_the_reply_names_the_subject_only_for_a_capability_claim() {
    let roster = vec![member("Jean-Daniel Tremblay")];
    let requester = Uuid::new_v4();
    let message = "As-tu bien regardé son historique Strava pour le plan ?";
    let denial = "Je n'ai jamais eu accès à l'historique de Jean-Daniel.";

    let claimed = resolve_ask_subject(message, denial, true, &roster, requester);
    assert_eq!(
        claimed,
        AskSubject::Peers(mentioned_peers(denial, &roster, requester)),
        "a capability claim about a named peer is adjudicated with that peer's data"
    );

    let ungrounded = resolve_ask_subject(message, denial, false, &roster, requester);
    assert_eq!(
        ungrounded,
        AskSubject::Requester,
        "an ungrounded or degenerate trigger never reads the subject off the reply"
    );
}

/// A reply that merely mentions a peer while answering the requester's own
/// question must not redirect the fetch — the requester asked about
/// themself.
#[test]
fn a_peer_mentioned_in_passing_does_not_hijack_a_self_ask() {
    let roster = vec![member("Philippe Tremblay")];
    let requester = Uuid::new_v4();
    let subject = resolve_ask_subject(
        "Propose-moi une sortie pour demain",
        "Comme Phil hier, pars sur 45 min tranquilles.",
        true,
        &roster,
        requester,
    );
    assert_eq!(subject, AskSubject::Requester);
}

/// Outside a group there is nobody to route to.
#[test]
fn an_empty_roster_always_resolves_to_the_requester() {
    let subject = resolve_ask_subject(
        "as-tu regardé l'historique de Jean-Daniel ?",
        "Je n'ai jamais eu accès à l'historique de Jean-Daniel.",
        true,
        &[],
        Uuid::new_v4(),
    );
    assert_eq!(subject, AskSubject::Requester);
}

/// The instruction never tells the model that data was fetched "on your
/// behalf": every sentence says whose data it is about.
#[test]
fn the_subject_instruction_attributes_every_side_and_relays_each_decline() {
    let fetched = vec!["Jean-Daniel Tremblay".to_owned()];
    let pregrounded = vec!["Marc Dubois".to_owned()];
    let declined = vec![(
        "Sophie Roy".to_owned(),
        "Sophie Roy hasn't shared their data with the group yet. They can opt in with \
         `/group consent yes`."
            .to_owned(),
    )];
    let text = subject_reask_instruction(true, &fetched, &pregrounded, &declined);

    assert!(
        !text.contains("on your behalf"),
        "never the requester-path wording: {text}"
    );
    assert!(text.contains(SUBJECT_FETCHED_MARKER));
    assert!(text.contains("Jean-Daniel Tremblay's activities above"));
    assert!(text.contains("Marc Dubois's activities were pre-loaded above"));
    assert!(text.contains("about Jean-Daniel Tremblay and Marc Dubois"));
    assert!(text.contains("athlete's OWN activities"));
    assert!(text.contains(SUBJECT_DECLINED_MARKER));
    assert!(text.contains("Sophie Roy's activities could NOT be read: Sophie Roy hasn't shared"));
    assert!(text.contains("never present anyone else's activities as Sophie Roy's"));
    assert!(
        text.contains("Nothing beyond what is listed above is unavailable"),
        "a relayed decline softens the closer so the honest answer is not contradicted: {text}"
    );
}

/// With every subject fetched and the requester's own data in hand, the
/// closer is the strict one.
#[test]
fn the_subject_instruction_is_strict_when_nothing_was_declined() {
    let text = subject_reask_instruction(true, &["Jean-Daniel Tremblay".to_owned()], &[], &[]);
    assert!(text.ends_with("Do not claim any connection or tool problem."));
    assert!(!text.contains(SUBJECT_DECLINED_MARKER));
}

/// The requester's own side failing non-auth is relayed as unavailable, not
/// hidden behind a "everything works" closer.
#[test]
fn the_subject_instruction_relays_the_requesters_own_outage() {
    let text = subject_reask_instruction(false, &["Jean-Daniel Tremblay".to_owned()], &[], &[]);
    assert!(text.contains("athlete's OWN activities could not be read this turn"));
    assert!(text.contains("do not state any of their numbers"));
    assert!(text.contains("Nothing beyond what is listed above is unavailable"));
}
