// ABOUTME: Startup validation of the server's environment variables and LLM provider credentials
// ABOUTME: Fails fast with a named variable instead of a cryptic error later in boot
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Environment validation run before the server boots.

use std::env;

use pierre_auth::config::credential_env;
use pierre_config::environment::LlmProviderType;
use pierre_core::errors::{AppError, AppResult};
use pierre_llm::config::LlmChain;
use pierre_tool_runtime::guardian;
use tracing::info;

type Result<T> = AppResult<T>;

/// Environment variable validation result
struct EnvValidation {
    name: &'static str,
    value: Option<String>,
    required: bool,
    description: &'static str,
}

/// Validate required environment variables at startup
///
/// Fails fast with clear error messages if critical variables are missing.
/// This prevents cryptic errors later in the startup process.
pub fn validate_required_environment() -> Result<()> {
    // The env-backed master key (LocalKekProvider) is unnecessary once the GCP Cloud
    // KMS KEK is compiled in and configured: the DEK is wrapped by KMS, so
    // KeyManager::configured_kek_provider never reads PIERRE_MASTER_ENCRYPTION_KEY.
    // Mirror that selection so KMS-only deployments don't demand the master key.
    #[cfg(feature = "gcp-kms")]
    let master_key_required = env::var("PIERRE_KMS_KEY_RESOURCE").is_err();
    #[cfg(not(feature = "gcp-kms"))]
    let master_key_required = true;

    let mut validations = vec![
        EnvValidation {
            name: "DATABASE_URL",
            value: env::var("DATABASE_URL").ok(),
            required: true,
            description: "Database connection string (e.g., sqlite:./data/users.db)",
        },
        EnvValidation {
            name: "PIERRE_MASTER_ENCRYPTION_KEY",
            value: env::var("PIERRE_MASTER_ENCRYPTION_KEY").ok(),
            required: master_key_required, // Required unless the GCP Cloud KMS KEK is active
            description:
                "Base64-encoded 32-byte master encryption key (generate: openssl rand -base64 32)",
        },
        EnvValidation {
            name: "BASE_URL",
            value: env::var("BASE_URL").ok(),
            required: false,
            description: "Public base URL of this server (e.g. https://app.pierre.ai). Used to construct hosted-login URLs for channel-initiated provider connections. Falls back to http://localhost:8081 for local development.",
        },
        EnvValidation {
            name: "MCP_RESOURCE_URL",
            value: env::var("MCP_RESOURCE_URL").ok(),
            required: false,
            description: "Origin MCP clients dial for /mcp (e.g. https://mcp.dravr.ai), with no path. Published as `resource` in /.well-known/oauth-protected-resource, used for the resource_metadata URL of every /mcp 401 challenge, and an RFC 8707 `resource` the OAuth authorize and token endpoints accept and bind tokens to. Defaults to BASE_URL.",
        },
        EnvValidation {
            name: "OAUTH2_SATELLITE_RESOURCES",
            value: env::var("OAUTH2_SATELLITE_RESOURCES").ok(),
            required: false,
            description: "Comma-separated resource identifiers of satellite MCP servers (e.g. https://commere.dravr.ai/mcp) the OAuth endpoints also mint audience-bound tokens for, carrying the grant's tenant. The platform's own /mcp refuses those tokens. Unset: none.",
        },
        EnvValidation {
            name: "PROVIDER_LINK_WEBHOOK_URL",
            value: env::var("PROVIDER_LINK_WEBHOOK_URL").ok(),
            required: false,
            description: "Optional outbound webhook URL for channel-initiated provider connections. When set, Dravr POSTs a `provider.linked` event to this URL on successful connection so the channel bot can post a confirmation.",
        },
        EnvValidation {
            name: "STRAVA_CLIENT_ID",
            value: credential_env("STRAVA_CLIENT_ID"),
            required: false,
            description: "Strava OAuth application client ID",
        },
        EnvValidation {
            name: "STRAVA_CLIENT_SECRET",
            value: credential_env("STRAVA_CLIENT_SECRET"),
            required: false,
            description: "Strava OAuth application client secret",
        },
        EnvValidation {
            name: "WHOOP_CLIENT_ID",
            value: credential_env("WHOOP_CLIENT_ID"),
            required: false,
            description: "Whoop OAuth application client ID for the shared Dravr-owned Whoop app. When set (with WHOOP_CLIENT_SECRET), Whoop connects via one-tap OAuth consent; when unset, Whoop falls back to bring-your-own OAuth app.",
        },
        EnvValidation {
            name: "WHOOP_CLIENT_SECRET",
            value: credential_env("WHOOP_CLIENT_SECRET"),
            required: false,
            description: "Whoop OAuth application client secret for the shared Dravr-owned Whoop app.",
        },
    ];

    // Append the chain head's provider-specific validations: the head serves
    // every turn first, so its credential is the one a boot must not miss.
    // Tiers behind it that fail to build are logged and skipped by the chain
    // assembly instead. Only enforce as required when PIERRE_LLM_CHAIN is
    // explicitly set — CI environments don't set it and don't need LLM to
    // start the server.
    let explicitly_configured = LlmChain::is_configured();
    let chain = LlmChain::from_env();
    let llm_provider = chain.head().provider;
    validations.extend(llm_provider_validations(
        llm_provider,
        chain.head().model.is_some(),
        explicitly_configured,
    ));

    // Pre-flight Copilot token format check — diagnoses today's outage
    // pattern (wrong token type rotated into Secret Manager) at boot
    // instead of as a 5xx on every chat call.
    validate_copilot_token_format(llm_provider, explicitly_configured)?;

    let mut missing_required = Vec::new();
    let mut missing_optional = Vec::new();

    for validation in &validations {
        if validation.value.is_none() {
            if validation.required {
                missing_required.push(validation);
            } else {
                missing_optional.push(validation);
            }
        }
    }

    // Print warnings for missing optional variables
    for validation in &missing_optional {
        eprintln!(
            "WARNING: {} not set - {}",
            validation.name, validation.description
        );
    }

    // Fail fast if required variables are missing
    if !missing_required.is_empty() {
        eprintln!();
        eprintln!(
            "ERROR: Required environment variables are missing! (LLM provider: {llm_provider})"
        );
        eprintln!();
        for validation in &missing_required {
            eprintln!("  {} - {}", validation.name, validation.description);
        }
        eprintln!();
        eprintln!("Have you sourced your .envrc? Run: source .envrc");
        eprintln!();
        let missing_names = missing_required
            .iter()
            .map(|v| v.name)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AppError::config(format!(
            "Missing required environment variables: {missing_names}"
        )));
    }

    // Fail the boot on a mis-typed GUARDIAN_* security-posture variable rather
    // than silently serve traffic under the laxer default (e.g. an operator
    // arming `GUARDIAN_MODE=enforce` who typed `enfroce` would otherwise run
    // `observe`). The Guardian is a security control; an unrecognized value is a
    // configuration error, not something to paper over with a default.
    let bad_guardian = guardian::validate_env();
    if !bad_guardian.is_empty() {
        let detail = bad_guardian
            .iter()
            .map(|(name, value)| format!("{name}={value:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AppError::config(format!(
            "Unrecognized GUARDIAN_* value(s): {detail}. Check the spelling — a \
             security-posture typo must not silently revert to the laxer default."
        )));
    }

    Ok(())
}

