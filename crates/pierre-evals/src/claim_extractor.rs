// ABOUTME: The claim-extraction stage of the bullshit detector — decomposes an agent reply into atomic claims
// ABOUTME: Splits on sentence boundaries and classifies each claim by keyword — pure Rust, no LLM call
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Claim Extractor
//!
//! Given a raw agent response, returns a list of atomic propositions, each
//! tagged with a [`ClaimCategory`].
//!
//! Extraction is heuristic (`extract_heuristic`): pure Rust that splits on
//! sentence boundaries and category-classifies via keyword matching, so it
//! runs on every reply with no LLM call and no cost.

use pierre_memory::ClaimCategory;
use serde::{Deserialize, Serialize};

/// Where a claim's text came from.
///
/// Downstream presentation keys off this: a [`ClaimSource::Reply`] claim is a
/// verbatim sentence of the reply the user is reading, so its position in that
/// reply says nothing about whether the warning is worth showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimSource {
    /// A sentence [`extract_heuristic`] split out of the agent reply.
    Reply,
    /// Text supplied directly by a caller (e.g. the `verify_claim` tool).
    Caller,
}

/// A single claim extracted from an agent reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedClaim {
    /// The raw claim text.
    pub text: String,
    /// Category assigned by the extractor.
    pub category: ClaimCategory,
    /// Where the claim text came from.
    pub source: ClaimSource,
}

/// Minimum word count for a sentence to be treated as a verifiable claim.
/// Fragments shorter than this — "Yes, but with restraint." (4 words),
/// "Hydrate well." (2) — are rhetorical glue, not factual claims, and
/// the verifier was over-flagging them as `unsupported` (audit,
/// 2026-05-07). Threshold of 5 keeps short factual claims like "Your
/// `VO2max` is around 58." in scope while still dropping the patterns
/// the audit flagged.
const MIN_CLAIM_WORDS: usize = 5;

/// Pure-Rust heuristic extraction for use without an LLM.
///
/// Splits the reply on sentence boundaries and assigns the best-matching
/// category based on keyword counts. Claims that score zero on every
/// category, or that fall under [`MIN_CLAIM_WORDS`], are dropped.
#[must_use]
pub fn extract_heuristic(agent_reply: &str) -> Vec<ExtractedClaim> {
    let mut out = Vec::new();
    for sentence in split_sentences(agent_reply) {
        let trimmed = sentence.trim();
        if trimmed.is_empty() {
            continue;
        }
        if word_count(trimmed) < MIN_CLAIM_WORDS {
            continue;
        }
        if let Some(category) = classify_heuristic(trimmed) {
            out.push(ExtractedClaim {
                text: trimmed.to_owned(),
                category,
                source: ClaimSource::Reply,
            });
        }
    }
    out
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().filter(|w| !w.is_empty()).count()
}

/// Split an agent reply into sentences on `.`/`!`/`?` boundaries.
///
/// Used by [`extract_heuristic`]; exposed so the heuristic building blocks
/// can be exercised directly.
#[must_use]
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len() {
        let ch = chars[i];
        current.push(ch);
        // A line break is a unit boundary. Without this, a newline-delimited
        // list with no terminal punctuation (e.g. a weekly training plan) is
        // swallowed into one giant multi-line "claim".
        if ch == '\n' {
            finalize_sentence(&mut current, &mut sentences);
            continue;
        }
        if matches!(ch, '!' | '?') {
            finalize_sentence(&mut current, &mut sentences);
            continue;
        }
        if ch == '.' {
            // Skip decimal points ("1.6") and ellipses — split only when the
            // `.` is followed by whitespace, end-of-text, or another sentence
            // terminator. This keeps numeric tokens intact for the detector.
            let next = chars.get(i + 1).copied();
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let is_decimal = prev.is_some_and(|c| c.is_ascii_digit())
                && next.is_some_and(|c| c.is_ascii_digit());
            let is_terminator =
                next.is_none_or(|c| c.is_whitespace() || matches!(c, '.' | '!' | '?'));
            if !is_decimal && is_terminator {
                finalize_sentence(&mut current, &mut sentences);
            }
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_owned());
    }
    sentences
}

fn finalize_sentence(buf: &mut String, out: &mut Vec<String>) {
    let trimmed = buf.trim().to_owned();
    if !trimmed.is_empty() {
        out.push(trimmed);
    }
    buf.clear();
}

/// Match a keyword against a sentence with whole-token boundaries.
///
/// Plain `.contains()` matched "carb" inside French "carburant" (fuel) and
/// misclassified training-intensity claims as Nutrition. Tokenize on non-word
/// chars first; a multi-word keyword like "vo2 max" or "long run" is matched
/// by sliding over the token window of equal length.
fn keyword_hits(lower: &str, keyword: &str) -> bool {
    let kw_tokens: Vec<&str> = keyword
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if kw_tokens.is_empty() {
        return false;
    }
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    tokens
        .windows(kw_tokens.len())
        .any(|w| w == kw_tokens.as_slice())
}

