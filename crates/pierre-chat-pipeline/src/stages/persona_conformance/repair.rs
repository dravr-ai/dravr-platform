// ABOUTME: The repair line the strict-mode style editor is handed for each persona-contract violation
// ABOUTME: A diagnosis alone is not enough — rules whose fix the editor cannot guess carry what satisfies them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Repair instructions for [`super::enforce_conformance`]'s style rewrite.

use pierre_contremaitre::persona_contracts::PersonaContract;

use super::{ContractViolation, FRAMEWORK_BOUND_ACRONYMS, VAGUE_MODIFIERS};

/// The line the style editor is asked to satisfy for one violation.
///
/// A detail is a diagnosis naming the one hedge the check stopped on. Handed
/// over alone, the editor read "'~' is wrong" and swapped `~` for `environ`,
/// shipping the same violation (2026-11-02 `power_athlete` alert). Exact
/// numbers therefore carry the whole hedge set, in every locale.
///
/// A missing framework citation is the same failure: the diagnosis quotes the
/// sentence but not what counts as a citation, so the editor glossed "FTP" as
/// "puissance au seuil" and shipped the same violation (2026-11-19
/// `power_athlete` alert). That repair carries the contract's allowlist, the
/// metrics that oblige one, and that the name goes in the number's own
/// sentence — the check reads one sentence at a time.
pub(super) fn repair_instruction(v: &ContractViolation, contract: &PersonaContract) -> String {
    match v.rule {
        "require_exact_numbers" => format!(
            "{}; state every number exactly, with none of these hedges next to it: {}",
            v.detail,
            VAGUE_MODIFIERS.join(", ")
        ),
        "require_framework_citation_per_numeric" => format!(
            "{}; every sentence that gives a number for {}, or for training monotony, strain or \
             polarization, must name, in that same sentence, the framework the number comes \
             from — one of: {}",
            v.detail,
            FRAMEWORK_BOUND_ACRONYMS.join(", "),
            contract.framework_allowlist.join(", ")
        ),
        _ => v.detail.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        check_framework_citation_per_numeric, contains_standalone_word, FRAMEWORK_BOUND_STEMS,
    };
    use super::*;

    #[test]
    fn exact_number_repair_names_every_hedge() {
        let detail = "vague modifier '~' sits next to a numeric value".to_owned();
        let line = repair_instruction(
            &ContractViolation {
                rule: "require_exact_numbers",
                detail: detail.clone(),
            },
            &PersonaContract::default(),
        );
        assert!(
            line.starts_with(&detail),
            "bug: the diagnosis must lead the repair line: {line}"
        );
        for hedge in VAGUE_MODIFIERS {
            assert!(
                line.contains(hedge),
                "bug: hedge {hedge} missing from the repair line: {line}"
            );
        }
    }

    #[test]
    fn other_repairs_pass_the_detail_through() {
        let detail = "reply exceeds the persona word budget".to_owned();
        let line = repair_instruction(
            &ContractViolation {
                rule: "max_words",
                detail: detail.clone(),
            },
            &PersonaContract::default(),
        );
        assert_eq!(
            line, detail,
            "bug: a rule without a repair override must hand the editor its detail unchanged"
        );
    }

    fn citing_contract() -> PersonaContract {
        PersonaContract {
            require_framework_citation_per_numeric: true,
            framework_allowlist: vec!["Banister".to_owned(), "Coggan".to_owned()],
            ..PersonaContract::default()
        }
    }

    /// The 2026-11-19 alert: the editor was told a citation was missing but
    /// not which names count, and glossed "FTP" instead of naming Coggan.
    #[test]
    fn citation_repair_names_the_allowlist_and_the_metrics() {
        let contract = citing_contract();
        let detail = "numeric claim without an allowlisted framework citation: '…'".to_owned();
        let line = repair_instruction(
            &ContractViolation {
                rule: "require_framework_citation_per_numeric",
                detail: detail.clone(),
            },
            &contract,
        );
        assert!(
            line.starts_with(&detail),
            "bug: the diagnosis must lead the repair line: {line}"
        );
        for framework in &contract.framework_allowlist {
            assert!(
                line.contains(framework.as_str()),
                "bug: framework {framework} missing from the repair line: {line}"
            );
        }
        for metric in FRAMEWORK_BOUND_ACRONYMS {
            assert!(
                contains_standalone_word(&line, metric),
                "bug: metric {metric} missing from the repair line: {line}"
            );
        }
        let words: Vec<String> = line
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .map(str::to_owned)
            .collect();
        for stem in FRAMEWORK_BOUND_STEMS {
            assert!(
                words.iter().any(|word| word.starts_with(stem)),
                "bug: worded metric '{stem}' missing from the repair line: {line}"
            );
        }
    }

    /// What the repair line asks for is what the check accepts: the alert's
    /// sentence fails, and naming Coggan in that sentence clears it.
    #[test]
    fn naming_the_framework_in_the_sentence_satisfies_the_check() {
        let contract = citing_contract();
        let bare = "Elle est de 2 × 15 min à 200–210 W, soit 88–93 % de ta FTP (puissance au seuil fonctionnelle).";
        let cited = "Elle est de 2 × 15 min à 200–210 W, soit 88–93 % de ta FTP (zones de Coggan).";

        let mut out = Vec::new();
        check_framework_citation_per_numeric(bare, &contract, &mut out);
        assert_eq!(out.len(), 1, "bug: the alert's sentence must still fire");

        out.clear();
        check_framework_citation_per_numeric(cited, &contract, &mut out);
        assert!(
            out.is_empty(),
            "bug: a sentence naming its framework must pass: {out:?}"
        );
    }
}
