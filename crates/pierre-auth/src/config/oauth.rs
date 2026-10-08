// ABOUTME: OAuth configuration types for fitness provider authentication
// ABOUTME: Handles Strava, Garmin, WHOOP, Terra OAuth and Firebase auth settings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::provider_seats::STRAVA_OAUTH_SEAT_CAP_DEFAULT;
use pierre_core::constants::{oauth2_client_retention, oauth_providers};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::gcp_token::METADATA_TOKEN_URL;
use pierre_core::redaction::redact_url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::iter;
use tracing::{debug, info, warn};
use url::Url;

use super::google_sign_in::GoogleSignInConfig;
use crate::oauth2_server::first_party::FirstPartyRedirects;

/// OAuth provider configuration for fitness platforms
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthConfig {
    /// Strava OAuth configuration
    pub strava: OAuthProviderConfig,
    /// Garmin OAuth configuration
    pub garmin: OAuthProviderConfig,
    /// WHOOP OAuth configuration
    pub whoop: OAuthProviderConfig,
    /// Terra OAuth configuration
    pub terra: OAuthProviderConfig,
}

impl OAuthConfig {
    /// Load OAuth configuration from environment, each provider through
    /// [`get_oauth_config`], the one reader of provider OAuth settings.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            strava: get_oauth_config(oauth_providers::STRAVA),
            garmin: get_oauth_config(oauth_providers::GARMIN),
            whoop: get_oauth_config(oauth_providers::WHOOP),
            terra: get_oauth_config(oauth_providers::TERRA),
        }
    }

    /// The configuration of `provider`, or `None` for a provider this
    /// configuration does not carry.
    #[must_use]
    pub fn provider(&self, provider: &str) -> Option<&OAuthProviderConfig> {
        match provider.to_lowercase().as_str() {
            p if p == oauth_providers::STRAVA => Some(&self.strava),
            p if p == oauth_providers::GARMIN => Some(&self.garmin),
            p if p == oauth_providers::WHOOP => Some(&self.whoop),
            p if p == oauth_providers::TERRA => Some(&self.terra),
            _ => None,
        }
    }
}

/// OAuth provider-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthProviderConfig {
    /// OAuth client ID
    pub client_id: Option<String>,
    /// OAuth client secret
    pub client_secret: Option<String>,
    /// OAuth redirect URI
    pub redirect_uri: Option<String>,
    /// OAuth scopes
    pub scopes: Vec<String>,
    /// Enable this provider
    pub enabled: bool,
}

/// The first 8 hex characters of the SHA-256 of `secret`: enough to tell
/// two secrets apart in a log line without logging either.
#[must_use]
pub(crate) fn secret_fingerprint(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    format!("{digest:x}").chars().take(8).collect()
}

impl OAuthProviderConfig {
    /// Compute SHA256 fingerprint of client secret for debugging (first 8 hex chars)
    /// This allows comparing secrets without logging actual values
    #[must_use]
    pub fn secret_fingerprint(&self) -> Option<String> {
        self.client_secret.as_deref().map(secret_fingerprint)
    }

    /// Validate OAuth credentials and log diagnostics
    /// Returns true if credentials appear valid, false otherwise
    pub fn validate_and_log(&self, provider_name: &str) -> bool {
        if !self.enabled {
            info!("OAuth provider {provider_name} is disabled");
            return true; // Disabled is valid state
        }

        let Some(client_id) = self.validate_client_id(provider_name) else {
            return false;
        };

        let Some(client_secret) = self.validate_client_secret(provider_name) else {
            return false;
        };

        self.log_credential_diagnostics(provider_name, client_id, client_secret);
        Self::validate_secret_length(provider_name, client_secret)
    }

    /// Validate client ID is present and non-empty
    fn validate_client_id(&self, provider_name: &str) -> Option<&str> {
        match &self.client_id {
            Some(id) if !id.is_empty() => Some(id.as_str()),
            _ => {
                warn!("OAuth provider {provider_name}: client_id is missing or empty");
                None
            }
        }
    }

    /// Validate client secret is present and non-empty
    fn validate_client_secret(&self, provider_name: &str) -> Option<&str> {
        match &self.client_secret {
            Some(secret) if !secret.is_empty() => Some(secret.as_str()),
            _ => {
                warn!("OAuth provider {provider_name}: client_secret is missing or empty");
                None
            }
        }
    }

