// ABOUTME: Under sciotte first the capture sweep refreshes the usr/pwd session and passes over the grant beside it
// ABOUTME: Pins the background share on the grant's fallback read, and that a webhook or a first read never scrapes

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte first, the capture sweep and the exact reads (carnet#1494).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A Strava athlete holding both a usr/pwd session (`sciotte`) and an OAuth
// grant (`strava`) is read through the session; the grant only answers when
// the session cannot. The sweep therefore refreshes the session and passes
// over the grant, so the API is never read beside the scrape, and a group
// snapshot walks the athlete's connections the same way. A session read
// that fails falls back to the grant, and that fallback read is still held
// to the background share of the signing app's budget (carnet#1491). The
// Strava webhook and the connect-time first read name the grant exactly:
// a pushed event or a fresh connect never starts a scrape.
//
// The scraper and Strava are loopback stand-ins (test doubles, per the
// repo's mock rule) that count every activity-list read they answer, so
// what reached each backend is counted, not inferred.
#![cfg(all(
    feature = "provider-strava",
    feature = "provider-sciotte",
    feature = "health-sync",
    feature = "client-messaging"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration, Utc};
use pierre_config::environment::ServerConfig;
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use pierre_database::repositories::{ActivityBackfillJobRow, BackfillJobKind, JobProgress};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::webhooks::WebhookRoutes;
use pierre_mcp_server::services::first_sync_post::FirstSyncPoster;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::request_budget::{
    budget_counter_key, budget_period, ProviderRateLimiter, BACKGROUND_SHARE_PERCENT, ONE_DAY,
    PLATFORM_SCOPE,
};
use pierre_routes_auth::FirstSyncHook;
use pierre_tool_runtime::capture_sweep::{
    refresh_captures, ConnectionRefresh, RefreshOutcome, SweepBudget, SweepSelection,
};
use pierre_tool_runtime::commitment_refresh::spawn_commitment_activity_refresh;
use pierre_tool_runtime::group_fitness::fetch_member_snapshots;
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::SecretString;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;
use crate::helpers::notify_capture::{capture_logs, named};
use crate::helpers::sciotte_mock::seed_sciotte_session;
use crate::messaging_fixtures::{CapturingChannel, FakeResolver};
use crate::recording_post::RecordingPost;

/// The Strava app the test server signs with.
const APP: &str = "test_client";
/// The app's day in this suite: small, so its background share is a count.
const DAY_BUDGET: u32 = 8;
/// The Strava athlete id the grant carries and the webhook events name.
const OWNER_ID: u64 = 5_151_515;
/// The subscription id the test deployment registered with Strava.
const SUBSCRIPTION_ID: u64 = 91;

/// A loopback backend that counts the activity-list reads it answers, and
/// answers `503` while `failing` is set.
#[derive(Default)]
struct Backend {
    reads: AtomicUsize,
    failing: AtomicBool,
}

impl Backend {
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

/// Serve `app` on loopback; the base URL to reach it at.
async fn serve(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A stand-in for the sciotte scraper over a Strava account holding one
/// ride from yesterday, its capture having reached the list head.
async fn spawn_scraper() -> (String, Arc<Backend>) {
    let backend = Arc::new(Backend::default());
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": "cap-verified-session" })) }),
        )
        .route(
            "/api/athlete",
            get(|| async { Json(json!({ "display_name": "Sweep Tester" })) }),
        )
        .route(
            "/api/activities",
            get(|State(backend): State<Arc<Backend>>| async move {
                backend.reads.fetch_add(1, Ordering::SeqCst);
                if backend.failing.load(Ordering::SeqCst) {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({ "error": "unavailable", "message": "scraper busy" })),
                    );
                }
                (
                    StatusCode::OK,
                    Json(json!({
                        "count": 1,
                        "activities": [{
                            "id": "15550001111",
                            "name": "Scraped ride",
                            "sport_type": "ride",
                            "start_date": (Utc::now() - Duration::days(1)).to_rfc3339(),
                            "duration_seconds": 3600,
                            "provider": "strava",
                            "distance_meters": 30000.0
                        }],
                        "head_complete": true
                    })),
                )
            }),
        )
        .with_state(Arc::clone(&backend));
    (serve(app).await, backend)
}

