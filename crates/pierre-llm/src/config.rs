// ABOUTME: LLM configuration types for provider selection and model settings
// ABOUTME: Contains LlmProviderType enum and LlmModelConfig for environment-based configuration
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use embacle::CliRunnerType;
use embacle::{AnthropicConfig, CopilotHeadlessConfig, CopilotSdkConfig};
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt::{Display, Formatter, Result as FmtResult};

/// LLM provider selection for chat functionality
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LlmProviderType {
    /// Groq provider - LPU-accelerated inference for Llama/Mixtral models
    Groq,
    /// Google Gemini provider - full-featured with vision support (default)
    #[default]
    Gemini,
    /// Local LLM provider - `OpenAI`-compatible endpoint (Ollama, vLLM, `LocalAI`)
    Local,
    /// `OpenRouter` provider - unified gateway to 200+ models via openrouter.ai
    OpenRouter,
    /// Cohere provider - Command A and Command R family via Cohere v2 chat API
    Cohere,
    /// Anthropic Messages API provider - Claude Haiku / Sonnet / Opus on
    /// `ANTHROPIC_API_KEY`, pooled over `ANTHROPIC_API_KEY_2`, `_3`, …
    Anthropic,
    /// Claude Code CLI provider - subprocess-based Anthropic Claude
    ClaudeCode,
    /// Cursor Agent CLI provider - subprocess-based Cursor AI
    CursorAgent,
    /// `OpenCode` CLI provider - subprocess-based open-source coding agent
    OpenCode,
    /// GitHub Copilot CLI provider - subprocess-based GitHub Copilot
    Copilot,
    /// GitHub Copilot Headless provider - ACP via `copilot --acp`
    CopilotHeadless,
    /// GitHub Copilot through its Rust SDK - `copilot-runtime --server --stdio`
    CopilotSdk,
    /// Warp terminal `oz` CLI provider
    WarpCli,
    /// Gemini CLI provider - subprocess-based Google Gemini
    GeminiCli,
    /// Codex CLI provider - subprocess-based `OpenAI` Codex
    CodexCli,
    /// Goose CLI provider - subprocess-based Goose coding agent
    GooseCli,
    /// Cline CLI provider - subprocess-based Cline coding agent
    ClineCli,
    /// Continue CLI provider - subprocess-based Continue coding agent
    ContinueCli,
    /// Kiro CLI provider - subprocess-based AWS Kiro coding agent
    KiroCli,
    /// Kilo Code CLI provider - subprocess-based Kilo coding agent
    KiloCli,
    /// Quota-aware router over several backends — leads with one, steps aside
    /// before its budget runs out, and returns when the window resets
    Router,
    /// `OpenAI`-compatible HTTP API provider via embacle (any `OpenAI`-compatible endpoint)
    OpenAiApi,
}

impl LlmProviderType {
    /// Parse a provider name, case-insensitively, accepting each provider's
    /// documented aliases. `None` for a name no provider answers to: a
    /// [`LlmChain`] entry naming one is skipped with a warning rather than
    /// silently built as some other provider.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let provider = match s.trim().to_lowercase().as_str() {
            "gemini" | "google" => Self::Gemini,
            "groq" => Self::Groq,
            "local" | "ollama" | "vllm" | "localai" => Self::Local,
            "openrouter" | "open_router" | "open-router" => Self::OpenRouter,
            "cohere" | "command" | "command-a" => Self::Cohere,
            "anthropic" | "anthropic_api" | "anthropic-api" => Self::Anthropic,
            "claude_code" | "claude-code" => Self::ClaudeCode,
            "cursor_agent" | "cursor-agent" => Self::CursorAgent,
            "opencode" | "open_code" => Self::OpenCode,
            "copilot" | "github_copilot" | "github-copilot" => Self::Copilot,
            "copilot_headless" | "copilot-headless" => Self::CopilotHeadless,
            "copilot_sdk" | "copilot-sdk" => Self::CopilotSdk,
            "warp_cli" | "warp-cli" | "warp" | "oz" => Self::WarpCli,
            "gemini_cli" | "gemini-cli" => Self::GeminiCli,
            "codex_cli" | "codex-cli" | "codex" => Self::CodexCli,
            "goose_cli" | "goose-cli" | "goose" => Self::GooseCli,
            "cline_cli" | "cline-cli" | "cline" => Self::ClineCli,
            "continue_cli" | "continue-cli" | "continue" => Self::ContinueCli,
            "kiro_cli" | "kiro-cli" | "kiro" => Self::KiroCli,
            "kilo_cli" | "kilo-cli" | "kilo" => Self::KiloCli,
            "openai_api" | "openai-api" | "openai" => Self::OpenAiApi,
            "router" | "quota_router" | "quota-router" => Self::Router,
            _ => return None,
        };
        Some(provider)
    }
}

