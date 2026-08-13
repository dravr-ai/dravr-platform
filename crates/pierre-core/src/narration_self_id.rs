// ABOUTME: Vocabulary for detecting first-person self-identification as a developer tool
// ABOUTME: Data table kept out of narration.rs, which is already over the line budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Self-identification vocabulary for the identity-leak detector.
//!
//! The main pattern table in [`super::narration`] is an allowlist of phrasings
//! already seen in the wild, so it has high precision and almost no recall.
//! These two lists let the detector match on *structure* instead — a
//! first-person copula binding to a developer-tool role — which closes the
//! synonym escape without widening the product allowlist.

use super::narration::{
    fold_into, is_negated_at, IdentityLeakMatch, IdentityPatternClass, FOLDED_IDENTITY,
    IDENTITY_NARRATION_PATTERNS,
};

/// Maximum folded characters between a first-person copula and the role noun
/// for the two to count as one self-identification. Wide enough for an article
/// and a short adjective («i m the coding cli»), tight enough that a copula in
/// one clause cannot bind to a noun in the next.
const SELF_ID_PROXIMITY: usize = 24;

/// First-person copulas, folded, in the five shipped locales. Trailing space is
/// deliberate: it anchors the word boundary without a regex engine.
const SELF_ID_CLAIMS: &[&str] = &[
    "i m ", "i am ", "je suis ", "soy ", "yo soy ", "ich bin ", "eu sou ", "sou ",
];

/// Role nouns that identify the speaker as a developer tool rather than a
/// coach. Kept to phrases a fitness coach has no reason to apply to *itself* —
/// an athlete's own job title is never matched, because a match requires the
/// assistant's own first-person copula immediately before it.
const SELF_ID_ROLES: &[&str] = &[
    "coding cli",
    "coding agent",
    "coding tool",
    "cli assistant",
    "cli agent",
    "cli tool",
    "command line assistant",
    "command line tool",
    "terminal assistant",
    "terminal agent",
    "code assistant",
    "code agent",
    "developer tool",
    "developer assistant",
    "software agent",
    "agent de codage",
    "outil de codage",
    "assistant terminal",
    "agente de codificacion",
    "herramienta de codificacion",
    "programmierassistent",
    "programmierwerkzeug",
    "agente de codificacao",
    "ferramenta de codificacao",
];
/// Second pass: **first-person self-identification as a developer tool**.
///
/// The table above is an allowlist of phrasings already seen in the wild, so it
/// has high precision and almost no recall — a break worded any other way sails
/// through. registre#25's delivered reply did exactly that: it mentioned the
/// product only inside a denial ("…not the GitHub Copilot CLI", correctly
/// suppressed by the negation guard) and then identified itself with a phrase
/// the table never listed ("I'm the coding CLI working in the `dravr-platform`
/// repo"). Every pattern hit was negated, nothing else matched, and the leak was
/// delivered.
///
/// Detecting the *structure* — a first-person copula binding to a developer-tool
/// role within [`SELF_ID_PROXIMITY`] characters — closes the synonym escape
/// without widening the product allowlist. Precision holds because the copula is
/// required: prose about the athlete's own job ("you're a software agent…") has
/// no assistant-side "I am" in front of it, and a genuine denial still passes
/// through [`is_negated_at`] like any other class.
fn self_identified_tool_match(folded: &str, dash_breaks: &[usize]) -> Option<IdentityLeakMatch> {
    for claim in SELF_ID_CLAIMS {
        for (claim_at, _) in folded.match_indices(claim) {
            let after = claim_at + claim.len();
            for role in SELF_ID_ROLES {
                // Locate the role in the full text rather than inside a fixed
                // window: a window would truncate a long role name («ferramenta
                // de codificacao») and silently miss it.
                let Some(offset) = folded.get(after..).and_then(|rest| rest.find(role)) else {
                    continue;
                };
                if offset > SELF_ID_PROXIMITY {
                    continue;
                }
                // Negation is judged at the ROLE, not the copula: the refusal
                // reads «i m not the coding cli», so the "not" sits between
                // them and a lookbehind from the copula cannot see it.
                if !is_negated_at(folded, dash_breaks, after + offset) {
                    return Some(IdentityLeakMatch {
                        class: IdentityPatternClass::CodingAssistant,
                        locale: "any",
                        pattern_index: SELF_ID_PATTERN_INDEX,
                    });
                }
            }
        }
    }
    None
}

/// Sentinel index reported by [`self_identified_tool_match`].
///
/// Sits past the end of [`IDENTITY_NARRATION_PATTERNS`] so telemetry can tell a
/// structural hit from a table hit, and so [`identity_leak_context`] knows not
/// to look the index up in the table.
pub const SELF_ID_PATTERN_INDEX: usize = usize::MAX;
/// Full identity-leak match: the pattern table first, then the structural pass.
///
/// Lives here rather than in `narration.rs` so the whole matcher reads in one
/// place — and because `narration.rs` is over the repo's line ceiling and the
/// ratchet only lets it shrink.
pub(crate) fn leak_match(text: &str) -> Option<IdentityLeakMatch> {
    let mut dash_breaks: Vec<usize> = Vec::new();
    let folded = fold_into(text, |at| dash_breaks.push(at));
    FOLDED_IDENTITY
        .iter()
        .enumerate()
        .find_map(|(idx, p)| {
            let pattern = &IDENTITY_NARRATION_PATTERNS[idx];
            let guarded = pattern.class.denial_is_legitimate();
            folded
                .match_indices(p.as_str())
                .any(|(at, _)| !guarded || !is_negated_at(&folded, &dash_breaks, at))
                .then_some(IdentityLeakMatch {
                    class: pattern.class,
                    locale: pattern.locale,
                    pattern_index: idx,
                })
        })
        .or_else(|| self_identified_tool_match(&folded, &dash_breaks))
}

/// Resolve the phrase a forensics window should centre on.
///
/// A structural hit has no table entry to look up, so it anchors on the role
/// noun that fired instead. Without this the window comes back empty for exactly
/// the leaks the second pass exists to catch.
pub(crate) fn anchor(folded: &str, pattern_index: usize) -> Option<String> {
    if pattern_index == SELF_ID_PATTERN_INDEX {
        return SELF_ID_ROLES
            .iter()
            .find(|role| folded.contains(**role))
            .map(|role| (*role).to_owned());
    }
    FOLDED_IDENTITY.get(pattern_index).cloned()
}
