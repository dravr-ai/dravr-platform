// ABOUTME: Builds embacle's HTTP providers (Anthropic, Gemini, Cohere, Groq, OpenRouter, OpenAI-compatible) for the platform
// ABOUTME: From the platform's env names at boot, or from a tenant's stored credentials for the BYO path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Construction of embacle's `http-api` providers.
//!
//! The vendor quirks, retries and SSE plumbing live in embacle; what stays
//! here is the platform's contract with its environment. Every provider gets
//! the shared LLM `reqwest::Client` (one pool, one set of timeouts), goes
//! through [`EmbacleProvider::from_runner`] like every other tier — a vendor
//! 429 reaches the platform as `ExternalRateLimited`, the upstream fault every
//! provider's rate limit or spent quota carries — and the model comes from
//! the model the tier's `PIERRE_LLM_CHAIN` entry names (`provider=model`)
//! before any vendor default.
//!
//! Gemini's boot-time model comes from `PIERRE_LLM_DEFAULT_MODEL` when its
//! entry names none, so a Gemini primary keeps the configuration it always
//! had. The others read their own `*_DEFAULT_MODEL` through embacle's
//! `from_env()`.
//!
//! Anthropic is the one HTTP provider that pools: each further key
//! (`ANTHROPIC_API_KEY_2`, `_3`, …, one per Anthropic organization with its
//! own prepaid credit) is its own tier right behind the first, named
//! `anthropic#N` — see `anthropic_pooled_accounts`.

use embacle::pool::PooledTier;
use embacle::types::{ChatMessage, ChatRequest, LlmProvider as EmbacleLlmProvider};
use embacle::{
    AnthropicCacheTtl, AnthropicConfig, AnthropicProvider, CohereConfig, CohereProvider,
    FunctionDeclaration, GeminiConfig, GeminiProvider, GroqConfig, GroqProvider,
    OpenAiCompatibleConfig, OpenAiCompatibleProvider, OpenRouterConfig, OpenRouterProvider,
};
use pierre_core::http_client::llm_inner_client;
use std::env;
use tracing::{debug, info};

use crate::config::{ChainEntry, HttpProvider, LlmModelConfig};
use crate::embacle_provider::EmbacleProvider;
use crate::errors::AppError;
use crate::LlmCapabilities;

/// Build the HTTP provider `kind` names from the process environment.
///
/// `model_override` wins over every env var; it is the model the tier's chain
/// entry names, in that provider's own namespace.
///
/// # Errors
///
/// Returns `AppError` when the provider's API key (or, for Gemini,
/// `PIERRE_LLM_DEFAULT_MODEL`) is unset.
pub(crate) fn build(
    kind: HttpProvider,
    model_override: Option<&str>,
) -> Result<EmbacleProvider, AppError> {
    match kind {
        HttpProvider::Gemini => gemini_from_env(model_override),
        HttpProvider::Groq => groq_from_env(model_override),
        HttpProvider::Cohere => cohere_from_env(model_override),
        HttpProvider::OpenRouter => openrouter_from_env(model_override),
        HttpProvider::Local => Ok(local_from_env(model_override)),
        HttpProvider::Anthropic => anthropic_from_env(model_override),
    }
}

/// Anthropic on `ANTHROPIC_API_KEY` and the `ANTHROPIC_*` settings embacle
/// reads, on `model_override` else `ANTHROPIC_DEFAULT_MODEL` else embacle's
/// default (`claude-haiku-5-5`).
///
/// LIMITATION(registre#1234): `anthropic_from_env` and `anthropic_pooled_accounts` authenticate every organization with a static `ANTHROPIC_API_KEY[_N]`, never a Workload Identity Federation token.
fn anthropic_from_env(model_override: Option<&str>) -> Result<EmbacleProvider, AppError> {
    let config = anthropic_config(AnthropicConfig::from_env()?, model_override);
    info!(model = %config.model, "Creating Anthropic provider");
    Ok(EmbacleProvider::from_runner(
        anthropic_provider(config),
        ANTHROPIC_DISPLAY_NAME,
    ))
}

