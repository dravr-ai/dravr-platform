// ABOUTME: ApprovalNotifier — the concrete UserApprovalNotifier wired in the binary:
// ABOUTME: sends the account-approved email and a localized message on each linked channel.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Binary-side implementation of [`UserApprovalNotifier`].
//!
//! Built once from the assembled [`ServerContext`] and injected (behind the
//! trait) into every approval path so REST, web-admin, the Slack ops button,
//! and registration auto-approve all notify the user the same way. Every step
//! is best-effort — failures are logged, never propagated, so a notification
//! can't fail the approval. Every outcome — sent, skipped, failed — is logged
//! at INFO or WARN with the masked recipient, since the deployed server runs
//! at INFO and an operator needs to tell "sent" from "skipped" there.

use std::sync::Arc;

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_REGISTRATION_APPROVED};
use pierre_database::RepositoryRegistry;
use pierre_email::ResendEmailService;
use pierre_middleware::redaction::mask_email;
use pierre_services::messaging_broadcast::send_to_linked_channels;
use pierre_services::user_approval::UserApprovalNotifier;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// Account-approved notifier: sends the approval email plus a localized message
/// on each of the user's linked messaging channels.
pub struct ApprovalNotifier {
    repos: Arc<RepositoryRegistry>,
    email_service: Option<Arc<ResendEmailService>>,
    strings: Arc<MessagingStringsRegistry>,
    frontend_url: Option<String>,
}

impl ApprovalNotifier {
    /// Build the injectable notifier from the assembled server context.
    #[must_use]
    pub fn from_context(resources: &ServerContext) -> Arc<dyn UserApprovalNotifier> {
        Arc::new(Self {
            repos: resources.common.repos.clone(),
            email_service: resources.common.email_service.clone(),
            strings: resources.mcp.messaging_strings_registry.clone(),
            frontend_url: resources.common.config.frontend_url.clone(),
        })
    }

    /// Send the account-approved email; no-op (logged) when email is unconfigured.
    async fn send_email(&self, email: &str, display_name: Option<&str>) {
        let recipient = mask_email(email);
        let Some(svc) = &self.email_service else {
            warn!(%recipient, "Email service not configured — skipping account-approved email");
            return;
        };
        match svc
            .send_registration_approved(email, display_name, self.frontend_url.as_deref())
            .await
        {
            Ok(()) => info!(%recipient, "Account-approved email sent"),
            Err(e) => warn!(%recipient, error = %e, "Failed to send account-approved email"),
        }
    }

    /// Send the approval message to each of the user's linked channels.
    ///
    /// Rendered per link locale through the shared proactive path — the same
    /// one the notification messaging sink uses, so "which channels has this
    /// user linked, and how do we reach them" is resolved in one place. That
    /// path reads the user's links across every tenant that stores one, each
    /// link once, so a single call reaches every chat without sending twice.
    async fn send_channel_messages(&self, user_id: Uuid) {
        send_to_linked_channels(self.repos.messaging.as_ref(), user_id, |locale| {
            self.strings.render(KEY_REGISTRATION_APPROVED, locale, &[])
        })
        .await;
    }
}

#[async_trait]
impl UserApprovalNotifier for ApprovalNotifier {
    async fn notify_user_approved(&self, user_id: Uuid, email: &str, display_name: Option<&str>) {
        self.send_email(email, display_name).await;
        self.send_channel_messages(user_id).await;
    }

    async fn notify_user_invited(&self, email: &str) {
        let recipient = mask_email(email);
        let Some(svc) = &self.email_service else {
            warn!(%recipient, "Email service not configured — skipping invitation email");
            return;
        };
        let Some(signup_url) = self.frontend_url.as_deref() else {
            warn!(%recipient, "No frontend URL configured — skipping invitation email");
            return;
        };
        match svc.send_invitation(email, signup_url).await {
            Ok(()) => info!(%recipient, "Invitation email sent"),
            Err(e) => warn!(%recipient, error = %e, "Failed to send invitation email"),
        }
    }
}
