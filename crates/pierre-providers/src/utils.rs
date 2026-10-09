// ABOUTME: Shared utilities for fitness provider implementations
// ABOUTME: Type conversions, retry logic, token refresh, and common patterns
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::constants::oauth_providers;
use crate::errors::{AppError, AppResult, ErrorCode};
use crate::http_client::SharedHttpClient;
use crate::request_budget::{self, RequestBudget};
use chrono::{TimeZone, Utc};
use reqwest::StatusCode;
use serde::Deserialize;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use super::core::{CredentialKind, OAuth2Credentials};
use super::errors::provider::ProviderError;
use super::spi::{OAuthRefresh, RefreshClientAuth};

/// Configuration for retry behavior
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial backoff delay in milliseconds
    pub initial_backoff_ms: u64,
    /// HTTP status codes that should trigger retries
    pub retryable_status_codes: Vec<StatusCode>,
    /// Estimated block duration for user-facing error messages (seconds)
    pub estimated_block_duration_secs: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 1000,
            retryable_status_codes: vec![StatusCode::TOO_MANY_REQUESTS],
            estimated_block_duration_secs: 3600, // 1 hour
        }
    }
}

/// Type conversion utilities for safe float-to-integer conversions
pub mod conversions {
    use num_traits::ToPrimitive;

    /// Safely convert f64 to u64, clamping to valid range
    /// Used for duration values from APIs that return floats
    #[must_use]
    pub fn f64_to_u64(value: f64) -> u64 {
        if !value.is_finite() {
            return 0;
        }
        let t = value.trunc();
        if t.is_sign_negative() {
            return 0;
        }
        t.to_u64().unwrap_or(u64::MAX)
    }

    /// Safely convert f32 to u32, clamping to valid range
    /// Used for metrics like heart rate, power, cadence
    #[must_use]
    pub fn f32_to_u32(value: f32) -> u32 {
        if !value.is_finite() {
            return 0;
        }
        let t = value.trunc();
        if t.is_sign_negative() {
            return 0;
        }
        t.to_u32().unwrap_or(u32::MAX)
    }

    /// Safely convert f64 to u32, clamping to valid range
    /// Used for calorie values and other metrics
    #[must_use]
    pub fn f64_to_u32(value: f64) -> u32 {
        if !value.is_finite() {
            return 0;
        }
        let t = value.trunc();
        if t.is_sign_negative() {
            return 0;
        }
        t.to_u32().unwrap_or(u32::MAX)
    }

    /// Convert f64 to f32 for fields whose dynamic range fits f32.
    ///
    /// Used for ambient temperature in Celsius and other values where the
    /// source API's precision (typically 1 decimal) is well within f32.
    /// `NaN` collapses to 0.0; oversized values saturate to f32 infinities
    /// rather than panicking. Precision loss is acceptable for display.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn f64_to_f32(value: f64) -> f32 {
        if value.is_nan() {
            return 0.0;
        }
        value as f32
    }
}

/// Result of checking if a response should be retried
enum RetryDecision {
    /// Continue with retry after backoff
    Retry { backoff_ms: u64 },
    /// Max retries reached, return error
    MaxRetriesExceeded,
    /// Not a retryable status, continue processing
    NotRetryable,
}

/// Check if a response status should trigger a retry
fn check_retry_status(
    status: StatusCode,
    attempt: u32,
    retry_config: &RetryConfig,
    provider_name: &str,
) -> RetryDecision {
    if !retry_config.retryable_status_codes.contains(&status) {
        return RetryDecision::NotRetryable;
    }

    let current_attempt = attempt + 1;
    if current_attempt >= retry_config.max_retries {
        warn!(
            "{provider_name} API rate limit exceeded - max retries ({}) reached",
            retry_config.max_retries
        );
        return RetryDecision::MaxRetriesExceeded;
    }

    let backoff_ms = retry_config.initial_backoff_ms * 2_u64.pow(current_attempt - 1);
    let status_code = status.as_u16();
    warn!(
        "{provider_name} API rate limit hit ({status_code}) - retry {current_attempt}/{} after {backoff_ms}ms backoff",
        retry_config.max_retries
    );

    RetryDecision::Retry { backoff_ms }
}