/// A stand-in for Strava's API whose `/athlete/activities` lists one ride.
async fn spawn_strava() -> (String, Arc<Backend>) {
    let backend = Arc::new(Backend::default());
    let app = Router::new()
        .route(
            "/athlete/activities",
            get(|State(backend): State<Arc<Backend>>| async move {
                backend.reads.fetch_add(1, Ordering::SeqCst);
                Json(json!([{
                    "id": 9_002,
                    "name": "API ride",
                    "type": "Ride",
                    "sport_type": "Ride",
                    "start_date": (Utc::now() - Duration::hours(3)).to_rfc3339(),
                    "distance": 32_000.0,
                    "elapsed_time": 4_200,
                    "total_elevation_gain": 310.0
                }]))
            }),
        )
        .with_state(Arc::clone(&backend));
    (serve(app).await, backend)
}

/// The scraper, Strava, and a server reaching both, whose Strava app is
/// [`APP`] and whose Strava webhook subscription is [`SUBSCRIPTION_ID`].
struct World {
    resources: Arc<ServerContext>,
    scraper: Arc<Backend>,
    strava: Arc<Backend>,
}

async fn world() -> World {
    let (scraper_url, scraper) = spawn_scraper().await;
    let (strava_url, strava) = spawn_strava().await;
    let resources =
        common::create_test_server_resources_with_config(config(&scraper_url, &strava_url))
            .await
            .unwrap();
    World {
        resources,
        scraper,
        strava,
    }
}

fn config(scraper_url: &str, strava_url: &str) -> ServerConfig {
    let mut config = common::test_server_config_with_sciotte(scraper_url);
    let api = common::test_server_config_with_api_base("strava", strava_url);
    config.oauth.endpoints = api.oauth.endpoints;
    common::set_server_app(&mut config, "strava", APP, "test_secret");
    config.webhooks.strava_subscription_ids = vec![SUBSCRIPTION_ID];
    config
}