/// Return provider-specific environment variable validations for the configured LLM provider
///
/// When `required` is true (`PIERRE_LLM_CHAIN` explicitly set), missing keys are fatal.
/// When false (env var not set, e.g. CI), missing keys produce warnings only.
/// `entry_names_model` is whether the head's chain entry carries its own model,
/// which a Gemini head then runs instead of `PIERRE_LLM_DEFAULT_MODEL`.
fn llm_provider_validations(
    provider: LlmProviderType,
    entry_names_model: bool,
    required: bool,
) -> Vec<EnvValidation> {
    match provider {
        LlmProviderType::Groq => vec![EnvValidation {
            name: "GROQ_API_KEY",
            value: env::var("GROQ_API_KEY").ok(),
            required,
            description: "Groq API key (get one at https://console.groq.com/keys)",
        }],
        LlmProviderType::Gemini => vec![
            EnvValidation {
                name: "GEMINI_API_KEY",
                value: env::var("GEMINI_API_KEY").ok(),
                required,
                description:
                    "Google Gemini API key (get one at https://makersuite.google.com/app/apikey)",
            },
            EnvValidation {
                name: "PIERRE_LLM_DEFAULT_MODEL",
                value: env::var("PIERRE_LLM_DEFAULT_MODEL").ok(),
                required: required && !entry_names_model,
                description: "Default Gemini model name (e.g., gemini-2.0-flash), run when \
                              the chain's gemini entry names no model",
            },
        ],
        LlmProviderType::Local => vec![EnvValidation {
            name: "LOCAL_LLM_BASE_URL",
            value: env::var("LOCAL_LLM_BASE_URL").ok(),
            required: false,
            description: "Local LLM base URL (defaults to http://localhost:11434/v1 for Ollama)",
        }],
        LlmProviderType::OpenRouter => vec![EnvValidation {
            name: "OPENROUTER_API_KEY",
            value: env::var("OPENROUTER_API_KEY").ok(),
            required,
            description: "OpenRouter API key (get one at https://openrouter.ai/keys)",
        }],
        LlmProviderType::Anthropic => vec![EnvValidation {
            name: "ANTHROPIC_API_KEY",
            value: env::var("ANTHROPIC_API_KEY").ok(),
            required,
            description: "Anthropic API key of the first organization in the key pool \
                          (ANTHROPIC_API_KEY_2, _3, … add one tier each)",
        }],
        LlmProviderType::Cohere => vec![EnvValidation {
            name: "COHERE_API_KEY",
            value: env::var("COHERE_API_KEY").ok(),
            required,
            description: "Cohere API key (get one at https://dashboard.cohere.com/api-keys)",
        }],
        LlmProviderType::ClaudeCode
        | LlmProviderType::Copilot
        | LlmProviderType::CursorAgent
        | LlmProviderType::OpenCode
        | LlmProviderType::CopilotHeadless
        | LlmProviderType::CopilotSdk
        | LlmProviderType::GeminiCli
        | LlmProviderType::CodexCli
        | LlmProviderType::GooseCli
        | LlmProviderType::ClineCli
        | LlmProviderType::ContinueCli
        | LlmProviderType::WarpCli
        | LlmProviderType::KiroCli
        | LlmProviderType::KiloCli => {
            // CLI/ACP providers handle their own auth; no env vars required at startup
            vec![]
        }
        // The router requires nothing at startup: without a credential a backend
        // is simply unmetered, which the provider warns about and which still
        // routes and still reroutes on a refusal. Both are declared so a missing
        // one is visible at boot rather than discovered as a quota surprise.
        LlmProviderType::Router => vec![
            EnvValidation {
                name: "CLAUDE_CODE_OAUTH_TOKEN",
                value: env::var("CLAUDE_CODE_OAUTH_TOKEN").ok(),
                required: false,
                description: "Anthropic OAuth token — without it the lead backend runs unmetered \
                              and can only fall back reactively",
            },
            EnvValidation {
                name: "COPILOT_GITHUB_TOKEN",
                value: env::var("COPILOT_GITHUB_TOKEN").ok(),
                required: false,
                description: "GitHub token — without it the Copilot backend runs unmetered",
            },
        ],
        LlmProviderType::OpenAiApi => vec![
            EnvValidation {
                name: "OPENAI_API_BASE_URL",
                value: env::var("OPENAI_API_BASE_URL").ok(),
                required,
                description: "OpenAI-compatible API base URL (e.g., https://api.openai.com/v1)",
            },
            EnvValidation {
                name: "OPENAI_API_KEY",
                value: env::var("OPENAI_API_KEY").ok(),
                required,
                description: "API key for the OpenAI-compatible endpoint",
            },
        ],
    }
}

