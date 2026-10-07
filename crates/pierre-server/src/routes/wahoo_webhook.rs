// ABOUTME: Wahoo workout webhook — check the body's token, resolve the owner, fetch their recent workouts into the cache
// ABOUTME: workout_summary is Wahoo's only event; retries and duplicates land idempotently on the activity cache
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Wahoo workout webhook
//!
//! Wahoo posts a `workout_summary` event to the URL set in its developer
//! portal (`https://app.dravr.ai/webhooks/wahoo/workouts`) whenever an
//! athlete who granted `offline_data` finishes a workout. The portal's
//! `webhook_token` arrives as a field of the JSON body, not as a signature
//! header: a request without the configured token is refused.
//!
//! The path is a sub-path because the billing router claims the single
//! segment `/webhooks/{provider}`. Wahoo retries a non-200 after 30 minutes,
//! 4, 24 and 72 hours, and can deliver an event twice: the fetch writes
//! through the activity cache keyed by workout id, so a duplicate lands the
//! same rows again, and the per-owner gate folds a burst into one fetch.
//!
//! Wahoo sends no deauthorization event; a revocation surfaces as a refused
//! refresh, which the platform's refresh turns into a disconnect
//! (`ProviderDescriptor::refused_refresh_is_revocation`).

use std::sync::{Arc, LazyLock};

use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::Value;
use tracing::{info, warn};

use pierre_core::constant_time::{configured_secret, matches_configured_secret};
use pierre_core::constants::oauth::WAHOO;
use pierre_providers::core::ActivityQueryParams;
use pierre_services::webhook_owner::WebhookOwner;
use pierre_tool_runtime::activity_fetch::fetch_provider_head;
use pierre_tool_runtime::runtime::ToolRuntime;

use super::strava_webhook_gate::{OwnerFetchGate, OWNER_FETCH_WINDOW};
use super::webhooks::{notify_owner, report_sync, resolve_owner, stamp_last_sync};
use crate::mcp::resources::ServerContext;

/// The variable holding the portal's `webhook_token`.
pub const WAHOO_WEBHOOK_TOKEN_ENV: &str = "WAHOO_WEBHOOK_TOKEN";

/// The only event Wahoo sends.
const WORKOUT_SUMMARY: &str = "workout_summary";

/// How far before the workout's start the fetch window opens: the summary
/// can arrive long after the ride (a device that syncs the next morning),
/// and the window is keyed on the workout, not on the delivery.
const WAHOO_WEBHOOK_LOOKBACK_HOURS: i64 = 24;

/// How far back the window opens when the event names no readable start.
const WAHOO_WEBHOOK_FALLBACK_DAYS: i64 = 7;

/// Cap on rows a webhook-triggered fetch reads: the window is a day or a
/// week, never hundreds of workouts.
const WAHOO_WEBHOOK_FETCH_LIMIT: usize = 50;

/// One fetch per owner per window, as Strava's events get: a burst of
/// summaries (or a retried duplicate) folds into one trailing fetch.
static WAHOO_OWNER_FETCH_GATE: LazyLock<OwnerFetchGate> =
    LazyLock::new(|| OwnerFetchGate::new(OWNER_FETCH_WINDOW));

/// Wahoo's workout webhook payload, as much of it as the handler reads: the
/// rest of the summary is fetched from the API rather than trusted from the
/// push.
#[derive(Debug, Deserialize)]
struct WahooWebhookEvent {
    /// `workout_summary`.
    event_type: String,
    /// The portal's token. Read as text or as a number, in case the portal
    /// value is all digits.
    #[serde(default)]
    webhook_token: Option<Value>,
    /// The Wahoo user the workout belongs to.
    #[serde(default)]
    user: Option<WahooWebhookUser>,
    /// The completed workout's summary.
    #[serde(default)]
    workout_summary: Option<WahooWebhookSummary>,
}

/// The `user` of a Wahoo event.
#[derive(Debug, Deserialize)]
struct WahooWebhookUser {
    id: i64,
}

/// The `workout_summary` of a Wahoo event: only its workout is read.
#[derive(Debug, Deserialize)]
struct WahooWebhookSummary {
    #[serde(default)]
    workout: Option<WahooWebhookWorkout>,
}

