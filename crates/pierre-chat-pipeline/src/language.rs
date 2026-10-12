// ABOUTME: Resolves which of the five platform languages a text is in: lingua when compiled in, the LLM otherwise
// ABOUTME: One resolver for the turn's locale and the reply-note locale, so the two cannot disagree
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The language a text is written in, among the five the platform speaks.
//!
//! The reply answers in the language of the question (carnet#825), and a
//! question is usually one short sentence. Two resolvers serve that:
//!
//! - **`lingua`**, behind the `language-detection` feature, which the shipped
//!   profiles enable (carnet#839): local and deterministic, at the cost of
//!   about 22 MB of language models in the binary.
//!   Restricted to the five languages, in high-accuracy mode with a minimum
//!   relative distance of 0.1 between the two best candidates, it resolved 69
//!   of 73 short athlete messages, left 2 undecided and mislabelled 2 — "E o
//!   meu `VO2max`?" and "J'ai couru le trail de Sherbrooke", where the only
//!   words are shared ones. A trigram detector such as `whatlang` resolved 6:
//!   it calls a verdict reliable only from about 130 characters.
//! - **The LLM**, asked for one language code with the contremaitre
//!   `language_classification` prompt: what a build with the feature asks when
//!   `lingua` cannot decide, and what a build without it always asks.
//!
//! A message under `MIN_DETECTABLE_CHARS` ("ok", "oui", "Yes") is an
//! acknowledgement, and keeps the language the conversation already runs in.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pierre_llm::call_record::{emit_call_record_with_text, CallRecordInputs, LlmCallRecorder};
use pierre_llm::served_tier::{observe_served_tier, ServedTier};
use pierre_llm::stage::LlmStage;
use pierre_llm::{ChatMessage, ChatProvider, ChatRequest};
use tokio::time::timeout;
use tracing::warn;

/// Below this many characters a message is an acknowledgement, and the
/// language the conversation already runs in is what the athlete wants.
const MIN_DETECTABLE_CHARS: usize = 12;

/// How long the turn waits on the LLM's answer before keeping the stored
/// locale: the classification runs before the reply, so it bounds latency
/// the athlete sees.
const CLASSIFICATION_TIMEOUT: Duration = Duration::from_secs(15);

/// How much of the message the LLM is shown. A language shows in its first
/// sentences; the rest would only cost tokens.
const CLASSIFICATION_EXCERPT_CHARS: usize = 600;

/// Where the classification prompt takes the athlete's message.
const MESSAGE_PLACEHOLDER: &str = "{{MESSAGE}}";

/// The LLM a turn may ask for its language, and where that call's usage goes.
#[derive(Clone, Copy)]
pub struct LocaleClassifier<'a> {
    /// The turn's own provider, which routes the call to the
    /// [`LlmStage::LanguageClassification`] model when it is the platform's
    /// head.
    pub provider: &'a ChatProvider,
    /// The contremaitre `language_classification` prompt, with
    /// `{{MESSAGE}}` where the message goes.
    pub prompt: &'a str,
    /// Receives one record for the classification call, typed
    /// [`LlmStage::LanguageClassification`]; `None` writes no usage row.
    pub recorder: Option<&'a Arc<dyn LlmCallRecorder>>,
}

/// The language of `text` when it can be read locally: `lingua` in a build
/// with the `language-detection` feature. `None` when the text is too
/// ambiguous to call or in none of the five languages.
#[cfg(feature = "language-detection")]
#[must_use]
pub fn detect_locale(text: &str) -> Option<&'static str> {
    lingua_detector::detect(text)
}

/// The language of `text` when it can be read locally. This build carries no
/// local detector (the `language-detection` feature is off), so it is always
/// `None` and [`resolve_turn_locale`] asks the LLM.
#[cfg(not(feature = "language-detection"))]
#[must_use]
pub const fn detect_locale(_text: &str) -> Option<&'static str> {
    None
}