    /// Log OAuth credential diagnostics (fingerprint, lengths, etc.)
    fn log_credential_diagnostics(
        &self,
        provider_name: &str,
        client_id: &str,
        client_secret: &str,
    ) {
        let fingerprint = self
            .secret_fingerprint()
            .unwrap_or_else(|| "none".to_owned());
        info!(
            "OAuth provider {provider_name}: enabled=true, client_id={client_id}, \
             secret_length={}, secret_fingerprint={fingerprint}",
            client_secret.len()
        );
    }

    /// Validate secret length meets minimum requirements
    fn validate_secret_length(provider_name: &str, client_secret: &str) -> bool {
        if client_secret.len() < 20 {
            warn!(
                "OAuth provider {provider_name}: client_secret is unusually short ({} chars) - \
                 this may indicate a configuration error",
                client_secret.len()
            );
            return false;
        }
        true
    }
}

/// `OAuth2` authorization server configuration (for Pierre acting as OAuth server)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2ServerConfig {
    /// `OAuth2` issuer URL for RFC 8414 discovery (format: <https://your-domain.com>)
    /// MUST be set in production to actual deployment domain. Defaults to `http://localhost:PORT` in development.
    pub issuer_url: String,
    /// Resource identifier of the MCP resource server (RFC 9728 §2, RFC 8707).
    ///
    /// The origin MCP clients dial, `scheme://host[:port]` with no path.
    ///
    /// Published as `resource` in `/.well-known/oauth-protected-resource`, the
    /// origin of the `resource_metadata` URL in every `/mcp` 401 challenge, the
    /// one value a `resource` parameter on `/oauth2/authorize` and
    /// `/oauth2/token` may name, and the audience of the tokens minted for it.
    /// `MCP_RESOURCE_URL`, defaulting to `BASE_URL`, so a deployment that
    /// answers to one hostname needs no setting.
    pub mcp_resource_url: String,
    /// The other origins that also serve `/mcp`, each an identifier of the
    /// same MCP resource server: `BASE_URL`, when `MCP_RESOURCE_URL` names
    /// another host.
    ///
    /// A client that dialed one of them is answered with that origin as its
    /// `resource` (RFC 9728 §3.3 has it refuse any other), may name it as the
    /// RFC 8707 `resource`, and presents tokens bound to it — so moving the
    /// published MCP host leaves every client of the previous one working.
    pub mcp_resource_aliases: Vec<String>,
    /// Default email for OAuth login page (dev/test only - do not use in production)
    pub default_login_email: Option<String>,
    /// Default password for OAuth login page (dev/test only - NEVER use in production!)
    pub default_login_password: Option<String>,
    /// How long RFC 7591 dynamic client registrations are kept, and how many
    /// no user has authorized may exist at once
    pub client_retention: ClientRetentionConfig,
    /// "Continue with Google" on the hosted login page: the Google OAuth web
    /// client the authorization server signs athletes in through. `None`
    /// when `GOOGLE_OAUTH_CLIENT_ID`/`GOOGLE_OAUTH_CLIENT_SECRET` are unset,
    /// which hides the button. Never serialized: it carries a client secret.
    /// Boxed so `ServerConfig`, which async setup holds by value across
    /// awaits, grows by one pointer rather than five strings.
    #[serde(skip)]
    pub google_sign_in: Option<Box<GoogleSignInConfig>>,
    /// Where Dravr's own web and mobile apps may receive their authorization
    /// code: the web app's origins (`FRONTEND_URL`, then the issuer), and
    /// whether a development build in Expo Go may sign in
    /// (`OAUTH_ALLOW_EXPO_GO_REDIRECT=true`, never on a deployed server).
    pub first_party_redirects: FirstPartyRedirects,
}

impl Default for OAuth2ServerConfig {
    fn default() -> Self {
        Self {
            issuer_url: "http://localhost:8081".to_owned(),
            mcp_resource_url: "http://localhost:8081".to_owned(),
            mcp_resource_aliases: Vec::new(),
            default_login_email: None,
            default_login_password: None,
            client_retention: ClientRetentionConfig::default(),
            google_sign_in: None,
            first_party_redirects: FirstPartyRedirects {
                web_origins: vec!["http://localhost:8081".to_owned()],
                allow_expo_go: false,
            },
        }
    }
}

