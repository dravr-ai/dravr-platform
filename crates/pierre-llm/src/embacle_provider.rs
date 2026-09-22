// ABOUTME: The one platform facade over every embacle runner: HTTP provider, CLI subprocess, Copilot turn provider, quota router, or a fallback chain over them
// ABOUTME: Env-driven construction, the chain assembly, the headless turn handle, and RunnerError-to-AppError bridging
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use embacle::auth::check_readiness;
use embacle::config::parse_timeout;
use embacle::pool::{credential_env_key, PooledTier};
use embacle::quota_http::{AnthropicUsageChecker, GithubHeadroomChecker};
use embacle::router::{Backend, PreferInOrder, RouterProvider};
use embacle::types::LlmProvider as EmbacleLlmProvider;
use embacle::{
    ClaudeCodeRunner, CliRunnerType, ClineCliRunner, CodexCliRunner, ContinueCliRunner,
    CopilotHeadlessConfig, CopilotHeadlessRunner, CopilotRunner, CopilotSdkConfig,
    CopilotSdkRunner, CursorAgentRunner, FallbackProvider, GeminiCliRunner, GooseCliRunner,
    HeadlessTurnProvider, KiloCliRunner, KiroCliRunner, OpenAiApiConfig, OpenAiApiRunner,
    OpenCodeRunner, ResponsePolicy, RunnerConfig, WarpCliRunner,
};
use futures_util::StreamExt;
use pierre_core::http_client::llm_inner_client;
use tracing::{info, info_span, warn, Instrument, Span};

use super::{ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider};
use crate::chain_observer::ChainObserver;
use crate::config::{LlmProviderType, ProviderConstruction};
use crate::errors::AppError;
use crate::http_env;
use crate::provider::ChatProvider;

/// The platform's one facade over an embacle runner.
///
/// Every provider the platform can talk to is an embacle
/// [`LlmProvider`](EmbacleLlmProvider): the HTTP APIs (Gemini, Cohere, Groq,
/// `OpenRouter`, any OpenAI-compatible endpoint), the CLI subprocess runners,
/// the two Copilot turn providers (ACP and the Rust SDK), the quota router, and
/// a [`FallbackProvider`] chain over any of them. This type presents whichever
/// one it holds behind the platform [`LlmProvider`] trait, bridging
/// `RunnerError` to [`AppError`].
///
/// A chain reports its **head** tier's identity: `name()` is what
/// `llm_usage.provider` and the price table are keyed on, and
/// `capabilities()` is what routes a turn to the native-function-calling loop
/// or the headless one. Reporting the union of the tiers would send Copilot
/// turns down the API loop.
pub struct EmbacleProvider {
    /// What `complete()`, `complete_stream()` and `health_check()` call: the
    /// runner itself, or the fallback chain over the tiers.
    runner: Arc<dyn EmbacleLlmProvider>,
    /// The first tier: what `name()`, `capabilities()`, `default_model()` and
    /// `available_models()` report. The same `Arc` as `runner` when solo.
    head: Arc<dyn EmbacleLlmProvider>,
    /// The head's Copilot turn provider, for `converse()` access — whichever
    /// transport it is, ACP or the SDK.
    turn_provider: Option<Arc<dyn HeadlessTurnProvider>>,
    /// The head's router, when it is one.
    ///
    /// `Box<dyn EmbacleLlmProvider>` cannot be downcast, and dispatch has to ask
    /// which backend is live on every turn — so the handle is kept here rather
    /// than recovered from `runner`.
    router: Option<Arc<RouterProvider>>,
    /// Cached display name (embacle returns `&str`, the platform trait needs `&'static str`)
    cached_display_name: &'static str,
    /// The chain minus its head, for the headless tool loop's re-run (see
    /// `run_headless_fallback` in pierre-tool-runtime). `None` when solo.
    fallback_tail: Option<Box<ChatProvider>>,
}

