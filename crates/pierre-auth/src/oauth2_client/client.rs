// ABOUTME: OAuth2 client implementation for fitness platform authentication
// ABOUTME: Generic OAuth2 client supporting multiple fitness platform providers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::http::oauth_client;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::constants::network_config::OAUTH_CODE_VERIFIER_LENGTH;
use pierre_core::constants::time::DEFAULT_TOKEN_EXPIRY_SECONDS;
use pierre_core::errors::{AppError, AppResult};
use rand::Rng;
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use url::Url;
use urlencoding::encode;

/// OAuth 2.0 client configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Config {
    /// OAuth client ID from provider
    pub client_id: String,
    /// OAuth client secret from provider; Debug prints a redaction and
    /// serialization omits it
    #[serde(skip_serializing)]
    pub client_secret: SecretString,
    /// Authorization endpoint URL
    pub auth_url: String,
    /// Token endpoint URL
    pub token_url: String,
    /// Redirect URI for OAuth callbacks
    pub redirect_uri: String,
    /// OAuth scopes to request
    pub scopes: Vec<String>,
    /// Whether to use PKCE for enhanced security
    pub use_pkce: bool,
}

impl OAuth2Config {
    /// The provider authorization URL this configuration sends an athlete to
    ///
    /// The one builder of a provider authorize URL. The parameters go out in
    /// a fixed order: `client_id`, `response_type=code`, `redirect_uri`,
    /// `state`, then `scope`, the `PKCE` challenge and `additional_params`
    /// (a provider's own, such as Strava's `approval_prompt=force`). Every
    /// value is percent-encoded except `client_id`, which goes out as the
    /// provider issued it.
    ///
    /// `scope` is left out when the configuration requests none: a provider
    /// whose scope is fixed server-side (Garmin) defines no `scope`
    /// parameter, and an empty `scope=` is not the same as omitting it. The
    /// challenge of `pkce` is sent only when the configuration uses `PKCE`.
    ///
    /// # Errors
    ///
    /// Returns an error if the authorization endpoint is not a URL
    pub fn authorization_url(
        &self,
        state: &str,
        pkce: &PkceParams,
        additional_params: &[(&str, &str)],
    ) -> AppResult<String> {
        Url::parse(&self.auth_url)
            .map_err(|e| AppError::invalid_input(format!("Invalid auth URL: {e}")))?;

        let mut url = format!(
            "{}?client_id={}&response_type=code&redirect_uri={}&state={}",
            self.auth_url,
            self.client_id,
            encode(&self.redirect_uri),
            encode(state)
        );

        let scope = self.scopes.join(" ");
        if !scope.is_empty() {
            url.push_str("&scope=");
            url.push_str(&encode(&scope));
        }

        if self.use_pkce {
            url.push_str("&code_challenge=");
            url.push_str(&encode(&pkce.code_challenge));
            url.push_str("&code_challenge_method=");
            url.push_str(&encode(&pkce.code_challenge_method));
        }

        for (key, value) in additional_params {
            url.push('&');
            url.push_str(&encode(key));
            url.push('=');
            url.push_str(&encode(value));
        }

        Ok(url)
    }
}

/// `PKCE` (Proof Key for Code Exchange) parameters for enhanced `OAuth2` security
#[derive(Clone)]
pub struct PkceParams {
    /// Randomly generated code verifier (43-128 characters)
    pub code_verifier: String,
    /// SHA256 hash of code verifier, base64url encoded
    pub code_challenge: String,
    /// Challenge method (always "S256" for SHA256)
    pub code_challenge_method: String,
}

impl fmt::Debug for PkceParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PkceParams")
            .field("code_verifier", &"[REDACTED]")
            .field("code_challenge", &self.code_challenge)
            .field("code_challenge_method", &self.code_challenge_method)
            .finish()
    }
}

impl PkceParams {
    /// Generate `PKCE` parameters with `S256` challenge method
    #[must_use]
    pub fn generate() -> Self {
        // Generate a cryptographically secure random code verifier (43-128 characters)
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
        let mut rng = rand::rng();
        let code_verifier: String = (0..OAUTH_CODE_VERIFIER_LENGTH)
            .map(|_| CHARS[rng.random_range(0..CHARS.len())] as char)
            .collect();

        // Create S256 code challenge
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let hash = hasher.finalize();
        let code_challenge = URL_SAFE_NO_PAD.encode(hash);

        Self {
            code_verifier,
            code_challenge,
            code_challenge_method: "S256".into(),
        }
    }
}

