// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Decides whether an extracted fact is new or a restatement of one the athlete already has
// ABOUTME: The same sentence again is caught here; a paraphrase is the extractor's call, and both merge into the anchor

//! Fact de-duplication.
//!
//! An athlete states one goal; every later turn's extraction re-derives it in
//! its own words, and each rewording used to append a new row. Three rows for
//! one goal is not merely clutter — the drift is lossy, and the athlete is
//! asked to forget them one by one.
//!
//! Two layers decide where a fact goes, and this module is the certain one:
//! same kind, same predicate code, same normalised object is the same
//! sentence said again, which costs one string comparison and cannot merge
//! two different facts.
//!
//! A paraphrase is not decided here. It is decided by the extractor, which
//! is shown the athlete's existing facts and answers which one a new fact
//! restates — see `memory_extraction`. That replaced a cosine threshold over
//! embeddings, which could not work at any value: measured on two vendors'
//! models, "a marathon in October" and "a half marathon in October" score
//! HIGHER against each other than one goal restated in another language does
//! against itself. Embeddings measure topical closeness, and two race goals
//! a month apart are as close as topics get. A reader distinguishes them; a
//! distance cannot.
//!
//! What a match merges into is the **anchor**: the athlete's own phrasing wins
//! over a model's rewording. An onboarding-sourced row is always the anchor
//! (the athlete typed it), oldest first; otherwise the oldest row of the group.
//! The anchor keeps its object text, and a merge never lowers its confidence —
//! the reported case had a 60% rewording demote "ultra" to "trail outing", and
//! that must not be able to overwrite what the athlete said.

use pierre_memory::{FactKind, FactSource, PredicateCode, UserFact};

/// How much of the athlete's memory a write is decided against. Served from
/// the harness config document so the cap moves without a deploy.
#[derive(Debug, Clone, Copy)]
pub struct DedupConfig {
    /// How many of the athlete's facts are compared, and how many the
    /// extractor is shown. One cap, so the model never answers about a fact
    /// this module would not have considered.
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
}

/// Fold an object down to what makes two statements the same string.
///
/// Case, surrounding whitespace, repeated spaces and trailing punctuation only.
/// Deliberately not stemming or accent-folding: « côte » and "cote" are
/// different words to an athlete, and the extractor's paraphrase answer
/// covers everything this cannot see.
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

/// Decide where `candidate` goes on the certain layer alone.
///
/// [`FactWrite::Insert`] means only "no existing fact says this in the same
/// words" — the caller still has the extractor's paraphrase answer to
/// consider before it writes a new row.
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
    anchor(&exact).map_or(FactWrite::Insert, |row| {
        FactWrite::MergeInto(row.id.clone())
    })
}

/// Whether `candidate` introduces a number the fact it claims to restate does
/// not have.
///
/// A restatement may drop detail — "le même ultra au Mont Albert" restates
/// "un ultra de 26 km au Mont Albert" — but it cannot introduce a quantity
/// the original never carried. "50 km" against a "26 km" anchor is a changed
/// race, and merging it would keep the old goal and discard the new one.
///
/// This is a structural guard, not a second opinion. The prompt already tells
/// the extractor that a different distance is a new fact, and measured against
/// two production providers one of them named the anchor anyway. The same file
/// learned this once before: prompt-only enforcement of provenance failed and
/// was replaced by a field the code checks (see `is_coach_prescription`).
///
/// Numbers are compared as whole tokens, so "3:30" is one quantity and not a
/// three and a thirty — otherwise "sub-3:30" would read as a restatement of
/// "sub-3".
#[must_use]
pub fn introduces_a_number(anchor_object: &str, candidate_object: &str) -> bool {
    let known = number_tokens(anchor_object);
    number_tokens(candidate_object)
        .into_iter()
        .any(|token| !known.contains(&token))
}

/// Every number-like token in `text`, lowercased: digits with the separators
/// that keep a quantity whole (`:` for a time, `.` and `,` for a decimal,
/// `h` for an hour).
fn number_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        let joins = ch.is_ascii_digit()
            || (!current.is_empty() && matches!(ch, ':' | '.' | ',' | 'h' | 'H'));
        if joins {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            out.push(current.trim_end_matches([':', '.', ',', 'h']).to_owned());
            current = String::new();
        }
    }
    if !current.is_empty() {
        out.push(current.trim_end_matches([':', '.', ',', 'h']).to_owned());
    }
    out.retain(|t| !t.is_empty());
    out
}

/// The anchor among `candidates`, exposed so the extractor's paraphrase answer
/// merges into the same row this module would have chosen.
///
/// A model naming any member of a group must not produce a different winner
/// than a literal repeat naming the group.
#[must_use]
pub fn anchor_of<'a>(candidates: &[&'a UserFact]) -> Option<&'a UserFact> {
    anchor(candidates)
}