/// The embacle HTTP API providers, as [`crate::http_env`] builds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpProvider {
    /// Google Gemini on `GEMINI_API_KEY`.
    Gemini,
    /// Groq on `GROQ_API_KEY`.
    Groq,
    /// Cohere on `COHERE_API_KEY`.
    Cohere,
    /// `OpenRouter` on `OPENROUTER_API_KEY`.
    OpenRouter,
    /// A local `OpenAI`-compatible endpoint on `LOCAL_LLM_*`.
    Local,
    /// The Anthropic Messages API on `ANTHROPIC_API_KEY`.
    Anthropic,
}

/// The construction path an [`LlmProviderType`] takes, see
/// [`LlmProviderType::construction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderConstruction {
    /// A CLI subprocess runner, keyed on its embacle runner type.
    Cli(CliRunnerType),
    /// One of embacle's HTTP API providers, built by [`crate::http_env`].
    HttpApi(HttpProvider),
    /// The Copilot ACP turn provider (`copilot --acp`).
    CopilotHeadless,
    /// The Copilot SDK turn provider (`copilot-runtime --server --stdio`).
    CopilotSdk,
    /// The `OpenAI` API runner (`OPENAI_API_*`).
    OpenAiApi,
    /// The quota router: Claude Code in front, Copilot Headless behind.
    Router,
}

impl LlmProviderType {
    /// Default model label for the Claude Code CLI runner (short alias)
    const DEFAULT_CLAUDE_CODE_MODEL: &'static str = "opus";

    /// Default model label for the Cursor Agent CLI runner
    const DEFAULT_CURSOR_AGENT_MODEL: &'static str = "sonnet-4";

    /// Default model label for the `OpenCode` CLI runner
    const DEFAULT_OPENCODE_MODEL: &'static str = "anthropic/claude-sonnet-4";

    /// Default model label for the Gemini CLI runner
    const DEFAULT_GEMINI_CLI_MODEL: &'static str = "gemini-2.5-pro";

    /// Default model label for the Codex CLI runner
    const DEFAULT_CODEX_CLI_MODEL: &'static str = "codex-mini";

    /// Default model label shared by the Goose/Cline/Continue/Warp/Kiro/Kilo CLI runners
    const DEFAULT_CLAUDE_SONNET_CLI_MODEL: &'static str = "claude-sonnet-4";

    /// Environment variable overriding the `OpenAI` API model
    const OPENAI_API_MODEL_ENV_VAR: &'static str = "OPENAI_API_MODEL";

    /// Default model label for the `OpenAI` API provider when unset
    const DEFAULT_OPENAI_API_MODEL: &'static str = "gpt-5.4";

    /// Environment variable overriding the Anthropic provider's default model
    /// (embacle's own name for it)
    const ANTHROPIC_DEFAULT_MODEL_ENV_VAR: &'static str = "ANTHROPIC_DEFAULT_MODEL";

