// ABOUTME: Environment configuration management for deployment-specific settings
// ABOUTME: Orchestrates loading of all configuration modules from environment variables
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Environment-based configuration management for production deployment
//!
//! This module serves as the main orchestrator for configuration loading,
//! delegating to specialized sub-modules for each configuration domain.

use dravr_cageux::config::intelligence::IntelligenceConfig;
use pierre_auth::config::oauth::provider_callback_uri;
use pierre_core::config::fitness::{FitnessConfig, WeatherApiConfig};
use pierre_core::constants::limits::DEFAULT_ACTIVITIES_LIMIT;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::redaction::redact_url;
use pierre_llm::config::LlmChain;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::env;
use tracing::{info, warn};

// Re-export types from sub-modules for convenience
// API providers
pub use crate::api_providers::{
    ExternalServicesConfig, GarminApiConfig, SciotteServiceConfig, StravaApiConfig,
};
// Cache and rate limiting
pub use crate::cache::{CacheConfig, RedisConnectionConfig};
pub use pierre_auth::config::RateLimitConfig;
// Database
pub use crate::database::{BackupConfig, DatabaseConfig, DatabaseUrl, PostgresPoolConfig};
// Goal management
pub use crate::goal_management::GoalManagementConfig;
// Request-path time bounds
pub use crate::timeouts::ServerTimeouts;
// How a chat turn sends its tool catalogue
pub use crate::chat_tools::ChatToolSettings;
// When Coach Home's attention queue raises a reason
pub use crate::coach_home::CoachHomeConfig;
// Inbound webhook secrets
pub use crate::webhooks::WebhookSecrets;
// Messaging channel credentials
pub use crate::channels::ChannelCredentials;
// Logging
pub use crate::logging::LoggingConfig;
// MCP
pub use crate::mcp::{AppBehaviorConfig, McpConfig, ProtocolConfig, TokioRuntimeConfig};
// Network
pub use crate::network::{CorsConfig, HttpClientConfig, SseBufferStrategy, SseConfig, TlsConfig};
// OAuth (canonical types live in pierre-auth)
pub use pierre_auth::config::oauth::{
    default_provider, FirebaseConfig, OAuth2ServerConfig, OAuthConfig, OAuthProviderConfig,
    ProviderEndpointOverride, ProviderEndpointOverrides,
};
// Security
pub use crate::security::{AuthConfig, MonitoringConfig, SecurityConfig, SecurityHeadersConfig};
// Sleep tool params (operational parameters, distinct from intelligence sleep config)
pub use crate::sleep_tool_params::SleepToolParamsConfig;
// Training zones (canonical types live in dravr-cageux)
pub use dravr_cageux::config::intelligence::TrainingZonesConfig;
// Core types
pub use crate::types::{Environment, LlmProviderType, LogLevel};

