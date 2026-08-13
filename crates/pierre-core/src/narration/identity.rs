// ABOUTME: Model-identity leak detection — negation lookbehind and match entry points
// ABOUTME: Split out of narration/mod.rs so each file stays legible
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::fold::fold_into;
use super::is_terminator;
use super::patterns::IdentityPatternClass;
use super::self_id;

/// Telemetry label for a matched identity-leak pattern.
///
/// Says which coarse class fired, in which locale, and the pattern's
/// index in [`IDENTITY_NARRATION_PATTERNS`]. Deliberately carries no
/// text — the withheld reply is never logged or persisted, so these
/// labels are the only per-leak signal that reaches operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityLeakMatch {
    /// Coarse pattern class (product flip vs roleplay framing vs …).
    pub class: IdentityPatternClass,
    /// Language of the matched phrase (`"any"` for product names).
    pub locale: &'static str,
    /// Index into [`IDENTITY_NARRATION_PATTERNS`], the finest-grained
    /// content-free label — pins the exact phrase via the source table.
    pub pattern_index: usize,
}

/// Negation markers that turn an identity phrase into a *denial*.
///
/// Separator-folded and whitespace-tokenized, so `n't` arrives as the bare
/// token `t` (`isn't` folds to `isn t`) and French `n'est` as `n est`.
pub(super) const NEGATION_TOKENS: &[&str] = &[
    // en
    "not", "never", "no", // fr
    "pas", "non", "jamais", "aucun", "aucune", // es / pt
    "nao", "não", "ningun", "ninguna", // de
    "nicht", "kein", "keine",
];

/// Punctuation that closes a clause only when what follows re-asserts an
/// identity ([`asserts_self`]).
///
/// A comma is as often a list separator as a clause break, and the two need
/// opposite answers: « je ne suis pas un coach, je suis GitHub Copilot »
/// starts a new predication the negation does not reach, while "you are not a
/// general-purpose AI, a language model, a chat bot" keeps every item under
/// the one negation — the shape of the identity anchor the coach is told to
/// embody, so treating its commas as clause breaks would withhold the coach's
/// own persona statement.
pub(super) const fn is_soft_clause_break(c: char) -> bool {
    matches!(c, ',' | ';' | ':')
}

/// Tokens that re-assert a first-person identity, marking the text after a
/// soft break as a new predication rather than another item in a negated
/// list. Subject pronouns for en/fr/de, plus the first-person copulas the
/// pro-drop languages use without one (« soy », « sou ») and the ones a
/// pronoun-dropping fold leaves behind. Folded, so `I'm` arrives as `i m`.
pub(super) const SELF_ASSERTION_TOKENS: &[&str] = &[
    "i", "je", "ich", "yo", "eu", "am", "suis", "bin", "soy", "sou",
];

/// `true` when `segment` re-asserts a first-person identity.
pub(super) fn asserts_self(segment: &str) -> bool {
    segment
        .split(' ')
        .any(|tok| SELF_ASSERTION_TOKENS.contains(&tok))
}

/// `true` when a negation marker in the identity phrase's **own clause**
/// denies it — i.e. the coach is rejecting the identity, not claiming it.
///
/// `dash_breaks` are the folded offsets [`fold_into`] reported for erased
/// clause-breaking dashes.
///
/// The clause bound is what makes the guard grammatical rather than merely
/// proximate. « Je ne suis pas un coach, je suis GitHub Copilot » negates
/// "un coach"; reading the negation across the comma marked a full persona
/// break as a denial and delivered it to the athlete, as did "I'm not able to
/// help with that. I'm GitHub Copilot CLI" across the sentence boundary and
/// "I'm not a fitness coach — I'm GitHub Copilot CLI" across the em dash.
pub(super) fn is_negated_at(folded: &str, dash_breaks: &[usize], at: usize) -> bool {
    let prefix = &folded[..at];
    // Start of the clause the phrase sits in: just past the nearest
    // qualifying punctuation break, or the nearest erased dash break. A
    // sentence terminator always qualifies; a comma only when the text it
    // opens re-asserts an identity rather than continuing a list.
    let punctuation_start = prefix
        .char_indices()
        .rev()
        .find(|&(offset, c)| {
            is_terminator(c)
                || (is_soft_clause_break(c) && asserts_self(&prefix[offset + c.len_utf8()..]))
        })
        .map_or(0, |(offset, c)| offset + c.len_utf8());
    let dash_start = dash_breaks
        .iter()
        .rev()
        .find(|&&offset| offset < at)
        .map_or(0, |&offset| offset + 1);
    // Both bounds land on a char boundary, so their maximum does too. The
    // clause is the whole guard: a character-count lookbehind would have to
    // be short enough to stay out of the previous clause, and a negation
    // legitimately sits a whole list away from the item it governs — "not a
    // general-purpose AI, a language model, a chat bot, or a coding …
    // assistant".
    let start = punctuation_start.max(dash_start);
    prefix[start..]
        .split(' ')
        .any(|tok| NEGATION_TOKENS.contains(&tok))
}