    /// Whether this provider spends `COPILOT_GITHUB_TOKEN`, the token whose
    /// GitHub core budget [`crate::chain_guard::copilot_headroom_checker`]
    /// measures — so whether the chain guard may pass it over when that
    /// budget is low or cannot be read.
    ///
    /// The Copilot CLI and both Copilot runtimes authenticate with it. Every
    /// other provider spends a credential the checker does not read, and the
    /// quota router leads with Claude Code and meters its own Copilot backend
    /// against the same budget, so none of them may be skipped for it.
    #[must_use]
    pub(crate) const fn spends_copilot_github_token(self) -> bool {
        match self.construction() {
            ProviderConstruction::Cli(runner_type) => matches!(
                runner_type,
                CliRunnerType::Copilot | CliRunnerType::CopilotHeadless | CliRunnerType::CopilotSdk
            ),
            ProviderConstruction::CopilotHeadless | ProviderConstruction::CopilotSdk => true,
            ProviderConstruction::HttpApi(_)
            | ProviderConstruction::OpenAiApi
            | ProviderConstruction::Router => false,
        }
    }

    /// How [`EmbacleProvider`](crate::EmbacleProvider) builds this provider.
    ///
    /// The CLI subprocess runners share one construction path keyed on their
    /// embacle [`CliRunnerType`]; the HTTP APIs, the two Copilot turn
    /// providers, the `OpenAI` API runner and the quota router each have a
    /// bespoke one. The chain assembly dispatches on this per entry, so every
    /// tier is built as the provider its [`LlmChain`] entry names.
    pub(crate) const fn construction(self) -> ProviderConstruction {
        match self {
            Self::ClaudeCode => ProviderConstruction::Cli(CliRunnerType::ClaudeCode),
            Self::Copilot => ProviderConstruction::Cli(CliRunnerType::Copilot),
            Self::CursorAgent => ProviderConstruction::Cli(CliRunnerType::CursorAgent),
            Self::OpenCode => ProviderConstruction::Cli(CliRunnerType::OpenCode),
            Self::GeminiCli => ProviderConstruction::Cli(CliRunnerType::GeminiCli),
            Self::CodexCli => ProviderConstruction::Cli(CliRunnerType::CodexCli),
            Self::GooseCli => ProviderConstruction::Cli(CliRunnerType::GooseCli),
            Self::ClineCli => ProviderConstruction::Cli(CliRunnerType::ClineCli),
            Self::ContinueCli => ProviderConstruction::Cli(CliRunnerType::ContinueCli),
            Self::WarpCli => ProviderConstruction::Cli(CliRunnerType::WarpCli),
            Self::KiroCli => ProviderConstruction::Cli(CliRunnerType::KiroCli),
            Self::KiloCli => ProviderConstruction::Cli(CliRunnerType::KiloCli),
            Self::Gemini => ProviderConstruction::HttpApi(HttpProvider::Gemini),
            Self::Groq => ProviderConstruction::HttpApi(HttpProvider::Groq),
            Self::Cohere => ProviderConstruction::HttpApi(HttpProvider::Cohere),
            Self::OpenRouter => ProviderConstruction::HttpApi(HttpProvider::OpenRouter),
            Self::Local => ProviderConstruction::HttpApi(HttpProvider::Local),
            Self::Anthropic => ProviderConstruction::HttpApi(HttpProvider::Anthropic),
            Self::CopilotHeadless => ProviderConstruction::CopilotHeadless,
            Self::CopilotSdk => ProviderConstruction::CopilotSdk,
            Self::OpenAiApi => ProviderConstruction::OpenAiApi,
            // The router is several runners behind one provider, so it has no single
            // CliRunnerType. Which one is live changes per turn, and the caller that
            // needs the turn provider asks `as_turn_provider()` instead.
            Self::Router => ProviderConstruction::Router,
        }
    }