/// Server configuration for HTTP and MCP protocols
///
/// A deployment builds it with [`Self::from_env`]. [`Self::default`] is a
/// configuration no deployment runs, for a caller that builds one by hand:
/// its weather vendors are asked at [`Self::REFUSED_ORIGIN`] with the
/// activity list's temperature backfill off, so a context built on it
/// reaches no weather host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Server port (handles both MCP and HTTP)
    pub http_port: u16,
    /// Frontend URL for OAuth redirects (development uses separate port)
    pub frontend_url: Option<String>,
    /// Log level
    pub log_level: LogLevel,
    /// Logging and PII redaction configuration
    pub logging: LoggingConfig,
    /// Database configuration
    pub database: DatabaseConfig,
    /// Authentication configuration
    pub auth: AuthConfig,
    /// Firebase authentication configuration for social logins
    pub firebase: FirebaseConfig,
    /// OAuth provider configurations
    pub oauth: OAuthConfig,
    /// `OAuth2` authorization server configuration
    pub oauth2_server: OAuth2ServerConfig,
    /// Security settings
    pub security: SecurityConfig,
    /// External service configuration
    pub external_services: ExternalServicesConfig,
    /// USDA `FoodData` Central API key (optional, for nutrition features);
    /// Debug prints a redaction and serialization omits it
    #[serde(skip_serializing)]
    pub usda_api_key: Option<SecretString>,
    /// Application behavior settings
    pub app_behavior: AppBehaviorConfig,
    /// HTTP client timeout configuration
    pub http_client: HttpClientConfig,
    /// SSE connection management configuration
    pub sse: SseConfig,
    /// Server host
    pub host: String,
    /// Base URL for OAuth and external URLs
    pub base_url: String,
    /// Public origin of the dravr.ai website, where docs sign-in links land
    /// (`WEBSITE_BASE_URL`, no trailing slash)
    pub website_base_url: String,
    /// MCP server configuration
    pub mcp: McpConfig,
    /// CORS configuration
    pub cors: CorsConfig,
    /// Cache configuration
    pub cache: CacheConfig,
    /// Rate limiting configuration
    pub rate_limiting: RateLimitConfig,
    /// Sleep tool operational parameters (activity limits, trend thresholds)
    pub sleep_tool_params: SleepToolParamsConfig,
    /// Maximum number of activities to fetch from a provider in any single request.
    /// Governs all activity-fetching paths: the direct `analyze_training_load` tool,
    /// group coaching snapshots, and chat orchestration context building. Higher
    /// values give more accurate CTL (42-day EMA) at the cost of more API calls and
    /// longer response latency — every additional activity is one more round-trip on
    /// providers that require per-activity detail fetches (e.g., headless scrapers).
    pub activity_fetch_limit: usize,
    /// Goal management and feasibility configuration
    pub goal_management: GoalManagementConfig,
    /// Training zone percentages configuration
    pub training_zones: TrainingZonesConfig,
    /// Tokio runtime configuration
    pub tokio_runtime: TokioRuntimeConfig,
    /// System monitoring configuration
    pub monitoring: MonitoringConfig,
    /// Resend API key for transactional emails (optional — emails are skipped
    /// if unset); Debug prints a redaction and serialization omits it
    #[serde(skip_serializing)]
    pub resend_api_key: Option<SecretString>,
    /// Sender email address for Resend (e.g., "Dravr <no-reply@dravr.ai>")
    pub resend_from_email: Option<String>,
    /// The LLM provider chain (`PIERRE_LLM_CHAIN`), read once here at boot.
    ///
    /// Everything that needs the chain — the provider build, the model a turn
    /// and a new conversation run on, a tenant's own-key provider, the admin
    /// settings view, the health probe — is handed this value; nothing reads
    /// the variable again per request.
    pub llm_chain: LlmChain,
    /// Request-path time bounds the operator retuned
    pub timeouts: ServerTimeouts,
    /// How a chat turn sends its tool catalogue
    pub chat_tools: ChatToolSettings,
    /// What inbound provider and messaging webhooks are verified against
    pub webhooks: WebhookSecrets,
    /// The messaging channels the environment configured: what the channel
    /// seed stores and the menu publishers send with. Empty unless
    /// [`Self::from_env`] built this config.
    #[serde(default)]
    pub channels: ChannelCredentials,
    /// When Coach Home's attention queue raises a reason
    #[serde(default)]
    pub coach_home: CoachHomeConfig,
    /// The deployment's fitness configuration, read once at startup
    /// (`FitnessConfig::load`).
    ///
    /// Weather is its one reader, and reads only `weather_api`: `enabled`
    /// (`FITNESS_WEATHER_ENABLED`, on unless it reads `false`) is whether an
    /// activity list is given the ambient temperature its source left out,
    /// and `base_url` is where the weather vendors are asked. At its default
    /// each vendor is asked on its own host; any other value is the single
    /// origin of the archive, the forecast, the gazetteer and
    /// `OpenWeatherMap`. No `weather_api` means no backfill, and the vendors'
    /// own hosts for the tools an athlete asks for weather with.
    ///
    /// The weather clients are built from this instance and never from a
    /// stored fitness configuration, which has the same shape: an athlete
    /// writes that document (`set_fitness_config`, the fitness configuration
    /// routes), so an origin read from it would let them aim the server's
    /// requests at a host of their choosing.
    ///
    /// [`ServerConfig::default`], and a serialized configuration that names
    /// none, carry [`unloaded_fitness`].
    #[serde(default = "unloaded_fitness")]
    pub fitness: FitnessConfig,
}

