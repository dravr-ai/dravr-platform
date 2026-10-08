// ABOUTME: Which model each background LLM stage runs on — the per-stage routing seam, resolved once at boot
// ABOUTME: Bound to the head provider it was resolved for; a tenant's own provider never carries one
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Stage routing
//!
//! A turn makes more LLM calls than the reply the athlete reads: the turn's
//! language, the facts worth remembering, a summary of old history, a verdict
//! on each claim, the advice to follow up, and later whether that advice
//! worked. None of them is read by the athlete, and each is a separate
//! Copilot session billed at its model's multiplier, so each can run on a
//! cheaper model than the reply without the athlete reading the difference.
//! "A match statement, not a classifier": the stage a call serves decides its
//! model.
//!
//! [`StageModels::resolve`] decides, once at boot, per stage:
//!
//! 1. `PIERRE_LLM_MODEL_<STAGE>` when it is set: that model, or the head's own
//!    model when it says `inherit`.
//! 2. Otherwise the stage's default for the head's provider:
//!    [`COPILOT_SDK_BACKGROUND_MODEL`] for every stage on `copilot_sdk`.
//! 3. Otherwise nothing: the call runs on the head's model (`PIERRE_LLM_MODEL`),
//!    exactly as it did before stages existed.
//!
//! What a stage model never reaches:
//!
//! - **A tier behind the head.** The runtime chain's strict policy clears
//!   `request.model` on every hop (embacle `ResponsePolicy::strict`), so a stage
//!   call that falls back — a provider fault, an empty completion, a head the
//!   guard passes over — runs on that tier's own model. The ids are the head's
//!   namespace; `claude-haiku-4.5` means nothing to Gemini.
//! - **A tenant's own provider.** A BYO key builds its provider from the
//!   tenant's credential, never through [`ChatProvider::from_env`], so it
//!   carries no stage models and every call it serves runs on its own model.
//! - **The athlete-facing reply.** The draft, the re-asks that rewrite it and
//!   the visual-block repair run on the turn's active model; they are not
//!   stages.
//!
//! [`ChatProvider::from_env`]: crate::ChatProvider::from_env

use std::env;

use tracing::{info, warn};

use crate::config::LlmProviderType;
use crate::LlmProvider;

/// The model every [`LlmStage`] defaults to when the head is `copilot_sdk`.
///
/// Haiku 4.5 bills 0.33 of a premium request per Copilot turn against 1.0 for
/// Sonnet (measured 2026-09-18), it is in the Copilot catalogue the SDK
/// validates against, and every stage is a machine-read call — the reply the
/// athlete reads stays on the main model. Set `PIERRE_LLM_MODEL_<STAGE>` to
/// move one stage, or to `inherit` to keep it on the main model.
pub const COPILOT_SDK_BACKGROUND_MODEL: &str = "claude-haiku-4.5";

/// A stage's environment value that keeps the stage on the head's own model.
const INHERIT: &str = "inherit";

/// The model every stage runs on when its variable is unset and the head is
/// `head`: [`COPILOT_SDK_BACKGROUND_MODEL`] on `copilot_sdk`, the head's own
/// model everywhere else.
///
/// One answer for every stage: each is a machine-read call, and no stage has
/// yet earned a model of its own.
fn background_default(head: LlmProviderType) -> Option<&'static str> {
    matches!(head, LlmProviderType::CopilotSdk).then_some(COPILOT_SDK_BACKGROUND_MODEL)
}

/// An LLM call the platform makes beside the reply, whose model can be chosen
/// apart from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmStage {
    /// Which of the five platform languages the athlete's message is in.
    LanguageClassification,
    /// The durable facts a finished turn taught about the athlete.
    MemoryExtraction,
    /// The summary that replaces a conversation's oldest turns.
    CompactionSummary,
    /// The verdict on one claim the reply makes.
    ClaimJudge,
    /// The checkable advice a reply gave, held until its outcome is known.
    AdviceCapture,
    /// Whether advice worked, once its observation window has closed.
    OutcomeEvaluation,
}

