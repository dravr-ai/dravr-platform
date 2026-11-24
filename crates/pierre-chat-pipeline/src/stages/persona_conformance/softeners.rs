// ABOUTME: The forbid_softeners rule — names every forbidden softener a reply carries, matched as a whole word or phrase
// ABOUTME: The detail is what the strict-mode style editor is told to fix, so it lists every hit, not the first
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! [`PersonaContract::forbid_softeners`] enforcement.

use pierre_contremaitre::persona_contracts::PersonaContract;

use super::repair::quoted_list;
use super::{contains_standalone_word, ContractViolation};

/// Enforce [`PersonaContract::forbid_softeners`]. Each entry is a
/// case-insensitive standalone word or phrase, so `around` never fires inside
/// `turnaround` nor `about` inside `roundabout`.
///
/// Every softener present is named, not the first one found: the detail is
/// also what the style editor is told to fix, and naming one let the rewrite
/// keep the others.
pub(super) fn check_softeners(
    reply: &str,
    contract: &PersonaContract,
    out: &mut Vec<ContractViolation>,
) {
    let lower = reply.to_lowercase();
    let found: Vec<&str> = contract
        .forbid_softeners
        .iter()
        .map(String::as_str)
        .filter(|s| contains_standalone_word(&lower, &s.to_lowercase()))
        .collect();
    if found.is_empty() {
        return;
    }
    let noun = if found.len() == 1 {
        "softener"
    } else {
        "softeners"
    };
    out.push(ContractViolation {
        rule: "forbid_softeners",
        detail: format!("{noun} {} detected", quoted_list(found.into_iter())),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn softener_violations(reply: &str) -> Vec<ContractViolation> {
        let contract = PersonaContract {
            forbid_softeners: ["I think", "a bit", "around", "roughly", "about", "un peu"]
                .map(str::to_owned)
                .to_vec(),
            ..PersonaContract::default()
        };
        let mut out = Vec::new();
        check_softeners(reply, &contract, &mut out);
        out
    }

    #[test]
    fn every_softener_present_is_named() {
        let found = softener_violations("Around 240 W, roughly. I think it held.");
        assert_eq!(found.len(), 1, "bug: one violation per rule: {found:?}");
        assert_eq!(
            found[0].detail, "softeners 'I think', 'around', 'roughly' detected",
            "bug: every softener in the reply must be named"
        );
    }

    #[test]
    fn one_softener_reads_singular() {
        let found = softener_violations("Threshold held roughly.");
        assert_eq!(found.len(), 1, "bug: 'roughly' must fire: {found:?}");
        assert_eq!(
            found[0].detail, "softener 'roughly' detected",
            "bug: a single softener must read singular"
        );
    }

    #[test]
    fn a_softener_inside_a_longer_word_is_not_one() {
        for reply in [
            "Turnaround: 48 h.",
            "Bitrate: 320.",
            "Roundabout route: 12 km.",
        ] {
            assert!(
                softener_violations(reply).is_empty(),
                "bug: {reply:?} carries no standalone softener"
            );
        }
    }

    #[test]
    fn a_multi_word_softener_matches_case_insensitively() {
        assert_eq!(
            softener_violations("Recovery: UN PEU short.").len(),
            1,
            "bug: an upper-case multi-word softener must fire"
        );
        assert_eq!(
            softener_violations("Load went up a bit.").len(),
            1,
            "bug: a multi-word softener must fire"
        );
    }
}
