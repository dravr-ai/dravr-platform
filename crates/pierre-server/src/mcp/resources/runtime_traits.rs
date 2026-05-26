// ABOUTME: pierre_runtime_context::*Ctx trait impls projecting ServerContext onto narrow per-subsystem facades
// ABOUTME: Each impl exposes only the Arc handles a downstream route/service crate needs (auth, billing, identity, dashboard, mcp-dispatch, a2a, sse, coaches, social, command)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::ServerContext;
use crate::config::admin::AdminConfigService;
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::{AuthManager, AuthResult};
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_config::environment::SseBufferStrategy;
use pierre_core::billing::BillingProvider;
use pierre_core::errors::AppResult;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use serde_json::Value;
use std::sync::Arc;

#[async_trait::async_trait]
impl pierre_runtime_context::MiddlewareCtx for ServerContext {
    fn auth_manager(&self) -> &Arc<AuthManager> {
        &self.auth.auth_manager
    }

    fn jwks_manager(&self) -> &Arc<JwksManager> {
        &self.auth.jwks_manager
    }

    fn repos(&self) -> &Arc<RepositoryRegistry> {
        &self.common.repos
    }

    fn csrf_manager(&self) -> &Arc<CsrfTokenManager> {
        &self.auth.csrf_manager
    }

    async fn authenticate_request(&self, auth_header: Option<&str>) -> AppResult<AuthResult> {
        self.auth
            .auth_middleware
            .authenticate_request(auth_header)
            .await
    }
}

impl pierre_runtime_context::BillingCtx for ServerContext {
    fn billing_provider(&self) -> &Arc<dyn BillingProvider> {
        &self.billing.billing_provider
    }

    fn repos(&self) -> &Arc<RepositoryRegistry> {
        &self.common.repos
    }
}

impl pierre_runtime_context::IdentityCtx for ServerContext {
    fn repos(&self) -> &Arc<RepositoryRegistry> {
        &self.common.repos
    }
}

impl pierre_runtime_context::DashboardCtx for ServerContext {
    fn repos(&self) -> &Arc<RepositoryRegistry> {
        &self.common.repos
    }
}

impl pierre_runtime_context::McpDispatchCtx for ServerContext {
    fn auth_manager(&self) -> &Arc<AuthManager> {
        &self.auth.auth_manager
    }

    fn jwks_manager(&self) -> &Arc<JwksManager> {
        &self.auth.jwks_manager
    }

    fn repos(&self) -> &Arc<RepositoryRegistry> {
        &self.common.repos
    }
}

#[cfg(feature = "protocol-a2a")]
impl pierre_runtime_context::A2ACtx for ServerContext {
    fn auth_manager(&self) -> &Arc<AuthManager> {
        &self.auth.auth_manager
    }

    fn jwks_manager(&self) -> &Arc<JwksManager> {
        &self.auth.jwks_manager
    }

    fn repos(&self) -> &Arc<RepositoryRegistry> {
        &self.common.repos
    }

    fn base_url(&self) -> &str {
        &self.common.config.base_url
    }
}

#[cfg(feature = "transport-sse")]
#[async_trait::async_trait]
impl pierre_runtime_context::SseCtx for ServerContext {
    fn auth_manager(&self) -> &Arc<AuthManager> {
        &self.auth.auth_manager
    }

    fn sse_buffer_overflow_strategy(&self) -> pierre_runtime_context::SseBufferOverflowStrategy {
        match self.common.config.sse.buffer_overflow_strategy {
            SseBufferStrategy::DropOldest => {
                pierre_runtime_context::SseBufferOverflowStrategy::DropOldest
            }
            SseBufferStrategy::DropNew => {
                pierre_runtime_context::SseBufferOverflowStrategy::DropNew
            }
            SseBufferStrategy::CloseConnection => {
                pierre_runtime_context::SseBufferOverflowStrategy::CloseConnection
            }
        }
    }