/// The workout a summary belongs to.
#[derive(Debug, Deserialize)]
struct WahooWebhookWorkout {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    starts: Option<String>,
}

impl WahooWebhookEvent {
    /// The token the request presented, as text; empty when it carried none.
    fn presented_token(&self) -> String {
        match &self.webhook_token {
            Some(Value::String(token)) => token.clone(),
            Some(Value::Number(token)) => token.to_string(),
            _ => String::new(),
        }
    }

    /// The workout's start, when the event names a readable one.
    fn workout_start(&self) -> Option<DateTime<Utc>> {
        let starts = self
            .workout_summary
            .as_ref()?
            .workout
            .as_ref()?
            .starts
            .as_deref()?;
        DateTime::parse_from_rfc3339(starts)
            .ok()
            .map(|at| at.with_timezone(&Utc))
    }

    /// The workout id, for the log line.
    fn workout_id(&self) -> Option<i64> {
        self.workout_summary.as_ref()?.workout.as_ref()?.id
    }

    /// Unix timestamp where the fetch window opens.
    fn fetch_window_start(&self, now: DateTime<Utc>) -> i64 {
        self.workout_start()
            .map_or_else(
                || now - Duration::days(WAHOO_WEBHOOK_FALLBACK_DAYS),
                |start| start - Duration::hours(WAHOO_WEBHOOK_LOOKBACK_HOURS),
            )
            .timestamp()
    }
}

