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
//! can't fail the approval.

use std::sync::Arc;

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_REGISTRATION_APPROVED};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_email::ResendEmailService;
use pierre_services::messaging_broadcast::send_to_linked_channels;
use pierre_services::user_approval::UserApprovalNotifier;
use tracing::warn;
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
        let Some(svc) = &self.email_service else {
            warn!("Email service not configured — skipping account-approved email");
            return;
        };
        if let Err(e) = svc
            .send_registration_approved(email, display_name, self.frontend_url.as_deref())
            .await
        {
            warn!(error = %e, "Failed to send account-approved email");
        }
    }

    /// Send the approval message to each of the user's linked channels.
    ///
    /// Rendered per link locale through the shared proactive path — the same
    /// one the notification messaging sink uses, so "which channels has this
    /// user linked, and how do we reach them" is resolved in one place.
    async fn send_channel_messages(&self, user_id: Uuid, tenant_id: TenantId) {
        send_to_linked_channels(
            self.repos.messaging.as_ref(),
            tenant_id,
            user_id,
            |locale| self.strings.render(KEY_REGISTRATION_APPROVED, locale, &[]),
        )
        .await;
    }
}

#[async_trait]
impl UserApprovalNotifier for ApprovalNotifier {
    async fn notify_user_approved(&self, user_id: Uuid, email: &str, display_name: Option<&str>) {
        self.send_email(email, display_name).await;

        // Resolve the user's tenant(s); channel links are tenant-scoped, so we
        // sweep every tenant the user belongs to (almost always one personal
        // tenant).
        let tenants = self
            .repos
            .tenants
            .list_for_user(user_id)
            .await
            .unwrap_or_default();
        for tenant in tenants {
            self.send_channel_messages(user_id, tenant.id).await;
        }
    }
}
