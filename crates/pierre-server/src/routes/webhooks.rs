// ABOUTME: Webhook endpoints for provider push events (WHOOP, Strava) — validate, resolve the owner, sync
// ABOUTME: A Strava activity event fetches the owner's recent activities; a WHOOP event runs the owner's health sync
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::env;
use std::future::ready;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use pierre_enforme::error::EnformeError;
use pierre_enforme::models::webhook::WebhookEvent;
use pierre_enforme::SyncOrchestrator;
use serde::Deserialize;
use tracing::{error, info, warn};
use uuid::Uuid;

use pierre_core::models::{OAuthNotification, TenantId};
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::activity_fetch::fetch_provider_head;
use pierre_tool_runtime::runtime::ToolRuntime;

use crate::mcp::resources::ServerContext;
use crate::routes::strava_webhook_gate::{subscription_refusal, STRAVA_OWNER_FETCH_GATE};

/// How far before the announced event a Strava webhook-triggered fetch reads.
///
/// Strava's payload carries only ids and the event time; the activity itself
/// can have been recorded days earlier (a watch that uploads late, a manual
/// entry backdated to the ride), so the window opens well before the event
/// rather than at it. A week bounds the read while still covering every
/// realistic upload delay.
const STRAVA_WEBHOOK_LOOKBACK_DAYS: i64 = 7;

/// Cap on rows a Strava webhook-triggered fetch reads.
///
/// The window is one week, and a busy week is tens of activities, never
/// hundreds; the cap only guards against an unbounded read.
const STRAVA_WEBHOOK_FETCH_LIMIT: usize = 50;

/// Webhook routes for provider push notifications.
///
/// WHOOP pushes a signed event per changed record; Strava pushes an id-only
/// event per activity change. Both routes answer the provider at once and do
/// the sync on the drain-tracked spawner, so a SIGTERM waits for it.
pub struct WebhookRoutes;

