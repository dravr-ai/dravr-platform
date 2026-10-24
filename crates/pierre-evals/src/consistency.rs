// ABOUTME: The consistency-check layer of the bullshit detector — cross-checks a claim against its sibling claims
// ABOUTME: Pure Rust, no LLM. Catches a reply that contradicts itself across separate sentences.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Consistency Cross-Check
//!
//! The rhetoric, deterministic-bounds, and evidence-retrieval layers judge each
//! claim in isolation. The consistency-check layer looks at the *other*
//! claims extracted from the same agent reply and flags a claim that directly
//! contradicts a sibling. A reply that says "your VO2max is 58" in one
//! sentence and "your VO2max is 72" in another is internally inconsistent
//! even if each number is individually plausible, and an agent who tells a
//! user to both "take creatine daily" and "avoid creatine" is contradicting
//! themselves.
//!
//! Two contradiction shapes are detected, both restricted to sibling claims
//! in the **same category** so unrelated numbers never collide:
//!
//! - **Numeric divergence** — both claims share a salient subject and quantify
//!   the same unit ("5 minutes" / "7 minutes"), but no figure of that unit on
//!   one side is close to one on the other.
//! - **Negation conflict** — one claim asserts a subject while the sibling
//!   negates the same subject ("take X" vs "avoid X" / "don't take X").
//!
//! The check is intentionally conservative: it requires a shared subject
//! token before comparing, so two genuinely different claims in the same
//! category (protein dosing vs hydration timing) never trip the layer.

use std::ptr;

use crate::claim_extractor::ExtractedClaim;

/// A self-contradiction found between a claim and one of its siblings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsistencyConflict {
    /// The sibling claim text that conflicts with the claim under test.
    pub conflicting_sibling: String,
    /// Human-readable description of the contradiction for the explanation layer.
    pub reason: String,
}

/// Relative tolerance for numeric divergence between sibling claims about the
/// same subject. Two figures within 10% of each other (e.g. rounding "1.6"
/// vs "1.7 g/kg") are treated as consistent; wider gaps are flagged.
const NUMERIC_TOLERANCE: f64 = 0.10;

/// Minimum length for a token to count as a "salient subject" shared between
/// claims. Filters articles, prepositions, and units ("the", "per", "kg").
const MIN_SUBJECT_LEN: usize = 4;

/// Words that negate the subject they attach to. A claim containing one of
/// these conflicts with a sibling asserting the same subject without one.
const NEGATION_MARKERS: &[&str] = &[
    "avoid",
    "never",
    "shouldn't",
    "no",
    "not",
    "without",
    "stop",
    "skip",
];

/// High-frequency filler words that pass the length filter but carry no
/// subject meaning. Excluding them keeps the shared-subject match anchored on
/// a content noun (e.g. "pace", "creatine") instead of "your" or "should",
/// which both improves the reported reason and avoids spurious matches between
/// two unrelated sentences that merely share a pronoun.
///
/// Listed for every shipped locale: an English-only list let a French warm-up
/// reply share "pour" and "environ" between two of its steps and read as a
/// self-contradiction (carnet#1151).
const SUBJECT_STOPWORDS: &[&str] = &[
    // English
    "your",
    "yours",
    "this",
    "that",
    "these",
    "those",
    "should",
    "would",
    "could",
    "around",
    "about",
    "based",
    "with",
    "from",
    "into",
    "over",
    "than",
    "then",
    "they",
    "them",
    "there",
    "here",
    "have",
    "will",
    "been",
    "every",
    "during",
    "daily",
    "still",
    "just",
    "very",
    "more",
    "most",
    "some",
    "such",
    "what",
    "when",
    "which",
    "while",
    "each",
    "also",
    // French
    "pour",
    "avec",
    "dans",
    "environ",
    "entre",
    "après",
    "apres",
    "avant",
    "puis",
    "chaque",
    "très",
    "tres",
    "plus",
    "moins",
    "sans",
    "sous",
    "votre",
    "notre",
    "cette",
    "leur",
    "comme",
    "mais",
    "donc",
    "aussi",
    "encore",
    "jusqu",
    "quand",
    "tout",
    "toute",
    "tous",
    "toutes",
    "fait",
    "faire",
    "fais",
    "pendant",
    "chacune",
    "chacun",
    "elle",
    "elles",
    // Spanish
    "para",
    "como",
    "sobre",
    "cada",
    "durante",
    "antes",
    "después",
    "despues",
    "desde",
    "hasta",
    "más",
    "este",
    "esta",
    "esto",
    "pero",
    "también",
    "tambien",
    "unos",
    "unas",
    // German
    "für",
    "fuer",
    "nach",
    "über",
    "ueber",
    "unter",
    "zwischen",
    "etwa",
    "ungefähr",
    "sehr",
    "mehr",
    "jede",
    "jeden",
    "jeder",
    "dann",
    "auch",
    "aber",
    "oder",
    "eine",
    "einen",
    "einem",
    "einer",
    "dein",
    "deine",
    "deinen",
    "deiner",
    "nicht",
    // Portuguese
    "cerca",
    "depois",
    "muito",
    "mais",
    "você",
    "voce",
    "então",
    "entao",
    "pelo",
    "pela",
];