    /// The model this provider runs when its chain entry names none, as far
    /// as the environment says without building the provider.
    ///
    /// Gemini runs `PIERRE_LLM_DEFAULT_MODEL`; Anthropic runs
    /// `ANTHROPIC_DEFAULT_MODEL`, else embacle's default (`claude-haiku-5-5`);
    /// the Copilot runtimes their `COPILOT_*_MODEL`; the CLI runners the label
    /// of their built-in default. `None` for Groq, Cohere, `OpenRouter` and a
    /// local endpoint, whose default is read by embacle's `from_env()` only
    /// once the provider is built with its key.
    #[must_use]
    pub(crate) fn default_model_from_env(self) -> Option<String> {
        match self {
            Self::Gemini => non_empty_env(LlmModelConfig::DEFAULT_MODEL_ENV_VAR),
            // The config's own default, read off a keyless instance so the
            // literal lives in embacle alone.
            Self::Anthropic => Some(
                non_empty_env(Self::ANTHROPIC_DEFAULT_MODEL_ENV_VAR)
                    .unwrap_or_else(|| AnthropicConfig::new(String::new()).model),
            ),
            Self::Groq | Self::Local | Self::OpenRouter | Self::Cohere => None,
            // Both Copilot variants derive the default from COPILOT_HEADLESS_MODEL
            // (env-injected, e.g. by terraform) rather than a hardcoded version
            // literal that drifts from deployed config.
            Self::CopilotHeadless | Self::Copilot => Some(CopilotHeadlessConfig::from_env().model),
            Self::CopilotSdk => Some(CopilotSdkConfig::from_env().model),
            // CLI runners pick their default model internally in new().
            // These fallbacks are only used for the conversation DB label when
            // RunnerConfig.model is None (no env override).
            Self::ClaudeCode => Some(Self::DEFAULT_CLAUDE_CODE_MODEL.to_owned()),
            Self::CursorAgent => Some(Self::DEFAULT_CURSOR_AGENT_MODEL.to_owned()),
            Self::OpenCode => Some(Self::DEFAULT_OPENCODE_MODEL.to_owned()),
            Self::GeminiCli => Some(Self::DEFAULT_GEMINI_CLI_MODEL.to_owned()),
            Self::CodexCli => Some(Self::DEFAULT_CODEX_CLI_MODEL.to_owned()),
            Self::GooseCli
            | Self::ClineCli
            | Self::ContinueCli
            | Self::WarpCli
            | Self::KiroCli
            | Self::KiloCli
            // The router leads with a Claude Code backend, so its default is the
            // CLI default. A backend it steps aside to reports its own model
            // through `name()` and `default_model()` once it is the live one.
            | Self::Router => Some(Self::DEFAULT_CLAUDE_SONNET_CLI_MODEL.to_owned()),
            Self::OpenAiApi => Some(
                non_empty_env(Self::OPENAI_API_MODEL_ENV_VAR)
                    .unwrap_or_else(|| Self::DEFAULT_OPENAI_API_MODEL.to_owned()),
            ),
        }
    }
}

/// The value of `name`, when it is set and not blank.
fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// The variables `PIERRE_LLM_CHAIN` replaced (carnet#875).
///
/// Nothing reads their values. They are named only so a process still
/// carrying them says so at boot: with the chain unset, a deployment or a
/// developer `.envrc` that sets only these would otherwise run the platform
/// default — Gemini alone, no tier behind it — without a word.
const RETIRED_CHAIN_ENV_VARS: [&str; 10] = [
    "PIERRE_LLM_PROVIDER",
    "PIERRE_LLM_MODEL",
    "PIERRE_LLM_RUNTIME_FALLBACK",
    "PIERRE_LLM_FALLBACK_ENABLED",
    "PIERRE_LLM_FALLBACK_MODEL",
    "PIERRE_LLM_FALLBACK_PROVIDER",
    "PIERRE_LLM_FALLBACK_PROVIDER_MODEL",
    "PIERRE_LLM_FALLBACK_WAIT_SECS",
    "PIERRE_LLM_TERTIARY_PROVIDER",
    "PIERRE_LLM_TERTIARY_PROVIDER_MODEL",
];