/// The locale a turn is conducted in: the language of the athlete's message,
/// read locally when possible and by the LLM otherwise, else `fallback` — the
/// locale the surface resolved before the turn.
pub async fn resolve_turn_locale(
    classifier: Option<LocaleClassifier<'_>>,
    text: &str,
    fallback: &str,
) -> String {
    if text.trim().chars().count() < MIN_DETECTABLE_CHARS {
        return fallback.to_owned();
    }
    if let Some(locale) = detect_locale(text) {
        return locale.to_owned();
    }
    let Some(classifier) = classifier else {
        return fallback.to_owned();
    };
    classify_locale_with_llm(classifier, text)
        .await
        .map_or_else(|| fallback.to_owned(), str::to_owned)
}

/// Ask the LLM which of the five languages `text` is in, recording the call's
/// usage like any other LLM call the turn makes.
///
/// Fail-open: an error, a timeout or an answer that names no platform
/// language is `None`, and the turn keeps its stored locale — a language
/// guess is never worth refusing the turn over.
async fn classify_locale_with_llm(
    classifier: LocaleClassifier<'_>,
    text: &str,
) -> Option<&'static str> {
    let LocaleClassifier {
        provider,
        prompt,
        recorder,
    } = classifier;
    let excerpt: String = text.chars().take(CLASSIFICATION_EXCERPT_CHARS).collect();
    // One user message: several embacle runners drop a system message.
    let prompt = render_classification_prompt(prompt, &excerpt);
    let request = provider.routed(
        LlmStage::LanguageClassification,
        ChatRequest::new(vec![ChatMessage::user(prompt.clone())]),
    );
    let requested_model = request
        .model
        .clone()
        .unwrap_or_else(|| provider.default_model().to_owned());
    let started = Instant::now();
    let outcome = timeout(
        CLASSIFICATION_TIMEOUT,
        observe_served_tier(provider.complete(&request)),
    )
    .await;
    let latency_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    // The row names the tier that answered: a stage model the head refuses
    // falls back to the next tier, whose own model the reply then carries.
    let record =
        |served: Option<ServedTier>, model: &str, usage, success, completion: Option<&str>| {
            emit_call_record_with_text(
                CallRecordInputs {
                    recorder,
                    provider: served.map_or_else(|| provider.name(), |tier| tier.provider),
                    model,
                    usage,
                    latency_ms,
                    success,
                    call_sequence: None,
                    tools_called: Vec::new(),
                },
                Some(&prompt),
                completion,
            );
        };
    match outcome {
        Ok((Ok(reply), served)) => {
            record(
                served,
                &reply.model,
                reply.usage.as_ref(),
                true,
                Some(&reply.content),
            );
            parse_locale_answer(&reply.content)
        }
        Ok((Err(e), served)) => {
            record(served, &requested_model, None, false, None);
            warn!(error = %e, "language classification failed; the turn keeps its stored locale");
            None
        }
        Err(_) => {
            // The request may already have been billed when the wait ends.
            record(None, &requested_model, None, false, None);
            warn!(
                timeout_secs = CLASSIFICATION_TIMEOUT.as_secs(),
                "language classification timed out; the turn keeps its stored locale"
            );
            None
        }
    }
}

/// The classification prompt with the message in place of `{{MESSAGE}}`.
///
/// A prompt that lost its placeholder in an edit still gets the message,
/// appended, so the model is never asked about a message it cannot see.
fn render_classification_prompt(template: &str, excerpt: &str) -> String {
    if template.contains(MESSAGE_PLACEHOLDER) {
        template.replace(MESSAGE_PLACEHOLDER, excerpt)
    } else {
        warn!("the language_classification prompt has no {MESSAGE_PLACEHOLDER} placeholder; appending the message");
        format!("{template}\n\n{excerpt}")
    }
}

/// The platform locale an LLM's one-word answer names, tolerating the
/// punctuation, quoting and casing a model wraps around it.
fn parse_locale_answer(answer: &str) -> Option<&'static str> {
    let word = answer
        .split_whitespace()
        .next()?
        .trim_matches(|c: char| !c.is_alphabetic())
        .to_lowercase();
    match word.as_str() {
        "en" | "english" => Some("en"),
        "fr" | "french" | "français" => Some("fr"),
        "es" | "spanish" | "español" => Some("es"),
        "de" | "german" | "deutsch" => Some("de"),
        "pt" | "portuguese" | "português" => Some("pt"),
        _ => None,
    }
}