/// Retention of RFC 7591 dynamic client registrations.
///
/// `POST /oauth2/register` is anonymous, so what it stores is bounded here
/// rather than by who calls it. A registration is *pending* until a refresh
/// token is issued through it — that is, until a user authorizes it. Pending
/// registrations are capped at [`max_pending_registrations`] and deleted
/// [`abandoned_after_secs`] after they were made; any registration is deleted
/// [`expired_grace_secs`] after its `expires_at`. The sweep that deletes them
/// runs every [`sweep_interval_secs`].
///
/// [`max_pending_registrations`]: Self::max_pending_registrations
/// [`abandoned_after_secs`]: Self::abandoned_after_secs
/// [`expired_grace_secs`]: Self::expired_grace_secs
/// [`sweep_interval_secs`]: Self::sweep_interval_secs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientRetentionConfig {
    /// Seconds an expired registration is kept past its `expires_at`
    /// (`OAUTH2_CLIENT_EXPIRED_GRACE_SECS`, default 30 days)
    pub expired_grace_secs: u64,
    /// Seconds after which a registration no user has authorized is deleted
    /// (`OAUTH2_CLIENT_ABANDONED_AFTER_SECS`, default 24 hours)
    pub abandoned_after_secs: u64,
    /// Seconds between retention sweeps (`OAUTH2_CLIENT_SWEEP_INTERVAL_SECS`,
    /// default 1 hour)
    pub sweep_interval_secs: u64,
    /// Registrations no user has authorized that may exist at once; the next is
    /// refused with 429 (`OAUTH2_MAX_PENDING_CLIENT_REGISTRATIONS`, default
    /// 10,000). `0` refuses every registration.
    pub max_pending_registrations: u64,
}

impl Default for ClientRetentionConfig {
    fn default() -> Self {
        Self {
            expired_grace_secs: oauth2_client_retention::EXPIRED_GRACE_SECS,
            abandoned_after_secs: oauth2_client_retention::ABANDONED_AFTER_SECS,
            sweep_interval_secs: oauth2_client_retention::SWEEP_INTERVAL_SECS,
            max_pending_registrations: oauth2_client_retention::MAX_PENDING_REGISTRATIONS,
        }
    }
}

impl ClientRetentionConfig {
    /// Load the retention policy from the environment; an unset or unparsable
    /// variable keeps its default.
    #[must_use]
    pub fn from_env() -> Self {
        let read = |name: &str, default: u64| {
            env::var(name)
                .ok()
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(default)
        };
        let defaults = Self::default();
        Self {
            expired_grace_secs: read(
                "OAUTH2_CLIENT_EXPIRED_GRACE_SECS",
                defaults.expired_grace_secs,
            ),
            abandoned_after_secs: read(
                "OAUTH2_CLIENT_ABANDONED_AFTER_SECS",
                defaults.abandoned_after_secs,
            ),
            sweep_interval_secs: read(
                "OAUTH2_CLIENT_SWEEP_INTERVAL_SECS",
                defaults.sweep_interval_secs,
            ),
            max_pending_registrations: read(
                "OAUTH2_MAX_PENDING_CLIENT_REGISTRATIONS",
                defaults.max_pending_registrations,
            ),
        }
    }
}

/// Resolve the OAuth issuer from the two addresses a deployment may know
/// itself by, falling back to the local form.
///
/// Split out of [`OAuth2ServerConfig::from_env`] so the precedence can be
/// tested without mutating process environment: `from_env` reads the same
/// three inputs and does nothing else with them.
#[must_use]
pub fn resolve_issuer_url(
    explicit: Option<&str>,
    base_url: Option<&str>,
    http_port: u16,
) -> String {
    first_configured_url(explicit, base_url, http_port)
}

/// Resolve the MCP resource identifier (`MCP_RESOURCE_URL`, else `BASE_URL`).
///
/// The same precedence as the issuer, then the local form, so an unset
/// `MCP_RESOURCE_URL` names the address the deployment already answers to.
///
/// A trailing `/` is dropped: the value is published verbatim as the RFC 9728
/// `resource`, and `/.well-known/oauth-protected-resource` is appended to it
/// for the 401 challenge. Like [`resolve_issuer_url`], split out of
/// [`OAuth2ServerConfig::from_env`] so the precedence is testable without
/// mutating the process environment.
#[must_use]
pub fn resolve_mcp_resource_url(
    explicit: Option<&str>,
    base_url: Option<&str>,
    http_port: u16,
) -> String {
    first_configured_url(explicit, base_url, http_port)
        .trim_end_matches('/')
        .to_owned()
}

