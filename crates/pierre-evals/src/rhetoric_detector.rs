// ABOUTME: The rhetoric filter of the bullshit detector — discards rhetorical / non-propositional utterances
// ABOUTME: Pure Rust, no LLM. Runs on every claim to filter motivational flourishes.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Rhetoric Detector
//!
//! The first gate in the bullshit detector pipeline. Given a sentence
//! or atomic claim extracted from an agent reply, decides whether it is:
//!
//! - **Rhetorical** — motivational ("you're crushing it!"), greeting,
//!   question to the user, or imperative with no factual predicate. Fast
//!   path, `ClaimStatus::Rhetorical`, no further layers run.
//! - **Propositional** — a factual claim that deserves verification.
//!
//! The detector is intentionally conservative: false negatives (letting a
//! rhetorical phrase through) are cheap because the deterministic-bounds
//! and evidence-retrieval layers will also decline to fire a verdict on
//! them. False positives (marking a real
//! claim as rhetorical) are worse because they bypass verification, so the
//! heuristics favour "still verify" over "drop on the floor".

/// Patterns that almost always identify rhetorical or non-propositional text.
/// These are matched case-insensitively against the *start* of the claim after
/// trimming leading punctuation and whitespace. Only the start: a phrase further
/// in does not make the sentence rhetorical, and matched anywhere `hey ` would
/// fire inside `they ` and `whey ` and skip every check. And only when nothing
/// claim-like follows: see [`opens_with_a_bare_courtesy`].
const RHETORICAL_PREFIXES: &[&str] = &[
    "great job",
    "well done",
    "keep going",
    "awesome work",
    "nice work",
    "amazing",
    "fantastic",
    "hi ",
    "hello ",
    "hey ",
    "good morning",
    "good afternoon",
    "good evening",
    "let me know",
    "feel free to",
    "what do you think",
    "how do you feel",
    "thanks for",
    "thank you",
    "absolutely",
    "of course",
    "sure thing",
    // French
    "bravo",
    "bien joué",
    "beau travail",
    "continue comme ça",
    "tu gères",
    "bonjour",
    "salut",
    "merci",
    "n'hésite pas",
    "n’hésite pas",
    "dis-moi",
    "bien sûr",
    // Spanish
    "buen trabajo",
    "bien hecho",
    "sigue así",
    "hola",
    "gracias",
    "no dudes",
    "avísame",
    "por supuesto",
    // German
    "gut gemacht",
    "weiter so",
    "hallo",
    "danke",
    "sag mir",
    "selbstverständlich",
    // Portuguese
    "bom trabalho",
    "muito bem",
    "continua assim",
    "olá",
    "obrigado",
    "obrigada",
    "não hesites",
    "diz-me",
    "claro que sim",
];

/// Encouragement that states nothing wherever it sits in the sentence: "You're
/// absolutely crushing it." Only phrases no factual claim would use belong here.
const RHETORICAL_ANYWHERE: &[&str] = &["crushing it", "you got this", "keep it up"];

/// Second-order keywords that make a sentence overwhelmingly likely to be a
/// question back to the user rather than a factual claim.
const QUESTION_MARKERS: &[&str] = &[
    "?",
    " do you ",
    " are you ",
    " have you ",
    " did you ",
    " could you ",
    " would you ",
    " can you ",
    " will you ",
];

/// Openings that ask the athlete for something rather than state anything:
/// "Tell me your FTP" is a question without its question mark. Only second-person
/// asks of the speaker belong here, each naming "me": a rule on every leading verb
/// would swallow prescriptions like "Eat 60 g of carbs per hour" or "Share your
/// volume across three sessions", which the later layers must still see.
const ASKS_OF_THE_ATHLETE: &[&str] = &[
    "tell me ",
    "give me your ",
    "send me your ",
    // French
    "dites-moi",
    "donne-moi ton ",
    "donne-moi ta ",
    "donne-moi tes ",
    "donnez-moi votre ",
    "donnez-moi vos ",
    "indique-moi",
    // Spanish
    "dime ",
    "dígame",
    "dame tu ",
    "dame tus ",
    "cuéntame",
    "indícame",
    // German
    "nenn mir",
    "gib mir dein",
    "sagen sie mir",
    // Portuguese
    "dá-me o teu",
    "dá-me a tua",
    "indica-me",
    "conta-me",
];

/// What ends an ask and starts something else in the same sentence, after which
/// the sentence may carry a claim: "Tell me your FTP, since creatine doubles it."
const CLAUSE_BREAKS: &[&str] = &[
    ",",
    ";",
    ":",
    " because ",
    " since ",
    " parce que ",
    " car ",
    " porque ",
    " weil ",
];

/// `true` for a bare ask of the athlete. A figure or a second clause sends the
/// sentence on to be checked: a missed ask costs the later layers one look, while
/// a claim passed off as an ask would skip verification.
fn asks_the_athlete(lower: &str) -> bool {
    ASKS_OF_THE_ATHLETE.iter().any(|p| lower.starts_with(p))
        && !lower.chars().any(|c| c.is_ascii_digit())
        && !CLAUSE_BREAKS.iter().any(|b| lower.contains(b))
}

/// `true` when the sentence opens with a courtesy and claims nothing after it.
///
/// The opening alone does not decide: "Bien sûr, la créatine double ta FTP."
/// opens with a courtesy and still makes a claim. A figure anywhere, or a second
/// clause of more than one word, sends the sentence on to be checked; a single
/// word after the break is an address ("Good morning, champ.", "Merci, Marie."),
/// which states nothing.
fn opens_with_a_bare_courtesy(lower: &str) -> bool {
    let Some(prefix) = RHETORICAL_PREFIXES.iter().find(|p| lower.starts_with(*p)) else {
        return false;
    };
    if lower.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    let rest = &lower[prefix.len()..];
    let Some(tail_start) = CLAUSE_BREAKS
        .iter()
        .filter_map(|b| rest.find(b).map(|at| at + b.len()))
        .min()
    else {
        return true;
    };
    rest[tail_start..]
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
        <= 1
}

/// Result of a rhetoric check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RhetoricVerdict {
    /// The claim is rhetorical and should skip deeper verification layers.
    Rhetorical,
    /// The claim is propositional and should pass to the deterministic-bounds layer.
    Propositional,
}

/// Classify a single extracted claim as rhetorical or propositional.
///
/// Empty or whitespace-only strings are considered rhetorical so the caller
/// never wastes an LLM call on them.
#[must_use]
pub fn classify(claim: &str) -> RhetoricVerdict {
    let trimmed = claim.trim_matches(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '.'));
    if trimmed.is_empty() {
        return RhetoricVerdict::Rhetorical;
    }

    let lower = trimmed.to_lowercase();

    if opens_with_a_bare_courtesy(&lower)
        || RHETORICAL_ANYWHERE.iter().any(|p| lower.contains(p))
        || asks_the_athlete(&lower)
    {
        return RhetoricVerdict::Rhetorical;
    }

    if QUESTION_MARKERS.iter().any(|m| lower.contains(m)) {
        return RhetoricVerdict::Rhetorical;
    }

    // Imperatives with no factual predicate ("try this", "give it a shot")
    // tend to lack any of the category-specific nouns. The deterministic-bounds layer will catch
    // them by scoring zero on all category patterns, so we let them through
    // here — false negatives in this gate are safe by design.
    RhetoricVerdict::Propositional
}