/// The further accounts of the Anthropic tier, in order.
///
/// One per `ANTHROPIC_API_KEY_N` (`_2`, `_3`, …, read until the first unset
/// or blank one), every other setting shared with the first, on the model
/// `entry` gives account N (its `anthropic#N=model`, else the tier's own).
/// Account N is named `anthropic#N` by [`PooledTier`], so a usage row, an
/// alert label or a health line says which organization's credit served — and
/// pricing keys it on the `anthropic` family.
///
/// Empty when there is no second key, and when the pool cannot be read at
/// all: an unset `ANTHROPIC_API_KEY` or an invalid `ANTHROPIC_*` setting fails
/// the first account the same way, and that tier's own failure is the one the
/// chain assembly reports.
#[must_use]
pub(crate) fn anthropic_pooled_accounts(entry: &ChainEntry) -> Vec<EmbacleProvider> {
    let configs = match AnthropicConfig::pool_from_env() {
        Ok(configs) => configs,
        Err(err) => {
            debug!(error = %err, "No Anthropic key pool: the first account did not configure");
            return Vec::new();
        }
    };
    configs
        .into_iter()
        .enumerate()
        .skip(1)
        .map(|(position, config)| {
            let tier = PooledTier::new(
                anthropic_provider(anthropic_config(config, entry.account_model(position + 1))),
                position,
            );
            info!(
                account = tier.name(),
                "Pooled a further Anthropic account as a chain tier"
            );
            EmbacleProvider::from_runner(Box::new(tier), ANTHROPIC_DISPLAY_NAME)
        })
        .collect()
}

/// `config` on `model_override`, when the chain entry names one, with the
/// platform's prefix cache lifetime.
///
/// The tools and system breakpoints live one hour unless
/// `ANTHROPIC_PREFIX_CACHE_TTL` says otherwise (carnet#1190): a coaching
/// conversation's turns land more than five minutes apart, so under embacle's
/// 5-minute default every turn rewrote its whole prefix. A 1-hour write costs
/// 2x input against 1.25x and is priced at that rate end to end
/// (`cached_write_1h_tokens`), so turning this on cannot under-report COGS.
fn anthropic_config(config: AnthropicConfig, model_override: Option<&str>) -> AnthropicConfig {
    let ttl_is_set = env::var(embacle::ANTHROPIC_PREFIX_CACHE_TTL_ENV)
        .is_ok_and(|value| !value.trim().is_empty());
    let config = with_platform_prefix_ttl(config, ttl_is_set);
    match model_override {
        Some(model) => config.with_model(model),
        None => config,
    }
}

/// One hour for the cached prefix, unless the environment named a TTL —
/// then `config` already carries the one it named.
const fn with_platform_prefix_ttl(config: AnthropicConfig, ttl_is_set: bool) -> AnthropicConfig {
    if ttl_is_set {
        config
    } else {
        config.with_prefix_cache_ttl(AnthropicCacheTtl::OneHour)
    }
}

/// An Anthropic provider over the shared LLM client.
fn anthropic_provider(config: AnthropicConfig) -> Box<dyn EmbacleLlmProvider> {
    Box::new(AnthropicProvider::with_client(
        config,
        llm_inner_client().clone(),
    ))
}

/// The one-line request a tool-catalogue count is taken around
const CATALOGUE_PROBE: &str = "Hi";

/// The input tokens `tools` cost on `model`, counted by Anthropic on the
/// tier's first account (carnet#1196).
///
/// One short request is counted with the tools and again without them; the
/// difference is the catalogue plus the tool-use preamble Anthropic adds
/// whenever tools are present (286 tokens on the 5.5 models). Counting is free
/// and rate-limited apart from Messages.
///
/// # Errors
///
/// Returns `AppError` when `ANTHROPIC_API_KEY` is unset or invalid, or when
/// the API refuses either count.
pub async fn anthropic_tool_tokens(
    model: &str,
    tools: &[FunctionDeclaration],
) -> Result<u32, AppError> {
    count_tool_tokens(AnthropicConfig::from_env()?, model, tools).await
}