#[cfg(feature = "language-detection")]
mod lingua_detector {
    use std::sync::LazyLock;

    use lingua::{Language, LanguageDetector, LanguageDetectorBuilder};

    /// The platform's languages, and the only candidates the detector weighs.
    const PLATFORM_LANGUAGES: [Language; 5] = [
        Language::English,
        Language::French,
        Language::Spanish,
        Language::German,
        Language::Portuguese,
    ];

    /// How far apart the two best candidates' confidences must be before the
    /// best one counts; closer than this, the text is ambiguous. 0.1 is where
    /// the measurement stopped mislabelling "Muéstrame mis últimas cinco
    /// actividades" (Spanish and Portuguese tie at 0.50) without dropping any
    /// message it already resolved.
    const MINIMUM_RELATIVE_DISTANCE: f64 = 0.1;

    /// Built once: the language models load on the first detection and stay
    /// resident (about 28 MB for the five languages).
    static DETECTOR: LazyLock<LanguageDetector> = LazyLock::new(|| {
        LanguageDetectorBuilder::from_languages(&PLATFORM_LANGUAGES)
            .with_minimum_relative_distance(MINIMUM_RELATIVE_DISTANCE)
            .build()
    });

    pub(super) fn detect(text: &str) -> Option<&'static str> {
        DETECTOR
            .detect_language_of(text)
            .map(|language| match language {
                Language::English => "en",
                Language::French => "fr",
                Language::Spanish => "es",
                Language::German => "de",
                Language::Portuguese => "pt",
            })
    }
}

#[cfg(test)]
mod tests {
    use std::mem;
    use std::sync::{Arc, Mutex, PoisonError};

    use async_trait::async_trait;
    use embacle::types::{
        ChatStream as EmbacleChatStream, LlmProvider as EmbacleLlmProvider, RunnerError,
    };
    use pierre_core::errors::AppError;
    use pierre_core::llm::{ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider};
    use pierre_llm::call_record::{LlmCallRecord, LlmCallRecorder};
    use pierre_llm::config::LlmProviderType;
    use pierre_llm::prompts::LANGUAGE_CLASSIFICATION_PROMPT;
    use pierre_llm::stage::{StageModels, COPILOT_SDK_BACKGROUND_MODEL};
    use pierre_llm::{ChatProvider, EmbacleProvider};

    use super::{
        classify_locale_with_llm, parse_locale_answer, render_classification_prompt,
        resolve_turn_locale, LocaleClassifier, MESSAGE_PLACEHOLDER,
    };

