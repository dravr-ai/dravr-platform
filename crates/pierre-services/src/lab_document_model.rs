// ABOUTME: The lab reader's configurable model: none by default, else one tool-free embacle HTTP provider or Ollama (LAB_READER_MODEL)
// ABOUTME: Implements barrage's LabDocumentModel over one recorded, bounded completion; it never joins the chat chain (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The lab reader's model
//!
//! dravr-barrage reads a lab report's text with its rules, or with a model
//! the caller supplies behind `LabDocumentModel`, handing it nothing but a
//! fixed prompt and the text, and verifying every value it returns against
//! the document (`dravr_barrage::import::ingest_reader`). JF 2026-11-19, on
//! how that model is chosen: *"Make it configurable, default none. We will use
//! either Embacle or Ollama"*.
//!
//! - [`ENV_LAB_READER_MODEL`] names it: unset or `none` reads with the rules;
//!   otherwise an [`LlmProviderType`] that reads without tools
//!   ([`LlmProviderType::reads_without_tools`]): one of embacle's HTTP APIs,
//!   Ollama (`local` / `ollama`, on `LOCAL_LLM_*`) or the `OpenAI` API. A CLI
//!   runner, a Copilot runtime or the router is refused at boot and the rules
//!   read: those drive agents that carry tools, and the reader is an isolated
//!   job with none (design row 53).
//! - [`ENV_LAB_READER_MODEL_NAME`] overrides the provider's default model.
//! - It never joins the chat chain: Ollama is allowed here only.
//!
//! Each read is one completion, bounded by [`LAB_MODEL_TIMEOUT`] and
//! recorded as an `llm_usage` row under [`LAB_READING_CALL_TYPE`]; a failure
//! or a timeout is the model's failure, and barrage then reads with the rules.

use std::env;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dravr_barrage::import::ingest_reader::{LabDocumentModel, LabModelError, ModelRequest};
use pierre_llm::call_record::{complete_recorded, LlmCallRecorder};
use pierre_llm::config::LlmProviderType;
use pierre_llm::{ChatMessage, ChatRequest, EmbacleProvider, LlmProvider};
use tokio::time::timeout;
use tracing::{info, warn};

/// The environment variable naming the lab reader's model.
pub const ENV_LAB_READER_MODEL: &str = "LAB_READER_MODEL";

/// The environment variable overriding that provider's model.
pub const ENV_LAB_READER_MODEL_NAME: &str = "LAB_READER_MODEL_NAME";

/// The longest one lab read waits for its model: a CPU-bound Ollama takes
/// minutes on a long prompt; past this the rules read.
pub const LAB_MODEL_TIMEOUT: Duration = Duration::from_secs(120);

/// The `llm_usage` call type of a lab read.
pub const LAB_READING_CALL_TYPE: &str = "lab_reading";

/// What [`ENV_LAB_READER_MODEL`] chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabModelChoice {
    /// The rules read: the default.
    Rules,
    /// This provider reads.
    Provider(LlmProviderType),
}

/// Why a [`ENV_LAB_READER_MODEL`] value reads as the rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabModelRefusal {
    /// No provider answers to the name.
    Unknown(String),
    /// The provider drives an agent that carries tools.
    CarriesTools(String),
}

impl fmt::Display for LabModelRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "{ENV_LAB_READER_MODEL}={name} names no provider"),
            Self::CarriesTools(name) => write!(
                f,
                "{ENV_LAB_READER_MODEL}={name} drives an agent that carries tools; the lab reader \
                 takes only an HTTP API provider or Ollama"
            ),
        }
    }
}

/// What `raw`, the value of [`ENV_LAB_READER_MODEL`], chooses.
///
/// # Errors
///
/// A [`LabModelRefusal`] for a name no provider answers to or a provider that
/// carries tools: the caller then reads with the rules.
pub fn parse_lab_model(raw: Option<&str>) -> Result<LabModelChoice, LabModelRefusal> {
    let Some(name) = raw.map(str::trim).filter(|name| !name.is_empty()) else {
        return Ok(LabModelChoice::Rules);
    };
    if name.eq_ignore_ascii_case("none") {
        return Ok(LabModelChoice::Rules);
    }
    let provider =
        LlmProviderType::parse(name).ok_or_else(|| LabModelRefusal::Unknown(name.to_owned()))?;
    if provider.reads_without_tools() {
        Ok(LabModelChoice::Provider(provider))
    } else {
        Err(LabModelRefusal::CarriesTools(name.to_owned()))
    }
}

/// The lab reader's model, chosen once at boot: `None` reads with the rules.
#[derive(Clone, Default)]
pub struct LabReaderModel {
    provider: Option<Arc<dyn LlmProvider>>,
}

