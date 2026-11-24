// ABOUTME: The AI consents content derived from an athlete's data owes, and the providers kept out of AI memory
// ABOUTME: A derived row names the consent keys of what it was drawn from; Garmin data never feeds memory or summaries (carnet#1157, #1158)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a derived row owes the consents of the data it was drawn from.
//!
//! A reply, a memory fact or a compaction summary built from a provider's
//! data is that provider's data too: Garmin's agreement (v9 §15.11) deems
//! prompts and logs derived from Garmin data to be Garmin data. Such a row
//! names the AI-consent keys ([`consent_key`](super::consent_key)) of what it
//! was drawn from, so a withdrawal reaches it: once the athlete withdraws a
//! key's consent, no model reads a row that names it (carnet#1157).
//!
//! Some keys are kept out of the agent's memory altogether
//! ([`KEPT_OUT_OF_AI_MEMORY`]): no fact is extracted from a turn that served
//! their data, no compaction summary carries it, and no model reads a fact or
//! summary that names them, consent or not (carnet#1158).
//!
//! A row stores its keys as one comma-separated column; NULL and empty both
//! name none. Rows written before the column existed name none: what they
//! were drawn from cannot be told after the fact.

use std::collections::BTreeSet;

use super::GARMIN;

/// The AI-consent keys whose data must never feed the agent's memory (its
/// facts about the athlete) or a conversation's compaction summaries.
///
/// Garmin's agreement v9 §15.11(c) bars using Garmin data to build an
/// AI-generated knowledge base. Whether a per-athlete coaching memory is one
/// is the question put to Garmin in writing; until Garmin authorizes it in
/// writing, Garmin-derived content stays out of both (carnet#1158).
pub const KEPT_OUT_OF_AI_MEMORY: [&str; 1] = [GARMIN];

/// Whether `key`'s data is kept out of the agent's memory and summaries
/// ([`KEPT_OUT_OF_AI_MEMORY`]).
#[must_use]
pub fn kept_out_of_ai_memory(key: &str) -> bool {
    KEPT_OUT_OF_AI_MEMORY
        .iter()
        .any(|kept_out| kept_out.eq_ignore_ascii_case(key))
}

/// Whether any of `sources` is kept out of the agent's memory.
#[must_use]
pub fn names_kept_out_source(sources: &[String]) -> bool {
    sources.iter().any(|key| kept_out_of_ai_memory(key))
}

/// The column a row stores `sources` in: `None` when it names none.
#[must_use]
pub fn consent_sources_column(sources: &[String]) -> Option<String> {
    let keys = normalized(sources.iter().map(String::as_str));
    (!keys.is_empty()).then(|| keys.into_iter().collect::<Vec<_>>().join(","))
}

/// The keys a stored column names, sorted and without repeats; none for NULL
/// or an empty column.
#[must_use]
pub fn consent_sources_from_column(column: Option<&str>) -> Vec<String> {
    column.map_or_else(Vec::new, |column| {
        normalized(column.split(',')).into_iter().collect()
    })
}

/// Every key either of `a` and `b` names, sorted and without repeats: what a
/// row standing for both owes.
#[must_use]
pub fn union_consent_sources(a: &[String], b: &[String]) -> Vec<String> {
    normalized(a.iter().chain(b).map(String::as_str))
        .into_iter()
        .collect()
}

/// The non-empty keys of `keys`, trimmed and lowercased.
fn normalized<'a>(keys: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    keys.map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|key| (*key).to_owned()).collect()
    }

    #[test]
    fn a_column_round_trips_sorted_and_without_repeats() {
        let column = consent_sources_column(&keys(&["whoop", "garmin", "garmin"]));
        assert_eq!(column.as_deref(), Some("garmin,whoop"));
        assert_eq!(
            consent_sources_from_column(column.as_deref()),
            keys(&["garmin", "whoop"])
        );
    }

    #[test]
    fn none_is_stored_as_null_and_null_or_empty_reads_as_none() {
        assert_eq!(consent_sources_column(&[]), None);
        assert_eq!(consent_sources_column(&keys(&[" "])), None);
        for column in [None, Some(""), Some(" , ")] {
            assert_eq!(consent_sources_from_column(column), Vec::<String>::new());
        }
    }

    #[test]
    fn a_union_names_every_key_once() {
        assert_eq!(
            union_consent_sources(&keys(&["whoop"]), &keys(&["garmin", "whoop"])),
            keys(&["garmin", "whoop"])
        );
    }

    // carnet#1158: Garmin, and only Garmin, is kept out of memory until
    // Garmin authorizes it in writing.
    #[test]
    fn garmin_alone_is_kept_out_of_memory() {
        assert!(kept_out_of_ai_memory("garmin"));
        assert!(kept_out_of_ai_memory("Garmin"));
        assert!(!kept_out_of_ai_memory("whoop"));
        assert!(!kept_out_of_ai_memory("strava"));
        assert!(names_kept_out_source(&keys(&["whoop", "garmin"])));
        assert!(!names_kept_out_source(&keys(&["whoop"])));
        assert!(!names_kept_out_source(&[]));
    }
}