impl EmbacleProvider {
    /// Build the provider `kind` names from the process environment.
    ///
    /// `model_override` wins over `PIERRE_LLM_MODEL` for this runner; the
    /// runtime fallback chain passes `PIERRE_LLM_FALLBACK_PROVIDER_MODEL` /
    /// `PIERRE_LLM_TERTIARY_PROVIDER_MODEL` through it so a secondary gets a
    /// model in its own namespace (Copilot's `claude-opus-4.8` is not a model
    /// Gemini or Cohere can resolve).
    ///
    /// # Errors
    ///
    /// Returns `AppError` when the runner's binary cannot be resolved, its API
    /// key is unset, or the router rejects its backend set.
    pub async fn from_provider_type(
        kind: LlmProviderType,
        model_override: Option<&str>,
    ) -> Result<Self, AppError> {
        match kind.construction() {
            ProviderConstruction::HttpApi(http) => http_env::build(http, model_override),
            ProviderConstruction::Cli(runner_type) => {
                let config = cli_runner_config(runner_type, model_override)?;
                Ok(Self::build_cli(runner_type, config))
            }
            ProviderConstruction::CopilotHeadless => Ok(Self::build_headless(model_override)),
            ProviderConstruction::CopilotSdk => Ok(Self::build_sdk(model_override)),
            ProviderConstruction::Router => Self::build_router(),
            ProviderConstruction::OpenAiApi => Ok(Self::build_openai_api(model_override).await),
        }
    }

    /// Wrap an already-built embacle runner whose rate limit is the
    /// account's quota — a CLI or Copilot runner, the evals' bench candidates,
    /// and tests that script a runner.
    ///
    /// Solo — no chain, no turn provider, no router.
    #[must_use]
    pub fn from_runner(runner: Box<dyn EmbacleLlmProvider>, display_name: &'static str) -> Self {
        let runner: Arc<dyn EmbacleLlmProvider> = Arc::from(runner);
        Self {
            head: Arc::clone(&runner),
            runner,
            turn_provider: None,
            router: None,
            cached_display_name: display_name,
            fallback_tail: None,
        }
    }

    /// Chain `tiers` in order behind one provider.
    ///
    /// The chain is embacle's [`FallbackProvider`] under
    /// [`ResponsePolicy::strict`] — a provider fault or an empty completion
    /// moves the request to the next tier with its model reset; a
    /// deterministic rejection propagates — observed by the platform's
    /// guarded [`ChainObserver`], which consults the chain guard before the
    /// primary and records the primary's outcome on the breaker.
    ///
    /// One tier is that tier, unchained. The tail (every tier but the first)
    /// is kept as its own unguarded chain for the headless tool loop's re-run.
    ///
    /// # Errors
    ///
    /// Returns a config error for an empty `tiers`.
    pub fn chain(tiers: Vec<Self>) -> Result<Self, AppError> {
        Self::chain_observed(tiers, ChainObserver::guarded())
    }

    fn chain_observed(mut tiers: Vec<Self>, observer: ChainObserver) -> Result<Self, AppError> {
        if tiers.len() <= 1 {
            return tiers.pop().ok_or_else(|| {
                AppError::config("A runtime fallback chain needs at least one provider")
            });
        }

        let runners = tiers
            .iter()
            .map(|tier| Box::new(Arc::clone(&tier.runner)) as Box<dyn EmbacleLlmProvider>)
            .collect();
        let chain = FallbackProvider::new(runners)?
            .with_fallthrough(ResponsePolicy::strict())
            .with_observer(Arc::new(observer));

        let head = tiers.remove(0);
        let tail = Self::chain_observed(tiers, ChainObserver::unguarded())?;

        Ok(Self {
            runner: Arc::new(chain),
            head: head.head,
            turn_provider: head.turn_provider,
            router: head.router,
            cached_display_name: head.cached_display_name,
            fallback_tail: Some(Box::new(ChatProvider::Embacle(tail))),
        })
    }