/// One tier of the LLM chain as configured: a provider and, optionally, the
/// model it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainEntry {
    /// The provider this tier is built as.
    pub provider: LlmProviderType,
    /// The model the entry names (`provider=model`); `None` leaves the
    /// provider's own default, as the environment sets it.
    pub model: Option<String>,
}

impl ChainEntry {
    /// The model this tier runs: the entry's own, else the provider's default.
    #[must_use]
    pub fn effective_model(&self) -> Option<String> {
        self.model
            .clone()
            .or_else(|| self.provider.default_model_from_env())
    }
}

impl Display for ChainEntry {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match &self.model {
            Some(model) => write!(f, "{}={model}", self.provider),
            None => write!(f, "{}", self.provider),
        }
    }
}

/// The ordered LLM chain the platform serves turns on, from `PIERRE_LLM_CHAIN`.
///
/// The variable is a comma-separated list of entries, each `provider` or
/// `provider=model`; the first entry is the primary, every later one the
/// tier a provider fault or an empty answer falls through to, in order:
///
/// ```text
/// PIERRE_LLM_CHAIN=anthropic=claude-haiku-5-5,copilot_sdk=claude-sonnet-5.5,claude_code=claude-sonnet-5,gemini=gemini-flash-lite-latest
/// ```
///
/// One entry is a single provider with no fallback to another one; a pooled
/// provider's further accounts (`CLAUDE_CODE_OAUTH_TOKEN_2`,
/// `ANTHROPIC_API_KEY_2`, …) still follow it, whatever its position. Unset or
/// blank is the platform default: one Gemini entry with no model of its own,
/// which runs `PIERRE_LLM_DEFAULT_MODEL`.
///
/// The grammar is forgiving where a mistake is harmless and loud where it is
/// not, and every repair is recorded in [`Self::issues`] for the boot log:
/// - whitespace around entries, names and models is trimmed; provider names
///   are case-insensitive, models are kept as written;
/// - an empty entry (`a,,b`, a trailing comma) is skipped;
/// - `provider=` (an empty model) is the provider's default model;
/// - an unknown provider name is skipped, never reinterpreted as another
///   provider — a typo must not boot a tier nobody configured;
/// - a provider named twice keeps its first entry: the later one would be
///   the same account behind itself;
/// - a value with no valid entry at all is the platform default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmChain {
    entries: Vec<ChainEntry>,
    issues: Vec<String>,
}

impl LlmChain {
    /// The environment variable the chain is read from.
    pub const ENV_VAR: &'static str = "PIERRE_LLM_CHAIN";

    /// The chain the environment configures; see the type docs for the grammar.
    #[must_use]
    pub fn from_env() -> Self {
        non_empty_env(Self::ENV_VAR).map_or_else(Self::platform_default, |raw| Self::parse(&raw))
    }

    /// Whether `PIERRE_LLM_CHAIN` is set to a non-blank value — the operator
    /// chose the chain, rather than the platform default applying.
    #[must_use]
    pub fn is_configured() -> bool {
        non_empty_env(Self::ENV_VAR).is_some()
    }

