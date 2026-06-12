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

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, DEFAULT_LOCALE, KEY_REGISTRATION_APPROVED,
};
use pierre_core::models::messaging::{ChannelConfig, ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_database::RepositoryRegistry;
use pierre_email::ResendEmailService;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::factory::create_adapter_from_config;
use pierre_messaging::turn::ConversationTurnId;
use pierre_services::user_approval::UserApprovalNotifier;
use serde_json::Value;
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
    async fn send_channel_messages(&self, user_id: Uuid, tenant_id: TenantId) {
        let db: &dyn MessagingRepository = self.repos.messaging.as_ref();
        let links = match db
            .list_user_channel_links(tenant_id, &user_id.to_string())
            .await
        {
            Ok(links) => links,
            Err(e) => {
                warn!(error = %e, "Failed to list channel links for approval message");
                return;
            }
        };
        for link in &links {
            self.send_one_channel(tenant_id, link).await;
        }
    }

    /// Deliver the localized approval message to a single linked channel.
    async fn send_one_channel(&self, tenant_id: TenantId, link: &Value) {
        let Some(channel_str) = link.get("channel_type").and_then(Value::as_str) else {
            return;
        };
        let Some(recipient) = link.get("channel_user_id").and_then(Value::as_str) else {
            return;
        };
        let locale = link
            .get("locale")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_LOCALE);

        let Ok(channel_type) = ChannelType::from_str(channel_str) else {
            warn!(channel = %channel_str, "Unknown channel type for approval message");
            return;
        };

        let Some((adapter, channel_config)) = self
            .resolve_sender(tenant_id, channel_str, channel_type)
            .await
        else {
            return;
        };

        let body = self.strings.render(KEY_REGISTRATION_APPROVED, locale, &[]);
        let outgoing = OutgoingMessage {
            channel_type,
            recipient_id: recipient.to_owned(),
            content: MessageContent::Text { body },
            turn_id: ConversationTurnId::new(),
            reply_to: None,
            thread_id: None,
        };

        if let Err(e) = adapter.send(&outgoing, &channel_config).await {
            warn!(error = %e, channel = %channel_str, "Failed to send approval message on channel");
        } else {
            info!(channel = %channel_str, "Sent account-approved message on channel");
        }
    }

    /// Resolve the channel adapter and its config for an outbound send, or
    /// `None` (logged) when the channel is unconfigured or undeserializable.
    async fn resolve_sender(
        &self,
        tenant_id: TenantId,
        channel_str: &str,
        channel_type: ChannelType,
    ) -> Option<(Arc<dyn MessagingChannel>, ChannelConfig)> {
        let raw_config = self.load_channel_config(tenant_id, channel_str).await?;
        let adapter = create_adapter_from_config(channel_type, &raw_config)
            .inspect_err(|e| {
                warn!(error = %e, channel = %channel_str, "Failed to build adapter for approval message");
            })
            .ok()?;
        let channel_config: ChannelConfig = serde_json::from_value(raw_config)
            .inspect_err(|e| {
                warn!(error = %e, "Failed to deserialize channel config for approval message");
            })
            .ok()?;
        Some((adapter, channel_config))
    }

    /// Load the raw channel config JSON for a tenant + channel, or `None`
    /// (logged) when absent or the lookup fails.
    async fn load_channel_config(&self, tenant_id: TenantId, channel_str: &str) -> Option<Value> {
        let db: &dyn MessagingRepository = self.repos.messaging.as_ref();
        match db.get_channel_config(tenant_id, channel_str).await {
            Ok(Some(cfg)) => Some(cfg),
            Ok(None) => {
                warn!(channel = %channel_str, "No channel config for approval message");
                None
            }
            Err(e) => {
                warn!(error = %e, "Failed to load channel config for approval message");
                None
            }
        }
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