    /// The chain minus its head, when this provider is a chain.
    ///
    /// The headless tool loop calls the head's turn provider directly, outside
    /// the chain, so the chain's own fallback never fires for it; on a provider
    /// fault it re-runs the whole loop against this tail instead.
    #[must_use]
    pub fn fallback_tail(&self) -> Option<&ChatProvider> {
        self.fallback_tail.as_deref()
    }

    /// The first tier on its own, with no chain behind it.
    ///
    /// A call through a chain exercises only whichever tier answers first, so
    /// a dead tier behind a live one is invisible until it is the only one
    /// left. The per-tier startup probe calls each tier through this, where a
    /// failure is that tier's and nobody answers for it. Shares the head's
    /// `Arc`, so a pooled or warm runner is probed as the instance that serves.
    #[must_use]
    pub fn head_alone(&self) -> Self {
        Self {
            runner: Arc::clone(&self.head),
            head: Arc::clone(&self.head),
            turn_provider: None,
            router: None,
            cached_display_name: self.cached_display_name,
            fallback_tail: None,
        }
    }

    /// The further accounts of a CLI primary, each an ordinary tier behind
    /// it: account N is the runner's config with the token found under
    /// `<CREDENTIAL>_N` (`CLAUDE_CODE_OAUTH_TOKEN_2`, `_3`, …, read until
    /// the first unset one) set on the child explicitly, named
    /// `<runner>#N` by [`PooledTier`]. Empty when the runner reads no
    /// credential from its environment, or no numbered token is set.
    ///
    /// An account whose runner fails to build is skipped with a warning:
    /// the binary is the primary's, so what breaks one breaks all, and the
    /// primary's own failure is the one the chain assembly reports.
    ///
    #[must_use]
    pub fn pooled_accounts(runner_type: CliRunnerType, model_override: Option<&str>) -> Vec<Self> {
        let Some(key) = credential_env_key(runner_type) else {
            return Vec::new();
        };
        let mut accounts = Vec::new();
        for position in 1.. {
            let Ok(token) = env::var(format!("{key}_{}", position + 1)) else {
                break;
            };
            if token.trim().is_empty() {
                break;
            }
            match cli_runner_config(runner_type, model_override) {
                Ok(config) => {
                    let runner = Self::cli_runner(runner_type, config.with_env(key, token));
                    let tier = PooledTier::new(runner, position);
                    info!(
                        runner = %runner_type,
                        account = tier.name(),
                        "Pooled a further account as a chain tier"
                    );
                    accounts.push(Self::from_runner(
                        Box::new(tier),
                        runner_display_name(runner_type),
                    ));
                }
                Err(err) => warn!(
                    runner = %runner_type,
                    account = position + 1,
                    error = %err,
                    "A pooled account did not build; skipped"
                ),
            }
        }
        accounts
    }

    /// The embacle runner for a CLI type, on this config.
    fn cli_runner(runner_type: CliRunnerType, config: RunnerConfig) -> Box<dyn EmbacleLlmProvider> {
        match runner_type {
            CliRunnerType::ClaudeCode => Box::new(ClaudeCodeRunner::new(config)),
            CliRunnerType::Copilot => Box::new(CopilotRunner::new(config)),
            CliRunnerType::CursorAgent => Box::new(CursorAgentRunner::new(config)),
            CliRunnerType::OpenCode => Box::new(OpenCodeRunner::new(config)),
            CliRunnerType::GeminiCli => Box::new(GeminiCliRunner::new(config)),
            CliRunnerType::CodexCli => Box::new(CodexCliRunner::new(config)),
            CliRunnerType::GooseCli => Box::new(GooseCliRunner::new(config)),
            CliRunnerType::ClineCli => Box::new(ClineCliRunner::new(config)),
            CliRunnerType::ContinueCli => Box::new(ContinueCliRunner::new(config)),
            CliRunnerType::WarpCli => Box::new(WarpCliRunner::new(config)),
            CliRunnerType::KiroCli => Box::new(KiroCliRunner::new(config)),
            CliRunnerType::KiloCli => Box::new(KiloCliRunner::new(config)),
            CliRunnerType::CopilotHeadless | CliRunnerType::CopilotSdk => {
                unreachable!("the Copilot runtimes are built by build_headless and build_sdk")
            }
        }
    }

