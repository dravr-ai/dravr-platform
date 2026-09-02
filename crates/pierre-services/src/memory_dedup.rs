// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Decides whether an extracted fact is new or a restatement of one the athlete already has
// ABOUTME: Exact key first, then cosine similarity over embeddings; a match merges into the anchor rather than appending a sibling

//! Fact de-duplication.
//!
//! An athlete states one goal; every later turn's extraction re-derives it in
//! its own words, and each rewording used to append a new row. Three rows for
//! one goal is not merely clutter — the drift is lossy, and the athlete is
//! asked to forget them one by one.
//!
//! Two layers decide where a fact goes, cheapest first:
//!
//! 1. **Exact key.** Same kind, same predicate code, same normalised object —
//!    a literal repeat never reaches the expensive path.
//! 2. **Similarity.** Otherwise the candidate's embedding is compared to the
//!    embeddings of the athlete's facts of that kind, and a score at or above
//!    the configured threshold is a restatement.
//!
//! What a match merges into is the **anchor**: the athlete's own phrasing wins
//! over a model's rewording. An onboarding-sourced row is always the anchor
//! (the athlete typed it), oldest first; otherwise the oldest row of the group.
//! The anchor keeps its object text, and a merge never lowers its confidence —
//! the reported case had a 60% rewording demote "ultra" to "trail outing", and
//! that must not be able to overwrite what the athlete said.

use pierre_memory::{FactKind, FactSource, PredicateCode, UserFact};

/// How the two layers are tuned. Served from the harness config document so
/// the threshold moves without a deploy.
#[derive(Debug, Clone, Copy)]
pub struct DedupConfig {
    /// Whether similarity matching runs at all. The exact-key layer is always
    /// on: it costs one string comparison and cannot produce a false merge.
    pub similarity_enabled: bool,
    /// Cosine score at or above which two facts of one kind are the same fact.
    pub similarity_threshold: f32,
    /// How many of the athlete's facts of that kind are compared.
    pub candidate_limit: usize,
}

impl DedupConfig {
    /// The candidate cap as the repository's row limit, saturating rather than
    /// wrapping on a mistuned value.
    #[must_use]
    pub fn candidate_limit_i64(&self) -> i32 {
        i32::try_from(self.candidate_limit).unwrap_or(i32::MAX)
    }
}

/// Where a candidate fact should be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactWrite {
    /// No existing fact says this; insert a new row.
    Insert,
    /// This restates the fact with the given id; merge into it.
    MergeInto(String),
}

/// The fact about to be written, before it has a row.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    /// Semantic kind — only facts of the same kind are ever compared.
    pub kind: FactKind,
    /// What the fact says, as a closed code.
    pub predicate_code: PredicateCode,
    /// The athlete's words for the value.
    pub object: &'a str,
    /// The candidate's embedding, when one could be computed.
    pub embedding: Option<&'a [f32]>,
}

/// Fold an object down to what makes two statements the same string.
///
/// Case, surrounding whitespace, repeated spaces and trailing punctuation only.
/// Deliberately not stemming or accent-folding: « côte » and "cote" are
/// different words to an athlete, and the similarity layer exists for
/// everything this cannot see.
#[must_use]
pub fn normalize_object(object: &str) -> String {
    let mut out = String::with_capacity(object.len());
    let mut last_was_space = true;
    for ch in object.trim().chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            last_was_space = false;
        }
    }
    while out.ends_with(['.', '!', '?', ',', ';', ':', ' ']) {
        out.pop();
    }
    out
}

/// Cosine similarity of two vectors, or `None` when either is empty, they are
/// different widths (two embedding providers), or either has no magnitude.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a <= 0.0 || norm_b <= 0.0 {
        return None;
    }
    Some(dot / (norm_a.sqrt() * norm_b.sqrt()))
}

/// The row a group of matching facts merges into.
///
/// An onboarding row wins because the athlete typed it; among equals the
/// oldest wins, so the anchor is stable as more restatements arrive.
fn anchor<'a>(matches: &[&'a UserFact]) -> Option<&'a UserFact> {
    matches.iter().copied().min_by(|a, b| {
        let a_key = (a.source != FactSource::Onboarding, a.created_at);
        let b_key = (b.source != FactSource::Onboarding, b.created_at);
        a_key.cmp(&b_key)
    })
}

/// Decide where `candidate` goes, given the athlete's existing facts.
///
/// `existing` should already be scoped to this athlete; facts of another kind
/// are ignored here rather than at the call site, so a caller that passes the
/// whole list still gets the right answer.
#[must_use]
pub fn decide(existing: &[UserFact], candidate: &Candidate<'_>, config: DedupConfig) -> FactWrite {
    let same_kind: Vec<&UserFact> = existing
        .iter()
        .filter(|fact| fact.kind == candidate.kind)
        .take(config.candidate_limit)
        .collect();
    if same_kind.is_empty() {
        return FactWrite::Insert;
    }

    // Layer 1: the same sentence, said again.
    let normalized = normalize_object(candidate.object);
    let exact: Vec<&UserFact> = same_kind
        .iter()
        .copied()
        .filter(|fact| {
            fact.predicate_code == candidate.predicate_code
                && normalize_object(&fact.object) == normalized
        })
        .collect();
    if let Some(row) = anchor(&exact) {
        return FactWrite::MergeInto(row.id.clone());
    }

    // Layer 2: the same fact, said differently.
    if !config.similarity_enabled {
        return FactWrite::Insert;
    }
    let Some(vector) = candidate.embedding else {
        return FactWrite::Insert;
    };
    let similar: Vec<&UserFact> = same_kind
        .iter()
        .copied()
        .filter(|fact| {
            fact.embedding.as_ref().is_some_and(|stored| {
                cosine_similarity(vector, stored)
                    .is_some_and(|score| score >= config.similarity_threshold)
            })
        })
        .collect();
    anchor(&similar).map_or(FactWrite::Insert, |row| {
        FactWrite::MergeInto(row.id.clone())
    })
}