/// Whether a request presents the configured token, or the status that
/// refuses it: 503 while `WAHOO_WEBHOOK_TOKEN` is unset or blank (a blank
/// expected value would let anyone in, and Wahoo retries once it is set),
/// 401 for a token that does not match.
fn token_refusal(event: &WahooWebhookEvent) -> Option<StatusCode> {
    let Some(expected) = configured_secret(WAHOO_WEBHOOK_TOKEN_ENV) else {
        warn!("Wahoo webhook refused: WAHOO_WEBHOOK_TOKEN is unset or blank; nothing synced");
        return Some(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !matches_configured_secret(expected.as_bytes(), event.presented_token().as_bytes()) {
        warn!("Wahoo webhook refused: the body's webhook_token does not match");
        return Some(StatusCode::UNAUTHORIZED);
    }
    None
}

/// The event a body carries and the Wahoo user it names, or the status the
/// request is answered with instead: 400 for a body that is not a Wahoo
/// event, the token refusal, and 200 for an event other than
/// `workout_summary` or one naming no user — nothing a retry would fix.
fn accepted_event(body: &[u8]) -> Result<(WahooWebhookEvent, String), StatusCode> {
    let event: WahooWebhookEvent = serde_json::from_slice(body).map_err(|e| {
        warn!(error = %e, "Failed to parse Wahoo webhook payload");
        StatusCode::BAD_REQUEST
    })?;
    if let Some(refusal) = token_refusal(&event) {
        return Err(refusal);
    }
    if event.event_type != WORKOUT_SUMMARY {
        info!(event_type = %event.event_type, "Wahoo webhook event other than a workout summary; ignored");
        return Err(StatusCode::OK);
    }
    let Some(owner_id) = event.user.as_ref().map(|user| user.id.to_string()) else {
        warn!("Wahoo workout summary names no user; nothing to sync");
        return Err(StatusCode::OK);
    };
    Ok((event, owner_id))
}

/// `POST /webhooks/wahoo/workouts`.
///
/// Answers Wahoo at once ([`accepted_event`] decides refusals) and fetches the
/// owner's recent workouts on the drain-tracked spawner, so a SIGTERM waits
/// for it.
pub(crate) fn wahoo_event(resources: &Arc<ServerContext>, body: &[u8]) -> StatusCode {
    let (event, owner_id) = match accepted_event(body) {
        Ok(accepted) => accepted,
        Err(status) => return status,
    };
    info!(
        wahoo_user_id = %owner_id,
        workout_id = ?event.workout_id(),
        "Received Wahoo workout summary"
    );
    let after = event.fetch_window_start(Utc::now());
    let resources = Arc::clone(resources);
    resources.common.turns.clone().spawn(Box::pin(async move {
        sync_wahoo_owner(&resources, &owner_id, after).await;
    }));
    StatusCode::OK
}

/// Fetch the recent workouts of the athlete a Wahoo event named, from
/// `after` on, through [`fetch_provider_head`]: it authenticates the provider
/// (refreshing the token right before the call), and writes the activities
/// through to the cache with the freshness mark. A landed fetch stamps
/// `last_sync` and re-arms the sync-failure notice; the athlete's live stream
/// hears about it when the window held a workout.
async fn sync_wahoo_owner(resources: &Arc<ServerContext>, owner_id: &str, after: i64) {
    let Some(WebhookOwner {
        user_id, tenant_id, ..
    }) = resolve_owner(resources, WAHOO, owner_id).await
    else {
        return;
    };
    let drain = resources.common.turns.drain_token();
    if !WAHOO_OWNER_FETCH_GATE.wait_for_turn(user_id, drain).await {
        return;
    }
    let runtime: Arc<dyn ToolRuntime> = Arc::clone(resources) as Arc<dyn ToolRuntime>;
    let params = ActivityQueryParams {
        after: Some(after),
        before: None,
        limit: Some(WAHOO_WEBHOOK_FETCH_LIMIT),
        offset: None,
    };
    match fetch_provider_head(&runtime, WAHOO, user_id, &tenant_id, &params).await {
        Ok(activities) => {
            let fetched = activities.len();
            info!(user_id = %user_id, fetched, "Wahoo webhook-triggered workout fetch completed");
            stamp_last_sync(resources, user_id, &tenant_id, WAHOO).await;
            report_sync(resources, user_id, &tenant_id, WAHOO, true).await;
            if fetched > 0 {
                notify_owner(
                    resources,
                    user_id,
                    WAHOO,
                    format!("Wahoo workout synced: {fetched} recent activities fetched"),
                )
                .await;
            }
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                error = %e,
                auth_required = e.provider_auth_required_provider().is_some(),
                "Wahoo webhook-triggered workout fetch failed"
            );
            // A dead credential is the disconnect notice's to tell about.
            if e.provider_auth_required_provider().is_none() {
                report_sync(resources, user_id, &tenant_id, WAHOO, false).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(json: &str) -> WahooWebhookEvent {
        serde_json::from_str(json).unwrap_or_else(|e| panic!("fixture: {e}"))
    }

    /// The Cloud API reference's sample, with a token in place.
    const SAMPLE: &str = r#"{
        "event_type": "workout_summary",
        "webhook_token": "portal-secret",
        "user": { "id": 60462 },
        "workout_summary": {
            "id": 8297, "ascent_accum": "450.00", "power_avg": "94.59",
            "file": { "url": "https://cdn.wahooligan.com/x.fit" },
            "workout": {
                "id": 56519, "starts": "2015-08-12T09:00:00.000Z", "minutes": 12,
                "name": "Friday Fun", "plan_id": null, "workout_token": "123",
                "workout_type_id": 40
            }
        }
    }"#;

    #[test]
    fn the_documented_sample_reads_its_owner_workout_and_token() {
        let sample = event(SAMPLE);
        assert_eq!(sample.event_type, WORKOUT_SUMMARY);
        assert_eq!(sample.user.as_ref().map(|user| user.id), Some(60_462));
        assert_eq!(sample.workout_id(), Some(56_519));
        assert_eq!(sample.presented_token(), "portal-secret");
    }

    #[test]
    fn the_window_opens_a_day_before_the_workout_started() {
        let sample = event(SAMPLE);
        let start = DateTime::parse_from_rfc3339("2015-08-12T09:00:00.000Z")
            .map(|at| at.timestamp())
            .unwrap_or_default();
        assert_eq!(sample.fetch_window_start(Utc::now()), start - 86_400);
    }

    #[test]
    fn an_event_without_a_start_falls_back_to_a_week() {
        let bare = event(r#"{"event_type": "workout_summary", "user": {"id": 1}}"#);
        let now = Utc::now();
        assert_eq!(
            bare.fetch_window_start(now),
            (now - Duration::days(WAHOO_WEBHOOK_FALLBACK_DAYS)).timestamp()
        );
        assert_eq!(bare.presented_token(), "", "no token presents nothing");
    }

    #[test]
    fn a_numeric_portal_token_reads_as_its_digits() {
        let numeric = event(r#"{"event_type": "workout_summary", "webhook_token": 123456}"#);
        assert_eq!(numeric.presented_token(), "123456");
    }
}