    /// Build a CLI subprocess runner
    fn build_cli(runner_type: CliRunnerType, config: RunnerConfig) -> Self {
        match runner_type {
            CliRunnerType::CopilotHeadless => return Self::build_headless(config.model.as_deref()),
            CliRunnerType::CopilotSdk => return Self::build_sdk(config.model.as_deref()),
            _ => {}
        }
        let binary_path = config.binary_path.clone();
        let runner = Self::cli_runner(runner_type, config);

        info!(
            runner = %runner_type,
            path = %binary_path.display(),
            model = runner.default_model(),
            available_models = ?runner.available_models(),
            "Creating CLI LLM runner"
        );

        spawn_readiness_warning(runner_type, binary_path);
        Self::from_runner(runner, runner_display_name(runner_type))
    }

    /// Build a Copilot Headless (ACP) runner (NDJSON JSON-RPC via `copilot --acp`)
    ///
    /// `model_override` wins over `PIERRE_LLM_MODEL`, which wins over the
    /// headless-specific `COPILOT_HEADLESS_MODEL`.
    ///
    /// LIMITATION(registre#104): `build_headless` binds one `CopilotHeadlessConfig::model` for every call in the turn — tool-loop iterations and the athlete-facing draft run on the same model, with no per-stage routing to a cheaper one.
    fn build_headless(model_override: Option<&str>) -> Self {
        let mut config = CopilotHeadlessConfig::from_env();
        if let Some(model) = unified_model(model_override) {
            config.model = model;
        }

        info!(model = %config.model, "Creating Copilot Headless runner (copilot --acp)");

        let headless: Arc<dyn HeadlessTurnProvider> =
            Arc::new(CopilotHeadlessRunner::with_config(config));
        Self::from_turn_provider(headless, "GitHub Copilot (Headless)")
    }

    /// Build a Copilot SDK runner (GitHub's Rust runtime over `copilot-runtime --server --stdio`).
    ///
    /// Same runtime the ACP path reaches, without the JS adapter: the system
    /// prompt travels in the runtime's own slot, the served model and cache
    /// counts come off `assistant.usage`, tool executions carry name, arguments
    /// and result, and an unknown model id fails instead of being remapped.
    /// `model_override` wins over `PIERRE_LLM_MODEL`, which wins over the
    /// SDK-specific `COPILOT_SDK_MODEL`.
    ///
    /// LIMITATION(registre#104): `build_sdk` binds one `CopilotSdkConfig::model` for every call in the turn — tool-loop iterations and the athlete-facing draft run on the same model, with no per-stage routing to a cheaper one.
    fn build_sdk(model_override: Option<&str>) -> Self {
        let mut config = CopilotSdkConfig::from_env();
        if let Some(model) = unified_model(model_override) {
            config.model = model;
        }

        info!(
            model = %config.model,
            runtime = ?config.runtime_path,
            "Creating Copilot SDK runner (copilot-runtime --server --stdio)"
        );

        let sdk: Arc<dyn HeadlessTurnProvider> = Arc::new(CopilotSdkRunner::with_config(config));
        Self::from_turn_provider(sdk, "GitHub Copilot (SDK)")
    }

