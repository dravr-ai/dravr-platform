// ABOUTME: Layer 0 of the bullshit detector — decomposes a coach reply into atomic claims
// ABOUTME: Uses pierre_llm::judge::ask_for_json when an LLM is available; static rules otherwise
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Claim Extractor (Layer 0)
//!
//! Given a raw coach response, returns a list of atomic propositions, each
//! tagged with a [`ClaimCategory`]. The extractor has two modes:
//!
//! - **LLM-based** (`extract_with_llm`) — invokes `ask_for_json` on the
//!   provided [`LlmProvider`] with the extraction prompt to receive a
//!   structured list.
//! - **Heuristic** (`extract_heuristic`) — pure-Rust fallback that splits
//!   on sentence boundaries and category-classifies via keyword matching.
//!   Used when Layer 0 must run without an LLM (dev, tests, cost cap).

use pierre_core::errors::AppResult;
use pierre_llm::judge::ask_for_json;
use pierre_llm::LlmProvider;
use pierre_memory::ClaimCategory;
use serde::{Deserialize, Serialize};

/// A single claim extracted from a coach reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedClaim {
    /// The raw claim text.
    pub text: String,
    /// Category assigned by the extractor.
    pub category: ClaimCategory,
}

/// JSON shape returned by the LLM extractor.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractionResponse {
    claims: Vec<RawExtractedClaim>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawExtractedClaim {
    text: String,
    category: String,
}

const EXTRACTION_SYSTEM_PROMPT: &str = r#"You are a claim extraction engine for a sports science
verification pipeline. Given a coach's reply to a user, split it into atomic
propositions and tag each with exactly one category from this set:

- physiological (HR, VO2max, lactate thresholds, substrate utilization)
- training_prescription (volume, intensity, periodization, workouts)
- nutrition (macros, hydration, fuelling timing)
- recovery (sleep, HRV, cold/heat therapy, active recovery)
- supplement (ergogenic aids, dosing)
- injury_rehab (return-to-play timelines, rehabilitation protocols)

Only extract factual claims. Discard greetings, motivation, questions, and
imperatives without factual predicates. Return strict JSON of the form:

{"claims":[{"text":"...","category":"..."}, ...]}

If no claims are factual, return {"claims":[]}."#;

/// Extract atomic claims from a coach reply using an LLM provider.
///
/// # Errors
///
/// Returns an error if the LLM call or JSON parse fails. Callers that need
/// graceful degradation should fall back to [`extract_heuristic`].
pub async fn extract_with_llm(
    provider: &dyn LlmProvider,
    coach_reply: &str,
) -> AppResult<Vec<ExtractedClaim>> {
    let response: ExtractionResponse =
        ask_for_json(provider, EXTRACTION_SYSTEM_PROMPT, coach_reply, 0.0).await?;

    Ok(response
        .claims
        .into_iter()
        .filter_map(|raw| {
            ClaimCategory::parse(&raw.category).map(|category| ExtractedClaim {
                text: raw.text,
                category,
            })
        })
        .collect())
}

/// Pure-Rust heuristic extraction for use without an LLM.
///
/// Splits the reply on sentence boundaries and assigns the best-matching
/// category based on keyword counts. Claims that score zero on every
/// category are dropped.
#[must_use]
pub fn extract_heuristic(coach_reply: &str) -> Vec<ExtractedClaim> {
    let mut out = Vec::new();
    for sentence in split_sentences(coach_reply) {
        let trimmed = sentence.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(category) = classify_heuristic(trimmed) {
            out.push(ExtractedClaim {
                text: trimmed.to_owned(),
                category,
            });
        }
    }
    out
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len() {
        let ch = chars[i];
        current.push(ch);
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

fn classify_heuristic(sentence: &str) -> Option<ClaimCategory> {
    let lower = sentence.to_lowercase();
    let mut best: Option<(ClaimCategory, usize)> = None;

    let buckets: [(ClaimCategory, &[&str]); 6] = [
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
            ],
        ),
        (
            ClaimCategory::Nutrition,
            &[
                "protein",
                "carbohydrate",
                "carb",
                "calorie",
                "gram",
                "macro",
                "hydration",
                "electrolyte",
                "meal",
                "fueling",
                "fuelling",
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
                "pt ",
                "strain",
                "sprain",
            ],
        ),
    ];

    for (cat, keywords) in buckets {
        let score = keywords.iter().filter(|kw| lower.contains(*kw)).count();
        if score > 0 && best.is_none_or(|(_, b)| score > b) {
            best = Some((cat, score));
        }
    }

    best.map(|(c, _)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_sentence_boundaries() {
        let s = split_sentences("Run 5x1km. Take 2 min rest. You got this!");
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn classifies_nutrition_claim() {
        assert_eq!(
            classify_heuristic("Aim for 1.6g of protein per kg of body weight."),
            Some(ClaimCategory::Nutrition)
        );
    }

    #[test]
    fn classifies_physiological_claim() {
        assert_eq!(
            classify_heuristic("Your VO2max sits around 58 ml/kg/min."),
            Some(ClaimCategory::Physiological)
        );
    }

    #[test]
    fn classifies_supplement_claim() {
        assert_eq!(
            classify_heuristic("Creatine at 5g per day supports power output."),
            Some(ClaimCategory::Supplement)
        );
    }

    #[test]
    fn rejects_non_factual_sentence() {
        assert_eq!(classify_heuristic("You're crushing it!"), None);
    }

    #[test]
    fn extract_heuristic_returns_multiple_claims() {
        let reply = "Aim for 1.6g of protein per kg of body weight. Your VO2max is around 58. \
                     Creatine at 5g per day helps recovery. Nice work today!";
        let claims = extract_heuristic(reply);
        assert!(claims.len() >= 3);
    }

    #[test]
    fn extract_heuristic_empty_on_motivation() {
        let reply = "You're crushing it! Keep going! Amazing work!";
        assert!(extract_heuristic(reply).is_empty());
    }
}
