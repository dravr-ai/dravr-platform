// ABOUTME: Narrow-context extractors layered on the canonical ServerContext
// ABOUTME: Lets services that only need a subset (auth, data, config, …) take that slice instead of the whole container
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Focused-context extractors on `ServerContext`.
//!
//! `ServerContext` (defined in `crate::mcp::resources`) is the single
//! canonical DI container. The extractor methods on this `impl` block
//! materialize the focused contexts (`AuthContext`, `DataContext`, …) on
//! demand for services that only want to depend on a subset of the
//! container — `AuthService`, for example, takes
//! `(AuthContext, ConfigContext, DataContext)` and is unaware of the rest.
//!
//! The extractors clone Arc handles, which is cheap. Handlers that need
//! the full container take `&Arc<ServerContext>` directly; only services
//! at narrow seams take focused contexts.

use crate::context::{
    AuthContext, ConfigContext, ExtensionContext, NotificationContext, SecurityContext,
};
use crate::mcp::resources::ServerContext;
use pierre_runtime_context::DataContext;

impl ServerContext {
    /// Extract authentication context (auth manager, middleware, JWT, Firebase).
    #[must_use]
    pub fn auth(&self) -> AuthContext {
        AuthContext::new(
            self.auth.auth_manager.clone(),
            self.auth.auth_middleware.clone(),
            self.auth.admin_jwt_secret.clone(),
            self.auth.jwks_manager.clone(),
            self.auth.firebase_auth.clone(),
        )
    }

    /// Extract data access context (database, repos, cache, providers, intelligence).
    #[must_use]
    pub fn data(&self) -> DataContext {
        DataContext::new(
            self.coach.database.clone(),
            self.common.repos.clone(),
            self.common.cache.clone(),
            self.fitness.provider_registry.clone(),
            self.fitness.activity_intelligence.clone(),
        )
    }

    /// Extract configuration context (server config, OAuth, A2A, admin config).
    ///
    /// Coexists with the `config` field (which is the bare `Arc<ServerConfig>`):
    /// `ctx.config` reaches the field, `ctx.config()` builds the focused context.
    #[must_use]
    pub fn config(&self) -> ConfigContext {
        ConfigContext::new(
            self.common.config.clone(),
            self.auth.tenant_oauth_client.clone(),
            #[cfg(feature = "protocol-a2a")]
            self.a2a.a2a_client_manager.clone(),
            #[cfg(feature = "protocol-a2a")]
            self.a2a.a2a_system_user_service.clone(),
            self.coach.admin_config.clone(),
        )
    }

    /// Extract notification context (WebSocket, SSE, OAuth notifications).
    #[must_use]
    pub fn notification(&self) -> NotificationContext {
        NotificationContext::new(
            #[cfg(feature = "transport-websocket")]
            self.sse.websocket_manager.clone(),
            #[cfg(feature = "transport-sse")]
            self.sse.sse_manager.clone(),
            self.auth.oauth_notification_sender.clone(),
        )
    }

    /// Extract security context (CSRF, redaction, rate limiting).
    #[must_use]
    pub fn security(&self) -> SecurityContext {
        SecurityContext::new(
            self.common.redaction_config.clone(),
            self.auth.oauth2_rate_limiter.clone(),
            self.auth.csrf_manager.clone(),
        )
    }

    /// Extract extension context (sampling peer, progress notifications, cancellation).
    #[must_use]
    pub fn extension(&self) -> ExtensionContext {
        ExtensionContext::new(
            self.sse.sampling_peer.clone(),
            self.sse.progress_notification_sender.clone(),
            self.sse.cancellation_registry.clone(),
        )
    }
}