    /// The variables `PIERRE_LLM_CHAIN` replaced that are still set, by
    /// name. Their values are never read: they are named only so a boot that
    /// still carries them says so.
    #[must_use]
    pub(crate) fn retired_variables_set() -> Vec<&'static str> {
        RETIRED_CHAIN_ENV_VARS
            .into_iter()
            .filter(|name| non_empty_env(name).is_some())
            .collect()
    }

    /// The model the chain's head runs, which is the model a turn asks for
    /// and the one a new conversation records.
    #[must_use]
    pub fn head_model_from_env() -> Option<String> {
        Self::from_env().head().effective_model()
    }

    /// Parse a chain value; see the type docs for the grammar.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let mut entries: Vec<ChainEntry> = Vec::new();
        let mut issues = Vec::new();
        for (position, item) in raw.split(',').enumerate() {
            let item = item.trim();
            if item.is_empty() {
                issues.push(format!("entry {} is empty; skipped", position + 1));
                continue;
            }
            let (name, model) = match item.split_once('=') {
                Some((name, model)) => (name.trim(), Some(model.trim())),
                None => (item, None),
            };
            let Some(provider) = LlmProviderType::parse(name) else {
                issues.push(format!(
                    "entry {} names no known provider ({name:?}); skipped",
                    position + 1
                ));
                continue;
            };
            if entries.iter().any(|entry| entry.provider == provider) {
                issues.push(format!(
                    "entry {} repeats {provider}; the first {provider} entry is kept",
                    position + 1
                ));
                continue;
            }
            entries.push(ChainEntry {
                provider,
                model: model.filter(|m| !m.is_empty()).map(str::to_owned),
            });
        }
        if entries.is_empty() {
            issues.push(format!(
                "{} has no valid entry; using the platform default ({})",
                Self::ENV_VAR,
                LlmProviderType::default()
            ));
            return Self {
                issues,
                ..Self::platform_default()
            };
        }
        Self { entries, issues }
    }

    /// One Gemini entry on its default model: what an unset chain runs.
    fn platform_default() -> Self {
        Self {
            entries: vec![ChainEntry {
                provider: LlmProviderType::default(),
                model: None,
            }],
            issues: Vec::new(),
        }
    }

    /// The entries in chain order, head first. Never empty.
    #[must_use]
    pub fn entries(&self) -> &[ChainEntry] {
        &self.entries
    }

    /// The primary: the entry every turn is asked of first.
    #[must_use]
    pub fn head(&self) -> &ChainEntry {
        // Every constructor leaves at least one entry.
        &self.entries[0]
    }

    /// What parsing repaired or dropped, for the boot log.
    #[must_use]
    pub fn issues(&self) -> &[String] {
        &self.issues
    }
}

impl Display for LlmChain {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        for (position, entry) in self.entries.iter().enumerate() {
            if position > 0 {
                write!(f, ",")?;
            }
            write!(f, "{entry}")?;
        }
        Ok(())
    }
}

impl Display for LlmProviderType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Groq => write!(f, "groq"),
            Self::Gemini => write!(f, "gemini"),
            Self::Local => write!(f, "local"),
            Self::OpenRouter => write!(f, "openrouter"),
            Self::Cohere => write!(f, "cohere"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::ClaudeCode => write!(f, "claude_code"),
            Self::CursorAgent => write!(f, "cursor_agent"),
            Self::OpenCode => write!(f, "opencode"),
            Self::Copilot => write!(f, "copilot"),
            Self::CopilotHeadless => write!(f, "copilot_headless"),
            Self::CopilotSdk => write!(f, "copilot_sdk"),
            Self::WarpCli => write!(f, "warp_cli"),
            Self::GeminiCli => write!(f, "gemini_cli"),
            Self::CodexCli => write!(f, "codex_cli"),
            Self::GooseCli => write!(f, "goose_cli"),
            Self::ClineCli => write!(f, "cline_cli"),
            Self::ContinueCli => write!(f, "continue_cli"),
            Self::KiroCli => write!(f, "kiro_cli"),
            Self::KiloCli => write!(f, "kilo_cli"),
            Self::OpenAiApi => write!(f, "openai_api"),
            Self::Router => write!(f, "router"),
        }
    }
}

/// The platform's unified model configuration.
///
/// Reads `PIERRE_LLM_DEFAULT_MODEL`, the model a Gemini primary and a BYO
/// Gemini tenant without a stored model run on.
#[derive(Debug, Clone)]
pub struct LlmModelConfig {
    /// Default model to use for all requests
    pub default_model: String,
}