/// The request headers that carry the authority a client dialed.
///
/// Most specific first: the `X-Forwarded-Host` the frontend proxy writes (the
/// backend's own `Host` is rewritten on the way in), then `Host`. Read in this
/// order into [`OAuth2ServerConfig::mcp_resource_for_host`].
pub const DIALED_HOST_HEADERS: [&str; 2] = ["x-forwarded-host", "host"];

/// The other origins that serve the MCP resource `canonical`.
///
/// That is `base_url`, when it is set, is itself an origin
/// (`scheme://host[:port]`, nothing after) and differs from `canonical`. A
/// `base_url` with a path is no resource identifier a client could have dialed
/// `/mcp` under, so it names no alias.
///
/// Split out of [`OAuth2ServerConfig::from_env`] so it is testable without
/// mutating the process environment, like [`resolve_mcp_resource_url`].
#[must_use]
pub fn resolve_mcp_resource_aliases(canonical: &str, base_url: Option<&str>) -> Vec<String> {
    base_url
        .map(|url| url.trim().trim_end_matches('/'))
        .filter(|url| *url != canonical)
        .filter(|url| {
            Url::parse(url).is_ok_and(|parsed| {
                matches!(parsed.scheme(), "http" | "https")
                    && parsed.host().is_some()
                    && parsed.path() == "/"
                    && parsed.query().is_none()
                    && parsed.fragment().is_none()
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
            })
        })
        .map(str::to_owned)
        .into_iter()
        .collect()
}

/// The first of `explicit` and `base_url` that is set and not blank, else
/// `http://localhost:{http_port}`.
fn first_configured_url(explicit: Option<&str>, base_url: Option<&str>, http_port: u16) -> String {
    explicit
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or_else(|| base_url.map(str::trim).filter(|v| !v.is_empty()))
        .map_or_else(|| format!("http://localhost:{http_port}"), str::to_owned)
}

/// The origins Dravr's web app is served from: the frontend's own URL when
/// it is set, then the issuer, which serves the app wherever the frontend
/// and the API share one origin. Each reduced to `scheme://host[:port]`;
/// one that does not parse is left out.
fn first_party_web_origins(frontend_url: Option<&str>, issuer_url: &str) -> Vec<String> {
    let mut origins: Vec<String> = Vec::new();
    for candidate in frontend_url.into_iter().chain(iter::once(issuer_url)) {
        let Ok(url) = Url::parse(candidate.trim()) else {
            warn!(
                url = %redact_url(candidate),
                "Ignoring a first-party web origin that does not parse"
            );
            continue;
        };
        let origin = url.origin().ascii_serialization();
        if url.origin().is_tuple() && !origins.contains(&origin) {
            origins.push(origin);
        }
    }
    origins
}