/// Validate `COPILOT_GITHUB_TOKEN` format when a Copilot CLI runner is the
/// active LLM provider.
///
/// Copilot CLI (`@github/copilot-linux-x64`) accepts three token types
/// (per `copilot login --help`):
///
///   1. Fine-grained PATs (v2) with the "Copilot Requests" permission —
///      `github_pat_…` prefix. This is what production uses today.
///   2. OAuth tokens from the GitHub Copilot CLI app — `ghu_…` prefix.
///   3. OAuth tokens from the GitHub CLI (`gh`) app — `gho_…` prefix.
///
/// Classic personal access tokens (`ghp_…`) and gh refresh tokens
/// (`ghr_…`) are not supported and cause runtime
/// `Authentication required` errors on every chat call.
///
/// This pre-check fails fast at boot with a specific diagnostic so a
/// bad rotation surfaces as "container did not start" instead of a 5xx
/// on every user message. Only enforced when:
///
///   * `PIERRE_LLM_CHAIN` is explicitly set with a Copilot variant at its head
///     (`required == true` path in [`validate_environment`]).
///   * `COPILOT_GITHUB_TOKEN` is present (callers may rely on
///     `~/.config/github-copilot/` credentials instead — in that case
///     the env var is empty and we let the CLI handle auth).
fn validate_copilot_token_format(provider: LlmProviderType, required: bool) -> AppResult<()> {
    if !required {
        return Ok(());
    }
    if !matches!(
        provider,
        LlmProviderType::Copilot | LlmProviderType::CopilotHeadless | LlmProviderType::CopilotSdk
    ) {
        return Ok(());
    }
    let Some(token) = env::var("COPILOT_GITHUB_TOKEN")
        .ok()
        .filter(|v| !v.is_empty())
    else {
        if provider == LlmProviderType::CopilotSdk {
            // The SDK runtime runs in a home of its own (ClientMode::Empty), with
            // no stored login to fall back to: without a token every turn fails
            // at the first model call, so the boot refuses instead.
            return Err(AppError::config(
                "A copilot_sdk head of PIERRE_LLM_CHAIN requires COPILOT_GITHUB_TOKEN (or GH_TOKEN / \
                 GITHUB_TOKEN): the Copilot runtime it spawns keeps no stored login. A \
                 fine-grained PAT needs the Copilot Requests permission; a gh OAuth token \
                 (gho_) works.",
            ));
        }
        // Empty env var is OK for the CLI transports — Copilot CLI falls back
        // to the on-disk credential store. We can't validate that store here.
        info!(
            provider = %provider,
            "COPILOT_GITHUB_TOKEN not set; Copilot CLI will use on-disk credentials"
        );
        return Ok(());
    };

    let prefix_kind = if token.starts_with("github_pat_") {
        "fine_grained_pat"
    } else if token.starts_with("ghu_") {
        "copilot_oauth"
    } else if token.starts_with("gho_") {
        "gh_cli_oauth"
    } else if token.starts_with("ghp_") {
        "classic_pat_REJECTED"
    } else if token.starts_with("ghr_") {
        "gh_refresh_token_REJECTED"
    } else {
        "unknown_REJECTED"
    };

    info!(
        provider = %provider,
        prefix_kind,
        token_len = token.len(),
        "COPILOT_GITHUB_TOKEN prefix detected"
    );

    (!prefix_kind.ends_with("_REJECTED")).ok_or_else(|| {
        AppError::config(format!(
            "COPILOT_GITHUB_TOKEN has unsupported prefix ({prefix_kind}); Copilot CLI accepts \
             github_pat_v2 fine-grained PATs (with Copilot Requests permission), gh_ OAuth \
             tokens, or ghu_ Copilot OAuth tokens. Classic ghp_ PATs and ghr_ refresh tokens \
             are rejected by Copilot's API and will fail every chat call. Generate a new \
             fine-grained PAT at https://github.com/settings/personal-access-tokens/new with \
             Account permissions → Copilot Requests (the Rust runtime behind copilot_sdk \
             needs it; the ACP transport also accepts Copilot Chat: Read)."
        ))
    })
}