/// The fitness configuration of a [`ServerConfig`] no deployment loaded:
/// [`FitnessConfig::default`] with every weather vendor asked at
/// [`ServerConfig::REFUSED_ORIGIN`] and the activity list's temperature
/// backfill off.
///
/// [`FitnessConfig::default`] names the vendors' own hosts, as a deployment
/// runs, so a configuration built by hand would otherwise reach them from
/// whatever test or tool built it (carnet#1626).
fn unloaded_fitness() -> FitnessConfig {
    FitnessConfig {
        weather_api: Some(WeatherApiConfig {
            base_url: ServerConfig::REFUSED_ORIGIN.to_owned(),
            enabled: false,
        }),
        ..FitnessConfig::default()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            http_port: u16::default(),
            frontend_url: None,
            log_level: LogLevel::default(),
            logging: LoggingConfig::default(),
            database: DatabaseConfig::default(),
            auth: AuthConfig::default(),
            firebase: FirebaseConfig::default(),
            oauth: OAuthConfig::default(),
            oauth2_server: OAuth2ServerConfig::default(),
            security: SecurityConfig::default(),
            external_services: ExternalServicesConfig::default(),
            usda_api_key: None,
            app_behavior: AppBehaviorConfig::default(),
            http_client: HttpClientConfig::default(),
            sse: SseConfig::default(),
            host: String::new(),
            base_url: String::new(),
            website_base_url: String::new(),
            mcp: McpConfig::default(),
            cors: CorsConfig::default(),
            cache: CacheConfig::default(),
            rate_limiting: RateLimitConfig::default(),
            sleep_tool_params: SleepToolParamsConfig::default(),
            activity_fetch_limit: usize::default(),
            goal_management: GoalManagementConfig::default(),
            training_zones: TrainingZonesConfig::default(),
            tokio_runtime: TokioRuntimeConfig::default(),
            monitoring: MonitoringConfig::default(),
            resend_api_key: None,
            resend_from_email: None,
            llm_chain: LlmChain::default(),
            timeouts: ServerTimeouts::default(),
            chat_tools: ChatToolSettings::default(),
            webhooks: WebhookSecrets::default(),
            channels: ChannelCredentials::default(),
            coach_home: CoachHomeConfig::default(),
            fitness: unloaded_fitness(),
        }
    }
}

impl ServerConfig {
    /// An origin no request reaches: nothing listens on port 1 of the
    /// loopback, so a connection there is refused at once and nothing is
    /// sent. [`Self::default`] asks the weather vendors here.
    pub const REFUSED_ORIGIN: &'static str = "http://127.0.0.1:1";