/// A new athlete holding a live Strava grant — token and active connection —
/// and a live usr/pwd session beside it.
async fn athlete_with_both(resources: &Arc<ServerContext>) -> (Uuid, TenantId) {
    let (user_id, _user, tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("sciotte-first-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    let now = Utc::now();
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant.to_string(),
            provider: "strava".to_owned(),
            // >= 40 chars and not "at_"-prefixed: the provider's token validation.
            access_token: SecretString::from(format!("strava_access_token_{:0>30}", 7)),
            refresh_token: Some(SecretString::from("strava_refresh")),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + Duration::hours(6)),
            scope: Some("activity:read_all".to_owned()),
            provider_user_id: Some(OWNER_ID.to_string()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    seed_sciotte_session(resources, user_id, tenant).await;
    for (provider, kind) in [
        ("strava", ConnectionType::OAuth),
        ("sciotte", ConnectionType::Manual),
    ] {
        repos
            .provider_connections
            .register_connection(user_id, tenant, provider, &kind, None)
            .await
            .unwrap();
    }
    (user_id, tenant)
}

/// The sweep's lines for `user_id`.
fn lines_of(connections: &[ConnectionRefresh], user_id: Uuid) -> Vec<&ConnectionRefresh> {
    connections
        .iter()
        .filter(|line| line.user_id == user_id.to_string())
        .collect()
}

/// The sweep refreshes the session, the provider's primary, by its own read,
/// and passes over the grant beside it: Strava's API is never read, and only
/// the session's line is in the report.
#[tokio::test]
async fn the_sweep_refreshes_the_session_and_never_reads_the_grant() {
    let w = world().await;
    let (user_id, _tenant) = athlete_with_both(&w.resources).await;
    let runtime: Arc<dyn ToolRuntime> = w.resources.clone();

    let report = refresh_captures(&runtime, SweepBudget::default(), SweepSelection::default())
        .await
        .expect("refresh report");

    let lines = lines_of(&report.connections, user_id);
    assert_eq!(
        lines.len(),
        1,
        "bug: the sweep walks the session alone: {lines:?}"
    );
    assert_eq!(lines[0].provider, "sciotte", "{lines:?}");
    assert!(
        matches!(
            lines[0].outcome,
            RefreshOutcome::Refreshed { activities: 1 }
        ),
        "bug: the session's own read refreshes it: {:?}",
        lines[0].outcome
    );
    assert_eq!(w.scraper.reads(), 1, "the session is read once");
    assert_eq!(
        w.strava.reads(),
        0,
        "bug: the sweep read Strava's API beside the scrape"
    );
    assert!(
        report.passed_over >= 1,
        "bug: the grant beside the session is counted as passed over"
    );
}

/// A session read the sweep makes that fails falls back to the grant, and
/// that read stays inside the background share of the app's day: with the
/// share free the grant answers once, and with the share spent the sweep
/// sends Strava nothing and records no failed sync on the grant, which
/// would pause the reads athletes wait on (carnet#1491).
#[tokio::test]
async fn the_sweeps_fallback_to_the_grant_keeps_to_the_background_share() {
    let w = world().await;
    let repos = &w.resources.common.repos;
    let limiter = ProviderRateLimiter::new(
        Arc::clone(&repos.usage_counters),
        Arc::clone(&repos.provider_connections),
    );
    limiter.set_budgets("strava", &[(DAY_BUDGET, ONE_DAY)]);
    let mut context = (*w.resources).clone();
    context.fitness.provider_registry = Arc::new(
        ProviderRegistry::new()
            .with_endpoint_overrides(&w.resources.common.config.oauth.endpoints)
            .with_sciotte_client(
                w.resources
                    .fitness
                    .provider_registry
                    .sciotte_client()
                    .cloned(),
            )
            .with_request_limiter(Arc::new(limiter)),
    );
    let runtime: Arc<dyn ToolRuntime> = Arc::new(context);
    let (user_id, tenant) = athlete_with_both(&w.resources).await;
    w.scraper.failing.store(true, Ordering::SeqCst);

    refresh_captures(&runtime, SweepBudget::default(), SweepSelection::default())
        .await
        .expect("refresh report");
    assert_eq!(w.scraper.reads(), 1, "the session is read first");
    assert_eq!(
        w.strava.reads(),
        1,
        "bug: a failed session read falls back to the grant once"
    );

    repos
        .usage_counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &budget_counter_key("strava", APP),
            &budget_period(Utc::now(), ONE_DAY),
            i64::from(DAY_BUDGET * BACKGROUND_SHARE_PERCENT / 100),
        )
        .await
        .unwrap();
    refresh_captures(&runtime, SweepBudget::default(), SweepSelection::default())
        .await
        .expect("refresh report");
    assert_eq!(w.scraper.reads(), 2, "the session is read first again");
    assert_eq!(
        w.strava.reads(),
        1,
        "bug: the sweep's fallback read Strava past the background share"
    );
    assert_eq!(
        repos
            .activity_cache
            .latest_activity_fetch_failure(user_id, &tenant, "strava")
            .await
            .unwrap(),
        None,
        "bug: a refusal of the sweep's share was recorded as the grant's failed sync"
    );
}

/// A Strava webhook reads the grant it was pushed for, exactly: the event
/// lands through Strava's API and never starts a scrape of the session
/// beside it.
#[tokio::test]
async fn a_strava_webhook_reads_the_grant_and_never_starts_a_scrape() {
    let w = world().await;
    athlete_with_both(&w.resources).await;
    let event = json!({
        "object_type": "activity",
        "object_id": 9_002,
        "aspect_type": "create",
        "owner_id": OWNER_ID,
        "subscription_id": SUBSCRIPTION_ID,
        "event_time": Utc::now().timestamp()
    });

    let response = WebhookRoutes::routes(Arc::clone(&w.resources))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhooks/strava")
                .header("content-type", "application/json")
                .body(Body::from(event.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "bug: the Strava webhook was refused"
    );
    let deadline = Instant::now() + StdDuration::from_secs(15);
    while w.strava.reads() == 0 || !w.resources.common.turns.is_empty() {
        assert!(
            Instant::now() < deadline,
            "the webhook's fetch never reached Strava"
        );
        sleep(StdDuration::from_millis(25)).await;
    }

    assert_eq!(w.strava.reads(), 1, "the event is read through the grant");
    assert_eq!(
        w.scraper.reads(),
        0,
        "bug: a pushed event started a scrape of the session"
    );
}

/// Records the pair's history walk as owed under the session and leased for
/// an hour, so no ask starts another and the resume sweep leaves it alone.
async fn pair_walk_owed(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let now = Utc::now().timestamp_millis();
    let recorded = resources
        .common
        .repos
        .activity_backfill_jobs
        .record_backfill_job(&ActivityBackfillJobRow {
            id: Uuid::new_v4().to_string(),
            tenant_id: tenant,
            user_id,
            provider: "sciotte".to_owned(),
            kind: BackfillJobKind::HistoryWalk,
            after_ts: None,
            before_ts: None,
            fetch_limit: None,
            conversation_id: None,
            created_at_ms: now,
            leased_until_ms: now + 60 * 60_000,
            attempts: 0,
            lease_token: Some(Uuid::new_v4().to_string()),
            progress: JobProgress::default(),
            turn_id: None,
            asked_message_id: None,
            walk_backend: None,
        })
        .await
        .unwrap();
    assert!(recorded, "bug: the pair's walk could not be recorded");
}

/// The connect-time first read names the backend just connected exactly:
/// a Strava grant's first read goes to Strava's API, never to the session
/// standing beside it. The history walk the read's depth ask may start does
/// read the session first (carnet#1482, JF 2026-11-12), so the pair's walk is
/// already owed here and every scrape counted is the first read's own.
#[tokio::test]
async fn a_first_read_of_the_grant_never_starts_a_scrape() {
    let w = world().await;
    let (user_id, tenant) = athlete_with_both(&w.resources).await;
    pair_walk_owed(&w.resources, user_id, tenant).await;
    let channel = Arc::new(CapturingChannel::for_channel(ChannelType::WhatsApp));
    let poster = FirstSyncPoster::new(
        Arc::downgrade(&w.resources),
        Arc::new(RecordingPost::posting(true)),
        Arc::new(FakeResolver::new(channel)),
    );

    poster.connected_on_demand(user_id, "strava").await;

    assert_eq!(w.strava.reads(), 1, "the first read goes to the grant");
    assert_eq!(
        w.scraper.reads(),
        0,
        "bug: a first read of the grant started a scrape of the session"
    );
}

/// A group snapshot of a member holding both walks the session alone: the
/// grant's name resolves to the session, so reading it too scraped the
/// session twice and wrote the scraped rows under the grant's key. The
/// session is scraped once, Strava's API is never read, and nothing is
/// cached under `strava`.
#[tokio::test]
async fn a_group_snapshot_scrapes_the_session_once_and_never_writes_it_under_the_grant() {
    let w = world().await;
    let (user_id, tenant) = athlete_with_both(&w.resources).await;
    let runtime: Arc<dyn ToolRuntime> = w.resources.clone();

    let snapshots = fetch_member_snapshots(&runtime, &[user_id], tenant).await;

    assert_eq!(
        snapshots.len(),
        1,
        "bug: the group read did not snapshot the one member"
    );
    assert_eq!(
        snapshots[0].recent_activities.len(),
        1,
        "the scraped ride reaches the snapshot once: {:?}",
        snapshots[0].recent_activities
    );
    assert_eq!(
        w.scraper.reads(),
        1,
        "bug: the snapshot scraped the session once per half of the pair"
    );
    assert_eq!(
        w.strava.reads(),
        0,
        "bug: the snapshot read Strava's API beside the scrape"
    );
    let cache = &w.resources.common.repos.activity_cache;
    assert!(
        cache
            .latest_activity_sync(user_id, &tenant, "sciotte")
            .await
            .unwrap()
            .is_some(),
        "the session's rows are written under the session"
    );
    assert!(
        cache
            .latest_activity_sync(user_id, &tenant, "strava")
            .await
            .unwrap()
            .is_none(),
        "bug: scraped rows were written under the grant's key"
    );
}

/// A coach's read of a group member holding both reads the session alone:
/// the grant's name resolves to the session, so reading it too scraped the
/// session a second time and wrote the scraped rows under the grant's key.
/// The session is scraped once per call, Strava's API is never read, and
/// nothing is cached under `strava`.
#[cfg(feature = "tools-groups")]
#[tokio::test]
async fn a_group_member_read_scrapes_the_session_once_and_never_writes_it_under_the_grant() {
    use dravr_tronc::mcp::tool::{McpTool, ToolContext};
    use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
    use pierre_core::models::groups::{
        CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
    };
    use pierre_tool_runtime::implementations::groups::GetGroupMemberActivitiesTool;

    let w = world().await;
    let (peer, peer_tenant) = athlete_with_both(&w.resources).await;
    let repos = &w.resources.common.repos;
    let (requester, _user, host_tenant) = common::create_test_user_with_plan(
        &w.resources.agent.database,
        &format!("group-reader-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    repos
        .users
        .update_display_name(peer, "Raph Both")
        .await
        .unwrap();
    let agent = repos
        .agents
        .create_system_agent(
            requester,
            host_tenant,
            &CreateSystemAgentRequest {
                title: "Group Coach".to_owned(),
                description: None,
                system_prompt: "Test prompt".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: AgentVisibility::Global,
            },
        )
        .await
        .unwrap()
        .id;
    let group_id = Uuid::new_v4();
    let now = Utc::now();
    repos
        .groups
        .create_group(
            host_tenant,
            &CoachingGroup {
                id: group_id,
                tenant_id: host_tenant.to_string(),
                name: "Both Squad".to_owned(),
                description: None,
                agent_id: agent.to_string(),
                owner_id: requester,
                coach_user_id: None,
                peer_data_sharing: true,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 20,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    for (user_id, tenant, role) in [
        (requester, host_tenant, GroupRole::Owner),
        (peer, peer_tenant, GroupRole::Member),
    ] {
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id,
                tenant_id: tenant.to_string(),
                role,
                peer_sharing_consent: true,
                coach_sharing_consent: true,
                coach_sharing_consent_at: None,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }
    let runtime: Arc<dyn ToolRuntime> = w.resources.clone();
    let ctx = ToolContext::new()
        .with_user(requester.to_string())
        .with_tenant(host_tenant.to_string())
        .with_auth_method("jwt_bearer");

    let response = GetGroupMemberActivitiesTool
        .execute(&runtime, &ctx, json!({ "member": "raph" }))
        .await;

    let payload = response
        .structured_content
        .expect("the tool answers with structured content");
    assert!(payload.get("error").is_none(), "{payload}");
    assert_eq!(payload["count"], 1, "the scraped ride once: {payload}");
    assert_eq!(
        w.scraper.reads(),
        1,
        "bug: the member read scraped the session once per half of the pair"
    );
    assert_eq!(
        w.strava.reads(),
        0,
        "bug: the member read reached Strava's API beside the scrape"
    );
    assert!(
        repos
            .activity_cache
            .latest_activity_sync(peer, &peer_tenant, "strava")
            .await
            .unwrap()
            .is_none(),
        "bug: scraped rows were written under the grant's key"
    );
}

/// The line the commitment refresh logs once it walked every connection.
const COMMITMENT_REFRESH_DONE: &str =
    "Commitment refresh: fetched the commitment window across connected providers";

/// A commitment refresh of an athlete holding both reads the session alone:
/// the grant's name resolves to the session, so walking it too scraped the
/// session a second time. The session is scraped once, Strava's API is never
/// read, and nothing is cached under `strava`.
#[tokio::test]
async fn a_commitment_refresh_scrapes_the_session_once_and_never_reads_the_api() {
    let w = world().await;
    let (user_id, tenant) = athlete_with_both(&w.resources).await;
    let runtime: Arc<dyn ToolRuntime> = w.resources.clone();
    let (events, _guard) = capture_logs();

    assert!(
        spawn_commitment_activity_refresh(
            Arc::clone(&runtime),
            user_id,
            tenant,
            (Utc::now() - Duration::days(7)).timestamp(),
        ),
        "fixture: no refresh is in flight for this athlete"
    );
    let finished = timeout(StdDuration::from_secs(20), async {
        while named(&events, COMMITMENT_REFRESH_DONE).is_empty() {
            sleep(StdDuration::from_millis(20)).await;
        }
    })
    .await;
    assert!(finished.is_ok(), "the refresh walked every connection");

    assert_eq!(
        w.scraper.reads(),
        1,
        "bug: the commitment refresh scraped the session once per half of the pair"
    );
    assert_eq!(
        w.strava.reads(),
        0,
        "bug: the commitment refresh reached Strava's API beside the scrape"
    );
    assert!(
        w.resources
            .common
            .repos
            .activity_cache
            .latest_activity_sync(user_id, &tenant, "strava")
            .await
            .unwrap()
            .is_none(),
        "bug: scraped rows were written under the grant's key"
    );
}