impl OAuth2ServerConfig {
    /// Load `OAuth2` authorization server configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        let http_port = env::var("HTTP_PORT")
            .or_else(|_| env::var("MCP_PORT"))
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8081);
        let base_url = env::var("BASE_URL").ok();
        // The MCP resource server is named separately from the issuer
        // (RFC 9728 expects the two to differ) so the MCP endpoint can be
        // published under its own hostname while the authorization server
        // stays where it is (carnet#484).
        let mcp_resource_url = resolve_mcp_resource_url(
            env::var("MCP_RESOURCE_URL").ok().as_deref(),
            base_url.as_deref(),
            http_port,
        );
        let issuer_url = resolve_issuer_url(
            env::var("OAUTH2_ISSUER_URL").ok().as_deref(),
            base_url.as_deref(),
            http_port,
        );
        let first_party_redirects = FirstPartyRedirects {
            web_origins: first_party_web_origins(
                env::var("FRONTEND_URL").ok().as_deref(),
                &issuer_url,
            ),
            allow_expo_go: env::var("OAUTH_ALLOW_EXPO_GO_REDIRECT")
                .is_ok_and(|value| value.trim().eq_ignore_ascii_case("true")),
        };
        Self {
            // The issuer is published verbatim in
            // `/.well-known/oauth-authorization-server` and
            // `/.well-known/oauth-protected-resource`, so a localhost default
            // that survives into a deployment tells every external MCP client
            // to send its authorization, token and JWKS requests to its own
            // machine. `OAUTH2_ISSUER_URL` is set nowhere in infra, so dev
            // advertised `http://localhost:8081` and no external client could
            // complete OAuth against it (carnet#358). BASE_URL is the address
            // the deployment already knows itself by — the same fallback the
            // redirect URIs above use — so the localhost form is now reached
            // only when neither is set, which is the local case it was written
            // for.
            issuer_url,
            mcp_resource_url: mcp_resource_url.clone(),
            // A client of BASE_URL's /mcp keeps working once MCP_RESOURCE_URL
            // publishes another host (carnet#639).
            mcp_resource_aliases: resolve_mcp_resource_aliases(
                &mcp_resource_url,
                base_url.as_deref(),
            ),
            default_login_email: env::var("OAUTH_DEFAULT_EMAIL").ok(),
            default_login_password: env::var("OAUTH_DEFAULT_PASSWORD").ok(),
            client_retention: ClientRetentionConfig::from_env(),
            google_sign_in: GoogleSignInConfig::from_env().map(Box::new),
            first_party_redirects,
        }
    }

    /// Refuse an `mcp_resource_url` that cannot be published as a resource
    /// identifier.
    ///
    /// It must be an absolute `http`/`https` URL naming an origin — no path,
    /// query, fragment or credentials. The protected-resource document is
    /// served only at `{origin}/.well-known/oauth-protected-resource`, and RFC
    /// 9728 §3.3 has a client reject a `resource` that differs from the URL it
    /// fetched that document from, so a value with a path publishes a
    /// document every conforming client refuses. `require_https` is set in
    /// production, where a bearer token must never cross a plain-HTTP hop.
    ///
    /// # Errors
    /// Returns an invalid-input error naming the defect.
    pub fn validate_mcp_resource_url(&self, require_https: bool) -> AppResult<()> {
        Self::validate_resource_origin(&self.mcp_resource_url, require_https)
    }

    /// Every identifier of the MCP resource server: `mcp_resource_url` first,
    /// then its aliases.
    #[must_use]
    pub fn mcp_resources(&self) -> Vec<&str> {
        iter::once(self.mcp_resource_url.as_str())
            .chain(self.mcp_resource_aliases.iter().map(String::as_str))
            .collect()
    }

    /// The MCP resource identifier a client that dialed `host` knows the
    /// server by: the one whose `host[:port]` it is, else `mcp_resource_url`.
    ///
    /// `host` is the dialed authority as a proxy forwards it (the first entry
    /// of an `X-Forwarded-Host` list) or the `Host` header. Only a configured
    /// identifier is ever returned, so a forged header can at most pick
    /// another of this server's own names.
    #[must_use]
    pub fn mcp_resource_for_host(&self, host: Option<&str>) -> &str {
        let Some(host) = host
            .and_then(|host| host.split(',').next())
            .map(str::trim)
            .filter(|host| !host.is_empty())
        else {
            return &self.mcp_resource_url;
        };
        self.mcp_resources()
            .into_iter()
            .find(|resource| {
                Url::parse(resource).is_ok_and(|url| {
                    url.host_str().is_some_and(|name| {
                        let authority = url
                            .port()
                            .map_or_else(|| name.to_owned(), |port| format!("{name}:{port}"));
                        authority.eq_ignore_ascii_case(host)
                    })
                })
            })
            .unwrap_or(&self.mcp_resource_url)
    }

    /// Refuse a resource identifier that is not an `http`/`https` origin.
    fn validate_resource_origin(value: &str, require_https: bool) -> AppResult<()> {
        let url = Url::parse(value).map_err(|e| {
            AppError::invalid_input(format!(
                "MCP_RESOURCE_URL must be an absolute URL ({e}): {value}"
            ))
        })?;
        match url.scheme() {
            "https" => {}
            "http" if !require_https => {}
            "http" => {
                return Err(AppError::invalid_input(format!(
                    "MCP_RESOURCE_URL must use HTTPS in production: {value}"
                )))
            }
            _ => {
                return Err(AppError::invalid_input(format!(
                    "MCP_RESOURCE_URL must be an http or https URL: {value}"
                )))
            }
        }
        let is_origin = url.host().is_some()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && url.username().is_empty()
            && url.password().is_none();
        if is_origin {
            Ok(())
        } else {
            Err(AppError::invalid_input(format!(
                "MCP_RESOURCE_URL must name an origin (scheme://host[:port]) with no path, \
                 query, fragment or credentials: {value}"
            )))
        }
    }
}

/// Identity Toolkit, the API behind Firebase Authentication's user records.
const IDENTITY_TOOLKIT_URL: &str = "https://identitytoolkit.googleapis.com";