    /// Load configuration from environment variables
    ///
    /// # Errors
    ///
    /// Returns an error if environment variables contain invalid values or required configuration is missing
    pub fn from_env() -> AppResult<Self> {
        Self::initialize_environment();

        let http_port = env::var("HTTP_PORT")
            .or_else(|_| env::var("MCP_PORT"))
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8081);
        let security = SecurityConfig::from_env()?;
        let config = Self {
            http_port,
            frontend_url: env::var("FRONTEND_URL").ok(),
            log_level: LogLevel::from_str_or_default(
                &env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_owned()),
            ),
            logging: LoggingConfig::from_env(),
            database: DatabaseConfig::from_env()?,
            auth: AuthConfig::from_env()?,
            firebase: FirebaseConfig::from_env(),
            oauth: OAuthConfig::from_env(),
            oauth2_server: OAuth2ServerConfig::from_env(&security.allowed_mobile_redirect_origins),
            security,
            external_services: ExternalServicesConfig::from_env(),
            usda_api_key: usda_key_from(env::var("USDA_API_KEY").ok().as_deref()),
            app_behavior: AppBehaviorConfig::from_env()?,
            http_client: HttpClientConfig::from_env(),
            sse: SseConfig::from_env()?,
            host: env::var("HOST").unwrap_or_else(|_| "localhost".to_owned()),
            // Unset, the server is reached on its own port on this host.
            base_url: env::var("BASE_URL")
                .unwrap_or_else(|_| format!("http://localhost:{http_port}")),
            website_base_url: website_base_url_from(env::var("WEBSITE_BASE_URL").ok().as_deref()),
            mcp: McpConfig::from_env(),
            cors: CorsConfig::from_env(),
            cache: CacheConfig::from_env(),
            rate_limiting: RateLimitConfig::from_env(),
            sleep_tool_params: SleepToolParamsConfig::from_env(),
            activity_fetch_limit: env::var("ACTIVITY_FETCH_LIMIT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_ACTIVITIES_LIMIT),
            goal_management: GoalManagementConfig::from_env(),
            training_zones: TrainingZonesConfig::from_env(),
            tokio_runtime: TokioRuntimeConfig::from_env(),
            monitoring: MonitoringConfig::from_env(),
            resend_api_key: env::var("RESEND_API_KEY").ok().map(SecretString::from),
            resend_from_email: env::var("RESEND_FROM_EMAIL").ok(),
            llm_chain: LlmChain::from_env(),
            timeouts: ServerTimeouts::from_env(),
            chat_tools: ChatToolSettings::from_env(),
            webhooks: WebhookSecrets::from_env(),
            channels: ChannelCredentials::from_env(),
            coach_home: CoachHomeConfig::from_env(),
            fitness: FitnessConfig::load()?,
        };