    /// Answers every completion with a fixed text, or fails when it has none.
    struct FixedAnswer(Option<&'static str>);

    #[async_trait]
    impl LlmProvider for FixedAnswer {
        fn name(&self) -> &'static str {
            "fixed_answer"
        }
        fn display_name(&self) -> &'static str {
            "Fixed-answer mock (language classification)"
        }
        fn capabilities(&self) -> LlmCapabilities {
            LlmCapabilities::empty()
        }
        fn default_model(&self) -> &'static str {
            "mock-model"
        }
        fn available_models(&self) -> &[String] {
            &[]
        }
        async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
            let Some(content) = self.0 else {
                return Err(AppError::internal("provider down"));
            };
            Ok(ChatResponse {
                content: content.to_owned(),
                model: "mock-model".to_owned(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }
        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            Err(AppError::internal("streaming is not used here"))
        }
        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    fn answering(content: Option<&'static str>) -> ChatProvider {
        ChatProvider::Custom(Arc::new(FixedAnswer(content)))
    }

    fn unrecorded(provider: &ChatProvider) -> LocaleClassifier<'_> {
        LocaleClassifier {
            provider,
            prompt: LANGUAGE_CLASSIFICATION_PROMPT,
            recorder: None,
        }
    }

    /// Keeps every record it is handed.
    #[derive(Default)]
    struct Captured(Mutex<Vec<LlmCallRecord>>);

    impl LlmCallRecorder for Captured {
        fn record(&self, record: LlmCallRecord) {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(record);
        }
    }

    impl Captured {
        fn taken(&self) -> Vec<LlmCallRecord> {
            mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
        }
    }

    #[test]
    fn a_model_answer_is_read_through_its_wrapping() {
        assert_eq!(parse_locale_answer("en"), Some("en"));
        assert_eq!(parse_locale_answer(" FR.\n"), Some("fr"));
        assert_eq!(parse_locale_answer("`de`"), Some("de"));
        assert_eq!(parse_locale_answer("Portuguese"), Some("pt"));
        assert_eq!(parse_locale_answer("other"), None);
        assert_eq!(parse_locale_answer("ja"), None);
        assert_eq!(parse_locale_answer(""), None);
    }

    /// The contremaitre prompt names where the message goes, and the message
    /// lands there; a prompt that lost the placeholder still carries it.
    #[test]
    fn the_message_reaches_the_classification_prompt() {
        assert!(
            LANGUAGE_CLASSIFICATION_PROMPT.contains(MESSAGE_PLACEHOLDER),
            "the compiled-in prompt marks where the message goes"
        );
        let rendered = render_classification_prompt(LANGUAGE_CLASSIFICATION_PROMPT, "Hola amigos");
        assert!(rendered.contains("Hola amigos"), "{rendered}");
        assert!(!rendered.contains(MESSAGE_PLACEHOLDER), "{rendered}");

        let without = render_classification_prompt("Name the language.", "Hola amigos");
        assert!(without.ends_with("Hola amigos"), "{without}");
    }

    #[tokio::test]
    async fn the_llm_names_the_language_when_asked() {
        let provider = answering(Some("es"));
        assert_eq!(
            classify_locale_with_llm(
                unrecorded(&provider),
                "¿Puedo hacer una tirada larga mañana?"
            )
            .await,
            Some("es")
        );
    }

    #[tokio::test]
    async fn a_failed_classification_keeps_the_stored_locale() {
        let provider = answering(None);
        assert_eq!(
            resolve_turn_locale(
                Some(unrecorded(&provider)),
                "Muéstrame mis últimas cinco actividades",
                "fr"
            )
            .await,
            "fr"
        );
    }

    #[tokio::test]
    async fn an_acknowledgement_keeps_the_stored_locale_without_asking() {
        // A provider that would answer "de" is never consulted for "oui".
        let provider = answering(Some("de"));
        assert_eq!(
            resolve_turn_locale(Some(unrecorded(&provider)), "oui", "en").await,
            "en"
        );
    }

    /// Lingua cannot call a Spanish/Portuguese tie, so the LLM decides — in a
    /// build with the feature and one without it alike.
    #[tokio::test]
    async fn what_the_local_detector_cannot_call_goes_to_the_llm() {
        let provider = answering(Some("es"));
        assert_eq!(
            resolve_turn_locale(
                Some(unrecorded(&provider)),
                "Muéstrame mis últimas cinco actividades",
                "fr"
            )
            .await,
            "es"
        );
    }

    /// The classification is an LLM call the turn pays for, so it is
    /// accounted like one: a record whether it answers or fails, with token
    /// counts estimated when the provider reports none, and none at all when
    /// the provider is never asked.
    #[tokio::test]
    async fn every_classification_call_is_recorded() {
        let captured = Arc::new(Captured::default());
        let recorder: Arc<dyn LlmCallRecorder> = captured.clone();
        let ask = "Muéstrame mis últimas cinco actividades";

        let answers = answering(Some("es"));
        let classifier = LocaleClassifier {
            provider: &answers,
            prompt: LANGUAGE_CLASSIFICATION_PROMPT,
            recorder: Some(&recorder),
        };
        assert_eq!(resolve_turn_locale(Some(classifier), ask, "fr").await, "es");
        let records = captured.taken();
        assert_eq!(records.len(), 1, "one call, one record: {records:?}");
        let call = &records[0];
        assert!(call.success);
        assert_eq!(call.provider, "fixed_answer");
        assert_eq!(call.model, "mock-model");
        assert!(
            call.token_counts_estimated && call.prompt_tokens > 0,
            "a provider that reports no usage is estimated, never recorded as free: {call:?}"
        );

        let fails = answering(None);
        let classifier = LocaleClassifier {
            provider: &fails,
            prompt: LANGUAGE_CLASSIFICATION_PROMPT,
            recorder: Some(&recorder),
        };
        assert_eq!(resolve_turn_locale(Some(classifier), ask, "fr").await, "fr");
        let records = captured.taken();
        assert_eq!(
            records.len(),
            1,
            "a failed call is recorded too: {records:?}"
        );
        assert!(!records[0].success);
        assert!(!records[0].is_unaccounted(), "{:?}", records[0]);

        let classifier = LocaleClassifier {
            provider: &answers,
            prompt: LANGUAGE_CLASSIFICATION_PROMPT,
            recorder: Some(&recorder),
        };
        assert_eq!(
            resolve_turn_locale(Some(classifier), "oui", "en").await,
            "en"
        );
        assert!(
            captured.taken().is_empty(),
            "an acknowledgement asks nothing, so nothing is recorded"
        );
    }

    /// A runner named `name` that names Spanish, or refuses as a provider
    /// fault when `answers` is false, and remembers the model each call asked
    /// for.
    struct SdkClassifier {
        name: &'static str,
        answers: bool,
        requested: Arc<Mutex<Vec<Option<String>>>>,
        models: Vec<String>,
    }

    #[async_trait]
    impl EmbacleLlmProvider for SdkClassifier {
        fn name(&self) -> &'static str {
            self.name
        }
        fn display_name(&self) -> &'static str {
            "Copilot SDK classifier (scripted)"
        }
        fn capabilities(&self) -> LlmCapabilities {
            LlmCapabilities::empty()
        }
        fn default_model(&self) -> &str {
            &self.models[0]
        }
        fn available_models(&self) -> &[String] {
            &self.models
        }
        async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, RunnerError> {
            self.requested
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request.model.clone());
            if !self.answers {
                return Err(RunnerError::external_service(self.name, "scripted refusal"));
            }
            Ok(ChatResponse {
                content: "es".to_owned(),
                model: request
                    .model
                    .clone()
                    .unwrap_or_else(|| self.models[0].clone()),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }
        async fn complete_stream(
            &self,
            _request: &ChatRequest,
        ) -> Result<EmbacleChatStream, RunnerError> {
            Err(RunnerError::internal("streaming is not used here"))
        }
        async fn health_check(&self) -> Result<bool, RunnerError> {
            Ok(true)
        }
    }

    /// The classification is a background stage: the platform's `copilot_sdk`
    /// head runs it on the stage model, and the record names that model.
    #[tokio::test]
    async fn the_classification_runs_on_its_stage_model() {
        let requested = Arc::new(Mutex::new(Vec::new()));
        let provider = ChatProvider::Embacle(
            EmbacleProvider::from_runner(
                Box::new(SdkClassifier {
                    name: "copilot_sdk",
                    answers: true,
                    requested: Arc::clone(&requested),
                    models: vec![
                        "claude-sonnet-5.5".to_owned(),
                        COPILOT_SDK_BACKGROUND_MODEL.to_owned(),
                    ],
                }),
                "Copilot SDK classifier (scripted)",
            )
            .with_stage_models(StageModels::resolve(
                Some(LlmProviderType::CopilotSdk),
                |_| None,
            )),
        );
        let captured = Arc::new(Captured::default());
        let recorder: Arc<dyn LlmCallRecorder> = captured.clone();
        let classifier = LocaleClassifier {
            provider: &provider,
            prompt: LANGUAGE_CLASSIFICATION_PROMPT,
            recorder: Some(&recorder),
        };

        let locale =
            classify_locale_with_llm(classifier, "Muéstrame mis últimas cinco actividades").await;

        assert_eq!(locale, Some("es"));
        assert_eq!(
            mem::take(&mut *requested.lock().unwrap_or_else(PoisonError::into_inner)),
            vec![Some(COPILOT_SDK_BACKGROUND_MODEL.to_owned())]
        );
        let records = captured.taken();
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0].model, COPILOT_SDK_BACKGROUND_MODEL);
        assert_eq!(records[0].provider, "copilot_sdk");
    }

    /// A stage model the head publishes and still refuses (an entitlement
    /// withdrawn after boot) moves the call to the next tier, which answers on
    /// its own model; the record names that tier, so the row is priced where
    /// the call was served rather than against the head.
    ///
    /// The only chain in this test binary whose head fails: the circuit
    /// breaker on a chain's primary is process-wide.
    #[tokio::test]
    async fn a_classification_the_head_refuses_is_recorded_against_the_tier_that_answered() {
        let head_requested = Arc::new(Mutex::new(Vec::new()));
        let tail_requested = Arc::new(Mutex::new(Vec::new()));
        let provider = ChatProvider::Embacle(
            EmbacleProvider::chain(vec![
                EmbacleProvider::from_runner(
                    Box::new(SdkClassifier {
                        name: "copilot_sdk",
                        answers: false,
                        requested: Arc::clone(&head_requested),
                        models: vec![
                            "claude-sonnet-5.5".to_owned(),
                            COPILOT_SDK_BACKGROUND_MODEL.to_owned(),
                        ],
                    }),
                    "Copilot SDK classifier (scripted)",
                )
                .with_stage_models(StageModels::resolve(
                    Some(LlmProviderType::CopilotSdk),
                    |_| None,
                )),
                EmbacleProvider::from_runner(
                    Box::new(SdkClassifier {
                        name: "claude_code",
                        answers: true,
                        requested: Arc::clone(&tail_requested),
                        models: vec!["claude-sonnet-5".to_owned()],
                    }),
                    "Claude Code classifier (scripted)",
                ),
            ])
            .unwrap(),
        );
        let captured = Arc::new(Captured::default());
        let recorder: Arc<dyn LlmCallRecorder> = captured.clone();
        let classifier = LocaleClassifier {
            provider: &provider,
            prompt: LANGUAGE_CLASSIFICATION_PROMPT,
            recorder: Some(&recorder),
        };

        let locale =
            classify_locale_with_llm(classifier, "Muéstrame mis últimas cinco actividades").await;

        assert_eq!(locale, Some("es"));
        assert_eq!(
            mem::take(
                &mut *head_requested
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
            ),
            vec![Some(COPILOT_SDK_BACKGROUND_MODEL.to_owned())]
        );
        assert_eq!(
            mem::take(
                &mut *tail_requested
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
            ),
            vec![None],
            "the tier behind the head resolves its own model"
        );
        let records = captured.taken();
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0].provider, "claude_code");
        assert_eq!(records[0].model, "claude-sonnet-5");
        assert!(records[0].success);
    }

    #[cfg(feature = "language-detection")]
    mod lingua {
        use super::super::detect_locale;

        #[test]
        fn short_questions_resolve_to_their_own_language() {
            // The three probes from the 2026-10-07 dev check, then one short
            // question per language.
            for (text, expected) in [
                (
                    "How much does a Big Mac cost in San Francisco these days?",
                    "en",
                ),
                ("Should I sell my Apple stock this week", "en"),
                (
                    "I have severe chest pain right now and feel dizzy. What's wrong with me?",
                    "en",
                ),
                ("How was my run yesterday?", "en"),
                ("Merci, ça m'aide beaucoup", "fr"),
                ("¿Cómo fue mi carrera de ayer?", "es"),
                ("Mein Knie tut nach dem Laufen weh", "de"),
                ("Obrigado, isso ajuda muito", "pt"),
            ] {
                assert_eq!(detect_locale(text), Some(expected), "{text}");
            }
        }

        #[test]
        fn a_tie_between_two_languages_is_left_to_the_caller() {
            assert_eq!(
                detect_locale("Muéstrame mis últimas cinco actividades"),
                None
            );
        }

        #[test]
        fn a_script_none_of_the_five_languages_uses_is_not_called() {
            assert_eq!(
                detect_locale(
                    "これは日本語のテストメッセージです。トレーニングについて質問があります。"
                ),
                None
            );
        }
    }
}
