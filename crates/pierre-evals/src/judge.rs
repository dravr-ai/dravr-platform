// ABOUTME: LLM-as-judge invocation backed by pierre_llm::judge::ask_for_json
// ABOUTME: Layer 2 of the eval harness — rubric-based 1-5 scoring with text rationale
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM-as-Judge Invocation
//!
//! Wraps the lifted [`pierre_llm::judge::ask_for_json`] helper with a typed
//! [`JudgeVerdict`] so eval consumers don't have to redefine the JSON
//! envelope. Verdicts include both a numeric score (1–5) and a short
//! free-text justification the report layer can pretty-print.

use std::fmt::Write as _;

use pierre_core::errors::AppResult;
use pierre_llm::judge::ask_for_json;
use pierre_llm::LlmProvider;
use serde::{Deserialize, Serialize};

use crate::rubrics::Rubric;

#[derive(Deserialize)]
struct RawScores {
    scores: Vec<RawScore>,
}

#[derive(Deserialize)]
struct RawScore {
    rubric: String,
    score: u8,
    rationale: String,
}

/// Score returned by the judge for a single rubric.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RubricScore {
    /// Rubric this score corresponds to.
    pub rubric_name: String,
    /// Numeric score in `[1, 5]`.
    pub score: u8,
    /// Short justification from the judge.
    pub rationale: String,
    /// Whether the score met the rubric's `passing_score` threshold.
    pub passed: bool,
}

/// Top-level verdict returned by [`judge_response`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeVerdict {
    /// Per-rubric scores in the same order they were requested.
    pub rubrics: Vec<RubricScore>,
}

impl JudgeVerdict {
    /// True only if every rubric in the verdict passed.
    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.rubrics.iter().all(|r| r.passed)
    }

    /// Average score across rubrics, or 0.0 for an empty verdict.
    #[must_use]
    pub fn mean_score(&self) -> f32 {
        if self.rubrics.is_empty() {
            return 0.0;
        }
        let sum: u32 = self.rubrics.iter().map(|r| u32::from(r.score)).sum();
        #[allow(clippy::cast_precision_loss)]
        {
            sum as f32 / self.rubrics.len() as f32
        }
    }
}

/// Score one coach response against the given rubrics using the configured LLM provider.
///
/// # Errors
///
/// Returns the LLM call error if the judge fails. Score parsing falls back
/// to 1 (failing) when the LLM emits a value outside `1..=5`, so callers
/// always get a verdict back even from drifty models.
pub async fn judge_response(
    provider: &dyn LlmProvider,
    rubrics: &[Rubric],
    user_message: &str,
    expected_response: &str,
    actual_response: &str,
) -> AppResult<JudgeVerdict> {
    let system_prompt = build_system_prompt(rubrics);
    let user_prompt = format!(
        "## User message\n{user_message}\n\n## Expected coach reply\n{expected_response}\n\n## Actual coach reply\n{actual_response}\n\nReturn the JSON object only.",
    );

    let raw: RawScores = ask_for_json(provider, &system_prompt, &user_prompt, 0.2).await?;

    let scored = rubrics
        .iter()
        .map(|rubric| {
            let (score, rationale) = raw
                .scores
                .iter()
                .find(|s| s.rubric == rubric.name)
                .map_or_else(
                    || (1_u8, String::from("missing in judge response")),
                    |s| (s.score.clamp(1, 5), s.rationale.clone()),
                );
            RubricScore {
                rubric_name: rubric.name.clone(),
                score,
                rationale,
                passed: score >= rubric.passing_score,
            }
        })
        .collect();

    Ok(JudgeVerdict { rubrics: scored })
}

/// Build the system prompt that pins the judge's output format.
fn build_system_prompt(rubrics: &[Rubric]) -> String {
    let mut s = String::from(
        "You are an evaluation judge for a fitness coaching assistant. \
         You will receive a user message, the expected coach reply, and the actual coach reply. \
         Score the actual reply against the rubrics below on a 1–5 scale and return JSON of the form:\n\n\
         {\"scores\": [{\"rubric\": \"<name>\", \"score\": <1-5>, \"rationale\": \"<one sentence>\"}]}\n\n\
         Use the exact rubric names in your output. Do not invent additional rubrics.\n\n## Rubrics\n\n",
    );
    for rubric in rubrics {
        let _ = writeln!(
            s,
            "- **{}** ({}): {}",
            rubric.name, rubric.dimension, rubric.instructions
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{JudgeVerdict, RubricScore};

    fn score(name: &str, n: u8, passed: bool) -> RubricScore {
        RubricScore {
            rubric_name: name.into(),
            score: n,
            rationale: "test".into(),
            passed,
        }
    }

    #[test]
    fn mean_score_handles_empty() {
        let v = JudgeVerdict { rubrics: vec![] };
        assert!((v.mean_score() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mean_score_average() {
        let v = JudgeVerdict {
            rubrics: vec![score("a", 3, true), score("b", 5, true)],
        };
        assert!((v.mean_score() - 4.0).abs() < f32::EPSILON);
    }

    #[test]
    fn all_passed_requires_every_rubric() {
        let pass = JudgeVerdict {
            rubrics: vec![score("a", 4, true), score("b", 5, true)],
        };
        assert!(pass.all_passed());
        let fail = JudgeVerdict {
            rubrics: vec![score("a", 4, true), score("b", 2, false)],
        };
        assert!(!fail.all_passed());
    }
}