/// Firebase Authentication configuration for social logins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirebaseConfig {
    /// Firebase project ID (required for token validation)
    pub project_id: Option<String>,
    /// Firebase API key (optional, for client-side SDK)
    pub api_key: Option<String>,
    /// Whether Firebase authentication is enabled
    pub enabled: bool,
    /// Base URL of the Identity Toolkit API the account delete removes the
    /// Firebase user through; Google's public endpoint unless
    /// `FIREBASE_IDENTITY_TOOLKIT_URL` points it at the Auth emulator or a
    /// test stub.
    pub identity_toolkit_url: String,
    /// Where the Google access token for Identity Toolkit is minted: the
    /// Cloud Run metadata server, whose service account holds
    /// `roles/firebaseauth.admin` on the Firebase project.
    pub access_token_url: String,
}

impl Default for FirebaseConfig {
    fn default() -> Self {
        Self {
            project_id: None,
            api_key: None,
            enabled: false,
            identity_toolkit_url: IDENTITY_TOOLKIT_URL.to_owned(),
            access_token_url: METADATA_TOKEN_URL.to_owned(),
        }
    }
}

impl FirebaseConfig {
    /// Check if Firebase is properly configured and enabled
    /// Returns `true` if Firebase is enabled and has a project ID configured
    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.enabled && self.project_id.is_some()
    }

    /// Load Firebase configuration from environment
    ///
    /// Environment variables:
    /// - `FIREBASE_PROJECT_ID` - Firebase project ID (required for token validation)
    /// - `FIREBASE_API_KEY` - Firebase API key (optional, for client-side SDK)
    /// - `FIREBASE_ENABLED` - Enable Firebase authentication (default: false)
    /// - `FIREBASE_IDENTITY_TOOLKIT_URL` - Identity Toolkit base URL (default:
    ///   Google's public endpoint)
    #[must_use]
    pub fn from_env() -> Self {
        let project_id = env::var("FIREBASE_PROJECT_ID").ok();
        let api_key = env::var("FIREBASE_API_KEY").ok();

        // Firebase is enabled if project_id is set and FIREBASE_ENABLED is not explicitly false
        let enabled = project_id.is_some()
            && env_var_or("FIREBASE_ENABLED", "true")
                .parse()
                .unwrap_or(true);

        if enabled {
            info!(
                project_id = project_id.as_deref().unwrap_or("(not set)"),
                "Firebase authentication enabled"
            );
        }

        Self {
            project_id,
            api_key,
            enabled,
            identity_toolkit_url: env_var_or("FIREBASE_IDENTITY_TOOLKIT_URL", IDENTITY_TOOLKIT_URL),
            access_token_url: METADATA_TOKEN_URL.to_owned(),
        }
    }
}

/// Get the deployment-wide default provider, if one is configured.
///
/// Reads the `PIERRE_DEFAULT_PROVIDER` environment variable. Returns `None`
/// when the variable is unset or empty. **There is no static fallback** —
/// historical behavior fell back to `oauth_providers::SYNTHETIC`, which
/// silently bound untargeted tool calls to seed data in production (the LLM
/// hallucinated over it because production builds don't even compile
/// `provider-synthetic` in).
///
/// Per-request callers should resolve via
/// `pierre_providers::activity_source::resolve_activity_source` to pick the
/// connection that answers the user's activity questions, and surface `AppError::no_provider_connected`
/// when the user has no provider connections at all — the existing
/// `auth_recovery` chat-pipeline stage mints a hosted-login URL and renders
/// the FR/EN reconnect copy from that error.
///
/// # Examples
///
/// ```bash
/// # Pin a global default at the deployment level
/// export PIERRE_DEFAULT_PROVIDER=strava
/// ```
#[must_use]
pub fn default_provider() -> Option<String> {
    let provider = env::var("PIERRE_DEFAULT_PROVIDER")
        .ok()
        .filter(|s| !s.is_empty())?;

    info!("Default provider configured (env override): {}", provider);
    Some(provider)
}

