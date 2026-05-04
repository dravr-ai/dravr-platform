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
pub(crate) mod linking;
pub(crate) mod slack_actions;
mod templates;
pub(crate) mod webhooks;

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;

use std::env;

use crate::errors::AppError;
use crate::mcp::resources::ServerContext;

/// Messaging gateway routes handler
pub struct MessagingRoutes;

impl MessagingRoutes {
    /// Create all messaging routes
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            // Webhook ingress (per-channel signature verification)
            .route(
                "/api/messaging/webhook/{channel}",
                get(webhooks::verify_webhook).post(webhooks::handle_webhook),
            )
            // Channel configuration CRUD
            .route("/api/messaging/channels", get(config::list_channel_configs))
            .route(
                "/api/messaging/channels/{channel}",
                get(config::get_channel_config),
            )
            .route(
                "/api/messaging/channels/{channel}",
                put(config::upsert_channel_config),
            )
            .route(
                "/api/messaging/channels/{channel}",
                delete(config::delete_channel_config),
            )
            // Channel linking (OAuth/deep-link account verification)
            .route(
                "/api/messaging/link/init/{channel}",
                post(linking::init_channel_link),
            )
            .route(
                "/api/messaging/link/callback/{channel}",
                get(linking::link_callback),
            )
            .route("/api/messaging/links", get(linking::list_channel_links))
            .route(
                "/api/messaging/links/{channel}",
                delete(linking::delete_channel_link),
            )
            // Webhook-initiated channel linking (HTML pages, public, no auth)
            .route("/messaging/link/{code}", get(linking::channel_link_page))
            .route("/messaging/link/auth", post(linking::channel_link_auth))
            // Slack interactive actions (approve/reject from ops notifications)
            .route("/api/ops/slack/actions", post(handle_slack_ops_action))
            .with_state(resources)
    }
}

/// Axum handler for Slack interactive actions (button clicks)
///
/// Verifies HMAC-SHA256 signature using `SLACK_SIGNING_SECRET` before
/// delegating to the action handler. Returns 200 immediately on auth failure
/// to avoid Slack retries.
async fn handle_slack_ops_action(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, AppError> {
    // Verify signature using the same signing secret as the messaging webhook
    let signing_secret = env::var("SLACK_SIGNING_SECRET")
        .map_err(|_| AppError::internal("SLACK_SIGNING_SECRET not configured"))?;

    if let Err(e) = slack_actions::verify_slack_signature(&signing_secret, &headers, &body) {
        tracing::warn!(error = %e, "Slack ops action signature verification failed");
        // Return 200 to prevent Slack from retrying with invalid signature
        return Ok((
            StatusCode::OK,
            Json(json!({ "response_type": "ephemeral", "text": "Signature verification failed" })),
        )
            .into_response());
    }

    match slack_actions::handle_slack_action(&resources, &body).await {
        Ok(response) => Ok(response.into_response()),
        Err(e) => {
            tracing::warn!(error = %e, "Slack ops action failed");
            // Return user-friendly error as ephemeral Slack message
            Ok((
                StatusCode::OK,
                Json(
                    json!({ "response_type": "ephemeral", "text": format!("Action failed: {e}") }),
                ),
            )
                .into_response())
        }
    }
}