    /// A Copilot turn provider, kept both as the runner and as the typed
    /// handle the headless tool loop converses through.
    ///
    /// LIMITATION(registre#102): nothing on this path marks the system-prompt +
    /// tool-surface prefix cacheable, and nothing can. `cache_control` is settable
    /// only on a request we build ourselves; here the ACP agent builds it. The gap
    /// is not an omission on our side — the ACP schema defines the two cache counts
    /// on `Usage` and no way to influence them (no `cache_control`, no breakpoint,
    /// no ephemeral marker anywhere in the protocol), and Copilot CLI 1.0.81
    /// advertises `loadSession`, `mcpCapabilities`, `promptCapabilities` and
    /// `sessionCapabilities{close,list}`, with no caching capability at all.
    ///
    /// Copilot does cache, and since embacle 0.22.0 the counts are reported and
    /// billed — but it caches its OWN preamble, never our prompt. Measured
    /// 2026-08-29 by `examples/acp_cache_boundary_probe.rs` in dravr-embacle, on
    /// CLI 1.0.81 / claude-sonnet-5: prefixes of 32, 10k, 20k and 40k tokens were
    /// each served exactly 13,964 cached tokens on the following turn — identical
    /// to the token across a 1,250x change in what we send — while the cache
    /// *write* tracked our prompt size (14,214 / 26,276 / 38,371 / 62,564). We pay
    /// the write premium on the whole prompt every turn and are served none of it
    /// back.
    ///
    /// So prompt LAYOUT is not a lever on this path: no ordering of our blocks can
    /// move `cachedReadTokens`, because our bytes are never in the cached region.
    /// Prompt SIZE is the only thing on our side of the boundary. Re-run the probe
    /// before believing otherwise; a vendor that began honouring our prefix would
    /// show the cached read growing with it.
    fn from_turn_provider(
        turn_provider: Arc<dyn HeadlessTurnProvider>,
        display_name: &'static str,
    ) -> Self {
        let runner = Arc::clone(&turn_provider) as Arc<dyn EmbacleLlmProvider>;
        Self {
            head: Arc::clone(&runner),
            runner,
            turn_provider: Some(turn_provider),
            router: None,
            cached_display_name: display_name,
            fallback_tail: None,
        }
    }

    /// Build the quota-aware router: Claude Code in front, Copilot Headless behind.
    ///
    /// Both backends are constructed once. `CopilotHeadlessRunner` pools
    /// subprocesses and caches its observed model list in a `OnceLock`, so
    /// rebuilding it on every switch would throw both away.
    ///
    /// The lead backend is metered against the real Anthropic budget, so the
    /// router steps aside *before* a turn is spent rather than after one fails.
    /// The Copilot backend is metered too, but on GitHub's core rate-limit
    /// headroom — a proxy, not the premium-request quota, because no endpoint
    /// for the latter exists. It is deliberately second: a proxy is enough to
    /// notice a collapsing budget, not enough to lead on.
    ///
    /// # Errors
    ///
    /// Returns `AppError` when the Claude Code binary cannot be resolved or the
    /// router rejects the backend set.
    fn build_router() -> Result<Self, AppError> {
        let claude_config = cli_runner_config(CliRunnerType::ClaudeCode, None)?;
        let claude: Box<dyn EmbacleLlmProvider> = Box::new(ClaudeCodeRunner::new(claude_config));

        let mut headless_config = CopilotHeadlessConfig::from_env();
        if let Some(model) = unified_model(None) {
            headless_config.model = model;
        }
        let headless: Arc<dyn HeadlessTurnProvider> =
            Arc::new(CopilotHeadlessRunner::with_config(headless_config));

        let lead = match env::var("CLAUDE_CODE_OAUTH_TOKEN") {
            Ok(token) if !token.is_empty() => {
                Backend::metered(claude, Box::new(AnthropicUsageChecker::new(token)))
            }
            // Without the token the budget cannot be read. Unmetered is the
            // honest state: the router still leads with Claude Code and still
            // reroutes on a refusal, it simply cannot step aside in advance.
            _ => {
                warn!(
                    "CLAUDE_CODE_OAUTH_TOKEN is unset — the router leads with Claude Code but \
                     cannot read its budget, so it can only fall back reactively"
                );
                Backend::unmetered(claude)
            }
        };

        let fallback = match env::var("COPILOT_GITHUB_TOKEN") {
            Ok(token) if !token.is_empty() => Backend::metered(
                Box::new(Arc::clone(&headless)),
                Box::new(GithubHeadroomChecker::new(token)),
            ),
            _ => Backend::unmetered(Box::new(Arc::clone(&headless))),
        }
        .with_headless(Arc::clone(&headless));

        let mut router = RouterProvider::new(vec![lead, fallback], Box::new(PreferInOrder))
            .map_err(|e| AppError::config(format!("router: {}", e.message)))?;
        if let Some(threshold) = threshold_from_env() {
            router = router.with_threshold(threshold);
        }
        let router = Arc::new(router);

        info!(
            threshold = threshold_from_env().unwrap_or(80.0),
            "Creating quota router (claude_code -> copilot_headless)"
        );

        let runner = Arc::clone(&router) as Arc<dyn EmbacleLlmProvider>;
        Ok(Self {
            head: Arc::clone(&runner),
            runner,
            turn_provider: None,
            router: Some(router),
            cached_display_name: "Quota Router",
            fallback_tail: None,
        })
    }