impl LlmStage {
    /// Every stage, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::LanguageClassification,
        Self::MemoryExtraction,
        Self::CompactionSummary,
        Self::ClaimJudge,
        Self::AdviceCapture,
        Self::OutcomeEvaluation,
    ];

    /// The `call_type` of the `llm_usage` row this stage's calls write, so its
    /// cost is counted apart from the reply's.
    #[must_use]
    pub const fn call_type(self) -> &'static str {
        match self {
            Self::LanguageClassification => "language_classification",
            Self::MemoryExtraction => "memory_extraction",
            Self::CompactionSummary => "compaction_summary",
            Self::ClaimJudge => "claim_judge",
            Self::AdviceCapture => "advice_capture",
            Self::OutcomeEvaluation => "outcome_eval",
        }
    }

    /// The environment variable naming this stage's model.
    #[must_use]
    pub const fn env_var(self) -> &'static str {
        match self {
            Self::LanguageClassification => "PIERRE_LLM_MODEL_LANGUAGE_CLASSIFICATION",
            Self::MemoryExtraction => "PIERRE_LLM_MODEL_MEMORY_EXTRACTION",
            Self::CompactionSummary => "PIERRE_LLM_MODEL_COMPACTION_SUMMARY",
            Self::ClaimJudge => "PIERRE_LLM_MODEL_CLAIM_JUDGE",
            Self::AdviceCapture => "PIERRE_LLM_MODEL_ADVICE_CAPTURE",
            Self::OutcomeEvaluation => "PIERRE_LLM_MODEL_OUTCOME_EVAL",
        }
    }

    /// Position in [`Self::ALL`], and in [`StageModels`]'s table.
    const fn index(self) -> usize {
        match self {
            Self::LanguageClassification => 0,
            Self::MemoryExtraction => 1,
            Self::CompactionSummary => 2,
            Self::ClaimJudge => 3,
            Self::AdviceCapture => 4,
            Self::OutcomeEvaluation => 5,
        }
    }
}

/// The model each [`LlmStage`] runs on, for one head provider.
///
/// `None` for a stage is the head's own model. Empty for every provider not
/// built by [`ChatProvider::from_env`](crate::ChatProvider::from_env) — a
/// tenant's own key, a test double — so the routing can only ever reach the
/// head it was resolved for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageModels {
    models: [Option<String>; LlmStage::ALL.len()],
}

impl StageModels {
    /// Every stage on the head's own model.
    #[must_use]
    pub fn inherit_all() -> Self {
        Self::default()
    }

