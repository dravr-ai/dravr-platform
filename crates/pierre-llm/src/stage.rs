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
//! worked — and, outside any turn, the ingestion job's transcription of a
//! fuel label. None of them is read by the athlete, and each is a separate
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
//! 3. Otherwise nothing: the call runs on the head's model (its
//!    `PIERRE_LLM_CHAIN` entry), exactly as it did before stages existed.
//!
//! Then [`StageModels::checked_against`] holds each one to the head: a model
//! the head's published list leaves out keeps its stage on the head's own
//! model until the provider publishes that id, with a warning naming the
//! stage's variable. The published list is the only catalogue known at boot —
//! on `copilot_sdk` it is embacle's static Copilot catalogue, which changes
//! only with an embacle release — and an id outside it risks the head refusing
//! it as an unavailable model, which the chain treats as a provider fault, so
//! every call of the stage would be served by the next tier — a pooled
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
//!   tenant's credential, never through [`ChatProvider::from_chain`], so it
//!   carries no stage models and every call it serves runs on its own model.
//! - **The athlete-facing reply.** The draft, the re-asks that rewrite it and
//!   the visual-block repair run on the turn's active model; they are not
//!   stages.
//!
//! The turn's own tool loop is routed the same way, through one more slot:
//! [`TOOL_ITERATION_ENV`] moves the iterations that only pick the next tool
//! onto a cheaper model on a head that bills per call, defaulting to
//! [`ANTHROPIC_TOOL_ITERATION_MODEL`] on `anthropic`, while the reply stays on
//! the turn's model.
//!
//! [`ChatProvider::from_chain`]: crate::ChatProvider::from_chain

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
/// `claude-sonnet-5.5` does. Embacle's Copilot catalogue publishes this id
/// since 0.35.0; a head whose published list leaves it out keeps every stage
/// on its own model ([`StageModels::checked_against`]). Set
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

/// The model a turn's tool-calling iterations default to when the head is
/// `anthropic`: the API's id for Claude Haiku 5.5, which the Anthropic
/// provider publishes.
///
/// The Anthropic API bills every call by its tokens, so an iteration that only
/// picks the next tool costs what its model costs; Haiku sits well below
/// Sonnet and Opus per token. The reply the athlete reads is still written by
/// the turn's own model ([`TOOL_ITERATION_ENV`] has the contract). Anthropic
/// ids spell the version with dashes. A turn whose own model is already this
/// one routes nothing.
pub const ANTHROPIC_TOOL_ITERATION_MODEL: &str = "claude-haiku-5-5";

/// The variable naming the model a turn's tool-calling iterations run on.
///
/// Read only by the platform's own tool loop — the one a head with native
/// function calling (Anthropic, Gemini) runs, where each iteration is its own
/// call. Each iteration runs on this model; when one answers without a tool
/// call, that answer is billed and discarded and the turn's own model is
/// asked again on the same messages, so the reply is always the turn's model's
/// own. A Copilot head drops it at boot with a warning: its runtime serves
/// the whole tool loop as one session billed per turn, so there is no
/// per-iteration call to route and nothing a cheaper model would save.
/// `inherit` keeps every iteration on the turn's model.
pub const TOOL_ITERATION_ENV: &str = "PIERRE_LLM_MODEL_TOOL_ITERATION";

/// The stage label the tool-iteration model is logged under at boot.
const TOOL_ITERATION_STAGE: &str = "tool_iteration";

/// The model a turn's tool-calling iterations run on when
/// [`TOOL_ITERATION_ENV`] is unset and the head is `head`:
/// [`ANTHROPIC_TOOL_ITERATION_MODEL`] on `anthropic`, the turn's own model
/// everywhere else.
fn tool_iteration_default(head: LlmProviderType) -> Option<&'static str> {
    matches!(head, LlmProviderType::Anthropic).then_some(ANTHROPIC_TOOL_ITERATION_MODEL)
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
    /// A fuel product's label facts, transcribed from its photos by the
    /// ingestion job (carnet#155). Needs a head that reads images.
    FuelPanelExtraction,
}