        config.validate()?;
        info!("Configuration loaded successfully");
        Ok(config)
    }

    /// Initialize environment by logging configuration source
    fn initialize_environment() {
        info!("Loading configuration from environment variables (.envrc via direnv)");
    }

    /// Validate configuration values
    ///
    /// # Errors
    ///
    /// Returns an error if configuration values are invalid or conflicting
    pub fn validate(&self) -> AppResult<()> {
        self.validate_oauth_providers();
        self.validate_oauth2_issuer_url()?;
        let production = self.security.headers.environment.is_production();
        self.oauth2_server.validate_mcp_resource_url(production)?;
        self.oauth2_server
            .validate_satellite_resources(production)?;
        self.security.validate_tls()?;
        Ok(())
    }

    /// Validate OAuth provider configurations
    fn validate_oauth_providers(&self) {
        if self.oauth.strava.enabled
            && (self.oauth.strava.client_id.is_none() || self.oauth.strava.client_secret.is_none())
        {
            warn!("Strava OAuth is enabled but missing client_id or client_secret");
        }
    }

    /// Validate `OAuth2` issuer URL according to RFC 8414 security requirements
    ///
    /// # Errors
    ///
    /// Returns an error if production issuer URL doesn't use HTTPS
    fn validate_oauth2_issuer_url(&self) -> AppResult<()> {
        // In production, issuer MUST use HTTPS to prevent token theft and MITM attacks
        if self.security.headers.environment.is_production() {
            if !self.oauth2_server.issuer_url.starts_with("https://") {
                return Err(AppError::invalid_input(format!(
                    "OAuth2 issuer URL must use HTTPS in production (RFC 8414 security requirement). Current: {}",
                    self.oauth2_server.issuer_url
                )));
            }
        } else if !self
            .oauth2_server
            .issuer_url
            .starts_with("http://localhost")
            && !self.oauth2_server.issuer_url.starts_with("https://")
        {
            // In development/testing, allow localhost HTTP but warn about non-localhost HTTP
            warn!(
                "OAuth2 issuer URL should use HTTPS or localhost in development: {}",
                self.oauth2_server.issuer_url
            );
        }
        Ok(())
    }

    /// Initialize all configurations including intelligence config
    ///
    /// # Errors
    ///
    /// Returns an error if intelligence configuration cannot be loaded or validated
    pub fn init_all_configs(&self) -> AppResult<()> {
        // Validate the layered intelligence config (compiled-in defaults +
        // INTELLIGENCE_* env vars) parses and validates. The canonical live
        // snapshot is owned by `ServerContext::cageux_config_registry`,
        // which applies the same layered stack plus the contremaitre YAML
        // overlay on startup; this call exists purely so startup fails fast
        // if an operator has supplied a bad env override.
        let intelligence_config = IntelligenceConfig::load()
            .map_err(|e| AppError::internal(format!("Intelligence config load failed: {e}")))?;
        info!(
            "Intelligence config validated (min duration: {}s)",
            intelligence_config
                .activity_analyzer
                .analysis
                .min_duration_seconds
        );

        info!("All configurations initialized successfully");
        Ok(())
    }

    /// Get a summary of the configuration for logging (without secrets)
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "Pierre MCP Server Configuration:\n\
             - Server Port: {}\n\
             - Log Level: {}\n\
             - Database: {}\n\
             - Strava OAuth: {}\n\
             - Strava Redirect URI: {}\n\
             - TLS: {}\n\
             - Rate Limiting: {}\n\
             - CI Mode: {}\n\
             - Protocol Version: {}",
            self.http_port,
            self.log_level,
            if self.database.url.is_sqlite() {
                "SQLite"
            } else {
                "PostgreSQL"
            },
            "API-Configured",
            self.oauth
                .strava
                .redirect_uri
                .as_ref()
                .map_or("Not configured", |s| s.as_str()),
            if self.security.tls.enabled {
                "Enabled"
            } else {
                "Disabled"
            },
            "Configured",
            self.app_behavior.ci_mode,
            self.app_behavior.protocol.mcp_version
        )
    }

    /// The redirect URI this server receives `provider`'s authorization codes
    /// at, under its own app for the provider and its public URL
    /// ([`provider_callback_uri`]).
    #[must_use]
    pub fn provider_callback_uri(&self, provider: &str) -> String {
        provider_callback_uri(
            provider,
            self.oauth.server_app(provider),
            &self.base_url,
            self.http_port,
        )
    }

    /// Get Strava API configuration
    #[must_use]
    pub const fn strava_api_config(&self) -> &StravaApiConfig {
        &self.external_services.strava_api
    }

    /// Check if CI mode is enabled
    #[must_use]
    pub const fn is_ci_mode(&self) -> bool {
        self.app_behavior.ci_mode
    }

    /// Resolve the credentials for the outbound transactional email service.
    ///
    /// Returns `Some((api_key, from_email))` only when both Resend values are
    /// present, non-empty, AND the server is not running in CI mode. This is the
    /// single gate that decides whether a live email service is built.
    ///
    /// The CI-mode guard is a safety floor: integration tests that boot via
    /// [`ServerConfig::from_env`] would otherwise inherit a developer's
    /// `RESEND_API_KEY` from `.envrc` and send real email to throwaway addresses
    /// on every registration/approval/password-reset flow they exercise, flooding
    /// the Resend dashboard. The non-empty check makes blanking the env var
    /// (`RESEND_API_KEY=`) disable email the same as unsetting it — `env::var().ok()`
    /// yields `Some("")` for a blank value, which would otherwise build a broken
    /// service that 401s on every send.
    #[must_use]
    pub fn outbound_email_credentials(&self) -> Option<(&str, &str)> {
        if self.is_ci_mode() {
            return None;
        }
        let api_key = self
            .resend_api_key
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .filter(|s| !s.is_empty())?;
        let from_email = self
            .resend_from_email
            .as_deref()
            .filter(|s| !s.is_empty())?;
        Some((api_key, from_email))
    }
}