    /// Build an `OpenAI`-compatible HTTP API runner (via embacle `OpenAiApiRunner`)
    ///
    /// Reads configuration from `OPENAI_API_*` env vars. `model_override`
    /// wins over `PIERRE_LLM_MODEL`, which wins over `OPENAI_API_MODEL`.
    async fn build_openai_api(model_override: Option<&str>) -> Self {
        let mut config = OpenAiApiConfig::from_env();
        if let Some(model) = unified_model(model_override) {
            config.model = model;
        }

        info!(
            base_url = %config.base_url,
            model = %config.model,
            "Creating OpenAI API runner"
        );

        let client = llm_inner_client().clone();
        let runner = OpenAiApiRunner::with_client(config, client).await;
        Self::from_runner(Box::new(runner), "OpenAI API")
    }

    /// Access the Copilot turn provider currently able to serve a native tool turn.
    ///
    /// `Some` for a Copilot Headless or SDK provider, and — for the router —
    /// only while Copilot is the backend actually answering. `None` otherwise,
    /// which is the correct answer rather than a missing capability: when the
    /// router has Claude Code live, dispatch must take the CLI text loop,
    /// because `claude -p` runs its own tool loop and takes `--mcp-config`.
    /// Handing back a stale runner would run the ACP loop against a provider
    /// that is not serving the turn.
    ///
    /// For a chain this is the head's, which is the tier the headless tool
    /// loop bypasses the chain to reach.
    ///
    /// Returns an owned `Arc` rather than a reference because the router
    /// resolves the live backend per call; there is no field to borrow from.
    #[must_use]
    pub fn as_turn_provider(&self) -> Option<Arc<dyn HeadlessTurnProvider>> {
        if let Some(router) = &self.router {
            return router.active_runner();
        }
        self.turn_provider.clone()
    }
}

impl fmt::Debug for EmbacleProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmbacleProvider")
            .field("runner", &self.runner.name())
            .field("head", &self.head.name())
            .field(
                "turn_provider",
                &self.turn_provider.as_ref().map(EmbacleLlmProvider::name),
            )
            .field("router", &self.router.is_some())
            .field("cached_display_name", &self.cached_display_name)
            .field("fallback_tail", &self.fallback_tail)
            .finish()
    }
}

#[async_trait]
impl LlmProvider for EmbacleProvider {
    fn name(&self) -> &'static str {
        self.head.name()
    }

    fn display_name(&self) -> &'static str {
        self.cached_display_name
    }

    fn capabilities(&self) -> LlmCapabilities {
        self.head.capabilities()
    }

    fn default_model(&self) -> &str {
        self.head.default_model()
    }

    fn available_models(&self) -> &[String] {
        self.head.available_models()
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        EmbacleLlmProvider::complete(&*self.runner, request)
            .instrument(self.request_span("complete", request))
            .await
            .map_err(AppError::from)
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let embacle_stream = EmbacleLlmProvider::complete_stream(&*self.runner, request)
            .instrument(self.request_span("complete_stream", request))
            .await
            .map_err(AppError::from)?;

        Ok(Box::pin(
            embacle_stream.map(|result| result.map_err(AppError::from)),
        ))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        EmbacleLlmProvider::health_check(&*self.runner)
            .instrument(info_span!(
                "llm.request",
                provider = self.name(),
                op = "health_check"
            ))
            .await
            .map_err(AppError::from)
    }
}

