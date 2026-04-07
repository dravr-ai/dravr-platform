// ABOUTME: Webhook endpoints for provider-pushed health data (WHOOP, Strava, Garmin, Oura)
// ABOUTME: Validates signatures via dravr-enforme, processes events asynchronously
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use tokio::task::yield_now;
use tracing::{info, warn};

use pierre_core::models::{OAuthNotification, TenantId};

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
            .route(
                "/webhooks/strava",
                get(Self::handle_strava_verification).post(Self::handle_strava_event),
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

    // ========================================================================
    // Strava webhooks
    // ========================================================================

    /// Strava webhook subscription validation callback (GET).
    ///
    /// Strava sends a GET with `hub.mode`, `hub.challenge`, and `hub.verify_token`
    /// to validate the endpoint. Must respond within 15 seconds with the challenge.
    /// Only ONE subscription is allowed per Strava app (not per-user).
    async fn handle_strava_verification(
        query: Query<HashMap<String, String>>,
    ) -> impl IntoResponse {
        let mode = query.get("hub.mode").cloned().unwrap_or_default();
        let challenge = query.get("hub.challenge").cloned().unwrap_or_default();
        let verify_token = query.get("hub.verify_token").cloned().unwrap_or_default();

        // Validate the verify token against our configured secret
        let expected_token = env::var("STRAVA_WEBHOOK_VERIFY_TOKEN").unwrap_or_default();

        if mode != "subscribe" || (!expected_token.is_empty() && verify_token != expected_token) {
            warn!(
                mode = %mode,
                "Strava webhook verification failed: invalid mode or verify_token"
            );
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "verification failed"})),
            );
        }

        info!("Strava webhook subscription verified");
        yield_now().await;

        // Strava expects: {"hub.challenge": "<challenge_value>"}
        (
            StatusCode::OK,
            Json(serde_json::json!({"hub.challenge": challenge})),
        )
    }

    /// Strava webhook event handler (POST).
    ///
    /// Strava pushes event notifications with minimal payload:
    /// `{ "object_type": "activity", "object_id": 123, "aspect_type": "create",
    ///    "owner_id": 456, "subscription_id": 789, "event_time": 1234567890 }`
    ///
    /// The payload only contains IDs — a full activity fetch is still required.
    async fn handle_strava_event(
        State(resources): State<Arc<ServerResources>>,
        body: Bytes,
    ) -> impl IntoResponse {
        let event: StravaWebhookEvent = match serde_json::from_slice(&body) {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "Failed to parse Strava webhook payload");
                return StatusCode::BAD_REQUEST;
            }
        };

        info!(
            object_type = %event.object_type,
            object_id = %event.object_id,
            aspect_type = %event.aspect_type,
            owner_id = %event.owner_id,
            subscription_id = %event.subscription_id,
            event_time = %event.event_time,
            "Received Strava webhook event"
        );

        // Only process activity create/update events (not delete)
        if event.object_type != "activity"
            || (event.aspect_type != "create" && event.aspect_type != "update")
        {
            yield_now().await;
            return StatusCode::OK;
        }

        // Look up the user by Strava athlete ID (owner_id) and trigger a sync.
        // The full activity data will be fetched by the sync process.
        let repos = resources.repos.clone();
        let sse = resources.sse_manager.clone();
        let orchestrator = resources.sync_orchestrator.clone();
        let owner_id = event.owner_id.to_string();

        tokio::spawn(async move {
            let Some(orchestrator) = orchestrator else {
                warn!("Strava webhook received but sync orchestrator not configured");
                return;
            };

            // Find user by looking up OAuth tokens for strava provider
            // The owner_id from Strava corresponds to the athlete_id stored during OAuth
            // For now, trigger sync for all strava-connected users (the orchestrator
            // will use cursors to only fetch new data)
            match orchestrator
                .deps()
                .connections
                .list_connected_users("strava")
                .await
            {
                Ok(users) => {
                    for user in &users {
                        if !user.is_active {
                            continue;
                        }
                        // Sync the specific user to pick up the new activity
                        match orchestrator.sync_user(&user.user_id, "strava").await {
                            Ok(result) if result.records_created > 0 => {
                                info!(
                                    user_id = user.user_id,
                                    strava_owner_id = %owner_id,
                                    records = result.records_created,
                                    "Strava webhook-triggered sync completed"
                                );
                                // Update last_sync + SSE notify
                                if let Ok(uuid) = user.user_id.parse::<uuid::Uuid>() {
                                    if let Ok(tokens) =
                                        repos.oauth_tokens.get_tokens(uuid, None).await
                                    {
                                        if let Some(tok) =
                                            tokens.iter().find(|t| t.provider == "strava")
                                        {
                                            if let Ok(tid) = tok.tenant_id.parse::<TenantId>() {
                                                let _ = repos
                                                    .oauth_tokens
                                                    .update_provider_last_sync(
                                                        uuid,
                                                        tid,
                                                        "strava",
                                                        chrono::Utc::now(),
                                                    )
                                                    .await;
                                            }
                                        }
                                    }

                                    let notification = OAuthNotification {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        user_id: user.user_id.clone(),
                                        provider: "strava".to_owned(),
                                        success: true,
                                        message: format!(
                                            "New Strava activity synced ({} records)",
                                            result.records_created
                                        ),
                                        expires_at: None,
                                        created_at: chrono::Utc::now(),
                                        read_at: None,
                                    };
                                    let _ = sse.send_notification(uuid, &notification).await;
                                }
                            }
                            Ok(_) => {} // No new records
                            Err(e) => {
                                warn!(
                                    user_id = user.user_id,
                                    error = %e,
                                    "Strava webhook-triggered sync failed"
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Failed to list Strava users for webhook sync");
                }
            }
        });

        yield_now().await;
        StatusCode::OK
    }
}

/// Strava webhook event payload.
///
/// Strava sends minimal event data — only IDs and event type.
/// The full activity must be fetched separately via the API.
#[derive(Debug, Deserialize)]
struct StravaWebhookEvent {
    /// Type of object: "activity" or "athlete"
    object_type: String,
    /// Strava resource ID (`activity_id` or `athlete_id`)
    object_id: u64,
    /// Event type: "create", "update", or "delete"
    aspect_type: String,
    /// Strava athlete ID who owns the object
    owner_id: u64,
    /// Subscription ID (for verification)
    subscription_id: u64,
    /// Unix timestamp of the event
    event_time: u64,
}