/// Whether a `BASE_URL` names a Cloudflare quick tunnel.
///
/// The dev scripts in `bin/` create quick tunnels, whose hostname is allocated
/// per `cloudflared` process and stops resolving when that process exits. A
/// `BASE_URL` on one of those hosts is reachable only while that tunnel is up,
/// so every OAuth callback and provider reconnect link built from it expires
/// with the process — which is worth saying out loud at startup. A durable
/// host, whether a deployment or an operator's own tunnel, is not this.
///
/// The match is on the host's suffix, matching `is_ephemeral_tunnel_base_url`
/// in `bin/tunnel-env.sh` so the startup log and the script that resets
/// `BASE_URL` agree about which hosts are ephemeral. A substring test would
/// also accept `trycloudflare.com.example.net`, where the label is part of a
/// different domain.
#[must_use]
pub fn is_ephemeral_tunnel_base_url(base_url: &str) -> bool {
    let without_scheme = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme);
    let host = host.rsplit_once('@').map_or(host, |(_, after)| after);
    let host = host.split_once(':').map_or(host, |(before, _)| before);
    host.strip_suffix('.')
        .unwrap_or(host)
        .to_ascii_lowercase()
        .ends_with(".trycloudflare.com")
}

/// Public origin of the dravr.ai website when `WEBSITE_BASE_URL` is unset.
pub const DEFAULT_WEBSITE_BASE_URL: &str = "https://dravr.ai";

/// The website origin docs sign-in links are built on: `WEBSITE_BASE_URL`
/// without its trailing slashes, or [`DEFAULT_WEBSITE_BASE_URL`] when it is
/// unset or blank.
fn website_base_url_from(value: Option<&str>) -> String {
    value
        .map(|v| v.trim().trim_end_matches('/'))
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_WEBSITE_BASE_URL)
        .to_owned()
}

/// The USDA key `USDA_API_KEY` holds, without the whitespace around it; `None`
/// when it is unset or blank.
///
/// A secret written with `gcloud secrets versions add` from `echo` ends in a
/// newline, Cloud Run hands the bytes over as they are, and the key goes in a
/// query string: USDA read `<key>%0A` as an invalid key and answered every
/// search on production with 403 `API_KEY_INVALID`, while the same key pasted
/// by hand answered 200.
fn usda_key_from(value: Option<&str>) -> Option<SecretString> {
    value
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(SecretString::from)
}