/// The error a provider's own `429` is once retries are spent: the same
/// [`ErrorCode::ExternalRateLimited`] a credential's request budget refuses
/// with, carrying the wait, so a caller tells "the provider is throttling"
/// apart from a failure either way.
fn rate_limit_error(
    status: StatusCode,
    provider_name: &str,
    retry_config: &RetryConfig,
) -> AppError {
    let minutes = retry_config.estimated_block_duration_secs / 60;
    let status_code = status.as_u16();
    let err = ProviderError::RateLimitExceeded {
        provider: provider_name.to_owned(),
        retry_after_secs: retry_config.estimated_block_duration_secs,
        limit_type: format!(
            "API rate limit ({status_code}) - max retries reached - wait ~{minutes} minutes"
        ),
    };
    AppError::new(ErrorCode::ExternalRateLimited, err.to_string())
        .with_retry_after(retry_config.estimated_block_duration_secs)
}

/// Map a provider's `401` onto the structured re-authentication error.
///
/// A provider rejecting the access token is the only authority on whether that
/// credential still works. The stored `expires_at` is a belief, and it can be
/// wrong in one direction that matters: Strava (and others) invalidate tokens on
/// **user revocation**, which does not move the expiry. A revoked-but-unexpired
/// credential therefore looks healthy forever, so nothing refreshes and every
/// call fails — the athlete's agent silently has no data.
///
/// Returning [`AppError::provider_auth_required`] instead of a generic external
/// error puts the failure on the path that already exists for it: the chat
/// pipeline reads the provider slug out of `details` and mints a hosted-login
/// link rather than letting the model rephrase a shrug.
///
/// Returns `None` for any other status so callers keep their own handling.
#[must_use]
pub fn auth_error_for_status(status: StatusCode, provider_slug: &str) -> Option<AppError> {
    (status == StatusCode::UNAUTHORIZED).then(|| AppError::provider_auth_required(provider_slug))
}

/// Create an API error for non-success responses.
///
/// Maps `401` onto the structured re-authentication error first — see
/// [`auth_error_for_status`] — so every provider routing through here gets the
/// reconnect path rather than a generic failure.
pub fn api_error(status: StatusCode, text: &str, provider_name: &str) -> AppError {
    error!(
        "{provider_name} API request failed - status: {status}, body_length: {} bytes",
        text.len()
    );
    debug!("{provider_name} API error response body: {text}");
    if let Some(auth) = auth_error_for_status(status, provider_name) {
        return auth;
    }
    let err = ProviderError::ApiError {
        provider: provider_name.to_owned(),
        status_code: status.as_u16(),
        message: format!("{provider_name} API request failed with status {status}"),
        retryable: status.as_u16() >= 500,
    };
    AppError::external_service(provider_name, err.to_string())
}

