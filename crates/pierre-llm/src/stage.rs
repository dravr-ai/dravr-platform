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
//! Then [`StageModels::checked_against`] holds each one to the head: a model
//! the head's published list leaves out puts its stage back on the head's own
//! model, with a warning naming the stage's variable. The head would refuse
//! that id as an unavailable model, which the chain treats as a provider fault,
//! so every call of the stage would be served by the next tier — a pooled
//! account — instead of the head. An empty list publishes nothing to check
//! against, and the configured model stands.
//!
//! What a stage model never reaches:
//!
//! - **A tier behind the head.** The runtime chain's strict policy clears
//!   `request.model` on every hop (embacle `ResponsePolicy::strict`), so a stage
//!   call that falls back — a provider fault, an empty completion, a head the
//!   guard passes over — runs on that tier's own model. The ids are the head's
//!   namespace; `claude-haiku-5.5` means nothing to Gemini.
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

/// The model every [`LlmStage`] defaults to when the head is `copilot_sdk`:
/// Copilot's id for Claude Haiku 5.5.
///
/// Haiku is Copilot's cheapest Claude tier — Haiku 4.5 billed 0.33 of a
/// premium request per turn against 1.0 for Sonnet (measured 2026-09-18) — and
/// every stage is a machine-read call, so the reply the athlete reads stays on
/// the main model. Copilot ids spell the version with a dot, as the head's own
/// `claude-sonnet-5.5` does. A head whose published list leaves the id out
/// keeps every stage on its own model ([`StageModels::checked_against`]). Set
/// `PIERRE_LLM_MODEL_<STAGE>` to move one stage, or to `inherit` to keep it on
/// the main model.
pub const COPILOT_SDK_BACKGROUND_MODEL: &str = "claude-haiku-5.5";

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
    /// Boxed because every provider carries one, routed or not: inline, the
    /// six slots would grow `ChatProvider::Embacle` by 144 bytes.
    models: Box<[Option<String>; LlmStage::ALL.len()]>,
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

    /// These stage models held to `head`: each model the head publishes, or
    /// that it cannot be checked against, stays; each one the head's published
    /// list leaves out is dropped, so its stage runs on the head's own model.
    ///
    /// Dropped rather than kept with a warning: the head refuses a model it
    /// cannot serve as an unavailable model, which the chain treats as a
    /// provider fault, so every call of that stage would move to the next tier
    /// — a pooled account on its own model — while the head sat idle. An empty
    /// list is a provider that publishes none, and the configured model is
    /// trusted. Logged once per stage, here, when the head is built.
    #[must_use]
    pub fn checked_against(self, head: &(impl LlmProvider + ?Sized)) -> Self {
        self.keep_published(head.name(), head.default_model(), head.available_models())
    }

    /// [`Self::checked_against`] over the head's name, own model and published
    /// list.
    fn keep_published(mut self, provider: &str, head_model: &str, available: &[String]) -> Self {
        for stage in LlmStage::ALL {
            let slot = &mut self.models[stage.index()];
            let Some(model) = slot.take() else {
                continue;
            };
            if available.is_empty() || available.contains(&model) {
                info!(
                    provider,
                    stage = stage.call_type(),
                    model = %model,
                    "LLM stage routed to its own model"
                );
                *slot = Some(model);
            } else {
                warn!(
                    provider,
                    stage = stage.call_type(),
                    model = %model,
                    env_var = stage.env_var(),
                    head_model,
                    "LLM stage model is not in the head provider's published list, so the \
                     stage runs on the head's own model — set the variable to a published id, \
                     or to `inherit` to silence this"
                );
            }
        }
        self
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
                Some(COPILOT_SDK_BACKGROUND_MODEL),
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
            Some(COPILOT_SDK_BACKGROUND_MODEL)
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
            Some(COPILOT_SDK_BACKGROUND_MODEL)
        );
    }

    fn published(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn a_stage_model_the_head_does_not_publish_runs_on_the_head_model() {
        let stages = StageModels::resolve(
            Some(LlmProviderType::CopilotSdk),
            lookup(&[("PIERRE_LLM_MODEL_CLAIM_JUDGE", "gpt-5-mini")]),
        )
        .keep_published(
            "copilot_sdk",
            "claude-sonnet-5.5",
            &published(&["claude-sonnet-5.5", "gpt-5-mini"]),
        );

        assert_eq!(
            stages.model_for(LlmStage::ClaimJudge),
            Some("gpt-5-mini"),
            "a published override is kept"
        );
        for stage in LlmStage::ALL {
            if stage != LlmStage::ClaimJudge {
                assert_eq!(
                    stages.model_for(stage),
                    None,
                    "{stage:?}: an unpublished id stays off the head, so the call is never \
                     refused onto the next tier"
                );
            }
        }
    }

    #[test]
    fn a_published_stage_model_is_kept_and_an_empty_list_trusts_the_configuration() {
        for available in [
            published(&["claude-sonnet-5.5", COPILOT_SDK_BACKGROUND_MODEL]),
            Vec::new(),
        ] {
            let stages = StageModels::resolve(Some(LlmProviderType::CopilotSdk), lookup(&[]))
                .keep_published("copilot_sdk", "claude-sonnet-5.5", &available);
            for stage in LlmStage::ALL {
                assert_eq!(
                    stages.model_for(stage),
                    Some(COPILOT_SDK_BACKGROUND_MODEL),
                    "{stage:?} against {available:?}"
                );
            }
        }
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