    /// Resolve every stage for a head of kind `head`, reading each stage's
    /// variable through `lookup`.
    ///
    /// `head` is `None` for a provider of no known kind, which gets no
    /// defaults. A blank variable reads as unset, as everywhere else in the
    /// LLM configuration.
    #[must_use]
    pub fn resolve(head: Option<LlmProviderType>, lookup: impl Fn(&str) -> Option<String>) -> Self {
        let mut stages = Self::default();
        for stage in LlmStage::ALL {
            let configured = lookup(stage.env_var())
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty());
            stages.models[stage.index()] = match configured {
                Some(value) if value.eq_ignore_ascii_case(INHERIT) => None,
                Some(value) => Some(value),
                None => head.and_then(background_default).map(str::to_owned),
            };
        }
        stages
    }

    /// [`Self::resolve`] over the process environment.
    #[must_use]
    pub fn from_env(head: Option<LlmProviderType>) -> Self {
        Self::resolve(head, |key| env::var(key).ok())
    }

    /// The model `stage` runs on, or `None` for the head's own model.
    #[must_use]
    pub fn model_for(&self, stage: LlmStage) -> Option<&str> {
        self.models[stage.index()].as_deref()
    }

    /// Log each stage's model, and warn for one the head does not publish.
    ///
    /// A warning, not a refusal, as for the head's own model: the published
    /// list can lag the vendor's catalogue. A stage model the head cannot serve
    /// is refused at call time as an unavailable model, which the chain treats
    /// as a provider fault, so the call moves to the next tier on that tier's
    /// own model — it is served, but not where it was routed.
    pub fn report_against(&self, head: &(impl LlmProvider + ?Sized)) {
        let available = head.available_models();
        for stage in LlmStage::ALL {
            let Some(model) = self.model_for(stage) else {
                continue;
            };
            if available.is_empty() || available.iter().any(|m| m == model) {
                info!(
                    provider = head.name(),
                    stage = stage.call_type(),
                    model,
                    "LLM stage routed to its own model"
                );
            } else {
                warn!(
                    provider = head.name(),
                    stage = stage.call_type(),
                    model,
                    env_var = stage.env_var(),
                    "LLM stage model is not in the head provider's published list — a call the \
                     provider refuses for it falls back to the next tier's own model"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| vars.get(key).cloned()
    }

    #[test]
    fn an_unset_stage_inherits_the_head_model_off_copilot_sdk() {
        for head in [
            None,
            Some(LlmProviderType::CopilotHeadless),
            Some(LlmProviderType::ClaudeCode),
            Some(LlmProviderType::Gemini),
        ] {
            let stages = StageModels::resolve(head, lookup(&[]));
            for stage in LlmStage::ALL {
                assert_eq!(stages.model_for(stage), None, "{stage:?} on {head:?}");
            }
        }
    }

    #[test]
    fn every_background_stage_defaults_to_haiku_on_copilot_sdk() {
        let stages = StageModels::resolve(Some(LlmProviderType::CopilotSdk), lookup(&[]));
        for stage in LlmStage::ALL {
            assert_eq!(
                stages.model_for(stage),
                Some("claude-haiku-4.5"),
                "{stage:?}"
            );
        }
    }

    #[test]
    fn a_set_stage_overrides_the_default_and_touches_no_other_stage() {
        let stages = StageModels::resolve(
            Some(LlmProviderType::CopilotSdk),
            lookup(&[("PIERRE_LLM_MODEL_CLAIM_JUDGE", "gpt-5-mini")]),
        );
        assert_eq!(stages.model_for(LlmStage::ClaimJudge), Some("gpt-5-mini"));
        assert_eq!(
            stages.model_for(LlmStage::MemoryExtraction),
            Some("claude-haiku-4.5")
        );

        let off_sdk = StageModels::resolve(
            Some(LlmProviderType::ClaudeCode),
            lookup(&[("PIERRE_LLM_MODEL_COMPACTION_SUMMARY", "haiku")]),
        );
        assert_eq!(
            off_sdk.model_for(LlmStage::CompactionSummary),
            Some("haiku")
        );
        assert_eq!(off_sdk.model_for(LlmStage::ClaimJudge), None);
    }

    #[test]
    fn inherit_keeps_a_stage_on_the_head_model_and_blank_reads_as_unset() {
        let stages = StageModels::resolve(
            Some(LlmProviderType::CopilotSdk),
            lookup(&[
                ("PIERRE_LLM_MODEL_MEMORY_EXTRACTION", " Inherit "),
                ("PIERRE_LLM_MODEL_OUTCOME_EVAL", "   "),
            ]),
        );
        assert_eq!(stages.model_for(LlmStage::MemoryExtraction), None);
        assert_eq!(
            stages.model_for(LlmStage::OutcomeEvaluation),
            Some("claude-haiku-4.5")
        );
    }

    #[test]
    fn every_stage_has_its_own_call_type_and_variable() {
        let mut call_types: Vec<_> = LlmStage::ALL.into_iter().map(LlmStage::call_type).collect();
        let mut env_vars: Vec<_> = LlmStage::ALL.into_iter().map(LlmStage::env_var).collect();
        call_types.sort_unstable();
        call_types.dedup();
        env_vars.sort_unstable();
        env_vars.dedup();
        assert_eq!(call_types.len(), LlmStage::ALL.len());
        assert_eq!(env_vars.len(), LlmStage::ALL.len());
        for (position, stage) in LlmStage::ALL.into_iter().enumerate() {
            assert_eq!(stage.index(), position);
        }
    }
}