impl LlmModelConfig {
    /// Environment variable for default model
    pub const DEFAULT_MODEL_ENV_VAR: &'static str = "PIERRE_LLM_DEFAULT_MODEL";

    /// Load model configuration from environment variables
    ///
    /// # Errors
    ///
    /// Returns an error if `PIERRE_LLM_DEFAULT_MODEL` is not set.
    pub fn from_env() -> Result<Self, String> {
        let default_model = env::var(Self::DEFAULT_MODEL_ENV_VAR).map_err(|_| {
            format!(
                "{} environment variable not set",
                Self::DEFAULT_MODEL_ENV_VAR
            )
        })?;

        Ok(Self { default_model })
    }

    /// Get the default model name
    #[must_use]
    pub fn default_model(&self) -> &str {
        &self.default_model
    }
}

#[cfg(test)]
mod tests {
    use super::{ChainEntry, LlmChain, LlmModelConfig, LlmProviderType, RETIRED_CHAIN_ENV_VARS};

    #[test]
    fn no_variable_still_read_is_reported_as_retired() {
        for live in [LlmChain::ENV_VAR, LlmModelConfig::DEFAULT_MODEL_ENV_VAR] {
            assert!(
                !RETIRED_CHAIN_ENV_VARS.contains(&live),
                "{live} is still read; reporting it as retired would tell an operator to remove it"
            );
        }
    }

    fn entry(provider: LlmProviderType, model: Option<&str>) -> ChainEntry {
        ChainEntry {
            provider,
            model: model.map(str::to_owned),
        }
    }

    #[test]
    fn the_deployed_order_parses_with_a_model_per_entry() {
        let chain = LlmChain::parse(
            "anthropic=claude-haiku-5-5,copilot_sdk=claude-sonnet-5.5,claude_code=claude-sonnet-5,gemini=gemini-flash-lite-latest",
        );
        assert_eq!(
            chain.entries(),
            [
                entry(LlmProviderType::Anthropic, Some("claude-haiku-5-5")),
                entry(LlmProviderType::CopilotSdk, Some("claude-sonnet-5.5")),
                entry(LlmProviderType::ClaudeCode, Some("claude-sonnet-5")),
                entry(LlmProviderType::Gemini, Some("gemini-flash-lite-latest")),
            ]
        );
        assert_eq!(chain.head().provider, LlmProviderType::Anthropic);
        assert!(chain.issues().is_empty(), "{:?}", chain.issues());
    }

    #[test]
    fn a_bare_provider_has_no_model_of_its_own() {
        let chain = LlmChain::parse("copilot_sdk");
        assert_eq!(chain.entries(), [entry(LlmProviderType::CopilotSdk, None)]);
    }

    #[test]
    fn whitespace_and_name_case_are_forgiven_but_a_model_is_kept_as_written() {
        let chain = LlmChain::parse("  Copilot_SDK = Claude-Sonnet-5.5 ,  GEMINI ");
        assert_eq!(
            chain.entries(),
            [
                entry(LlmProviderType::CopilotSdk, Some("Claude-Sonnet-5.5")),
                entry(LlmProviderType::Gemini, None),
            ]
        );
    }

    #[test]
    fn an_empty_model_is_the_provider_default() {
        let chain = LlmChain::parse("gemini=");
        assert_eq!(chain.entries(), [entry(LlmProviderType::Gemini, None)]);
    }

    #[test]
    fn empty_entries_are_skipped_and_reported() {
        let chain = LlmChain::parse("copilot_sdk,,gemini,");
        assert_eq!(
            chain.entries(),
            [
                entry(LlmProviderType::CopilotSdk, None),
                entry(LlmProviderType::Gemini, None),
            ]
        );
        assert_eq!(chain.issues().len(), 2, "{:?}", chain.issues());
    }