/// Check a claim against its sibling claims for a direct contradiction.
///
/// `siblings` is the full set of claims extracted from the same reply,
/// including `claim` itself — the claim under test is skipped by identity so
/// callers can pass the whole slice without filtering.
///
/// Returns the first [`ConsistencyConflict`] found, or `None` when the claim
/// is consistent with every sibling.
#[must_use]
pub fn find_contradiction(
    claim: &ExtractedClaim,
    siblings: &[ExtractedClaim],
) -> Option<ConsistencyConflict> {
    let claim_subjects = salient_subjects(&claim.text);
    if claim_subjects.is_empty() {
        return None;
    }
    let claim_quantities = quantities(&claim.text);
    let claim_negated = is_negated(&claim.text);

    for sibling in siblings {
        // Skip the claim under test itself.
        if ptr::eq(sibling, claim) || sibling == claim {
            continue;
        }
        // Only compare within the same category — different categories never
        // contradict each other by construction (a nutrition figure and a
        // physiology figure are about different things).
        if sibling.category != claim.category {
            continue;
        }

        let Some(subject) = shared_subject(&claim_subjects, &sibling.text) else {
            continue;
        };

        // Negation conflict: one asserts the subject, the other negates it.
        if claim_negated != is_negated(&sibling.text) {
            return Some(ConsistencyConflict {
                conflicting_sibling: sibling.text.clone(),
                reason: format!(
                    "claim about '{subject}' is asserted in one sentence and negated in another"
                ),
            });
        }

        // Numeric divergence: both quantify the same unit about the shared
        // subject but the figures disagree beyond tolerance.
        let sibling_quantities = quantities(&sibling.text);
        if let Some((a, b)) = diverging_pair(&claim_quantities, &sibling_quantities) {
            return Some(ConsistencyConflict {
                conflicting_sibling: sibling.text.clone(),
                reason: format!(
                    "claim about '{subject}' gives {a} here but {b} in another sentence"
                ),
            });
        }
    }
    None
}

/// Lowercase salient subject tokens (alphanumeric, length >=
/// [`MIN_SUBJECT_LEN`], containing at least one letter).
///
/// Splitting on non-alphanumeric keeps compound subject tokens like "vo2max"
/// intact instead of shattering them on the embedded digit. Pure-number
/// tokens are excluded — those are figures, compared separately by
/// [`quantities`], not subjects.
fn salient_subjects(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| {
            t.len() >= MIN_SUBJECT_LEN
                && t.chars().any(char::is_alphabetic)
                && !SUBJECT_STOPWORDS.contains(t)
        })
        .map(ToOwned::to_owned)
        .collect()
}

/// Return the first subject token shared between `claim_subjects` and `other`.
fn shared_subject(claim_subjects: &[String], other: &str) -> Option<String> {
    let other_subjects = salient_subjects(other);
    claim_subjects
        .iter()
        .find(|s| other_subjects.iter().any(|o| o == *s))
        .cloned()
}

/// One figure in a claim and the unit it counts: "15 à 20 minutes" holds
/// `(15, minutes)` and `(20, minutes)`, "4 lignes droites" `(4, lignes)`.
#[derive(Debug, Clone, PartialEq)]
struct Quantity {
    value: f64,
    /// The first word after the figure, lowercased; `None` when the figure
    /// ends the sentence ("your `VO2max` is 58").
    unit: Option<String>,
}

/// Words that join the two ends of a range ("15 à 20 minutes", "4 to 6
/// strides"), skipped when looking for the unit a figure counts.
const RANGE_CONNECTORS: &[&str] = &[
    "to", "or", "and", "à", "a", "ou", "et", "y", "o", "bis", "oder", "und", "até", "ate", "e",
];