impl LlmStage {
    /// Every stage, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::LanguageClassification,
        Self::MemoryExtraction,
        Self::CompactionSummary,
        Self::ClaimJudge,
        Self::AdviceCapture,
        Self::OutcomeEvaluation,
        Self::FuelPanelExtraction,
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
            Self::FuelPanelExtraction => "fuel_panel_extraction",
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
            Self::FuelPanelExtraction => "PIERRE_LLM_MODEL_FUEL_PANEL_EXTRACTION",
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
            Self::FuelPanelExtraction => 6,
        }
    }
}

/// The model each [`LlmStage`] runs on, for one head provider.
///
/// `None` for a stage is the head's own model. Empty for every provider not
/// built by [`ChatProvider::from_chain`](crate::ChatProvider::from_chain) — a
/// tenant's own key, a test double — so the routing can only ever reach the
/// head it was resolved for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageModels {
    /// Boxed because every provider carries one, routed or not: inline, the
    /// seven slots would grow `ChatProvider::Embacle` by 168 bytes.
    models: Box<[Option<String>; LlmStage::ALL.len()]>,
    /// The model a turn's tool-calling iterations run on
    /// ([`TOOL_ITERATION_ENV`]), `None` for the turn's own model.
    tool_iteration: Option<String>,
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
            stages.models[stage.index()] =
                configured_model(&lookup, stage.env_var(), head.and_then(background_default));
        }
        stages.tool_iteration = configured_model(
            &lookup,
            TOOL_ITERATION_ENV,
            head.and_then(tool_iteration_default),
        );
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

    /// The model a turn's tool-calling iterations run on, or `None` for the
    /// turn's own model.
    #[must_use]
    pub fn tool_iteration_model(&self) -> Option<&str> {
        self.tool_iteration.as_deref()
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
    ///
    /// The tool-iteration model is also dropped on a head without native
    /// function calling: that head serves a turn's whole tool loop as one
    /// session, so the platform loop that reads it never runs there.
    #[must_use]
    pub fn checked_against(self, head: &(impl LlmProvider + ?Sized)) -> Self {
        self.for_tool_loop(head.name(), head.capabilities().supports_function_calling())
            .keep_published(head.name(), head.default_model(), head.available_models())
    }

    /// These stage models for a head whose turns run the platform's own tool
    /// loop when `per_call` is set; otherwise without a tool-iteration model,
    /// with a warning naming its variable when one was configured.
    fn for_tool_loop(mut self, provider: &str, per_call: bool) -> Self {
        if per_call {
            return self;
        }
        if let Some(model) = self.tool_iteration.take() {
            warn!(
                provider,
                model = %model,
                env_var = TOOL_ITERATION_ENV,
                "LLM tool-iteration model ignored: this head serves a turn's whole tool loop as \
                 one session on the turn's model, so there is no iteration to route — unset the \
                 variable or set it to `inherit` to silence this"
            );
        }
        self
    }

    /// [`Self::checked_against`] over the head's name, own model and published
    /// list.
    fn keep_published(mut self, provider: &str, head_model: &str, available: &[String]) -> Self {
        let head = PublishedHead {
            provider,
            head_model,
            available,
        };
        for stage in LlmStage::ALL {
            head.keep(
                &mut self.models[stage.index()],
                stage.call_type(),
                stage.env_var(),
            );
        }
        head.keep(
            &mut self.tool_iteration,
            TOOL_ITERATION_STAGE,
            TOOL_ITERATION_ENV,
        );
        self
    }
}

/// The value of `env_var`, trimmed: `None` for `inherit`, the variable's
/// model when one is set, `default` when it is unset or blank.
fn configured_model(
    lookup: &impl Fn(&str) -> Option<String>,
    env_var: &str,
    default: Option<&str>,
) -> Option<String> {
    let configured = lookup(env_var)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    match configured {
        Some(value) if value.eq_ignore_ascii_case(INHERIT) => None,
        Some(value) => Some(value),
        None => default.map(str::to_owned),
    }
}

/// What a head publishes, for holding each routed model to it.
struct PublishedHead<'a> {
    provider: &'a str,
    head_model: &'a str,
    available: &'a [String],
}