/// Admit one GET to `url` against `budget`, then send it with `access_token`
/// as the bearer.
async fn send_admitted(
    client: &SharedHttpClient,
    url: &str,
    access_token: &str,
    provider_name: &str,
    budget: Option<&RequestBudget>,
) -> AppResult<reqwest::Response> {
    request_budget::admit(budget, provider_name).await?;
    client
        .get(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .map_err(|e| {
            AppError::external_service(provider_name, format!("Failed to send request: {e}"))
        })
}

/// Make an authenticated HTTP GET request with retry logic
///
/// # Errors
///
/// A `401` is mapped by [`auth_error_for_status`] before anything else reads
/// the response, so every provider routing through here reaches the reconnect
/// path and no hook can shadow it, whatever the vendor's 401 body says.
/// `vendor_error` then reads any other non-success body before the generic
/// mapping does, so a provider keeps its own error vocabulary (Strava's 404 →
/// `NotFound`, Whoop's 404 → `NoDataAvailable`). Return `None` — or pass
/// [`no_vendor_error`] — to take the generic mapping.
///
/// Each attempt, retries included, is first admitted against `budget`, the
/// credential's request budget; a refusal ends the request with
/// [`ErrorCode::ExternalRateLimited`](crate::errors::ErrorCode) and nothing
/// more is sent.
///
/// # Errors
///
/// Returns an error if:
/// - The credential's request budget refuses the request
/// - No access token is available
/// - All retry attempts are exhausted
/// - Network request fails
/// - Response parsing fails
pub async fn api_request_with_retry<T, F>(
    client: &SharedHttpClient,
    url: &str,
    access_token: &str,
    provider_name: &str,
    retry_config: &RetryConfig,
    budget: Option<&RequestBudget>,
    vendor_error: F,
) -> AppResult<T>
where
    T: for<'de> Deserialize<'de>,
    F: Fn(StatusCode, &str) -> Option<AppError>,
{
    info!("Starting {provider_name} API request to: {url}");

    let mut attempt = 0;
    loop {
        // Every attempt is a request the provider counts, retries included.
        let response = send_admitted(client, url, access_token, provider_name, budget).await?;

        let status = response.status();
        info!("Received HTTP response with status: {status}");

        match check_retry_status(status, attempt, retry_config, provider_name) {
            RetryDecision::Retry { backoff_ms } => {
                attempt += 1;
                sleep(Duration::from_millis(backoff_ms)).await;
                continue;
            }
            RetryDecision::MaxRetriesExceeded => {
                return Err(rate_limit_error(status, provider_name, retry_config));
            }
            RetryDecision::NotRetryable => {}
        }

        if !status.is_success() {
            // A rejected credential is the one failure whose handling no
            // provider owns: the athlete has to reconnect, whatever the body
            // says about why.
            if let Some(auth) = auth_error_for_status(status, provider_name) {
                return Err(auth);
            }
            let text = response.text().await.unwrap_or_default();
            // The vendor's own reading of the failure next: a body that names
            // a missing resource or a scope gap carries more than the status
            // code does, and only the provider can decode it.
            if let Some(err) = vendor_error(status, &text) {
                return Err(err);
            }
            return Err(api_error(status, &text, provider_name));
        }

        info!("Parsing JSON response from {provider_name} API");
        return response.json().await.map_err(|e| {
            error!("Failed to parse JSON response: {e}");
            AppError::external_service(provider_name, format!("Failed to parse API response: {e}"))
        });
    }
}

/// The generic error mapping, for a provider whose failures carry nothing the
/// status code does not already say.
#[must_use]
pub const fn no_vendor_error(_status: StatusCode, _body: &str) -> Option<AppError> {
    None
}

/// The form field a WHOOP refresh adds.
///
/// WHOOP rotates refresh tokens and returns a new one only when
/// `scope=offline` is sent on the refresh; without it the single-use refresh
/// token is consumed but not replaced, so the next refresh fails with HTTP 400
/// `invalid_request`. Written once: the WHOOP descriptor declares it and
/// [`RefreshRequest::whoop`] sends it.
pub const WHOOP_REFRESH_EXTRA_FORM: &[(&str, &str)] = &[("scope", "offline")];

/// One `OAuth2` refresh, with the vendor differences the standard flow leaves open.
#[derive(Debug, Clone, Copy)]
pub struct RefreshRequest<'a> {
    /// The provider's token endpoint.
    pub token_url: &'a str,
    /// Registered client identifier.
    pub client_id: &'a str,
    /// Registered client secret.
    pub client_secret: &'a str,
    /// The stored refresh token being exchanged.
    pub refresh_token: &'a str,
    /// Provider slug, used for logs and for the error's `provider` field.
    pub provider_name: &'a str,
    /// Fields the vendor requires beyond the four standard ones.
    pub extra_form: &'a [(&'a str, &'a str)],
    /// Where the client credentials travel: the form body, or a Basic header.
    pub client_auth: RefreshClientAuth,
}

impl<'a> RefreshRequest<'a> {
    /// A refresh with the client credentials in the form body and no vendor
    /// field: the `OAuth2` default, and what Strava expects.
    #[must_use]
    pub const fn form_fields(
        provider_name: &'a str,
        token_url: &'a str,
        client_id: &'a str,
        client_secret: &'a str,
        refresh_token: &'a str,
    ) -> Self {
        Self {
            token_url,
            client_id,
            client_secret,
            refresh_token,
            provider_name,
            extra_form: &[],
            client_auth: RefreshClientAuth::RequestBody,
        }
    }

    /// The refresh a provider's descriptor declares
    /// ([`ProviderDescriptor::oauth_refresh`](super::spi::ProviderDescriptor::oauth_refresh)):
    /// its client authentication and its extra form fields.
    #[must_use]
    pub const fn described(
        provider_name: &'a str,
        token_url: &'a str,
        client_id: &'a str,
        client_secret: &'a str,
        refresh_token: &'a str,
        refresh: OAuthRefresh,
    ) -> Self {
        Self {
            token_url,
            client_id,
            client_secret,
            refresh_token,
            provider_name,
            extra_form: refresh.extra_form,
            client_auth: refresh.client_auth,
        }
    }