/// OAuth 2.0 access token with expiration and refresh capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Token {
    /// The access token string
    #[serde(serialize_with = "pierre_core::secret::serialize_exposed")]
    pub access_token: SecretString,
    /// Token type (usually "Bearer")
    pub token_type: String,
    /// Expiration timestamp (UTC)
    pub expires_at: Option<Timestamp>,
    /// Optional refresh token for getting new access tokens
    #[serde(serialize_with = "pierre_core::secret::serialize_exposed_option")]
    pub refresh_token: Option<SecretString>,
    /// Granted OAuth scopes
    pub scope: Option<String>,
    /// Provider-side user identifier captured from the token-exchange response
    /// (Strava `athlete.id`). `None` for providers that do not return an owner
    /// id and for refresh responses. Used to map provider push
    /// events (e.g. Strava webhooks) back to the single owning user.
    pub provider_user_id: Option<String>,
}

impl OAuth2Token {
    /// Check if the token is expired
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .is_some_and(|expires_at| expires_at <= Timestamp::now())
    }
}

/// OAuth 2.0 client for fitness platform authentication
#[derive(Debug)]
pub struct OAuth2Client {
    config: OAuth2Config,
    client: reqwest::Client,
}

impl OAuth2Client {
    /// Create a new `OAuth2` client with the given configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be built.
    pub fn new(config: OAuth2Config) -> AppResult<Self> {
        let client = oauth_client()?;
        Ok(Self { config, client })
    }

    /// Get the `OAuth2` configuration
    #[must_use]
    pub const fn config(&self) -> &OAuth2Config {
        &self.config
    }

    /// Get the HTTP client for making OAuth requests
    #[must_use]
    pub const fn http_client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Exchange authorization code for tokens
    ///
    /// # Errors
    ///
    /// Returns an error if the token exchange request fails or response is invalid
    pub async fn exchange_code(&self, code: &str) -> AppResult<OAuth2Token> {
        let params = [
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.expose_secret()),
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", self.config.redirect_uri.as_str()),
        ];

        let http_response = self
            .client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| {
                AppError::external_service("oauth", format!("Failed to send token request: {e}"))
            })?;

        let status = http_response.status();
        let body = http_response.text().await.map_err(|e| {
            AppError::external_service("oauth", format!("Failed to read token response body: {e}"))
        })?;

        if !status.is_success() {
            return Err(code_exchange_refusal(status, &body));
        }

        let response: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            AppError::external_service(
                "oauth",
                format!("Failed to parse token response ({} bytes): {e}", body.len()),
            )
        })?;

        Ok(Self::token_from_response(response))
    }

    /// Exchange authorization code with `PKCE` support
    ///
    /// # Errors
    ///
    /// Returns an error if the token exchange request fails or response is invalid
    pub async fn exchange_code_with_pkce(
        &self,
        code: &str,
        pkce: &PkceParams,
    ) -> AppResult<OAuth2Token> {
        let mut params = vec![
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.expose_secret()),
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", self.config.redirect_uri.as_str()),
        ];

        if self.config.use_pkce {
            params.push(("code_verifier", &pkce.code_verifier));
        }

        let http_response = self
            .client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| {
                AppError::external_service("oauth", format!("Failed to send token request: {e}"))
            })?;

        let status = http_response.status();
        let body = http_response.text().await.map_err(|e| {
            AppError::external_service("oauth", format!("Failed to read token response body: {e}"))
        })?;

        if !status.is_success() {
            return Err(code_exchange_refusal(status, &body));
        }

        let response: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            AppError::external_service(
                "oauth",
                format!("Failed to parse token response ({} bytes): {e}", body.len()),
            )
        })?;

        Ok(Self::token_from_response(response))
    }

    #[must_use]
    fn token_from_response(response: TokenResponse) -> OAuth2Token {
        let expires_at = response.expires_in.map(|seconds| {
            Timestamp::now()
                + Elapsed::seconds(i64::try_from(seconds).unwrap_or(DEFAULT_TOKEN_EXPIRY_SECONDS))
        });

        // Capture the provider-side owner id when the response carries one:
        // Strava returns a nested `athlete.id` as a number, Intervals.icu as a
        // string. Other providers omit it, leaving this `None`.
        let provider_user_id = response.athlete.map(|athlete| athlete.id.into_string());

        OAuth2Token {
            access_token: response.access_token,
            token_type: response.token_type,
            expires_at,
            refresh_token: response.refresh_token,
            scope: response.scope,
            provider_user_id,
        }
    }
}

/// OAuth 2.0 token response from provider
#[derive(Debug, Deserialize)]
struct TokenResponse {
    /// The access token issued by the authorization server
    access_token: SecretString,
    /// The type of token (usually "Bearer")
    token_type: String,
    /// Token lifetime in seconds
    expires_in: Option<u64>,
    /// Refresh token for obtaining new access tokens
    refresh_token: Option<SecretString>,
    /// Space-separated list of granted scopes
    scope: Option<String>,
    /// Strava and Intervals.icu return the authenticated athlete inline in the
    /// token response; only its id is needed to map it back to a user.
    athlete: Option<AthleteId>,
}