/// Report the address the server hands out, beside the one it listens on.
///
/// The listen address is where the process accepts connections; `BASE_URL` is
/// what goes into OAuth callbacks and the provider reconnect links an athlete
/// taps. A quick tunnel's hostname stops resolving the moment its `cloudflared`
/// process exits, so a `BASE_URL` still naming one is reported at WARN and is
/// greppable in `logs/pierre-server.log` instead of surfacing only as "server
/// can't be found" in a browser. The value goes through the canonical
/// log-boundary redactor: `BASE_URL` is operator-set and the URL grammar admits
/// `https://user:pass@host`.
pub fn log_effective_base_url(base_url: &str) {
    if is_ephemeral_tunnel_base_url(base_url) {
        warn!(
            base_url = %redact_url(base_url),
            "BASE_URL names a Cloudflare quick tunnel; OAuth callbacks and reconnect links resolve only while that tunnel is up"
        );
    } else {
        info!(
            base_url = %redact_url(base_url),
            "BASE_URL in effect for OAuth callbacks and reconnect links"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        usda_key_from, website_base_url_from, DatabaseUrl, FitnessConfig, OAuthProviderConfig,
        ServerConfig, WeatherApiConfig, DEFAULT_WEBSITE_BASE_URL,
    };
    use secrecy::{ExposeSecret, SecretString};

    const PG_PASSWORD: &str = "pg-password-6f1c";
    const STRAVA_SECRET: &str = "strava-client-secret-9a2e";
    const FIREBASE_KEY: &str = "firebase-api-key-41d7";
    const LOGIN_PASSWORD: &str = "default-login-password-c03b";
    const REDIS_PASSWORD: &str = "redis-password-2b90";
    const USDA_KEY: &str = "usda-api-key-77f0";
    const RESEND_KEY: &str = "re_resend-key-1d3a";

    #[test]
    fn a_usda_key_read_from_a_secret_loses_the_newline_it_was_written_with() {
        let key = usda_key_from(Some(&format!("{USDA_KEY}\n"))).expect("a key is present");
        assert_eq!(
            key.expose_secret(),
            USDA_KEY,
            "the trailing newline reached the key"
        );
        let key = usda_key_from(Some(&format!("  {USDA_KEY}\r\n"))).expect("a key is present");
        assert_eq!(
            key.expose_secret(),
            USDA_KEY,
            "the surrounding whitespace reached the key"
        );
    }

    #[test]
    fn a_blank_usda_key_is_no_key() {
        assert!(usda_key_from(None).is_none(), "an unset key read as a key");
        assert!(
            usda_key_from(Some("")).is_none(),
            "an empty key read as a key"
        );
        assert!(
            usda_key_from(Some(" \n")).is_none(),
            "a whitespace-only key read as a key"
        );
    }

    /// What `secrecy` prints in place of a `SecretString`.
    const SECRET_MARKER: &str = "SecretBox<str>([REDACTED])";

    const ALL_SECRETS: [&str; 7] = [
        PG_PASSWORD,
        STRAVA_SECRET,
        FIREBASE_KEY,
        LOGIN_PASSWORD,
        REDIS_PASSWORD,
        USDA_KEY,
        RESEND_KEY,
    ];

    /// A configuration with every secret-bearing field set to a known value.
    fn config_holding_every_secret() -> ServerConfig {
        let mut config = ServerConfig {
            usda_api_key: Some(SecretString::from(USDA_KEY)),
            resend_api_key: Some(SecretString::from(RESEND_KEY)),
            ..ServerConfig::default()
        };
        config.database.url = DatabaseUrl::parse_url(&format!(
            "postgresql://pierre:{PG_PASSWORD}@db.internal:5432/pierre"
        ))
        .unwrap();
        config.oauth.strava = OAuthProviderConfig {
            client_id: Some("strava-client-id".to_owned()),
            client_secret: Some(SecretString::from(STRAVA_SECRET)),
            enabled: true,
            ..OAuthProviderConfig::default()
        };
        config.firebase.api_key = Some(SecretString::from(FIREBASE_KEY));
        config.oauth2_server.default_login_password = Some(SecretString::from(LOGIN_PASSWORD));
        config.cache.redis_url = Some(SecretString::from(format!(
            "redis://:{REDIS_PASSWORD}@cache.internal:6379"
        )));
        config
    }

    #[test]
    fn debug_of_the_server_config_prints_no_secret() {
        let printed = format!("{:?}", config_holding_every_secret());
        for secret in ALL_SECRETS {
            assert!(!printed.contains(secret), "{secret} leaked: {printed}");
        }
        // Six fields are SecretString; the database URL goes through redact_url.
        assert_eq!(printed.matches(SECRET_MARKER).count(), 6, "{printed}");
        assert!(
            printed.contains("postgresql://***:***@db.internal:5432/pierre"),
            "{printed}"
        );
        // Non-secret neighbours still print, so the Debug output stays useful.
        assert!(printed.contains("strava-client-id"), "{printed}");
    }

    #[test]
    fn alternate_debug_of_the_server_config_prints_no_secret() {
        let printed = format!("{:#?}", config_holding_every_secret());
        for secret in ALL_SECRETS {
            assert!(!printed.contains(secret), "{secret} leaked: {printed}");
        }
        assert_eq!(printed.matches(SECRET_MARKER).count(), 6, "{printed}");
    }

    #[test]
    fn serializing_the_server_config_omits_every_secret() {
        let json = serde_json::to_string(&config_holding_every_secret()).unwrap();
        for secret in ALL_SECRETS {
            assert!(!json.contains(secret), "{secret} leaked: {json}");
        }
        for field in [
            "client_secret",
            "default_login_password",
            "redis_url",
            "usda_api_key",
            "resend_api_key",
        ] {
            assert!(!json.contains(field), "{field} serialized: {json}");
        }
        assert!(
            json.contains("postgresql://***:***@db.internal:5432/pierre"),
            "{json}"
        );
    }

    #[test]
    fn the_secret_accessors_expose_the_configured_values() {
        let mut config = config_holding_every_secret();
        config.resend_from_email = Some("Dravr <no-reply@dravr.ai>".to_owned());
        assert_eq!(
            config.outbound_email_credentials(),
            Some((RESEND_KEY, "Dravr <no-reply@dravr.ai>"))
        );
        assert_eq!(
            config.database.url.connection_string().expose_secret(),
            format!("postgresql://pierre:{PG_PASSWORD}@db.internal:5432/pierre")
        );
    }

    #[test]
    fn the_website_origin_drops_trailing_slashes() {
        assert_eq!(
            website_base_url_from(Some(" https://staging.dravr.ai// ")),
            "https://staging.dravr.ai"
        );
    }

    #[test]
    fn an_unset_or_blank_website_origin_is_dravr_ai() {
        assert_eq!(website_base_url_from(None), DEFAULT_WEBSITE_BASE_URL);
        assert_eq!(website_base_url_from(Some("  ")), DEFAULT_WEBSITE_BASE_URL);
        assert_eq!(website_base_url_from(Some("/")), DEFAULT_WEBSITE_BASE_URL);
    }

    #[test]
    fn a_configuration_built_by_hand_asks_no_weather_vendor() {
        let weather_api = ServerConfig::default()
            .fitness
            .weather_api
            .expect("a hand-built configuration names where weather is asked");
        assert_eq!(
            weather_api.base_url,
            ServerConfig::REFUSED_ORIGIN,
            "bug: a hand-built configuration asks a weather vendor"
        );
        assert!(
            !weather_api.enabled,
            "a hand-built configuration backfills no temperature"
        );

        // One that names no fitness configuration reads the same.
        let mut serialized = serde_json::to_value(ServerConfig::default()).unwrap();
        serialized.as_object_mut().unwrap().remove("fitness");
        let read: ServerConfig = serde_json::from_value(serialized).unwrap();
        let read_weather = read.fitness.weather_api.unwrap();
        assert_eq!(
            read_weather.base_url,
            ServerConfig::REFUSED_ORIGIN,
            "bug: a configuration naming no fitness section asks a weather vendor"
        );
        assert!(
            !read_weather.enabled,
            "bug: a configuration naming no fitness section backfills temperatures"
        );

        // The fitness configuration a deployment loads starts from its own
        // default, the vendors' hosts, never from this one.
        let deployment = FitnessConfig::default().weather_api.unwrap();
        assert_eq!(
            deployment.base_url,
            WeatherApiConfig::DEFAULT_BASE_URL,
            "bug: a deployment's fitness default no longer names the vendors' hosts"
        );
        assert!(
            deployment.enabled,
            "bug: a deployment's fitness default turned the temperature backfill off"
        );
    }
}