/// One spelling per unit, so "1.6 g" and "1.6 grams" compare: the short form
/// and the spelled-out forms of the five shipped locales.
fn canonical_unit(unit: &str) -> String {
    let canonical = match unit {
        "g" | "gram" | "grams" | "gramme" | "grammes" | "gramo" | "gramos" | "gramm" | "grama"
        | "gramas" => "g",
        "kg" | "kilo" | "kilos" | "kilogram" | "kilograms" | "kilogramme" | "kilogrammes" => "kg",
        "min" | "mins" | "minute" | "minutes" | "minuto" | "minutos" | "minuten" | "mn" => "min",
        "s" | "sec" | "secs" | "second" | "seconds" | "seconde" | "secondes" | "segundo"
        | "segundos" | "sekunde" | "sekunden" => "s",
        "h" | "hr" | "hrs" | "hour" | "hours" | "heure" | "heures" | "hora" | "horas"
        | "stunde" | "stunden" => "h",
        "km" | "kilometre" | "kilometres" | "kilometer" | "kilometers" | "kilomètre"
        | "kilomètres" | "kilómetro" | "kilómetros" | "quilómetro" | "quilômetro" => "km",
        "m" | "metre" | "metres" | "meter" | "meters" | "mètre" | "mètres" | "metro" | "metros" => {
            "m"
        }
        "w" | "watt" | "watts" | "vatios" => "w",
        "kcal" | "kilocalorie" | "kilocalories" | "kilocalorías" | "kilokalorien" => "kcal",
        other => other,
    };
    canonical.to_owned()
}

/// Extract every figure of a claim with the unit it counts, normalizing the
/// French decimal comma ("1,6") to a period so locale-mixed replies compare
/// correctly.
///
/// Tokens are split on any non-alphanumeric, non-period character, so a digit
/// embedded in a word ("vo2max") stays attached to its letters and is never
/// read as the figure `2`. A figure glued to its unit ("80m") keeps it.
fn quantities(text: &str) -> Vec<Quantity> {
    let normalized = normalize_decimal_commas(text).to_lowercase();
    let tokens: Vec<&str> = normalized
        .split(|c: char| !(c.is_alphanumeric() || c == '.'))
        .map(|t| t.trim_matches('.'))
        .filter(|t| !t.is_empty())
        .collect();
    let mut out = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let digits_end = token
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(token.len());
        if digits_end == 0 {
            continue;
        }
        let (figure, glued) = token.split_at(digits_end);
        if !glued.is_empty() && !glued.chars().all(char::is_alphabetic) {
            continue;
        }
        let Ok(value) = figure.parse::<f64>() else {
            continue;
        };
        let unit = if glued.is_empty() {
            tokens[i + 1..]
                .iter()
                .find(|next| {
                    !RANGE_CONNECTORS.contains(next)
                        && !next.chars().all(|c| c.is_ascii_digit() || c == '.')
                })
                .map(|next| (*next).to_owned())
        } else {
            Some(glued.to_owned())
        };
        out.push(Quantity {
            value,
            unit: unit.map(|u| canonical_unit(&u)),
        });
    }
    out
}

/// Replace a decimal comma between two digits ("1,6") with a period so the
/// numeric tokenizer parses it, while leaving list commas ("3, 4, 5") alone.
fn normalize_decimal_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &ch) in chars.iter().enumerate() {
        if ch == ',' {
            let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_digit = chars.get(i + 1).is_some_and(char::is_ascii_digit);
            out.push(if prev_digit && next_digit { '.' } else { ch });
        } else {
            out.push(ch);
        }
    }
    out
}

/// True when the text contains a negation marker as a whole word.
fn is_negated(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .any(|word| NEGATION_MARKERS.contains(&word))
}

/// Find a pair of figures — one from each claim, counting the same unit —
/// that diverge beyond [`NUMERIC_TOLERANCE`]. Returns the first diverging
/// pair, or `None` when no unit is quantified on both sides or any same-unit
/// pair agrees within tolerance.
///
/// Figures of different units never compare: a warm-up's "15 à 20 minutes" in
/// total and its "4 lignes droites" are two steps of one routine, not two
/// answers to one question (carnet#1151).
fn diverging_pair(a: &[Quantity], b: &[Quantity]) -> Option<(f64, f64)> {
    let pairs: Vec<(f64, f64)> = a
        .iter()
        .flat_map(|x| {
            b.iter()
                .filter(move |y| x.unit == y.unit)
                .map(move |y| (x.value, y.value))
        })
        .collect();
    // If any same-unit figure on one side is close to one on the other, the
    // claims agree on at least one figure and we treat them as consistent.
    if pairs.iter().any(|(x, y)| within_tolerance(*x, *y)) {
        return None;
    }
    pairs.first().copied()
}

/// True when two figures are within [`NUMERIC_TOLERANCE`] of each other,
/// guarding against division by zero when both are zero.
fn within_tolerance(x: f64, y: f64) -> bool {
    let scale = x.abs().max(y.abs());
    if scale == 0.0 {
        return true;
    }
    (x - y).abs() / scale <= NUMERIC_TOLERANCE
}
