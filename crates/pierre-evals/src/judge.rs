// ABOUTME: LLM-as-judge invocation backed by pierre_llm::judge::ask_for_json
// ABOUTME: Rubric scoring for the eval harness plus the bullshit detector's LLM-judge claim layer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM-as-Judge Invocation
//!
//! Wraps the lifted [`pierre_llm::judge::ask_for_json`] helper with typed
//! verdicts so consumers don't have to redefine the JSON envelope. Two
//! distinct judges live here:
//!
//! - [`judge_response`] — rubric-based 1–5 scoring of a whole coach reply
//!   against the golden fixtures, used by the eval report layer.
//! - [`judge_claim`] — the bullshit detector's **LLM-judge** fallback. When
//!   the earlier layers cannot reach a confident verdict on a single claim, the
//!   claim is handed to the LLM with the retrieved evidence (if any) and
//!   asked to return a structured supported / contradicted / unverifiable
//!   judgement.

use std::fmt::Write as _;

use pierre_core::errors::AppResult;
use pierre_llm::judge::ask_for_json;
use pierre_llm::LlmProvider;
use pierre_memory::ClaimStatus;
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

/// JSON shape the LLM-judge claim layer is asked to return.
#[derive(Debug, Clone, Deserialize)]
struct RawClaimJudgement {
    verdict: String,
    confidence: f32,
    rationale: String,
}

/// A structured judgement for a single claim, produced by [`judge_claim`].
#[derive(Debug, Clone)]
pub struct ClaimJudgement {
    /// Mapped verdict status for the claim.
    pub status: ClaimStatus,
    /// Judge confidence in `[0.0, 1.0]`, clamped from the raw response.
    pub confidence: f32,
    /// Short free-text justification from the judge.
    pub rationale: String,
}

const CLAIM_JUDGE_SYSTEM_PROMPT: &str = r#"You are the final-stage fact checker in a sports-science
verification pipeline. Earlier deterministic layers could not reach a confident verdict on the
claim below, so you are the tiebreaker. Judge ONLY whether the claim is consistent with mainstream
sports-science and exercise-physiology consensus.

Return strict JSON of the form:

{"verdict":"<supported|contradicted|unverifiable>","confidence":<0.0-1.0>,"rationale":"<one sentence>"}

- "supported" — the claim aligns with well-established consensus.
- "contradicted" — the claim conflicts with well-established consensus.
- "unverifiable" — the claim is too vague, too speculative, or outside any consensus to judge.

Be conservative: prefer "unverifiable" over guessing. Use the exact verdict strings above."#;

/// Judge a single claim with the configured LLM provider — the bullshit
/// detector's LLM-judge fallback.
///
/// `evidence_context` carries any propositions the evidence-retrieval layer retrieved (possibly
/// empty) so the judge can ground its verdict instead of relying on parametric
/// memory alone. The raw verdict string is mapped to a [`ClaimStatus`];
/// anything the model returns outside the three known verdicts is treated as
/// [`ClaimStatus::Unverifiable`] so a drifty model never fabricates support.
///
/// # Errors
///
/// Returns the LLM call error if the judge fails or its response cannot be
/// parsed into the expected JSON envelope.
pub async fn judge_claim(
    provider: &dyn LlmProvider,
    claim_text: &str,
    evidence_context: &str,
) -> AppResult<ClaimJudgement> {
    let evidence_block = if evidence_context.trim().is_empty() {
        "No corpus evidence was retrieved for this claim.".to_owned()
    } else {
        format!("Retrieved evidence:\n{evidence_context}")
    };
    let user_prompt = format!(
        "## Claim\n{claim_text}\n\n## Context\n{evidence_block}\n\nReturn the JSON object only.",
    );

    let raw: RawClaimJudgement =
        ask_for_json(provider, CLAIM_JUDGE_SYSTEM_PROMPT, &user_prompt, 0.1).await?;

    let status = match raw.verdict.trim().to_lowercase().as_str() {
        "supported" => ClaimStatus::Supported,
        "contradicted" => ClaimStatus::Contradicted,
        _ => ClaimStatus::Unverifiable,
    };

    Ok(ClaimJudgement {
        status,
        confidence: raw.confidence.clamp(0.0, 1.0),
        rationale: raw.rationale,
    })
}