/// Minimal projection of a provider's inline athlete object in the token
/// response. Only the id is captured.
#[derive(Debug, Deserialize)]
struct AthleteId {
    /// Provider-side athlete identifier.
    id: ProviderAthleteId,
}

/// An inline athlete id, which Strava sends as a number and Intervals.icu as a
/// string. A number read as the only shape would fail the whole token response
/// on the other.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ProviderAthleteId {
    /// Strava's numeric athlete id.
    Number(i64),
    /// Intervals.icu's athlete id.
    Text(String),
}

impl ProviderAthleteId {
    fn into_string(self) -> String {
        match self {
            Self::Number(id) => id.to_string(),
            Self::Text(id) => id,
        }
    }
}

/// The error for a token endpoint that refused an authorization code exchange.
///
/// A standard `invalid_grant` answer (RFC 6749 Section 5.2) is the provider
/// refusing the code it was handed, one that is expired, already used or
/// forged: the caller's to fix by authorizing again, and anyone can send one.
/// Every other refusal is the provider refusing this server (its client
/// credentials, its redirect URI) or failing outright, and stays an upstream
/// fault, including a non-standard body that names no error.
fn code_exchange_refusal(status: StatusCode, body: &str) -> AppError {
    let refused_grant = serde_json::from_str::<Value>(body)
        .is_ok_and(|v| v.get("error").and_then(Value::as_str) == Some("invalid_grant"));
    if refused_grant {
        AppError::auth_invalid(format!(
            "OAuth token exchange refused the authorization code: HTTP {status}: {body}"
        ))
    } else {
        AppError::external_service(
            "oauth",
            format!("Token endpoint returned HTTP {status}: {body}"),
        )
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::{OAuth2Config, PkceParams};

    fn config(scope: &str, use_pkce: bool) -> OAuth2Config {
        OAuth2Config {
            client_id: "12345".to_owned(),
            client_secret: SecretString::from("secret"),
            auth_url: "https://provider.test/oauth/authorize".to_owned(),
            token_url: "https://provider.test/oauth/token".to_owned(),
            redirect_uri: "https://app.test/api/oauth/callback/strava".to_owned(),
            scopes: vec![scope.to_owned()],
            use_pkce,
        }
    }

    fn pkce() -> PkceParams {
        PkceParams {
            code_verifier: "verifier".to_owned(),
            code_challenge: "chal-_lenge".to_owned(),
            code_challenge_method: "S256".to_owned(),
        }
    }

    const BASE: &str = "https://provider.test/oauth/authorize?client_id=12345&response_type=code\
        &redirect_uri=https%3A%2F%2Fapp.test%2Fapi%2Foauth%2Fcallback%2Fstrava";

    #[test]
    fn a_comma_separated_scope_and_a_provider_parameter_follow_the_challenge() {
        let url = config("activity:read_all,profile:read_all", true)
            .authorization_url("user:nonce", &pkce(), &[("approval_prompt", "force")])
            .unwrap();

        assert_eq!(
            url,
            format!(
                "{BASE}&state=user%3Anonce&scope=activity%3Aread_all%2Cprofile%3Aread_all\
                 &code_challenge=chal-_lenge&code_challenge_method=S256&approval_prompt=force"
            )
        );
    }

    #[test]
    fn a_space_separated_scope_is_percent_encoded_not_plus_encoded() {
        let url = config("read:recovery offline", true)
            .authorization_url("s", &pkce(), &[])
            .unwrap();

        assert_eq!(
            url,
            format!(
                "{BASE}&state=s&scope=read%3Arecovery%20offline\
                 &code_challenge=chal-_lenge&code_challenge_method=S256"
            )
        );
    }

    #[test]
    fn a_configuration_without_pkce_sends_no_challenge() {
        let url = config("ACTIVITY:READ,WELLNESS:READ", false)
            .authorization_url("s", &pkce(), &[])
            .unwrap();

        assert_eq!(
            url,
            format!("{BASE}&state=s&scope=ACTIVITY%3AREAD%2CWELLNESS%3AREAD")
        );
    }

    #[test]
    fn no_scope_to_request_leaves_the_parameter_out() {
        let url = config("", true)
            .authorization_url("s", &pkce(), &[])
            .unwrap();

        assert_eq!(
            url,
            format!("{BASE}&state=s&code_challenge=chal-_lenge&code_challenge_method=S256")
        );
    }

    #[test]
    fn a_provider_parameter_is_percent_encoded() {
        let url = config("", false)
            .authorization_url("s", &pkce(), &[("access type", "off line&x")])
            .unwrap();

        assert_eq!(url, format!("{BASE}&state=s&access%20type=off%20line%26x"));
    }

    #[test]
    fn an_endpoint_that_is_not_a_url_is_refused() {
        let mut config = config("read", false);
        config.auth_url = "not-a-valid-url".to_owned();

        assert!(config.authorization_url("s", &pkce(), &[]).is_err());
    }
}
