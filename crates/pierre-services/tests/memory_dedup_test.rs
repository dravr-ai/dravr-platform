// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins fact de-duplication on the case that filed it — one goal stored three times
// ABOUTME: An exact repeat merges, a different goal does not, and the athlete's own words stay the anchor

//! Tests for de-duplicating an extracted fact against the athlete's existing ones.

use chrono::{Duration, Utc};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode, UserFact};
use pierre_services::memory_dedup::{
    anchor_of, decide, introduces_a_number, normalize_object, Candidate, DedupConfig, FactWrite,
};

fn config() -> DedupConfig {
    DedupConfig {
        candidate_limit: 50,
    }
}

/// A stored fact, built through a small builder so the fixtures read as
/// sentences rather than as eight positional arguments.
struct Stored {
    id: &'static str,
    kind: FactKind,
    predicate_code: PredicateCode,
    object: &'static str,
    confidence: f32,
    source: FactSource,
    /// Larger is older, so the anchor tie-break is explicit in each fixture.
    age_minutes: i64,
}

impl Stored {
    fn goal(id: &'static str, object: &'static str) -> Self {
        Self {
            id,
            kind: FactKind::Goal,
            predicate_code: PredicateCode::WorkingToward,
            object,
            confidence: 1.0,
            source: FactSource::Conversation,
            age_minutes: 10,
        }
    }

    fn kind(mut self, kind: FactKind, predicate_code: PredicateCode) -> Self {
        self.kind = kind;
        self.predicate_code = predicate_code;
        self
    }

    fn onboarded(mut self) -> Self {
        self.source = FactSource::Onboarding;
        self
    }

    fn confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence;
        self
    }

    fn aged(mut self, age_minutes: i64) -> Self {
        self.age_minutes = age_minutes;
        self
    }

    fn build(self) -> UserFact {
        let created = Utc::now() - Duration::minutes(self.age_minutes);
        UserFact {
            id: self.id.into(),
            tenant_id: "t1".into(),
            user_id: "u1".into(),
            coach_id: None,
            scope: MemoryScope::User,
            kind: self.kind,
            pillar: None,
            predicate_code: self.predicate_code,
            object: self.object.into(),
            confidence: self.confidence,
            source: self.source,
            valid_until: None,
            source_msg_id: None,
            created_at: created,
            updated_at: created,
        }
    }
}

#[test]
fn an_athlete_with_no_facts_gets_a_new_row() {
    assert_eq!(
        decide(
            &[],
            &Candidate {
                kind: FactKind::Goal,
                predicate_code: PredicateCode::WorkingToward,
                object: "Un ultra de 26 km au Mont Albert",
            },
            config()
        ),
        FactWrite::Insert
    );
}

#[test]
fn the_same_sentence_again_merges_into_the_athletes_own_words() {
    // The cheap layer: no model call, and it cannot produce a false merge.
    let existing = vec![
        Stored::goal("anchor", "Un ultra de 26 km au Mont Albert en Gaspésie")
            .onboarded()
            .aged(30)
            .build(),
    ];
    let write = decide(
        &existing,
        &Candidate {
            kind: FactKind::Goal,
            predicate_code: PredicateCode::WorkingToward,
            // Same words, different case and trailing punctuation.
            object: "  un ultra de 26 KM au Mont Albert en Gaspésie.  ",
        },
        config(),
    );
    assert_eq!(write, FactWrite::MergeInto("anchor".into()));
}

#[test]
fn without_an_onboarding_row_the_oldest_is_the_anchor() {
    // Same sentence stored twice, neither typed at onboarding: the older row
    // is the one later restatements fold into, so the anchor does not drift
    // as more arrive.
    let older = Stored::goal("older", "morning sessions")
        .kind(FactKind::Preference, PredicateCode::Prefer)
        .confidence(0.6)
        .aged(60)
        .build();
    let newer = Stored::goal("newer", "Morning sessions.")
        .kind(FactKind::Preference, PredicateCode::Prefer)
        .confidence(0.9)
        .aged(10)
        .build();
    let write = decide(
        &[newer, older],
        &Candidate {
            kind: FactKind::Preference,
            predicate_code: PredicateCode::Prefer,
            object: "  MORNING SESSIONS  ",
        },
        config(),
    );
    assert_eq!(write, FactWrite::MergeInto("older".into()));
}

