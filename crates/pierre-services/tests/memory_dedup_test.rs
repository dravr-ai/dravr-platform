// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins fact de-duplication on the case that filed it — one goal stored three times
// ABOUTME: Exact repeats merge without embeddings, paraphrases merge on similarity, and the athlete's own words stay the anchor

//! Tests for de-duplicating an extracted fact against the athlete's existing ones.

use chrono::{Duration, Utc};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode, UserFact};
use pierre_services::memory_dedup::{
    cosine_similarity, decide, normalize_object, Candidate, DedupConfig, FactWrite,
};

fn config() -> DedupConfig {
    DedupConfig {
        similarity_enabled: true,
        similarity_threshold: 0.86,
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
    embedding: Option<Vec<f32>>,
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
            embedding: None,
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

    fn embedded(mut self, embedding: &[f32]) -> Self {
        self.embedding = Some(embedding.to_vec());
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
            embedding: self.embedding,
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
                embedding: Some(&[1.0, 0.0]),
            },
            config()
        ),
        FactWrite::Insert
    );
}

#[test]
fn the_same_sentence_again_merges_without_any_embedding() {
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
            embedding: None,
        },
        config(),
    );
    assert_eq!(write, FactWrite::MergeInto("anchor".into()));
}

#[test]
fn a_paraphrase_merges_into_the_athletes_own_words() {
    // The case that filed carnet#194: the onboarding answer at 100%, then two
    // model rewordings. All three must land on the onboarding row.
    let onboarding = Stored::goal("onboarding", "Un ultra de 26 km au Mont Albert en Gaspésie")
        .onboarded()
        .aged(30)
        .embedded(&[1.0, 0.0, 0.0])
        .build();
    let first_rewording = Stored::goal("rewording", "a 26 km ultra at Mont Albert in Gaspésie")
        .kind(FactKind::Goal, PredicateCode::TrainingFor)
        .confidence(0.7)
        .aged(20)
        .embedded(&[0.99, 0.14, 0.0])
        .build();
    let existing = vec![first_rewording, onboarding];

    // The third arrival — "a 26 km trail outing at Mont Albert", the lossy one.
    let write = decide(
        &existing,
        &Candidate {
            kind: FactKind::Goal,
            predicate_code: PredicateCode::TrainingFor,
            object: "a 26 km trail outing at Mont Albert",
            embedding: Some(&[0.98, 0.2, 0.0]),
        },
        config(),
    );
    assert_eq!(
        write,
        FactWrite::MergeInto("onboarding".into()),
        "an onboarding row is the anchor even when a closer rewording exists"
    );
}

#[test]
fn without_an_onboarding_row_the_oldest_is_the_anchor() {
    let older = Stored::goal("older", "morning sessions")
        .kind(FactKind::Preference, PredicateCode::Prefer)
        .confidence(0.6)
        .aged(60)
        .embedded(&[1.0, 0.0])
        .build();
    let newer = Stored::goal("newer", "training early in the day")
        .kind(FactKind::Preference, PredicateCode::Prefer)
        .confidence(0.9)
        .aged(10)
        .embedded(&[0.99, 0.1])
        .build();
    let write = decide(
        &[newer, older],
        &Candidate {
            kind: FactKind::Preference,
            predicate_code: PredicateCode::Prefer,
            object: "runs before work",
            embedding: Some(&[0.995, 0.05]),
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
        .embedded(&[1.0, 0.0, 0.0])
        .build()];
    let write = decide(
        &existing,
        &Candidate {
            kind: FactKind::Goal,
            predicate_code: PredicateCode::WorkingToward,
            object: "swim 2 km without stopping",
            embedding: Some(&[0.1, 0.99, 0.0]),
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
        .embedded(&[1.0, 0.0, 0.0])
        .build()];
    let write = decide(
        &existing,
        &Candidate {
            kind: FactKind::Goal,
            predicate_code: PredicateCode::WorkingToward,
            object: "a 26 km ultra at Mont Albert",
            embedding: Some(&[1.0, 0.0, 0.0]),
        },
        config(),
    );
    assert_eq!(write, FactWrite::Insert, "an injury is not a goal");
}

#[test]
fn similarity_off_keeps_only_the_exact_layer() {
    let cfg = DedupConfig {
        similarity_enabled: false,
        ..config()
    };
    let existing = vec![Stored::goal("anchor", "a 26 km ultra at Mont Albert")
        .onboarded()
        .aged(30)
        .embedded(&[1.0, 0.0])
        .build()];

    // A paraphrase now inserts...
    assert_eq!(
        decide(
            &existing,
            &Candidate {
                kind: FactKind::Goal,
                predicate_code: PredicateCode::WorkingToward,
                object: "a 26 km trail race at Mont Albert",
                embedding: Some(&[0.999, 0.01]),
            },
            cfg
        ),
        FactWrite::Insert
    );
    // ...but the same sentence still merges, because that layer never turns off.
    assert_eq!(
        decide(
            &existing,
            &Candidate {
                kind: FactKind::Goal,
                predicate_code: PredicateCode::WorkingToward,
                object: "A 26 km ultra at Mont Albert",
                embedding: None,
            },
            cfg
        ),
        FactWrite::MergeInto("anchor".into())
    );
}

#[test]
fn a_fact_with_no_embedding_is_never_merged_by_similarity() {
    // A row stored before embeddings existed must not match everything.
    let existing = vec![Stored::goal("legacy", "a 26 km ultra at Mont Albert")
        .onboarded()
        .aged(30)
        .build()];
    assert_eq!(
        decide(
            &existing,
            &Candidate {
                kind: FactKind::Goal,
                predicate_code: PredicateCode::WorkingToward,
                object: "a 26 km trail race at Mont Albert",
                embedding: Some(&[1.0, 0.0]),
            },
            config()
        ),
        FactWrite::Insert
    );
}

#[test]
fn normalization_folds_only_what_it_should() {
    assert_eq!(normalize_object("  Sub-3   Marathon.  "), "sub-3 marathon");
    assert_eq!(normalize_object("Un ULTRA de 26 km!"), "un ultra de 26 km");
    // Accents are meaning, not noise: these stay different strings.
    assert_ne!(normalize_object("côte"), normalize_object("cote"));
}

#[test]
fn cosine_is_undefined_rather_than_wrong() {
    assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
    assert_eq!(cosine_similarity(&[], &[]), None, "no vector, no score");
    assert_eq!(
        cosine_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.0]),
        None,
        "two providers' widths never compare"
    );
    assert_eq!(
        cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]),
        None,
        "a zero vector has no direction"
    );
}