    fn jwks_manager(&self) -> &Arc<JwksManager> {
        &self.auth.jwks_manager
    }

    fn repos(&self) -> &Arc<RepositoryRegistry> {
        &self.common.repos
    }

    async fn authenticate_request(&self, auth_header: Option<&str>) -> AppResult<AuthResult> {
        self.auth
            .auth_middleware
            .authenticate_request(auth_header)
            .await
    }
}

// NOTE: this `AdminConfigLookup` impl is on `AdminConfigService`, not
// `ServerContext`. It conceptually belongs alongside `AdminConfigService`
// (crate::config::admin) — kept here for now to avoid expanding the scope of
// the resources.rs decomposition.
#[async_trait::async_trait]
impl pierre_runtime_context::AdminConfigLookup for AdminConfigService {
    async fn get_value(&self, key: &str, tenant_id: Option<&str>) -> AppResult<Option<Value>> {
        Self::get_value(self, key, tenant_id).await
    }

    async fn get_override_value(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<Value>> {
        Self::get_override_value(self, key, tenant_id).await
    }
}

impl pierre_runtime_context::CoachesCtx for ServerContext {
    fn database(&self) -> &Arc<Database> {
        &self.coach.database
    }

    fn coach_generation_prompt(&self) -> String {
        Self::coach_generation_prompt(self)
    }

    fn admin_config(&self) -> Option<Arc<dyn pierre_runtime_context::AdminConfigLookup>> {
        self.coach
            .admin_config
            .as_ref()
            .map(|svc| Arc::clone(svc) as Arc<dyn pierre_runtime_context::AdminConfigLookup>)
    }

    #[cfg(feature = "client-notifications")]
    fn notification_service(&self) -> Option<&Arc<pierre_notifications::NotificationService>> {
        self.common.notification_service.as_ref()
    }

    fn chat_provider(&self) -> Option<&Arc<pierre_llm::ChatProvider>> {
        self.common.chat_provider.as_ref()
    }

    fn llm_provider(&self) -> Option<&Arc<dyn pierre_llm::LlmProvider>> {
        self.common.llm_provider.as_ref()
    }
}

#[cfg(all(feature = "client-groups", feature = "client-notifications"))]
mod social_ctx_impl {
    use super::ServerContext;
    use async_trait::async_trait;
    use pierre_groups::GroupService;
    use pierre_notifications::NotificationService;
    use pierre_runtime_context::SocialCtx;
    use std::sync::Arc;

    #[async_trait]
    impl SocialCtx for ServerContext {
        fn notification_service(&self) -> Option<&Arc<NotificationService>> {
            self.common.notification_service.as_ref()
        }

        fn group_service(&self) -> &Arc<GroupService> {
            &self.common.group_service
        }

        async fn admin_config_get(
            &self,
            key: &str,
            tenant_id: Option<&str>,
        ) -> Option<serde_json::Value> {
            let service = self.coach.admin_config.as_ref()?;
            service.get_value(key, tenant_id).await.ok().flatten()
        }
    }
}

// `CommandCtx` is gated on `client-messaging` because the trait is consumed
// only by `pierre-commands`, which is itself an optional dep behind that
// flag. With the flag off the slash-command dispatcher / handlers never link,
// so the impl carries no value.
#[cfg(feature = "client-messaging")]
mod command_ctx_impl {
    use super::ServerContext;
    use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
    use pierre_database::RepositoryRegistry;
    use pierre_groups::GroupService;
    use pierre_runtime_context::CommandCtx;
    use std::sync::Arc;

    impl CommandCtx for ServerContext {
        fn repos(&self) -> &Arc<RepositoryRegistry> {
            &self.common.repos
        }

        fn group_service(&self) -> &Arc<GroupService> {
            &self.common.group_service
        }

        fn messaging_strings_registry(&self) -> &Arc<MessagingStringsRegistry> {
            &self.mcp.messaging_strings_registry
        }
    }
}
