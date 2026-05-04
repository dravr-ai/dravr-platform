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
    AuthContext, ConfigContext, DataContext, ExtensionContext, NotificationContext, SecurityContext,
};
use crate::mcp::resources::ServerContext;

impl ServerContext {
    /// Extract authentication context (auth manager, middleware, JWT, Firebase).
    #[must_use]
    pub fn auth(&self) -> AuthContext {
        AuthContext::new(
            self.auth_manager.clone(),
            self.auth_middleware.clone(),
            self.admin_jwt_secret.clone(),
            self.jwks_manager.clone(),
            self.firebase_auth.clone(),
        )
    }

    /// Extract data access context (database, repos, cache, providers, intelligence).
    #[must_use]
    pub fn data(&self) -> DataContext {
        DataContext::new(
            self.database.clone(),
            self.repos.clone(),
            self.cache.clone(),
            self.provider_registry.clone(),
            self.activity_intelligence.clone(),
        )
    }

    /// Extract configuration context (server config, OAuth, A2A, admin config).
    ///
    /// Coexists with the `config` field (which is the bare `Arc<ServerConfig>`):
    /// `ctx.config` reaches the field, `ctx.config()` builds the focused context.
    #[must_use]
    pub fn config(&self) -> ConfigContext {
        ConfigContext::new(
            self.config.clone(),
            self.tenant_oauth_client.clone(),
            #[cfg(feature = "protocol-a2a")]
            self.a2a_client_manager.clone(),
            #[cfg(feature = "protocol-a2a")]
            self.a2a_system_user_service.clone(),
            self.admin_config.clone(),
        )
    }

    /// Extract notification context (WebSocket, SSE, OAuth notifications).
    #[must_use]
    pub fn notification(&self) -> NotificationContext {
        NotificationContext::new(
            #[cfg(feature = "transport-websocket")]
            self.websocket_manager.clone(),
            #[cfg(feature = "transport-sse")]
            self.sse_manager.clone(),
            self.oauth_notification_sender.clone(),
        )
    }

    /// Extract security context (CSRF, redaction, rate limiting).
    #[must_use]
    pub fn security(&self) -> SecurityContext {
        SecurityContext::new(
            self.redaction_config.clone(),
            self.oauth2_rate_limiter.clone(),
            self.csrf_manager.clone(),
            self.csrf_middleware.clone(),
        )
    }

    /// Extract extension context (sampling peer, progress notifications, cancellation).
    #[must_use]
    pub fn extension(&self) -> ExtensionContext {
        ExtensionContext::new(
            self.sampling_peer.clone(),
            self.progress_notification_sender.clone(),
            self.cancellation_registry.clone(),
        )
    }
}
