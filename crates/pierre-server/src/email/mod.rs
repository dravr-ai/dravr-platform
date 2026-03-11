// ABOUTME: Email sending via the Resend API
// ABOUTME: Provides transactional email delivery for password reset codes and notifications
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

mod templates;

use crate::errors::{AppError, AppResult};
use crate::utils::http_client::shared_client;
use reqwest::Client;
use serde::Serialize;
use tracing::{info, warn};

/// Resend API base URL
const RESEND_API_URL: &str = "https://api.resend.com/emails";

/// Email payload for the Resend API
#[derive(Serialize)]
struct ResendEmailPayload {
    from: String,
    to: Vec<String>,
    subject: String,
    html: String,
}

/// Email service backed by the Resend transactional email API
pub struct ResendEmailService {
    /// HTTP client for API requests
    client: &'static Client,
    /// Resend API key
    api_key: String,
    /// Sender email address (e.g., "Pierre <noreply@pierre.dev>")
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

        let response = self
            .client
            .post(RESEND_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("Failed to send email via Resend: {e}")))?;

        if response.status().is_success() {
            info!(
                to = to,
                subject = subject,
                "Email sent successfully via Resend"
            );
            Ok(())
        } else {
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
            Err(AppError::internal(format!(
                "Resend API error (HTTP {status}): {body}"
            )))
        }
    }

    /// Send a password reset code email
    ///
    /// # Errors
    ///
    /// Returns an error if email delivery fails.
    pub async fn send_password_reset_code(&self, to: &str, code: &str) -> AppResult<()> {
        let html = templates::password_reset_code_html(code);
        self.send_email(to, "Your password reset code", &html).await
    }
}
