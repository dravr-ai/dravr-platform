// ABOUTME: Activity reads elect a provider by capability before recency — a detector never outranks a recorder
// ABOUTME: Drives the real election chain get_activities and the training-history computes resolve through
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#133. The provider that answers an activity question nobody pinned
//! to a provider used to be elected by health and recency alone, so a WHOOP
//! connection added after Strava was primary for `get_activities` simply for
//! being newest — the 2026-08-22 incident served a 200 km ride as a
//! distance-less "run" from it. WHOOP's descriptor declares activities but
//! not `RECORDED_ACTIVITIES` (its workouts are detected from heart rate), and
//! the election now ranks on that after health and before recency.
//!
//! Each test drives a production entry point, not the election helper: the
//! chain `get_activities` resolves its provider through
//! (`resolve_provider_for_tool`). The training-history computes elect nothing
//! (carnet#836): they read every connection (`resolve_compute_backends`), so
//! these tests pin only that each connection carries its own health.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pierre_core::models::{ConnectionType, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::training_history_read::{resolve_compute_backends, ComputeBackend};
use pierre_tool_runtime::context::{AuthMethod, ToolExecutionContext};
use pierre_tool_runtime::protocol::provider_helpers::resolve_provider_for_tool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use tokio::time::sleep;
use uuid::Uuid;

mod common;

struct Athlete {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
}

async fn athlete() -> Athlete {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, user) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .expect("user has a tenant")
        .id;
    Athlete {
        resources,
        user_id,
        tenant,
    }
}

impl Athlete {
    /// Connect `provider` after a short pause, so registration order is
    /// visible to the `connected_at DESC` tiebreak.
    async fn connect(&self, provider: &str) {
        sleep(Duration::from_millis(20)).await;
        self.resources
            .common
            .repos
            .provider_connections
            .register_connection(
                self.user_id,
                self.tenant,
                provider,
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
    }

    async fn touch(&self, provider: &str) {
        sleep(Duration::from_millis(20)).await;
        self.resources
            .common
            .repos
            .provider_connections
            .touch_last_used(self.user_id, self.tenant, provider)
            .await
            .unwrap();
    }

    /// The connections a training-history compute reads.
    async fn compute_backends(&self) -> Vec<ComputeBackend> {
        resolve_compute_backends(&self.resources.common.repos, self.tenant, self.user_id)
            .await
            .unwrap()
    }

    /// The provider an unpinned `get_activities` call is served by.
    async fn activity_provider(&self) -> String {
        let runtime: Arc<dyn ToolRuntime> = self.resources.clone();
        let context = ToolExecutionContext::new(
            self.user_id,
            Some(self.tenant),
            runtime,
            AuthMethod::JwtBearer,
        );
        resolve_provider_for_tool(&json!({}), &context)
            .await
            .expect("the athlete has connections")
    }
}

/// The incident shape: Strava first, WHOOP connected after it AND used last,
/// both healthy. Recency elected WHOOP; capability elects Strava.
#[tokio::test]
async fn a_detector_connected_and_used_last_does_not_answer_activity_reads() {
    let athlete = athlete().await;
    athlete.connect("strava").await;
    athlete.connect("whoop").await;
    athlete.touch("whoop").await;

    assert_eq!(
        athlete.activity_provider().await,
        "strava",
        "a heart-rate detector must not be primary for activities over a recording source"
    );

    let backends = athlete.compute_backends().await;
    assert!(
        backends
            .iter()
            .any(|b| b.slug == "strava" && !b.requires_reauth),
        "training-history computes read the healthy recording source: {backends:?}"
    );
}

/// Health still ranks first: a dead recorder never shadows a healthy
/// connection, and among the healthy ones a recorder beats a detector even
/// when the detector is newer. `get_activities` serves no dead primary's own
/// cache, so electing the dead Strava here would blank the turn; electing the
/// healthy detector instead would be the 2026-08-22 incident again.
#[tokio::test]
async fn a_dead_recorder_yields_to_the_healthy_recorder_not_the_newer_detector() {
    let athlete = athlete().await;
    athlete.connect("sciotte_garmin").await;
    athlete.connect("strava").await;
    athlete.connect("whoop").await;
    athlete.touch("strava").await;
    athlete
        .resources
        .common
        .repos
        .provider_connections
        .mark_needs_reauth(
            athlete.user_id,
            athlete.tenant,
            "strava",
            Some("invalid_grant"),
            Utc::now(),
        )
        .await
        .unwrap();

    assert_eq!(
        athlete.activity_provider().await,
        "sciotte_garmin",
        "the healthy recorder answers — not the dead one used last, not the newer detector"
    );
    let backends = athlete.compute_backends().await;
    assert!(
        backends
            .iter()
            .any(|b| b.slug == "strava" && b.requires_reauth),
        "the lapsed Strava is still read, and marked for its reconnect: {backends:?}"
    );
    assert!(
        backends
            .iter()
            .any(|b| b.slug == "sciotte_garmin" && !b.requires_reauth),
        "the healthy recorder is read beside it: {backends:?}"
    );
    assert_eq!(
        backends.first().map(|b| b.requires_reauth),
        Some(false),
        "healthy connections come first"
    );
}

/// Among providers that serve activities equally well, health and recency
/// still decide — capability reorders ranks, never within one.
#[tokio::test]
async fn between_two_recording_sources_health_and_recency_still_decide() {
    let athlete = athlete().await;
    athlete.connect("strava").await;
    athlete.connect("sciotte_garmin").await;
    athlete.connect("whoop").await;
    athlete.touch("strava").await;
    athlete.touch("whoop").await;

    assert_eq!(
        athlete.activity_provider().await,
        "strava",
        "the most recently used recorder wins; the detector used after it does not"
    );

    athlete
        .resources
        .common
        .repos
        .provider_connections
        .mark_needs_reauth(
            athlete.user_id,
            athlete.tenant,
            "strava",
            Some("invalid_grant"),
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        athlete.activity_provider().await,
        "sciotte_garmin",
        "a healthy recorder outranks a dead one of the same rank"
    );
}

/// An athlete whose only connection detects workouts still gets it: it is
/// the only answer there is, and electing nothing would send them to the
/// "connect a provider" flow they have already been through.
#[tokio::test]
async fn a_lone_detector_still_answers() {
    let athlete = athlete().await;
    athlete.connect("whoop").await;

    assert_eq!(athlete.activity_provider().await, "whoop");
}