    /// A WHOOP refresh, carrying [`WHOOP_REFRESH_EXTRA_FORM`].
    #[must_use]
    pub const fn whoop(
        token_url: &'a str,
        client_id: &'a str,
        client_secret: &'a str,
        refresh_token: &'a str,
    ) -> Self {
        Self {
            token_url,
            client_id,
            client_secret,
            refresh_token,
            provider_name: oauth_providers::WHOOP,
            extra_form: WHOOP_REFRESH_EXTRA_FORM,
            client_auth: RefreshClientAuth::RequestBody,
        }
    }
}

/// Standard token refresh response structure
#[derive(Debug, Deserialize)]
pub struct TokenRefreshResponse {
    /// New access token from the OAuth provider
    pub access_token: String,
    /// Optional new refresh token (if rotated by provider)
    pub refresh_token: Option<String>,
    /// Token expiration time in seconds from now
    #[serde(default)]
    pub expires_in: Option<i64>,
    /// Token expiration as Unix timestamp
    #[serde(default)]
    pub expires_at: Option<i64>,
}

/// Refresh `OAuth2` access token using refresh token
///
/// The client credentials travel where `client_auth` says: as fields of the
/// form body, the `OAuth2` default Strava and WHOOP expect, or in a Basic
/// `Authorization` header with neither in the body. `extra_form` carries the
/// fields a vendor requires beyond the standard ones
/// ([`WHOOP_REFRESH_EXTRA_FORM`]).
///
/// # Errors
///
/// Returns an error if:
/// - HTTP request fails
/// - Token endpoint returns error, whose text names the HTTP status and the
///   vendor's error body (`Token endpoint returned HTTP 400 Bad Request: {...}`)
/// - Response parsing fails, whose text leaves the body out: a success body is
///   the token pair
pub async fn refresh_oauth_token(
    client: &SharedHttpClient,
    request: &RefreshRequest<'_>,
) -> AppResult<OAuth2Credentials> {
    let RefreshRequest {
        token_url,
        client_id,
        client_secret,
        refresh_token,
        provider_name,
        extra_form,
        client_auth,
    } = *request;
    info!("Refreshing {provider_name} access token");

    let mut post = client.post(token_url);
    let mut params: Vec<(&str, &str)> = Vec::with_capacity(4 + extra_form.len());
    match client_auth {
        RefreshClientAuth::RequestBody => {
            params.push(("client_id", client_id));
            params.push(("client_secret", client_secret));
        }
        RefreshClientAuth::BasicHeader => {
            post = post.basic_auth(client_id, Some(client_secret));
        }
    }
    params.push(("grant_type", "refresh_token"));
    params.push(("refresh_token", refresh_token));
    params.extend_from_slice(extra_form);

    let response = post.form(&params).send().await.map_err(|e| {
        AppError::external_service(
            provider_name,
            format!("Failed to send token refresh request: {e}"),
        )
    })?;

    // A refusal carries the status and the vendor's error body: what the
    // grant's standing is read from (a dead refresh token, a rejected client,
    // a rate limit). The body of a refusal holds no token.
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::external_service(
            provider_name,
            format!("Token endpoint returned HTTP {status}: {body}"),
        ));
    }

    let token_response: TokenRefreshResponse = response.json().await.map_err(|e| {
        AppError::external_service(
            provider_name,
            format!("Failed to parse token refresh response: {e}"),
        )
    })?;

    // Calculate expiry time
    let expires_at = token_response
        .expires_at
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .or_else(|| {
            token_response
                .expires_in
                .map(|secs| Utc::now() + chrono::Duration::seconds(secs))
        });

    Ok(OAuth2Credentials {
        client_id: client_id.to_owned(),
        client_secret: client_secret.to_owned(),
        access_token: Some(token_response.access_token),
        refresh_token: token_response.refresh_token,
        expires_at,
        scopes: vec![], // Preserve original scopes in caller
        kind: CredentialKind::OAuthBearer,
        request_budget: None,
    })
}

/// Check if credentials are authenticated (has valid access token)
#[must_use]
pub fn is_authenticated(credentials: &Option<OAuth2Credentials>) -> bool {
    credentials.as_ref().is_some_and(|creds| {
        creds.access_token.is_some()
            && creds
                .expires_at
                .is_none_or(|expires_at| Utc::now() < expires_at)
    })
}
