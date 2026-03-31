// ABOUTME: Webhook endpoints for provider-pushed health data (WHOOP, Garmin, Oura)
// ABOUTME: Validates signatures via dravr-enforme, processes events asynchronously
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tokio::task::yield_now;

use crate::mcp::resources::ServerResources;

/// Webhook routes for health data provider push notifications.
///
/// Providers like WHOOP, Garmin, and Oura push data updates via webhooks
/// rather than requiring us to poll their APIs. These routes validate
/// the webhook signature and queue the event for async processing.
pub struct WebhookRoutes;

impl WebhookRoutes {
    /// Mount webhook routes for all supported providers.
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            .route(
                "/webhooks/whoop",
                get(Self::handle_whoop_verification).post(Self::handle_whoop_event),
            )
            .with_state(resources)
    }

    /// WHOOP webhook verification challenge (GET).
    /// WHOOP sends a GET request with a challenge token to verify the endpoint.
    async fn handle_whoop_verification(query: Query<HashMap<String, String>>) -> impl IntoResponse {
        // Return the challenge token as-is to verify endpoint ownership
        let challenge = query.get("challenge").cloned().unwrap_or_default();
        // Yield to the runtime scheduler between request parsing and response
        yield_now().await;
        (StatusCode::OK, challenge)
    }

    /// WHOOP webhook event handler (POST).
    /// Validates HMAC-SHA256 signature, then queues the event for async processing.
    async fn handle_whoop_event(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> impl IntoResponse {
        tracing::info!(
            provider = "whoop",
            body_len = body.len(),
            has_signature = headers.contains_key("x-whoop-signature"),
            "Received WHOOP webhook event"
        );

        let Some(orchestrator) = resources.sync_orchestrator.clone() else {
            tracing::warn!("WHOOP webhook received but sync orchestrator is not configured");
            return StatusCode::SERVICE_UNAVAILABLE;
        };

        // Validate and process the webhook asynchronously to avoid blocking the response
        let payload = body.to_vec();
        tokio::spawn(async move {
            if let Err(e) = orchestrator
                .handle_webhook("whoop", &headers, &payload)
                .await
            {
                tracing::error!(error = %e, "Failed to process WHOOP webhook event");
            }
        });

        // Yield to the runtime scheduler before returning the response
        yield_now().await;
        StatusCode::OK
    }
}