#[test]
fn a_different_goal_is_not_merged() {
    // The failure that matters most: two real goals collapsed into one.
    let existing = vec![Stored::goal("marathon", "sub-3 marathon")
        .onboarded()
        .aged(30)
        .build()];
    let write = decide(
        &existing,
        &Candidate {
            kind: FactKind::Goal,
            predicate_code: PredicateCode::WorkingToward,
            object: "swim 2 km without stopping",
        },
        config(),
    );
    assert_eq!(write, FactWrite::Insert);
}

#[test]
fn facts_of_another_kind_are_never_compared() {
    let existing = vec![Stored::goal("injury", "a 26 km ultra at Mont Albert")
        .kind(FactKind::Injury, PredicateCode::RecoveringFrom)
        .onboarded()
        .aged(30)
        .build()];
    let write = decide(
        &existing,
        &Candidate {
            kind: FactKind::Goal,
            predicate_code: PredicateCode::WorkingToward,
            object: "a 26 km ultra at Mont Albert",
        },
        config(),
    );
    assert_eq!(write, FactWrite::Insert, "an injury is not a goal");
}

#[test]
fn normalization_folds_only_what_it_should() {
    assert_eq!(normalize_object("  Sub-3   Marathon.  "), "sub-3 marathon");
    assert_eq!(normalize_object("Un ULTRA de 26 km!"), "un ultra de 26 km");
    // Accents are meaning, not noise: these stay different strings.
    assert_ne!(normalize_object("côte"), normalize_object("cote"));
}

/// The anchor rule is shared with the extractor's paraphrase answer, so a
/// model naming any member of a group cannot pick a row that a literal repeat
/// naming the same group would not.
#[test]
fn the_anchor_is_the_athletes_own_words_whichever_member_is_named() {
    let onboarding = Stored::goal("f-onboarding", "Un ultra de 26 km au Mont Albert")
        .onboarded()
        .aged(500)
        .build();
    let restated = Stored::goal("f-restated", "un ultra de 26 km au mont albert")
        .aged(100)
        .build();
    let group: Vec<&UserFact> = vec![&restated, &onboarding];

    assert_eq!(
        anchor_of(&group).map(|f| f.id.as_str()),
        Some("f-onboarding"),
        "the row the athlete typed wins, whatever order the group arrives in"
    );
    assert_eq!(
        anchor_of(&[]).map(|f| f.id.as_str()),
        None,
        "an empty group names no anchor"
    );
}

/// The case a cosine threshold could not get right, and the reason this layer
/// is the certain one: two goals a month apart embed closer to each other than
/// one goal restated in another language embeds to itself. A comparison of
/// words keeps them apart because the words differ.
#[test]
fn two_goals_on_one_template_are_never_merged_here() {
    let stored = Stored::goal("f-marathon", "a marathon in October").build();

    assert_eq!(
        decide(
            &[stored],
            &Candidate {
                kind: FactKind::Goal,
                predicate_code: PredicateCode::WorkingToward,
                object: "a half marathon in October",
            },
            config(),
        ),
        FactWrite::Insert,
        "a different distance is a different goal, however close the wording"
    );
}

/// The guard behind the extractor's paraphrase answer.
///
/// Found by putting the real prompt to two production providers: asked to
/// extract "finalement je passe au 50 km au Mont Albert" against a 26 km
/// anchor, one named the anchor as a restatement. Merging that keeps the old
/// race and discards the new one, which is the failure the whole feature
/// exists to avoid — so the code refuses it rather than trusting the prompt.
#[test]
fn a_changed_quantity_is_never_a_restatement() {
    let anchor = "Un ultra de 26 km au Mont Albert en Gaspésie";

    assert!(
        introduces_a_number(anchor, "finalement je passe au 50 km au Mont Albert"),
        "a different distance is a different race"
    );
    assert!(
        introduces_a_number("sub-3 marathon", "sub-3:30 marathon"),
        "a time is one quantity: 3:30 must not read as a restatement of 3"
    );
}

/// The guard must not eat the restatements it exists alongside — including one
/// that drops the number entirely, which is how athletes actually repeat
/// themselves.
#[test]
fn a_restatement_may_repeat_or_drop_a_number_but_not_add_one() {
    let anchor = "Un ultra de 26 km au Mont Albert en Gaspésie";

    assert!(
        !introduces_a_number(anchor, "le 26 km au Mont Albert"),
        "the same number again is the same fact"
    );
    assert!(
        !introduces_a_number(anchor, "that 26 km ultra at Mont Albert"),
        "the cross-language restatement both providers got right"
    );
    assert!(
        !introduces_a_number(anchor, "le même ultra au Mont Albert"),
        "dropping the detail is still the same goal"
    );
    assert!(
        !introduces_a_number("morning sessions", "runs before work"),
        "no numbers on either side is not a disagreement"
    );
}