impl PublishedHead<'_> {
    /// Keep `slot`'s model when the head publishes it (or publishes nothing to
    /// check against), and drop it with a warning naming `env_var` otherwise.
    fn keep(&self, slot: &mut Option<String>, stage: &str, env_var: &str) {
        let Some(model) = slot.take() else {
            return;
        };
        if self.available.is_empty() || self.available.contains(&model) {
            info!(
                provider = self.provider,
                stage,
                model = %model,
                "LLM stage routed to its own model"
            );
            *slot = Some(model);
        } else {
            warn!(
                provider = self.provider,
                stage,
                model = %model,
                env_var,
                head_model = self.head_model,
                "LLM stage model is not in the head provider's published list, so the \
                 stage stays on the head's own model until the provider publishes this id \
                 — set the variable to a published id, or to `inherit` to silence this"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::fmt::Debug;
    use std::sync::{Arc, Mutex, PoisonError};

    use tracing::callsite::rebuild_interest_cache;
    use tracing::field::{Field, Visit};
    use tracing::subscriber::with_default;
    use tracing::{Event, Level, Subscriber};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::Registry;

    use super::*;

    /// The fields of every WARN event logged while the layer is installed.
    #[derive(Clone, Default)]
    struct Warnings(Arc<Mutex<Vec<BTreeMap<String, String>>>>);

    struct FieldText<'a>(&'a mut BTreeMap<String, String>);

    impl Visit for FieldText<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
    }

    impl<S: Subscriber> Layer<S> for Warnings {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            if *event.metadata().level() == Level::WARN {
                let mut fields = BTreeMap::new();
                event.record(&mut FieldText(&mut fields));
                self.0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(fields);
            }
        }
    }

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

    /// Held by every test that reaches `keep_published`'s log lines: tracing
    /// caches each callsite's interest, and a test reaching one with no
    /// subscriber while another installs its own can cache it as never
    /// interested, so the capturing test would see no warning.
    static LOG_CALLSITES: Mutex<()> = Mutex::new(());

    /// `stages` held to a head, with the log callsites to itself.
    fn held(
        stages: StageModels,
        provider: &str,
        head_model: &str,
        available: &[String],
    ) -> StageModels {
        let _callsites = LOG_CALLSITES.lock().unwrap_or_else(PoisonError::into_inner);
        stages.keep_published(provider, head_model, available)
    }

    #[test]
    fn a_stage_model_the_head_does_not_publish_runs_on_the_head_model() {
        let stages = held(
            StageModels::resolve(
                Some(LlmProviderType::CopilotSdk),
                lookup(&[("PIERRE_LLM_MODEL_CLAIM_JUDGE", "gpt-5-mini")]),
            ),
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

    /// The boot log is the only place an operator learns a stage was held to
    /// the head: one warning per dropped stage, naming its variable and the
    /// model it stays on until the provider publishes the id.
    #[test]
    fn each_dropped_stage_warns_once_naming_its_variable_and_the_head_model() {
        let warnings = Warnings::default();
        let callsites = LOG_CALLSITES.lock().unwrap_or_else(PoisonError::into_inner);
        let stages = with_default(Registry::default().with(warnings.clone()), || {
            // An earlier test may have reached the warning with no subscriber,
            // caching the callsite as never interested; rebuild the cache now
            // that this thread's subscriber is installed.
            rebuild_interest_cache();
            StageModels::resolve(
                Some(LlmProviderType::CopilotSdk),
                lookup(&[("PIERRE_LLM_MODEL_CLAIM_JUDGE", "gpt-5-mini")]),
            )
            .keep_published(
                "copilot_sdk",
                "claude-sonnet-5.5",
                &published(&["claude-sonnet-5.5", "gpt-5-mini"]),
            )
        });
        drop(callsites);

        assert_eq!(stages.model_for(LlmStage::ClaimJudge), Some("gpt-5-mini"));
        let logged = warnings.0.lock().unwrap_or_else(PoisonError::into_inner);
        let dropped: Vec<LlmStage> = LlmStage::ALL
            .into_iter()
            .filter(|stage| *stage != LlmStage::ClaimJudge)
            .collect();
        assert_eq!(logged.len(), dropped.len(), "{logged:?}");
        for (stage, fields) in dropped.into_iter().zip(logged.iter()) {
            let field = |name: &str| fields.get(name).map(String::as_str);
            assert_eq!(field("stage"), Some(stage.call_type()), "{fields:?}");
            assert_eq!(field("env_var"), Some(stage.env_var()), "{fields:?}");
            assert_eq!(field("model"), Some(COPILOT_SDK_BACKGROUND_MODEL));
            assert_eq!(field("head_model"), Some("claude-sonnet-5.5"));
            assert!(
                field("message")
                    .is_some_and(|message| message.contains("until the provider publishes")),
                "{fields:?}"
            );
        }
    }

    #[test]
    fn a_published_stage_model_is_kept_and_an_empty_list_trusts_the_configuration() {
        for available in [
            published(&["claude-sonnet-5.5", COPILOT_SDK_BACKGROUND_MODEL]),
            Vec::new(),
        ] {
            let stages = held(
                StageModels::resolve(Some(LlmProviderType::CopilotSdk), lookup(&[])),
                "copilot_sdk",
                "claude-sonnet-5.5",
                &available,
            );
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

    #[test]
    fn tool_iterations_default_to_haiku_on_anthropic_and_to_the_turns_model_elsewhere() {
        let anthropic = StageModels::resolve(Some(LlmProviderType::Anthropic), lookup(&[]));
        assert_eq!(
            anthropic.tool_iteration_model(),
            Some(ANTHROPIC_TOOL_ITERATION_MODEL)
        );
        for stage in LlmStage::ALL {
            assert_eq!(
                anthropic.model_for(stage),
                None,
                "{stage:?}: the iteration default moves no background stage"
            );
        }
        for head in [
            None,
            Some(LlmProviderType::CopilotSdk),
            Some(LlmProviderType::CopilotHeadless),
            Some(LlmProviderType::ClaudeCode),
            Some(LlmProviderType::Gemini),
        ] {
            assert_eq!(
                StageModels::resolve(head, lookup(&[])).tool_iteration_model(),
                None,
                "{head:?}"
            );
        }
    }

    #[test]
    fn the_tool_iteration_variable_overrides_inherits_and_is_held_to_the_head() {
        let set = StageModels::resolve(
            Some(LlmProviderType::Gemini),
            lookup(&[(TOOL_ITERATION_ENV, " gemini-flash-lite-latest ")]),
        );
        assert_eq!(set.tool_iteration_model(), Some("gemini-flash-lite-latest"));
        let inherit = StageModels::resolve(
            Some(LlmProviderType::Anthropic),
            lookup(&[(TOOL_ITERATION_ENV, "INHERIT")]),
        );
        assert_eq!(inherit.tool_iteration_model(), None);

        let warnings = Warnings::default();
        let callsites = LOG_CALLSITES.lock().unwrap_or_else(PoisonError::into_inner);
        let unpublished = with_default(Registry::default().with(warnings.clone()), || {
            // As in the stage warning test: rebuild the callsite cache under
            // this thread's subscriber.
            rebuild_interest_cache();
            StageModels::resolve(Some(LlmProviderType::Anthropic), lookup(&[])).keep_published(
                "anthropic",
                "claude-sonnet-5-5",
                &published(&["claude-sonnet-5-5"]),
            )
        });
        drop(callsites);
        assert_eq!(
            unpublished.tool_iteration_model(),
            None,
            "an iteration model the head does not publish stays on the turn's model"
        );
        let logged = warnings.0.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(logged.len(), 1, "{logged:?}");
        let field = |name: &str| logged[0].get(name).map(String::as_str);
        assert_eq!(field("stage"), Some(TOOL_ITERATION_STAGE));
        assert_eq!(field("env_var"), Some(TOOL_ITERATION_ENV));
        assert_eq!(field("model"), Some(ANTHROPIC_TOOL_ITERATION_MODEL));
        assert_eq!(field("head_model"), Some("claude-sonnet-5-5"));
    }

    #[test]
    fn a_head_without_native_function_calling_drops_the_tool_iteration_model() {
        let set = StageModels::resolve(
            Some(LlmProviderType::CopilotSdk),
            lookup(&[(TOOL_ITERATION_ENV, "claude-haiku-5.5")]),
        );
        assert_eq!(set.tool_iteration_model(), Some("claude-haiku-5.5"));
        let copilot = set.clone().for_tool_loop("copilot_sdk", false);
        assert_eq!(copilot.tool_iteration_model(), None);
        for stage in LlmStage::ALL {
            assert_eq!(
                copilot.model_for(stage),
                set.model_for(stage),
                "{stage:?}: the background stages keep their models"
            );
        }
        assert_eq!(
            set.for_tool_loop("anthropic", true).tool_iteration_model(),
            Some("claude-haiku-5.5")
        );
    }
}
