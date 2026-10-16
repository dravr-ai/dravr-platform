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

use dravr_tronc::notifications::{ResendBody, ResendClient, ResendEmail};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::http_client::api_inner_client;
use templates::CoachAccessOrigin;
use tracing::{debug, info, warn};

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

/// Subject line for the dravr.ai docs sign-in link.
pub const SUBJECT_WEBSITE_SIGN_IN: &str = "Your sign-in link for the Dravr docs";

/// Subject line for the operator-sent invitation.
pub const SUBJECT_INVITATION: &str = "You're invited to Dravr";

/// Subject line of the operator email announcing a coach-access request a
/// coach asked for.
pub const SUBJECT_COACH_ACCESS_REQUESTED: &str = "Dravr: a coach is asking for coach access";
/// Subject line of the operator email announcing a coach-access request filed
/// for a coach from the Intervals.icu roster of a key they linked.
pub const SUBJECT_COACH_ACCESS_FILED: &str =
    "Dravr: a coach-access request was filed from an Intervals.icu roster";

/// Email service backed by the Resend transactional email API.
///
/// Sends through dravr-tronc's [`ResendClient`], the one Resend send path the
/// process has: tronc's alert mailer shares the same `RESEND_API_KEY` and the
/// same `429` retry policy (wait out the advertised reset, bounded).
pub struct ResendEmailService {
    /// The Resend client, over the shared API connection pool
    client: ResendClient,
    /// Sender email address (e.g., "Dravr <no-reply@dravr.ai>")
    from_email: String,
}

impl ResendEmailService {
    /// Create a Resend email service sending as `from_email`.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when `api_key` is empty.
    pub fn new(api_key: String, from_email: String) -> AppResult<Self> {
        let client = ResendClient::new(api_inner_client().clone(), api_key)
            .map_err(|e| AppError::config(format!("Resend email service: {e}")))?;
        Ok(Self { client, from_email })
    }

    /// Send an email via the Resend API
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request to Resend fails or returns a
    /// non-success status that retrying a `429` did not cure.
    async fn send_email(&self, to: &str, subject: &str, html_body: &str) -> AppResult<()> {
        let email = ResendEmail {
            from: self.from_email.clone(),
            to: vec![to.to_owned()],
            subject: subject.to_owned(),
            body: ResendBody::Html(html_body.to_owned()),
        };
        // The recipient is PII, so only DEBUG carries it; INFO and WARN name the
        // message by subject, and the caller's own log names the user by id.
        // `resend_id` is the key Resend's dashboard lists this email under; it is
        // left off the event when Resend's success answer carried no readable id.
        debug!(to, subject, "Sending email via Resend");
        match self.client.send_with_receipt(&email).await {
            Ok(receipt) => {
                info!(
                    subject,
                    resend_id = receipt.id.as_deref(),
                    "Email sent successfully via Resend"
                );
                Ok(())
            }
            Err(e) => {
                warn!(subject, error = %e, "Resend email send failed");
                Err(AppError::internal(format!(
                    "Failed to send email via Resend: {e}"
                )))
            }
        }
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

    /// Send the magic link that opens the members part of the dravr.ai docs.
    ///
    /// `sign_in_url` carries a single-use token and lands on the website's
    /// callback; `ttl_minutes` is rendered into the copy.
    ///
    /// # Errors
    ///
    /// Returns an error if email delivery fails.
    pub async fn send_website_sign_in_link(
        &self,
        to: &str,
        sign_in_url: &str,
        ttl_minutes: i64,
    ) -> AppResult<()> {
        let html = templates::website_sign_in_html(sign_in_url, ttl_minutes);
        self.send_email(to, SUBJECT_WEBSITE_SIGN_IN, &html).await
    }

    /// Send an invitation carrying its single-use accept link.
    ///
    /// `accept_url` opens the app's accept page with the invitation token,
    /// where the invitee chooses their own password; `ttl_days` is rendered
    /// into the copy.
    ///
    /// # Errors
    ///
    /// Returns an error if email delivery fails.
    pub async fn send_invitation(
        &self,
        to: &str,
        accept_url: &str,
        ttl_days: i64,
    ) -> AppResult<()> {
        let html = templates::invitation_html(accept_url, ttl_days);
        self.send_email(to, SUBJECT_INVITATION, &html).await
    }

    /// Tell a super-admin that a coach-access request opened (carnet#738):
    /// one the coach asked for, or one filed for them from their
    /// Intervals.icu roster, as `origin` says.
    ///
    /// `review_url` links to the admin console queue; without one the email
    /// names where to find it.
    ///
    /// # Errors
    ///
    /// Returns an error if email delivery fails.
    pub async fn send_coach_access_requested(
        &self,
        to: &str,
        requester_email: &str,
        requester_name: Option<&str>,
        group_name: Option<&str>,
        review_url: Option<&str>,
        origin: CoachAccessOrigin,
    ) -> AppResult<()> {
        let html = templates::coach_access_requested_html(
            requester_email,
            requester_name,
            group_name,
            review_url,
            origin,
        );
        let subject = match origin {
            CoachAccessOrigin::Asked => SUBJECT_COACH_ACCESS_REQUESTED,
            CoachAccessOrigin::IntervalsIcuRoster { .. } => SUBJECT_COACH_ACCESS_FILED,
        };
        self.send_email(to, subject, &html).await
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