impl WebhookRoutes {
    /// Mount webhook routes for all supported providers.
    ///
    /// The verification handlers and the Strava event handler do no async
    /// work themselves (the Strava sync is spawned), so they are plain
    /// functions wrapped in [`ready`]; only the WHOOP event handler awaits.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route(
                "/webhooks/whoop",
                get(|query: Query<HashMap<String, String>>| {
                    ready(Self::whoop_verification(&query))
                })
                .post(Self::handle_whoop_event),
            )
            .route(
                "/webhooks/strava",
                get(|query: Query<HashMap<String, String>>| {
                    ready(Self::strava_verification(&query))
                })
                .post(
                    |State(resources): State<Arc<ServerContext>>, body: Bytes| {
                        ready(Self::strava_event(&resources, &body))
                    },
                ),
            )
            .with_state(resources)
    }

    /// WHOOP webhook verification challenge (GET).
    ///
    /// WHOOP sends a GET with a `challenge` query parameter to verify the
    /// endpoint; echoing it back proves ownership.
    fn whoop_verification(query: &HashMap<String, String>) -> (StatusCode, String) {
        let challenge = query.get("challenge").cloned().unwrap_or_default();
        (StatusCode::OK, challenge)
    }

    /// WHOOP webhook event handler (POST).
    ///
    /// The orchestrator's WHOOP provider verifies the `x-whoop-signature`
    /// HMAC of the `x-whoop-signature-timestamp` and the body, keyed with
    /// the WHOOP app's client secret (`WHOOP_WEBHOOK_SECRET`), refuses a
    /// timestamp outside its replay window, and parses the body into events.
    /// Each event names the WHOOP-side user id; that id is mapped to the one
    /// platform user whose WHOOP token carries it (captured at token exchange
    /// and on refresh), and that user's health sync runs on the drain-tracked
    /// spawner. Several events for one user in a payload run one sync.
    ///
    /// A body that fails validation is refused with 401 and nothing is
    /// synced; an unparseable one with 400; a missing secret with 503.
    async fn handle_whoop_event(
        State(resources): State<Arc<ServerContext>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> StatusCode {
        info!(
            provider = "whoop",
            body_len = body.len(),
            has_signature = headers.contains_key("x-whoop-signature"),
            "Received WHOOP webhook event"
        );

        let Some(orchestrator) = resources.fitness.sync_orchestrator.clone() else {
            warn!("WHOOP webhook received but sync orchestrator is not configured");
            return StatusCode::SERVICE_UNAVAILABLE;
        };

        let events = match orchestrator.handle_webhook("whoop", &headers, &body).await {
            Ok(events) => events,
            Err(e) => {
                warn!(error = %e, "WHOOP webhook refused; nothing synced");
                return whoop_refusal_status(&e);
            }
        };

        for provider_user_id in distinct_owners(&events) {
            let resources = Arc::clone(&resources);
            let orchestrator = Arc::clone(&orchestrator);
            resources.common.turns.clone().spawn(Box::pin(async move {
                sync_whoop_owner(&resources, &orchestrator, &provider_user_id).await;
            }));
        }

        StatusCode::OK
    }

    // ========================================================================
    // Strava webhooks
    // ========================================================================

    /// Strava webhook subscription validation callback (GET).
    ///
    /// Strava sends a GET with `hub.mode`, `hub.challenge`, and `hub.verify_token`
    /// to validate the endpoint. Must respond within 15 seconds with the challenge.
    /// Only ONE subscription is allowed per Strava app (not per-user); the
    /// `pierre-cli strava-webhook subscribe` command registers it with the
    /// same `STRAVA_WEBHOOK_VERIFY_TOKEN` this handler checks.
    fn strava_verification(
        query: &HashMap<String, String>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let mode = query.get("hub.mode").cloned().unwrap_or_default();
        let challenge = query.get("hub.challenge").cloned().unwrap_or_default();
        let verify_token = query.get("hub.verify_token").cloned().unwrap_or_default();

        // Validate the verify token against our configured secret. An unset
        // token refuses every handshake: an empty expected value would
        // otherwise let anyone register a subscription against this route.
        let expected_token = env::var("STRAVA_WEBHOOK_VERIFY_TOKEN").unwrap_or_default();

        if mode != "subscribe" || expected_token.is_empty() || verify_token != expected_token {
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
    /// The payload only carries ids. For an activity create or update, the
    /// owner is resolved from the Strava athlete id and their recent
    /// activities are fetched through the platform's OAuth activity path on
    /// the drain-tracked spawner (see [`sync_strava_owner`]); the fetch writes
    /// through to the activity cache with the freshness mark. Deletes and
    /// athlete events are acknowledged and not fetched.
    ///
    /// An event whose `subscription_id` is not one this deployment
    /// registered is refused with 403, and every event with 503 while
    /// `STRAVA_WEBHOOK_SUBSCRIPTION_ID` is unset (see `strava_webhook_gate`).
    fn strava_event(resources: &Arc<ServerContext>, body: &[u8]) -> StatusCode {
        let event: StravaWebhookEvent = match serde_json::from_slice(body) {
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

        // Strava does not sign events: the registered subscription id is the
        // one thing a genuine event carries that a forger must guess.
        if let Some(refusal) = subscription_refusal(event.subscription_id) {
            return refusal;
        }

        if !event.is_activity_write() {
            return StatusCode::OK;
        }

        let resources = Arc::clone(resources);
        resources.common.turns.clone().spawn(Box::pin(async move {
            sync_strava_owner(&resources, &event).await;
        }));

        StatusCode::OK
    }
}

/// The HTTP status a refused WHOOP webhook answers with.
///
/// A signature that does not verify is the caller's problem (401); a body
/// the provider cannot parse is malformed (400); a missing secret or an
/// unregistered provider is this server's configuration (503), which WHOOP
/// retries once it is fixed.
fn whoop_refusal_status(error: &EnformeError) -> StatusCode {
    match error {
        EnformeError::WebhookValidationFailed { .. } => StatusCode::UNAUTHORIZED,
        EnformeError::SerializationError { .. } => StatusCode::BAD_REQUEST,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// The provider-side user ids named by a payload, first occurrence first.
///
/// One WHOOP payload can carry several events for the same user; each user
/// is synced once per payload, in the order the provider listed them.
fn distinct_owners(events: &[WebhookEvent]) -> Vec<String> {
    let mut owners: Vec<String> = Vec::with_capacity(events.len());
    for event in events {
        if !owners.contains(&event.user_id) {
            owners.push(event.user_id.clone());
        }
    }
    owners
}

/// Resolve the platform user owning a provider-side id.
///
/// The id is captured into `provider_user_id` when the token is stored, so it
/// resolves to exactly one `(user, tenant)`. An unknown owner is skipped and
/// logged — a push event is never broadcast to every connected user.
async fn resolve_owner(
    resources: &ServerContext,
    provider: &str,
    provider_user_id: &str,
) -> Option<(Uuid, String)> {
    match resources
        .common
        .repos
        .oauth_tokens
        .find_user_by_provider_user_id(provider, provider_user_id)
        .await
    {
        Ok(Some(owner)) => Some(owner),
        Ok(None) => {
            warn!(
                provider = %provider,
                provider_user_id = %provider_user_id,
                "webhook for an unknown provider user id — no linked user; skipping"
            );
            None
        }
        Err(e) => {
            error!(
                provider = %provider,
                provider_user_id = %provider_user_id,
                error = %e,
                "failed to resolve webhook owner to a user"
            );
            None
        }
    }
}

/// Stamp the provider's `last_sync` for the owner: the sync happened now.
async fn stamp_last_sync(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &str,
    provider: &str,
) {
    let tenant = match TenantId::parse_str(tenant_id) {
        Ok(tenant) => tenant,
        Err(e) => {
            warn!(user_id = %user_id, provider = %provider, error = %e, "webhook owner carries an unparseable tenant id");
            return;
        }
    };
    if let Err(e) = resources
        .common
        .repos
        .oauth_tokens
        .update_provider_last_sync(user_id, tenant, provider, Utc::now())
        .await
    {
        warn!(user_id = %user_id, provider = %provider, error = %e, "failed to stamp last_sync after webhook sync");
    }
}

/// Tell the owner's live SSE stream what a webhook sync landed.
async fn notify_owner(resources: &ServerContext, user_id: Uuid, provider: &str, message: String) {
    let notification = OAuthNotification {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider: provider.to_owned(),
        success: true,
        message,
        expires_at: None,
        created_at: Utc::now(),
        read_at: None,
    };
    // A user with no open SSE stream simply has nothing to notify.
    let _ = resources
        .sse
        .sse_manager
        .send_notification(user_id, &notification)
        .await;
}

/// Fetch the recent activities of the user whose Strava athlete id a webhook
/// named.
///
/// Goes through [`fetch_provider_head`]: it authenticates the OAuth provider
/// (refreshing the token when needed), fetches the window that opens
/// [`STRAVA_WEBHOOK_LOOKBACK_DAYS`] before the event, and writes the rows
/// through to the activity cache with the freshness mark. A successful fetch
/// stamps `last_sync`; the athlete is notified only when the window held at
/// least one activity.
///
/// Fetches go through [`STRAVA_OWNER_FETCH_GATE`]: an event arriving within
/// the debounce window of the owner's last fetch waits for one trailing fetch
/// at the window's end, or folds into the one already pending. A trailing
/// fetch still waiting when the drain signal fires is dropped; the next sync
/// picks its activity up.
async fn sync_strava_owner(resources: &Arc<ServerContext>, event: &StravaWebhookEvent) {
    let owner_id = event.owner_id.to_string();
    let Some((user_id, tenant_id)) = resolve_owner(resources, "strava", &owner_id).await else {
        return;
    };

    let drain = resources.common.turns.drain_token();
    if !STRAVA_OWNER_FETCH_GATE.wait_for_turn(user_id, drain).await {
        return;
    }

    let runtime: Arc<dyn ToolRuntime> = Arc::clone(resources) as Arc<dyn ToolRuntime>;
    let params = ActivityQueryParams {
        after: Some(event.fetch_window_start()),
        before: None,
        limit: Some(STRAVA_WEBHOOK_FETCH_LIMIT),
        offset: None,
    };

    match fetch_provider_head(&runtime, "strava", user_id, &tenant_id, &params).await {
        Ok(activities) => {
            let fetched = activities.len();
            info!(
                user_id = %user_id,
                strava_owner_id = %owner_id,
                object_id = %event.object_id,
                aspect_type = %event.aspect_type,
                fetched,
                "Strava webhook-triggered activity fetch completed"
            );
            stamp_last_sync(resources, user_id, &tenant_id, "strava").await;
            if fetched > 0 {
                notify_owner(
                    resources,
                    user_id,
                    "strava",
                    format!(
                        "Strava activity {}d: {fetched} recent activities fetched",
                        event.aspect_type
                    ),
                )
                .await;
            }
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                strava_owner_id = %owner_id,
                error = %e,
                auth_required = e.provider_auth_required_provider().is_some(),
                "Strava webhook-triggered activity fetch failed"
            );
        }
    }
}

/// Run the health sync of the user whose WHOOP id a validated event named.
///
/// The orchestrator syncs every data type the WHOOP provider supplies from
/// the user's cursors, so one call picks up whatever the event announced.
/// New records are landed by [`land_whoop_records`]; a sync that found none
/// or failed is logged and leaves `last_sync` and the SSE stream alone.
async fn sync_whoop_owner(
    resources: &Arc<ServerContext>,
    orchestrator: &SyncOrchestrator,
    provider_user_id: &str,
) {
    let Some((user_id, tenant_id)) = resolve_owner(resources, "whoop", provider_user_id).await
    else {
        return;
    };

    match orchestrator.sync_user(&user_id.to_string(), "whoop").await {
        Ok(result) if result.records_created > 0 => {
            land_whoop_records(
                resources,
                user_id,
                &tenant_id,
                provider_user_id,
                result.records_created,
            )
            .await;
        }
        Ok(result) => {
            info!(
                user_id = %user_id,
                whoop_user_id = %provider_user_id,
                errors = result.records_errored,
                "WHOOP webhook-triggered sync found no new records"
            );
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                whoop_user_id = %provider_user_id,
                error = %e,
                "WHOOP webhook-triggered sync failed"
            );
        }
    }
}

/// Record that a WHOOP webhook sync created `records` for the owner: stamp
/// `last_sync` and tell the athlete's SSE stream how many landed.
async fn land_whoop_records(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &str,
    provider_user_id: &str,
    records: u32,
) {
    info!(
        user_id = %user_id,
        whoop_user_id = %provider_user_id,
        records,
        "WHOOP webhook-triggered sync completed"
    );
    stamp_last_sync(resources, user_id, tenant_id, "whoop").await;
    notify_owner(
        resources,
        user_id,
        "whoop",
        format!("WHOOP data synced ({records} records)"),
    )
    .await;
}

/// Strava webhook event payload.
///
/// Strava sends minimal event data — only IDs and event type.
/// The activities themselves are fetched separately via the API.
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
    /// The subscription the event was delivered for; must be one of
    /// `strava_webhook_gate::registered_subscription_ids`
    subscription_id: u64,
    /// Unix timestamp of the event
    event_time: u64,
}

impl StravaWebhookEvent {
    /// Whether the event announces an activity that now exists on Strava.
    ///
    /// Only an activity create or update has something to fetch; a delete
    /// and every athlete event (deauthorization) are acknowledged and left
    /// alone.
    fn is_activity_write(&self) -> bool {
        self.object_type == "activity"
            && (self.aspect_type == "create" || self.aspect_type == "update")
    }

    /// Unix timestamp where the fetch window for this event opens.
    ///
    /// [`STRAVA_WEBHOOK_LOOKBACK_DAYS`] before the event time, so an
    /// activity uploaded late is still inside the window. An event time the
    /// clock cannot represent falls back to now.
    fn fetch_window_start(&self) -> i64 {
        let event_time = i64::try_from(self.event_time).unwrap_or_else(|_| Utc::now().timestamp());
        event_time - STRAVA_WEBHOOK_LOOKBACK_DAYS * 86_400
    }
}