impl fmt::Debug for LabReaderModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LabReaderModel")
            .field("provider", &self.provider.as_ref().map(|p| p.name()))
            .finish()
    }
}

impl LabReaderModel {
    /// No model: the rules read.
    #[must_use]
    pub fn rules() -> Self {
        Self::default()
    }

    /// `provider` reads.
    #[must_use]
    pub fn over(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            provider: Some(provider),
        }
    }

    /// The model [`ENV_LAB_READER_MODEL`] and [`ENV_LAB_READER_MODEL_NAME`]
    /// name, or the rules when none is named, the name is refused, or the
    /// provider cannot be built (each logged once, here).
    pub async fn from_env() -> Self {
        let raw = env::var(ENV_LAB_READER_MODEL).ok();
        let model_name = env::var(ENV_LAB_READER_MODEL_NAME).ok();
        let provider = match parse_lab_model(raw.as_deref()) {
            Ok(LabModelChoice::Rules) => return Self::rules(),
            Ok(LabModelChoice::Provider(provider)) => provider,
            Err(refusal) => {
                warn!(%refusal, "The lab reader reads with its rules");
                return Self::rules();
            }
        };
        match EmbacleProvider::from_provider_type(provider, model_name.as_deref()).await {
            Ok(built) => {
                info!(
                    provider = built.name(),
                    "The lab reader's model is configured"
                );
                Self::over(Arc::new(built))
            }
            Err(error) => {
                warn!(%error, "The lab reader's model cannot be built; it reads with its rules");
                Self::rules()
            }
        }
    }

    /// Whether a model is configured.
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.provider.is_some()
    }

    /// The model for one read, its call recorded through `recorder`; `None`
    /// when the rules read.
    #[must_use]
    pub fn for_read(
        &self,
        recorder: Option<Arc<dyn LlmCallRecorder>>,
    ) -> Option<Arc<dyn LabDocumentModel>> {
        let provider = Arc::clone(self.provider.as_ref()?);
        let model: Arc<dyn LabDocumentModel> = Arc::new(RecordedLabModel { provider, recorder });
        Some(model)
    }
}

/// One lab read's model: a bounded completion, recorded.
struct RecordedLabModel {
    provider: Arc<dyn LlmProvider>,
    recorder: Option<Arc<dyn LlmCallRecorder>>,
}

#[async_trait]
impl LabDocumentModel for RecordedLabModel {
    fn name(&self) -> &str {
        self.provider.default_model()
    }

    async fn extract(&self, request: &ModelRequest) -> Result<String, LabModelError> {
        // One user message: several embacle runners drop a system message.
        let prompt = format!("{}\n\n{}", request.prompt, request.document);
        let chat = ChatRequest::new(vec![ChatMessage::user(prompt)]).with_temperature(0.0);
        let reply = timeout(
            LAB_MODEL_TIMEOUT,
            complete_recorded(self.provider.as_ref(), &chat, self.recorder.as_ref()),
        )
        .await
        .map_err(|_| LabModelError(format!("no answer within {LAB_MODEL_TIMEOUT:?}")))?
        .map_err(|e| LabModelError(e.to_string()))?;
        Ok(reply.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_none_reads_with_the_rules() {
        for raw in [None, Some(""), Some("  "), Some("none"), Some("NONE")] {
            assert_eq!(
                parse_lab_model(raw),
                Ok(LabModelChoice::Rules),
                "bug: {raw:?} chose a model"
            );
        }
    }

    #[test]
    fn an_http_api_or_ollama_is_chosen() {
        assert_eq!(
            parse_lab_model(Some("ollama")),
            Ok(LabModelChoice::Provider(LlmProviderType::Local)),
            "bug: ollama"
        );
        assert_eq!(
            parse_lab_model(Some("gemini")),
            Ok(LabModelChoice::Provider(LlmProviderType::Gemini)),
            "bug: gemini"
        );
    }

    #[test]
    fn a_runner_with_tools_or_an_unknown_name_is_refused() {
        assert_eq!(
            parse_lab_model(Some("claude_code")),
            Err(LabModelRefusal::CarriesTools("claude_code".to_owned())),
            "bug: a CLI runner was admitted"
        );
        assert_eq!(
            parse_lab_model(Some("copilot_sdk")),
            Err(LabModelRefusal::CarriesTools("copilot_sdk".to_owned())),
            "bug: a Copilot runtime was admitted"
        );
        assert_eq!(
            parse_lab_model(Some("mystery")),
            Err(LabModelRefusal::Unknown("mystery".to_owned())),
            "bug: an unknown name was admitted"
        );
    }

    #[test]
    fn the_default_reads_with_the_rules() {
        let model = LabReaderModel::rules();
        assert!(!model.is_configured(), "bug: a model by default");
        assert!(model.for_read(None).is_none(), "bug: a read got a model");
    }
}