impl EmbacleProvider {
    /// The span an outbound LLM call runs under. embacle's providers take a
    /// plain `reqwest::Client`, so the trace tree records the call here — by
    /// head provider and the model the request resolves to — rather than
    /// through client middleware.
    fn request_span(&self, op: &'static str, request: &ChatRequest) -> Span {
        info_span!(
            "llm.request",
            provider = self.name(),
            model = request
                .model
                .as_deref()
                .unwrap_or_else(|| self.default_model()),
            op,
        )
    }
}

/// The model a Copilot or `OpenAI` API runner is built with: the explicit
/// override first, then `PIERRE_LLM_MODEL`, the unified override for every
/// provider. `None` leaves the runner's own env-derived default in place.
fn unified_model(model_override: Option<&str>) -> Option<String> {
    model_override
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .or_else(|| env::var("PIERRE_LLM_MODEL").ok().filter(|m| !m.is_empty()))
}

/// Check a CLI runner's readiness off the constructing task and warn when it
/// is not ready — the operator's boot-time signal that a runner will fail its
/// first turn (binary missing, not logged in), without blocking construction.
fn spawn_readiness_warning(runner_type: CliRunnerType, binary_path: PathBuf) {
    tokio::spawn(async move {
        let readiness = check_readiness(&runner_type, &binary_path).await;
        match readiness {
            Ok(status) if status.is_ready() => {}
            Ok(status) => warn!(
                runner = %runner_type,
                status = %status,
                "CLI LLM runner is not ready"
            ),
            Err(error) => warn!(
                runner = %runner_type,
                error = %error,
                "CLI LLM runner readiness could not be checked"
            ),
        }
    });
}

/// The environment variables a CLI runner authenticates from.
///
/// embacle's sandbox clears the child's environment and passes only
/// `default_allowed_env_keys()` — `HOME`, `PATH`, `TERM`, `USER`, `LANG`. A
/// CLI reads its credential from its own variable, so that variable must pass
/// too or every call fails before reaching the API: Claude Code answers
/// "Not logged in · Please run /login" with an exit code of 1 and zero
/// tokens, which is how `claude_code` served no turn on 2026-09-21 while the
/// chain's span label said it had. The match is exhaustive so a new runner
/// declares its credential, or its absence, here.
#[must_use]
pub const fn cli_credential_env_keys(runner_type: CliRunnerType) -> &'static [&'static str] {
    match runner_type {
        CliRunnerType::ClaudeCode => &["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"],
        CliRunnerType::Copilot | CliRunnerType::CopilotHeadless | CliRunnerType::CopilotSdk => {
            &["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"]
        }
        CliRunnerType::GeminiCli => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        CliRunnerType::CodexCli => &["OPENAI_API_KEY"],
        // No credential this platform provisions: the runner reads a stored
        // login under HOME, which the default allowlist already passes.
        CliRunnerType::CursorAgent
        | CliRunnerType::OpenCode
        | CliRunnerType::GooseCli
        | CliRunnerType::ClineCli
        | CliRunnerType::ContinueCli
        | CliRunnerType::WarpCli
        | CliRunnerType::KiroCli
        | CliRunnerType::KiloCli => &[],
    }
}

