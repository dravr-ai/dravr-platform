// ABOUTME: Email sending via the Resend API
// ABOUTME: Provides transactional email delivery for password reset codes and notifications
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Email
//!
//! Resend-backed transactional email service for the Pierre platform. Exposes
//! [`ResendEmailService`] for sending password-reset codes, lifecycle notifications,
//! and other transactional emails. Templates live in the [`templates`] submodule.

/// HTML email templates for transactional and lifecycle emails
pub mod templates;

use std::time::Duration;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::http_client::api_client as shared_client;
use pierre_core::http_client::SharedHttpClient;
use reqwest::header::HeaderMap;
use reqwest::{Response, StatusCode};
use serde::Serialize;
use tokio::time::sleep;
use tracing::{info, warn};

/// Resend API base URL
const RESEND_API_URL: &str = "https://api.resend.com/emails";

/// Maximum number of automatic retries after a `429 Too Many Requests`.
const MAX_RETRIES: u32 = 3;

/// Upper bound on how long a caller is blocked waiting for a rate-limit reset.
/// Resend's per-second limit resets within `1s`; a reset window longer than
/// this signals a daily/monthly cap, where blocking the request path (e.g. the
/// inline password-reset handler) is worse than dropping the send.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Backoff used when a `429` arrives without a usable reset header.
const DEFAULT_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Floor applied to every retry so a `0`-second reset cannot become a tight loop.
const RETRY_DELAY_FLOOR: Duration = Duration::from_millis(200);

/// Parse Resend's rate-limit reset hint into a backoff [`Duration`].
///
/// Prefers the standard `Retry-After` header (delta-seconds), falling back to
/// Resend's `RateLimit-Reset` (whole seconds until the window resets). Returns
/// `None` when neither header carries a parseable non-negative integer.
#[must_use]
pub fn parse_retry_delay(
    retry_after: Option<&str>,
    ratelimit_reset: Option<&str>,
) -> Option<Duration> {
    let secs = |v: Option<&str>| v.and_then(|s| s.trim().parse::<u64>().ok());
    secs(retry_after)
        .or_else(|| secs(ratelimit_reset))
        .map(Duration::from_secs)
}

/// Decide whether a `429` should be retried, and how long to wait first.
///
/// Returns `Some(delay)` to sleep then retry, or `None` to give up. Gives up
/// once `attempt` reaches `MAX_RETRIES`, or when the reset window exceeds
/// `MAX_RETRY_DELAY` (a long window means a daily/monthly cap, not a momentary
/// burst). A missing `hint` falls back to `DEFAULT_RETRY_DELAY`; every retry
/// waits at least `RETRY_DELAY_FLOOR`.
#[must_use]
pub fn plan_retry(attempt: u32, hint: Option<Duration>) -> Option<Duration> {
    if attempt >= MAX_RETRIES {
        return None;
    }
    let delay = hint.unwrap_or(DEFAULT_RETRY_DELAY);
    if delay > MAX_RETRY_DELAY {
        return None;
    }
    Some(delay.max(RETRY_DELAY_FLOOR))
}

/// Extract a retry backoff from a Resend response's rate-limit headers.
fn retry_delay_from_headers(headers: &HeaderMap) -> Option<Duration> {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    parse_retry_delay(get("retry-after"), get("ratelimit-reset"))
}

/// Build a structured error from a non-success Resend response, logging the body.
async fn send_error_from_response(to: &str, response: Response) -> AppError {
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "no body".to_owned());
    warn!(
        to = to,
        status = %status,
        body = body,
        "Resend API returned non-success status"
    );
    AppError::internal(format!("Resend API error (HTTP {status}): {body}"))
}

/// Email payload for the Resend API
#[derive(Serialize)]
struct ResendEmailPayload {
    from: String,
    to: Vec<String>,
    subject: String,
    html: String,
}

/// Subject line of the password-reset code email.
pub const SUBJECT_PASSWORD_RESET: &str = "Your password reset code";
/// Subject line of the "registration received, pending approval" email.
pub const SUBJECT_REGISTRATION_PENDING: &str = "Welcome to Dravr — account pending review";
/// Subject line of the account-approved email.
pub const SUBJECT_REGISTRATION_APPROVED: &str = "Your Dravr account is approved";
/// Subject line of the channel-linking verification code email.
///
/// Named constants because a subject is user-visible brand surface: as inline
/// literals they escaped the rebrand pass, and this one shipped the internal
/// codename to every user who linked a chat channel until 2026-08.
pub const SUBJECT_CHANNEL_LINKING_CODE: &str = "Your Dravr verification code";
/// Subject line for the post-registration address-confirmation email.
pub const SUBJECT_EMAIL_VERIFICATION: &str = "Confirm your email for Dravr";