/// Athlete-seat capacity for the shared Dravr Strava OAuth app.
///
/// Strava enforces a per-application athlete limit ("Number of athletes allowed
/// to connect" in the API settings dashboard) and does **not** expose it via any
/// API response header, so the platform cannot read it at runtime. This returns
/// the operator-configured cap from the `STRAVA_OAUTH_SEAT_CAP` environment
/// variable, falling back to [`STRAVA_OAUTH_SEAT_CAP_DEFAULT`] (the Standard Tier
/// entry-level cap). Bump the env var after self-upgrading the app's tier on
/// Strava.
///
/// A non-numeric or zero value falls back to the default — a zero cap would
/// wedge every athlete onto the scraper fallback, which is never the intent.
///
/// # Examples
///
/// ```bash
/// export STRAVA_OAUTH_SEAT_CAP=10
/// ```
#[must_use]
pub fn strava_oauth_seat_cap() -> u32 {
    env::var("STRAVA_OAUTH_SEAT_CAP")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|&cap| cap > 0)
        .unwrap_or(STRAVA_OAUTH_SEAT_CAP_DEFAULT)
}

/// Get OAuth provider configuration by provider name.
///
/// This is the one reader of a provider's OAuth settings in the environment:
/// [`OAuthConfig::from_env`] builds `ServerConfig.oauth` from it, and runtime
/// lookups call it.
///
/// - `client_id` / `client_secret`: `<P>_CLIENT_ID` / `<P>_CLIENT_SECRET`,
///   else `PIERRE_<P>_CLIENT_ID` / `PIERRE_<P>_CLIENT_SECRET` (Terra names its
///   pair `TERRA_DEV_ID` / `TERRA_API_KEY`)
/// - `redirect_uri`: `<P>_REDIRECT_URI`, `None` when unset: the caller then
///   uses the server's own callback URL
/// - `scopes`: `PIERRE_<P>_SCOPES`, else `<P>_SCOPES`, else the provider's
///   default scopes
/// - `enabled`: both credentials are set
///
/// An unknown provider gets the empty, disabled default.
///
/// # Arguments
/// * `provider_name` - The provider name (e.g., "strava", "garmin", "whoop")
#[must_use]
pub fn get_oauth_config(provider_name: &str) -> OAuthProviderConfig {
    let (id_key, secret_key, default_scopes) = match provider_name {
        p if p == oauth_providers::STRAVA => (
            "STRAVA_CLIENT_ID",
            "STRAVA_CLIENT_SECRET",
            parse_scopes(oauth_providers::STRAVA_DEFAULT_SCOPES),
        ),
        // Garmin's scope is fixed server-side and its authorization takes
        // none (Garmin Connect Developer Program OAuth2.0 PKCE
        // Specification), so no scope is requested unless one is configured.
        p if p == oauth_providers::GARMIN => {
            ("GARMIN_CLIENT_ID", "GARMIN_CLIENT_SECRET", Vec::new())
        }
        p if p == oauth_providers::WHOOP => (
            "WHOOP_CLIENT_ID",
            "WHOOP_CLIENT_SECRET",
            parse_scopes(oauth_providers::WHOOP_DEFAULT_SCOPES),
        ),
        p if p == oauth_providers::TERRA => (
            "TERRA_DEV_ID",
            "TERRA_API_KEY",
            parse_scopes(oauth_providers::TERRA_DEFAULT_SCOPES),
        ),
        p if p == oauth_providers::INTERVALS_ICU => (
            "INTERVALS_ICU_CLIENT_ID",
            "INTERVALS_ICU_CLIENT_SECRET",
            oauth_providers::INTERVALS_ICU_DEFAULT_SCOPES
                .iter()
                .map(|scope| (*scope).to_owned())
                .collect(),
        ),
        p if p == oauth_providers::WAHOO => (
            "WAHOO_CLIENT_ID",
            "WAHOO_CLIENT_SECRET",
            oauth_providers::WAHOO_DEFAULT_SCOPES
                .iter()
                .map(|scope| (*scope).to_owned())
                .collect(),
        ),
        _ => {
            debug!(
                "Unknown provider '{}', returning default config",
                provider_name
            );
            return OAuthProviderConfig::default();
        }
    };
    let upper = provider_name.to_uppercase();
    let client_id = env::var(id_key)
        .or_else(|_| env::var(format!("PIERRE_{upper}_CLIENT_ID")))
        .ok();
    let client_secret = env::var(secret_key)
        .or_else(|_| env::var(format!("PIERRE_{upper}_CLIENT_SECRET")))
        .ok();
    let scopes = env::var(format!("PIERRE_{upper}_SCOPES"))
        .or_else(|_| env::var(format!("{upper}_SCOPES")))
        .map_or(default_scopes, |s| parse_scopes(&s));

    OAuthProviderConfig {
        enabled: client_id.is_some() && client_secret.is_some(),
        client_id,
        client_secret,
        redirect_uri: env::var(format!("{upper}_REDIRECT_URI")).ok(),
        scopes,
    }
}