/// Build a `RunnerConfig` for a runner type from environment variables,
/// with `model_override` winning over `PIERRE_LLM_MODEL` / `CLI_LLM_MODEL`.
///
/// Resolves the binary path via `CLI_LLM_BINARY` env var override or `which`
/// discovery, lets the runner's credential variables through the sandbox
/// ([`cli_credential_env_keys`]), then applies `CLI_LLM_*` overrides for
/// model, timeout, and args.
///
/// # Errors
///
/// Returns `AppError` when the runner's binary cannot be resolved.
pub fn cli_runner_config(
    runner_type: CliRunnerType,
    model_override: Option<&str>,
) -> Result<RunnerConfig, AppError> {
    let binary_override = env::var("CLI_LLM_BINARY").ok();
    let binary_path =
        embacle::resolve_binary(runner_type.binary_name(), binary_override.as_deref())?;

    let mut config = RunnerConfig::new(binary_path);
    let mut allowed = config.allowed_env_keys.clone();
    allowed.extend(
        cli_credential_env_keys(runner_type)
            .iter()
            .map(|key| (*key).to_owned()),
    );
    config = config.with_allowed_env_keys(allowed);
    config = apply_env_overrides(config, model_override);
    Ok(config)
}

/// Apply environment variable overrides to a `RunnerConfig`
///
/// Model resolution order (highest priority first):
///   1. `model_override` argument (used by the runtime fallback chain to
///      give the secondary a different model than the primary)
///   2. `PIERRE_LLM_MODEL`
///   3. `CLI_LLM_MODEL`
fn apply_env_overrides(mut config: RunnerConfig, model_override: Option<&str>) -> RunnerConfig {
    let model = model_override
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            env::var("PIERRE_LLM_MODEL")
                .or_else(|_| env::var("CLI_LLM_MODEL"))
                .ok()
                .filter(|m| !m.is_empty())
        });
    if let Some(model) = model {
        config = config.with_model(model);
    }

    if let Ok(timeout_str) = env::var("CLI_LLM_TIMEOUT_SECS") {
        if let Ok(timeout) = parse_timeout(&timeout_str) {
            config = config.with_timeout(timeout);
        }
    }

    if let Ok(extra) = env::var("CLI_LLM_EXTRA_ARGS") {
        let args: Vec<String> = extra
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        if !args.is_empty() {
            config = config.with_extra_args(args);
        }
    }

    if let Ok(workdir) = env::var("CLI_LLM_WORKING_DIR") {
        if !workdir.is_empty() {
            config = config.with_working_directory(PathBuf::from(workdir));
        }
    }

    config
}

/// Map a CLI runner type to its static display name string
const fn runner_display_name(runner_type: CliRunnerType) -> &'static str {
    match runner_type {
        CliRunnerType::ClaudeCode => "Claude Code",
        CliRunnerType::Copilot => "GitHub Copilot (CLI)",
        CliRunnerType::CursorAgent => "Cursor Agent",
        CliRunnerType::OpenCode => "OpenCode",
        CliRunnerType::GeminiCli => "Gemini (CLI)",
        CliRunnerType::CodexCli => "Codex (CLI)",
        CliRunnerType::GooseCli => "Goose (CLI)",
        CliRunnerType::ClineCli => "Cline (CLI)",
        CliRunnerType::ContinueCli => "Continue (CLI)",
        CliRunnerType::WarpCli => "Warp (CLI)",
        CliRunnerType::KiroCli => "Kiro (CLI)",
        CliRunnerType::KiloCli => "Kilo Code (CLI)",
        CliRunnerType::CopilotHeadless => "GitHub Copilot (Headless)",
        CliRunnerType::CopilotSdk => "GitHub Copilot (SDK)",
    }
}

/// The share of a window at which the router steps aside, from the environment.
///
/// Absent or unparseable means the router's own default. A threshold outside
/// 0-100 is refused rather than clamped: a typo that reads as "never step
/// aside" is the failure this whole provider exists to prevent.
fn threshold_from_env() -> Option<f32> {
    let raw = env::var("PIERRE_LLM_ROUTER_THRESHOLD").ok()?;
    match raw.trim().parse::<f32>() {
        Ok(pct) if (0.0..=100.0).contains(&pct) => Some(pct),
        _ => {
            warn!(
                value = %raw,
                "PIERRE_LLM_ROUTER_THRESHOLD is not a percentage between 0 and 100 — using the \
                 router default instead of a value that could disable stepping aside"
            );
            None
        }
    }
}