/// Email service backed by the Resend transactional email API
pub struct ResendEmailService {
    /// HTTP client for API requests
    client: &'static SharedHttpClient,
    /// Resend API key
    api_key: String,
    /// Sender email address (e.g., "Dravr <no-reply@dravr.ai>")
    from_email: String,
}

impl ResendEmailService {
    /// Create a new Resend email service
    #[must_use]
    pub fn new(api_key: String, from_email: String) -> Self {
        Self {
            client: shared_client(),
            api_key,
            from_email,
        }
    }

    /// Send an email via the Resend API
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request to Resend fails or returns a non-success status.
    async fn send_email(&self, to: &str, subject: &str, html_body: &str) -> AppResult<()> {
        let payload = ResendEmailPayload {
            from: self.from_email.clone(),
            to: vec![to.to_owned()],
            subject: subject.to_owned(),
            html: html_body.to_owned(),
        };

        let mut attempt: u32 = 0;
        loop {
            let response = self.post_once(&payload).await?;
            let status = response.status();
            if status.is_success() {
                info!(
                    to = to,
                    subject = subject,
                    "Email sent successfully via Resend"
                );
                return Ok(());
            }

            // Honor Resend's rate-limit reset window: on a 429, wait out the
            // advertised reset (capped) and retry, rather than dropping the send.
            let retry = if status == StatusCode::TOO_MANY_REQUESTS {
                plan_retry(attempt, retry_delay_from_headers(response.headers()))
            } else {
                None
            };
            let Some(delay) = retry else {
                return Err(send_error_from_response(to, response).await);
            };

            attempt += 1;
            warn!(
                to = to,
                attempt = attempt,
                delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                "Resend rate limit (HTTP 429) — waiting for reset, then retrying"
            );
            sleep(delay).await;
        }
    }

    /// Perform a single POST of the payload to the Resend API.
    async fn post_once(&self, payload: &ResendEmailPayload) -> AppResult<Response> {
        self.client
            .post(RESEND_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(payload)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("Failed to send email via Resend: {e}")))
    }

    /// Send a password reset code email
    ///
    /// # Errors
    ///
    /// Returns an error if email delivery fails.
    pub async fn send_password_reset_code(&self, to: &str, code: &str) -> AppResult<()> {
        let html = templates::password_reset_code_html(code);
        self.send_email(to, SUBJECT_PASSWORD_RESET, &html).await
    }

    /// Send a "registration received, pending approval" email
    ///
    /// Delivered immediately after self-registration when the account lands
    /// in Pending status. Lets the user know that an admin will review the
    /// account and that a follow-up email will arrive on approval.
    ///
    /// # Errors
    ///
    /// Returns an error if email delivery fails.
    pub async fn send_registration_pending(
        &self,
        to: &str,
        display_name: Option<&str>,
    ) -> AppResult<()> {
        let html = templates::registration_pending_html(display_name);
        self.send_email(to, SUBJECT_REGISTRATION_PENDING, &html)
            .await
    }

    /// Send a "your account has been approved" email
    ///
    /// Delivered after an admin approves a pending registration, or after
    /// auto-approval during registration. When a `sign_in_url` is provided
    /// the email renders a call-to-action button; otherwise it falls back
    /// to a plain notice.
    ///
    /// # Errors
    ///
    /// Returns an error if email delivery fails.
    pub async fn send_registration_approved(
        &self,
        to: &str,
        display_name: Option<&str>,
        sign_in_url: Option<&str>,
    ) -> AppResult<()> {
        let html = templates::registration_approved_html(display_name, sign_in_url);
        self.send_email(to, SUBJECT_REGISTRATION_APPROVED, &html)
            .await
    }

    /// Send the post-registration address-confirmation email.
    ///
    /// `verify_url` carries a single-use `<selector>.<verifier>` token;
    /// `ttl_minutes` is rendered into the copy so the expiry is stated rather
    /// than discovered when the link stops working.
    ///
    /// # Errors
    ///
    /// Returns an error if email delivery fails.
    pub async fn send_email_verification(
        &self,
        to: &str,
        display_name: Option<&str>,
        verify_url: &str,
        ttl_minutes: i64,
    ) -> AppResult<()> {
        let html = templates::email_verification_html(display_name, verify_url, ttl_minutes);
        self.send_email(to, SUBJECT_EMAIL_VERIFICATION, &html).await
    }

    /// Send a channel linking verification code email
    ///
    /// # Errors
    ///
    /// Returns an error if email delivery fails.
    pub async fn send_channel_linking_code(
        &self,
        to: &str,
        code: &str,
        channel_name: &str,
    ) -> AppResult<()> {
        let html = templates::channel_linking_code_html(code, channel_name);
        self.send_email(to, SUBJECT_CHANNEL_LINKING_CODE, &html)
            .await
    }
}