/// Provider endpoint configuration tuple from environment variables
///
/// Returns: (`auth_url`, `token_url`, `api_base_url`, `revoke_url`, `scopes`)
pub type ProviderEnvConfig = (String, String, String, Option<String>, Vec<String>);

/// Load provider-specific endpoint configuration from environment variables
///
/// Reads provider endpoints from `PIERRE_<PROVIDER>_*` environment variables.
/// Falls back to provided defaults if environment variables are not set. The
/// client credentials are read by [`get_oauth_config`] alone.
///
/// # Environment Variables
///
/// For each provider (e.g., STRAVA, GARMIN):
/// - `PIERRE_<PROVIDER>_AUTH_URL` - OAuth authorization URL (optional)
/// - `PIERRE_<PROVIDER>_TOKEN_URL` - OAuth token URL (optional)
/// - `PIERRE_<PROVIDER>_API_BASE_URL` - Provider API base URL (optional)
/// - `PIERRE_<PROVIDER>_REVOKE_URL` - Token revocation URL (optional)
/// - `PIERRE_<PROVIDER>_SCOPES` - Scopes separated by commas or spaces (optional)
///
/// # Examples
///
/// ```bash
/// # Strava configuration
/// export PIERRE_STRAVA_SCOPES="activity:read_all,profile:read_all"
///
/// # Garmin configuration (optional URLs override defaults)
/// export PIERRE_GARMIN_API_BASE_URL=https://custom-garmin-api.example.com
/// ```
#[must_use]
pub fn load_provider_env_config(
    provider: &str,
    default_auth_url: &str,
    default_token_url: &str,
    default_api_base_url: &str,
    default_revoke_url: Option<&str>,
    default_scopes: &[String],
) -> ProviderEnvConfig {
    let provider_upper = provider.to_uppercase();

    // Load URLs with defaults
    let auth_url = env::var(format!("PIERRE_{provider_upper}_AUTH_URL"))
        .unwrap_or_else(|_| default_auth_url.to_owned());

    let token_url = env::var(format!("PIERRE_{provider_upper}_TOKEN_URL"))
        .unwrap_or_else(|_| default_token_url.to_owned());

    let api_base_url = env::var(format!("PIERRE_{provider_upper}_API_BASE_URL"))
        .unwrap_or_else(|_| default_api_base_url.to_owned());

    let revoke_url = env::var(format!("PIERRE_{provider_upper}_REVOKE_URL"))
        .ok()
        .or_else(|| default_revoke_url.map(ToOwned::to_owned));

    // Load scopes with default, read the way `get_oauth_config` reads the
    // same variable: commas or whitespace, so a space-separated override
    // (WHOOP's and Wahoo's own form) never collapses into one scope.
    let scopes = env::var(format!("PIERRE_{provider_upper}_SCOPES"))
        .ok()
        .map_or_else(|| default_scopes.to_vec(), |s| parse_scopes(&s));

    (auth_url, token_url, api_base_url, revoke_url, scopes)
}

/// Parse a scope list separated by commas or whitespace (Strava joins its
/// scopes with commas, WHOOP with spaces)
#[must_use]
pub fn parse_scopes(scopes_str: &str) -> Vec<String> {
    scopes_str
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Get environment variable or default value
fn env_var_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_party_web_origins_are_the_frontend_then_the_issuer_reduced_to_origins() {
        assert_eq!(
            first_party_web_origins(
                Some(" https://app.dravr.ai/some/path?x=1 "),
                "https://api.dravr.ai/"
            ),
            vec!["https://app.dravr.ai", "https://api.dravr.ai"]
        );
    }

    #[test]
    fn first_party_web_origins_drop_duplicates_and_what_names_no_origin() {
        // One origin serving both the app and the API is listed once.
        assert_eq!(
            first_party_web_origins(Some("http://localhost:8081/"), "http://localhost:8081"),
            vec!["http://localhost:8081"]
        );
        // An unparsable FRONTEND_URL, or an opaque-origin one, adds nothing.
        assert_eq!(
            first_party_web_origins(Some("not a url"), "https://api.dravr.ai"),
            vec!["https://api.dravr.ai"]
        );
        assert_eq!(
            first_party_web_origins(Some("dravr://auth/callback"), "https://api.dravr.ai"),
            vec!["https://api.dravr.ai"]
        );
    }
}