/// First identity-leak pattern the reply *claims*, `None` when clean.
///
/// Matches in table order — product names first, so a reply that both
/// names Copilot and talks about role-play reports the more conclusive
/// product hit.
///
/// **Denials do not count.** A hit whose every occurrence is denied by a
/// negation marker *in its own clause* is the coach correctly rejecting the
/// identity (« Non, je ne suis pas GitHub Copilot — je suis Dravr ») and must
/// reach the athlete. Left unguarded this was not a theoretical false positive
/// but the *only* thing the matcher caught: across a 48-run live A/B on
/// 2026-07-25 all 5 boundary matches were correct denials, while the real
/// break — Copilot's own « I'm powered by Claude Sonnet 5 (model ID: …) »
/// clause — went undetected. Worse, the failure was self-reinforcing: a leak
/// prompts the athlete to ask "are you Copilot?", and the correct answer was
/// then withheld too. The clause bound is what keeps that guard from becoming
/// a bypass in turn — see [`is_negated_at`].
#[must_use]
pub fn identity_leak_match(text: &str) -> Option<IdentityLeakMatch> {
    self_id::leak_match(text)
}

/// A bounded, separator-folded window of the reply around the matched identity
/// phrase — the minimum forensics needed to tell a CLAIM from a DENIAL.
///
/// `pattern_index` alone cannot: «I'm GitHub Copilot», «I'm NOT GitHub Copilot»
/// and «I'm powered by Claude Sonnet 5» are three completely different
/// diagnoses that log identically today, which is why every historical leak
/// count is an upper bound rather than a measurement. The 2026-07-25 A/B found
/// all five live matches were correct denials; without a window there is no way
/// to know that from telemetry.
///
/// Deliberately bounded and folded rather than the whole reply: the withheld
/// text is withheld for a reason, so this returns at most `window_chars` either
/// side of the matched phrase, lowercased with separators collapsed. That is
/// enough to read the model's own words about its own identity and nothing
/// more — it is not athlete data, and the cap keeps an accidental PII spill
/// short. Returns `None` when the reply is clean.
#[must_use]
pub fn identity_leak_context(text: &str, window_chars: usize) -> Option<String> {
    let hit = identity_leak_match(text)?;
    // The dash-break offsets are only needed by the negation lookbehind, which
    // `identity_leak_match` above has already applied — this call just needs the
    // folded text, so the callback discards them rather than allocating.
    let folded = fold_into(text, |_| ());

    // A structural hit has no table entry to look up, so anchor the window on
    // the role noun that fired instead. Without this the forensics window comes
    // back empty for exactly the leaks the second pass exists to catch.
    let pattern = self_id::anchor(&folded, hit.pattern_index)?;
    let at = folded.find(pattern.as_str())?;

    // Char-safe on both ends: the folded text is still UTF-8 and a raw byte
    // slice here would panic on an accented character — the exact defect class
    // that took down a whole turn in 5c2c38c81.
    let start = folded[..at]
        .char_indices()
        .rev()
        .nth(window_chars.saturating_sub(1))
        .map_or(0, |(offset, _)| offset);
    let tail = &folded[at..];
    let take = pattern.chars().count() + window_chars;
    let end = at
        + tail
            .char_indices()
            .nth(take)
            .map_or(tail.len(), |(offset, _)| offset);

    Some(folded[start..end].to_owned())
}

/// Boolean shorthand for [`identity_leak_match`].
///
/// `true` when the reply anywhere identifies as the underlying model/
/// provider or frames its own persona as a roleplay/injection to refuse
/// — a whole persona break. The caller withholds the entire reply.
#[must_use]
pub fn contains_identity_leak(text: &str) -> bool {
    identity_leak_match(text).is_some()
}
