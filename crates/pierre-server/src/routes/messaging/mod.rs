// ABOUTME: Messaging gateway route module organizing webhook and config handlers
// ABOUTME: Provides MessagingRoutes with router wiring for multi-channel messaging
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Messaging gateway routes
//!
//! This module handles multi-channel messaging endpoints for webhook ingress
//! and channel configuration management. All config endpoints require JWT
//! authentication. Webhook endpoints use channel-specific signature verification.

mod config;
mod linking;
mod templates;
mod webhooks;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;

use crate::mcp::resources::ServerResources;

/// Messaging gateway routes handler
pub struct MessagingRoutes;

impl MessagingRoutes {
    /// Create all messaging routes
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            // Webhook ingress (per-channel signature verification)
            .route(
                "/api/messaging/webhook/:channel",
                get(webhooks::verify_webhook).post(webhooks::handle_webhook),
            )
            // Channel configuration CRUD
            .route("/api/messaging/channels", get(config::list_channel_configs))
            .route(
                "/api/messaging/channels/:channel",
                get(config::get_channel_config),
            )
            .route(
                "/api/messaging/channels/:channel",
                put(config::upsert_channel_config),
            )
            .route(
                "/api/messaging/channels/:channel",
                delete(config::delete_channel_config),
            )
            // Channel linking (OAuth/deep-link account verification)
            .route(
                "/api/messaging/link/init/:channel",
                post(linking::init_channel_link),
            )
            .route(
                "/api/messaging/link/callback/:channel",
                get(linking::link_callback),
            )
            .route("/api/messaging/links", get(linking::list_channel_links))
            .route(
                "/api/messaging/links/:channel",
                delete(linking::delete_channel_link),
            )
            // Webhook-initiated channel linking (HTML pages, public, no auth)
            .route("/messaging/link/:code", get(linking::channel_link_page))
            .route("/messaging/link/auth", post(linking::channel_link_auth))
            .with_state(resources)
    }
}