/// The with-and-without count behind [`anthropic_tool_tokens`], on `config`.
async fn count_tool_tokens(
    config: AnthropicConfig,
    model: &str,
    tools: &[FunctionDeclaration],
) -> Result<u32, AppError> {
    let provider = AnthropicProvider::with_client(
        anthropic_config(config, Some(model)),
        llm_inner_client().clone(),
    );
    let bare = ChatRequest::new(vec![ChatMessage::user(CATALOGUE_PROBE)]);
    let with_tools = bare.clone().with_tools(tools.to_vec());
    let without = provider.count_tokens(&bare).await?;
    let with = provider.count_tokens(&with_tools).await?;
    Ok(with.saturating_sub(without))
}

/// Gemini on `GEMINI_API_KEY`, on `model_override` else `PIERRE_LLM_DEFAULT_MODEL`.
fn gemini_from_env(model_override: Option<&str>) -> Result<EmbacleProvider, AppError> {
    let model = match model_override {
        Some(model) => model.to_owned(),
        None => {
            LlmModelConfig::from_env()
                .map_err(AppError::config)?
                .default_model
        }
    };
    let config = GeminiConfig::from_env()?.with_model(model);
    info!(model = %config.model, "Creating Gemini provider");
    Ok(EmbacleProvider::from_runner(
        Box::new(GeminiProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        GEMINI_DISPLAY_NAME,
    ))
}

/// Groq on `GROQ_API_KEY` and `GROQ_DEFAULT_MODEL`, overridden by `model_override`.
fn groq_from_env(model_override: Option<&str>) -> Result<EmbacleProvider, AppError> {
    let mut config = GroqConfig::from_env()?;
    if let Some(model) = model_override {
        config = config.with_model(model);
    }
    info!(model = %config.model, "Creating Groq provider");
    Ok(EmbacleProvider::from_runner(
        Box::new(GroqProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        GROQ_DISPLAY_NAME,
    ))
}

/// Cohere on `COHERE_API_KEY` and `COHERE_DEFAULT_MODEL`, overridden by `model_override`.
fn cohere_from_env(model_override: Option<&str>) -> Result<EmbacleProvider, AppError> {
    let mut config = CohereConfig::from_env()?;
    if let Some(model) = model_override {
        config = config.with_model(model);
    }
    info!(model = %config.model, "Creating Cohere provider");
    Ok(EmbacleProvider::from_runner(
        Box::new(CohereProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        COHERE_DISPLAY_NAME,
    ))
}

/// `OpenRouter` on `OPENROUTER_API_KEY` and `OPENROUTER_DEFAULT_MODEL`,
/// overridden by `model_override`.
fn openrouter_from_env(model_override: Option<&str>) -> Result<EmbacleProvider, AppError> {
    let mut config = OpenRouterConfig::from_env()?;
    if let Some(model) = model_override {
        config = config.with_model(model);
    }
    info!(model = %config.model, "Creating OpenRouter provider");
    Ok(EmbacleProvider::from_runner(
        Box::new(OpenRouterProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        OPENROUTER_DISPLAY_NAME,
    ))
}

/// A local `OpenAI`-compatible endpoint on `LOCAL_LLM_BASE_URL` /
/// `LOCAL_LLM_MODEL` / `LOCAL_LLM_API_KEY`, overridden by `model_override`.
fn local_from_env(model_override: Option<&str>) -> EmbacleProvider {
    let mut config = OpenAiCompatibleConfig::from_env();
    if let Some(model) = model_override {
        model.clone_into(&mut config.default_model);
    }
    info!(
        provider = %config.provider_name,
        base_url = %config.base_url,
        model = %config.default_model,
        "Creating local OpenAI-compatible provider"
    );
    local_provider(config)
}

/// Gemini from a tenant's own key (the BYO path), on the tenant's default
/// model when it stored one and on `PIERRE_LLM_DEFAULT_MODEL` otherwise.
///
/// # Errors
///
/// Returns `AppError` when no model can be resolved.
pub fn gemini_with_key(
    api_key: &str,
    default_model: Option<String>,
) -> Result<EmbacleProvider, AppError> {
    let model = match default_model {
        Some(model) => model,
        None => {
            LlmModelConfig::from_env()
                .map_err(AppError::config)?
                .default_model
        }
    };
    let config = GeminiConfig::new(api_key).with_model(model);
    Ok(EmbacleProvider::from_runner(
        Box::new(GeminiProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        GEMINI_DISPLAY_NAME,
    ))
}

/// Groq from a tenant's own key (the BYO path).
#[must_use]
pub fn groq_with_key(api_key: String, default_model: Option<String>) -> EmbacleProvider {
    let mut config = GroqConfig::new(api_key);
    if let Some(model) = default_model {
        config = config.with_model(model);
    }
    EmbacleProvider::from_runner(
        Box::new(GroqProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        GROQ_DISPLAY_NAME,
    )
}

/// Cohere from a tenant's own key (the BYO path).
#[must_use]
pub fn cohere_with_key(api_key: String, default_model: Option<String>) -> EmbacleProvider {
    let mut config = CohereConfig::new(api_key);
    if let Some(model) = default_model {
        config = config.with_model(model);
    }
    EmbacleProvider::from_runner(
        Box::new(CohereProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        COHERE_DISPLAY_NAME,
    )
}

/// A local OpenAI-compatible endpoint from a tenant's stored base URL, key
/// and model (the BYO path). Reports itself as `local`.
#[must_use]
pub fn local_from_credentials(
    base_url: String,
    api_key: Option<String>,
    model: String,
) -> EmbacleProvider {
    let config = OpenAiCompatibleConfig {
        base_url,
        api_key,
        default_model: model,
        provider_name: "local".to_owned(),
        display_name: "Local LLM".to_owned(),
        capabilities: LlmCapabilities::STREAMING
            | LlmCapabilities::FUNCTION_CALLING
            | LlmCapabilities::SYSTEM_MESSAGES,
        // The shared client below carries the request timeout; this one is
        // read only by a provider that builds its own client.
        ..OpenAiCompatibleConfig::default()
    };
    local_provider(config)
}

/// An `OpenAI`-compatible endpoint from an explicit config: the local
/// integration and scenario drivers, and the evals' Ollama candidates.
///
/// Every platform construction goes through here rather than
/// `OpenAiCompatibleProvider::new`, which builds a client of its own on
/// embacle's 120 s default: the shared LLM client carries the platform's 300 s
/// request timeout, and a CPU-bound Ollama takes more than two minutes to
/// process a full coaching prompt. The display name is the one the config's
/// `provider_name` has always carried on the platform.
#[must_use]
pub fn local_provider(config: OpenAiCompatibleConfig) -> EmbacleProvider {
    let display_name = local_display_name(&config.provider_name);
    EmbacleProvider::from_runner(
        Box::new(OpenAiCompatibleProvider::with_client(
            config,
            llm_inner_client().clone(),
        )),
        display_name,
    )
}

const ANTHROPIC_DISPLAY_NAME: &str = "Anthropic API";
const GEMINI_DISPLAY_NAME: &str = "Google Gemini";
const GROQ_DISPLAY_NAME: &str = "Groq (Llama/Mixtral)";
const COHERE_DISPLAY_NAME: &str = "Cohere (Command)";
const OPENROUTER_DISPLAY_NAME: &str = "OpenRouter";

/// The static display name for each `name()` an OpenAI-compatible provider
/// reports; the platform trait wants a `&'static str`.
fn local_display_name(provider_name: &str) -> &'static str {
    match provider_name {
        "ollama" => "Ollama (Local)",
        "vllm" => "vLLM (Local)",
        "localai" => "LocalAI",
        _ => "Local LLM",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use embacle::{AnthropicCacheTtl, AnthropicConfig, FunctionDeclaration};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use super::{count_tool_tokens, with_platform_prefix_ttl};

    /// What the endpoint counts for a request that carries a `tools` block
    const WITH_TOOLS: u32 = 1_830;
    /// What the endpoint counts for the same request without one
    const WITHOUT_TOOLS: u32 = 8;

    /// One HTTP/1.1 request off `stream`: its head, then as many body bytes
    /// as its `content-length` names.
    async fn read_request(stream: &mut TcpStream) -> String {
        let mut raw = Vec::new();
        let mut buf = [0_u8; 4096];
        loop {
            let n = stream.read(&mut buf).await.expect("read the request");
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&raw);
            if let Some(head_end) = text.find("\r\n\r\n") {
                let length = text[..head_end]
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if raw.len() >= head_end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&raw).into_owned()
    }

    /// Serve `POST /v1/messages/count_tokens` on a loopback port and record
    /// every request. With `refuse` each one is answered 401, as Anthropic
    /// answers a revoked key; otherwise the count is [`WITH_TOOLS`] when the
    /// body carries tools, else [`WITHOUT_TOOLS`].
    async fn count_tokens_endpoint(refuse: bool) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let request = read_request(&mut stream).await;
                let (status, body) = if refuse {
                    (
                        "401 Unauthorized",
                        r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.to_owned(),
                    )
                } else {
                    let tokens = if request.contains(r#""tools":"#) {
                        WITH_TOOLS
                    } else {
                        WITHOUT_TOOLS
                    };
                    ("200 OK", format!(r#"{{"input_tokens":{tokens}}}"#))
                };
                recorded.lock().expect("record the request").push(request);
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("answer the request");
                stream.shutdown().await.expect("close the connection");
            }
        });
        (base_url, seen)
    }

    /// A one-tool catalogue whose name a request body shows.
    fn catalogue() -> Vec<FunctionDeclaration> {
        vec![FunctionDeclaration {
            name: "get_activities".to_owned(),
            description: "The athlete's recent activities".to_owned(),
            parameters: Some(json!({
                "type": "object",
                "properties": { "limit": { "type": "integer" } }
            })),
        }]
    }

    #[tokio::test]
    async fn the_count_is_what_the_tools_add_on_the_model_asked_for() {
        let (base_url, seen) = count_tokens_endpoint(false).await;
        // embacle's default model is Haiku, so a request naming Opus proves
        // the argument reached both counts.
        let config = AnthropicConfig::new("sk-ant-test-key").with_base_url(base_url);

        let tokens = count_tool_tokens(config, "claude-opus-5-5", &catalogue())
            .await
            .expect("the endpoint counts both requests");

        assert_eq!(
            tokens,
            WITH_TOOLS - WITHOUT_TOOLS,
            "the figure is the difference the tools make, never either raw count"
        );
        let requests = seen.lock().expect("read the requests").clone();
        assert_eq!(
            requests.len(),
            2,
            "one count with the tools and one without: {requests:?}"
        );
        for request in &requests {
            assert!(
                request.starts_with("POST /v1/messages/count_tokens "),
                "every request goes to the counting endpoint, never Messages: {request}"
            );
            assert!(
                request.contains(r#""model":"claude-opus-5-5""#),
                "every count runs on the model asked for: {request}"
            );
        }
        let with_tools: Vec<&String> = requests
            .iter()
            .filter(|request| request.contains(r#""tools":"#))
            .collect();
        let [with_tools] = with_tools.as_slice() else {
            panic!("exactly one count carries the tools: {requests:?}");
        };
        assert!(
            with_tools.contains("get_activities"),
            "the tools counted are the catalogue handed in: {with_tools}"
        );
    }

    #[tokio::test]
    async fn a_refused_count_is_an_error_never_a_zero() {
        let (base_url, seen) = count_tokens_endpoint(true).await;
        let config = AnthropicConfig::new("sk-ant-test-key").with_base_url(base_url);

        let result = count_tool_tokens(config, "claude-opus-5-5", &catalogue()).await;

        assert!(
            result.is_err(),
            "a refused key must surface as an error, got {result:?}"
        );
        assert!(
            !seen.lock().expect("read the requests").is_empty(),
            "the refusal must come from the endpoint, not from building the request"
        );
    }

    #[test]
    fn the_prefix_lives_an_hour_unless_the_environment_names_a_ttl() {
        let unset = with_platform_prefix_ttl(AnthropicConfig::new("k"), false);
        assert_eq!(
            unset.prefix_cache_ttl,
            AnthropicCacheTtl::OneHour,
            "bug: with no TTL named the prefix must live an hour"
        );

        let named = AnthropicConfig::new("k").with_prefix_cache_ttl(AnthropicCacheTtl::FiveMinutes);
        let kept = with_platform_prefix_ttl(named, true);
        assert_eq!(
            kept.prefix_cache_ttl,
            AnthropicCacheTtl::FiveMinutes,
            "bug: a TTL the environment names must win over the platform default"
        );
    }
}