/// Assign a sentence to its best-matching [`ClaimCategory`] by keyword count.
///
/// Returns `None` when no category scores (e.g., motivational filler). Used by
/// [`extract_heuristic`]; exposed so the heuristic building blocks can be
/// exercised directly.
#[must_use]
pub fn classify_heuristic(sentence: &str) -> Option<ClaimCategory> {
    let lower = sentence.to_lowercase();
    let mut best: Option<(ClaimCategory, usize)> = None;

    let buckets: [(ClaimCategory, &[&str]); 7] = [
        (
            ClaimCategory::Physiological,
            &[
                "vo2max",
                "vo2 max",
                "heart rate",
                "hr max",
                "hrmax",
                "lactate",
                "threshold",
                "substrate",
                "zone",
                "ftp",
                // French
                "fc",
                "fc moy",
                "fc max",
                "fréquence cardiaque",
                "frequence cardiaque",
                "bpm",
                "seuil",
            ],
        ),
        (
            ClaimCategory::TrainingPrescription,
            &[
                "interval",
                "tempo",
                "long run",
                "easy run",
                "rep",
                "set",
                "periodization",
                "mileage",
                "volume",
                "intensity",
                "taper",
                // French
                "intervalle",
                "fractionné",
                "fractionne",
                "sortie longue",
                "endurance fondamentale",
                "séance",
                "seance",
                "kilométrage",
                "kilometrage",
                "périodisation",
                "periodisation",
                "affûtage",
                "affutage",
            ],
        ),
        (
            ClaimCategory::Nutrition,
            &[
                "protein",
                "carbohydrate",
                "carbohydrates",
                "carb",
                "carbs",
                "calorie",
                "gram",
                "macro",
                "hydration",
                "electrolyte",
                "meal",
                "fueling",
                "fuelling",
                // French
                "protéine",
                "protéines",
                "proteine",
                "proteines",
                "glucide",
                "glucides",
                "calorie",
                "macros",
                "hydratation",
                "électrolyte",
                "electrolyte",
                "repas",
                "ravitaillement",
            ],
        ),
        (
            ClaimCategory::Recovery,
            &[
                "sleep",
                "hrv",
                "cold plunge",
                "ice bath",
                "sauna",
                "recovery",
                "rest day",
                // French
                "sommeil",
                "vfc",
                "récupération",
                "recuperation",
                "jour de repos",
            ],
        ),
        (
            ClaimCategory::Supplement,
            &[
                "creatine",
                "caffeine",
                "beta-alanine",
                "bcaa",
                "supplement",
                "dose",
                "dosing",
                // French
                "créatine",
                "creatine",
                "caféine",
                "cafeine",
                "complément",
                "complement",
                "dosage",
            ],
        ),
        (
            ClaimCategory::InjuryRehab,
            &[
                "rehab",
                "return to play",
                "rtp",
                "achilles",
                "tendon",
                "physical therapy",
                "pt",
                "strain",
                "sprain",
                // French
                "rééducation",
                "reeducation",
                "kiné",
                "kine",
                "tendinite",
                "entorse",
                "claquage",
            ],
        ),
        (
            // Keyed on *reference to the athlete's own record* rather than on
            // subject matter, because subject matter is what the other six
            // already partition. "Zone 2 sits at 70% of max HR" is physiology;
            // "you ran 21 km on Sunday" is a database row. What separates them
            // is the second-person past tense and the time reference, so those
            // are what this bucket scores.
            ClaimCategory::AthleteData,
            &[
                "you ran",
                "you rode",
                "you swam",
                "you covered",
                "you logged",
                "you completed",
                "your longest",
                "your fastest",
                "your last",
                "your recent",
                "yesterday",
                "last week",
                "last month",
                "last sunday",
                "this week",
                "so far this",
                // French
                "tu as couru",
                "tu as roulé",
                "tu as roule",
                "tu as nagé",
                "tu as nage",
                "ta plus longue",
                "hier",
                "la semaine dernière",
                "la semaine derniere",
                "le mois dernier",
                "cette semaine",
            ],
        ),
    ];

    for (cat, keywords) in buckets {
        // A prescription is not a claim about the past, whatever time words it
        // carries. "This week, aim for 40 km" scores on "this week" and would
        // otherwise be routed to the athlete-data layer, which checks it against
        // activities it was never about — a guaranteed miss on a correct reply.
        // Only this category needs the guard: the other six are keyed on subject
        // matter, which tense does not change.
        if cat == ClaimCategory::AthleteData && is_prescriptive(&lower) {
            continue;
        }
        let score = keywords
            .iter()
            .filter(|kw| keyword_hits(&lower, kw))
            .count();
        if score > 0 && best.is_none_or(|(_, b)| score > b) {
            best = Some((cat, score));
        }
    }

    best.map(|(c, _)| c)
}

/// Whether a sentence prescribes future work rather than reporting past work.
///
/// Both directions of error are real, which is why this is neither a bare
/// substring test nor an open-ended one:
///
/// - A **false positive** un-routes a genuine history claim from the only layer
///   that can check it — the failure this workstream exists to prevent.
/// - A **false negative** is worse than it looks for the athlete Phase 5 serves.
///   A providerless athlete's agent prescribes in figures ("easy 5 km tomorrow"),
///   and an unrecognised prescription reaches the athlete-data layer, where
///   "no provider" licenses `Contradicted` at 0.95 — telling the athlete their
///   agent invented a number that was never a claim about the past at all.
///
/// Matching goes through [`keyword_hits`], the same token-boundary test every
/// category bucket in this file uses. A raw `contains` fired `"should"` inside
/// `"shoulder"` and `"target"` inside `"targeted"`, silently reclassifying
/// ordinary sentences about a sore shoulder.
fn is_prescriptive(lower: &str) -> bool {
    const MARKERS: [&str; 26] = [
        // Modal / intent
        "aim for",
        "let's",
        "lets",
        "we'll",
        "you'll",
        "should",
        "try to",
        "plan to",
        "target",
        "recommend",
        "suggest",
        "go for",
        "start with",
        "focus on",
        // Future time
        "tomorrow",
        "next week",
        "this week",
        // French
        "vise",
        "on va",
        "tu devrais",
        "la semaine prochaine",
        "cette semaine",
        "demain",
        "je te propose",
        "commence par",
        "essaie",
    ];
    MARKERS.iter().any(|m| keyword_hits(lower, m))
}