    #[test]
    fn an_unknown_provider_is_skipped_never_reinterpreted() {
        let chain = LlmChain::parse("anthropc=claude-haiku-5-5,copilot_sdk");
        assert_eq!(
            chain.entries(),
            [entry(LlmProviderType::CopilotSdk, None)],
            "a typo must not become a Gemini tier"
        );
        assert!(
            chain.issues()[0].contains("anthropc"),
            "{:?}",
            chain.issues()
        );
    }

    #[test]
    fn a_repeated_provider_keeps_its_first_entry() {
        let chain = LlmChain::parse("claude_code=claude-sonnet-5,gemini,claude-code=opus");
        assert_eq!(
            chain.entries(),
            [
                entry(LlmProviderType::ClaudeCode, Some("claude-sonnet-5")),
                entry(LlmProviderType::Gemini, None),
            ]
        );
        assert!(chain.issues()[0].contains("repeats claude_code"));
    }

    #[test]
    fn a_pool_tier_name_is_not_an_entry() {
        // `anthropic#2` comes from ANTHROPIC_API_KEY_2, never from the list.
        let chain = LlmChain::parse("anthropic,anthropic#2");
        assert_eq!(chain.entries(), [entry(LlmProviderType::Anthropic, None)]);
    }

    #[test]
    fn no_valid_entry_is_the_platform_default_and_says_so() {
        let chain = LlmChain::parse("nope, ,");
        assert_eq!(chain.entries(), [entry(LlmProviderType::Gemini, None)]);
        assert!(chain
            .issues()
            .last()
            .is_some_and(|issue| issue.contains("platform default")));
    }

    #[test]
    fn the_chain_renders_in_its_own_grammar() {
        let raw = "anthropic=claude-haiku-5-5,copilot_sdk,gemini=gemini-flash-lite-latest";
        assert_eq!(LlmChain::parse(raw).to_string(), raw);
    }

    #[test]
    fn every_provider_name_round_trips_through_the_parser() {
        for provider in [
            LlmProviderType::Groq,
            LlmProviderType::Gemini,
            LlmProviderType::Local,
            LlmProviderType::OpenRouter,
            LlmProviderType::Cohere,
            LlmProviderType::Anthropic,
            LlmProviderType::ClaudeCode,
            LlmProviderType::CursorAgent,
            LlmProviderType::OpenCode,
            LlmProviderType::Copilot,
            LlmProviderType::CopilotHeadless,
            LlmProviderType::CopilotSdk,
            LlmProviderType::WarpCli,
            LlmProviderType::GeminiCli,
            LlmProviderType::CodexCli,
            LlmProviderType::GooseCli,
            LlmProviderType::ClineCli,
            LlmProviderType::ContinueCli,
            LlmProviderType::KiroCli,
            LlmProviderType::KiloCli,
            LlmProviderType::Router,
            LlmProviderType::OpenAiApi,
        ] {
            assert_eq!(
                LlmProviderType::parse(&provider.to_string()),
                Some(provider),
                "{provider} must be selectable by the name it logs under"
            );
        }
    }

    #[test]
    fn the_anthropic_api_does_not_spend_the_copilot_token() {
        // The chain guard passes a tier over for a low GitHub budget only when
        // it spends that budget; an Anthropic head must never be skipped for it.
        assert!(!LlmProviderType::Anthropic.spends_copilot_github_token());
        assert!(LlmProviderType::CopilotSdk.spends_copilot_github_token());
    }

    #[test]
    fn an_entry_model_wins_over_the_provider_default() {
        let named = entry(LlmProviderType::Anthropic, Some("claude-sonnet-5-5"));
        assert_eq!(
            named.effective_model().as_deref(),
            Some("claude-sonnet-5-5")
        );
        let cli = entry(LlmProviderType::ClaudeCode, None);
        assert_eq!(cli.effective_model().as_deref(), Some("opus"));
    }
}
