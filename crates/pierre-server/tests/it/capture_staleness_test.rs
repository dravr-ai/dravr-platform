// ABOUTME: A live connection served from cache while its provider went quiet must be reportable
// ABOUTME: Pins the divergence rule and the cross-tenant snapshot read behind /admin/diagnostics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! jf@dravr.ai's sciotte capture stopped after 2026-08-28 02:59Z and was still
//! stopped two days later. Three real activities never landed. Nothing noticed,
//! because `activity_fetch_freshness` — which held the honest time of the last
//! successful provider fetch the whole time — was read by nothing except a
//! per-user freshness report nobody was looking at (carnet#149).
//!
//! What makes the failure detectable is a DIVERGENCE, not an age.
//! `provider_connections.last_used_at` is touched at the serve chokepoint on
//! every serve, including one the durable cache answered;
//! `activity_fetch_freshness.fetched_at` advances only when a fetch genuinely
//! reached the provider. Recently served AND long unfetched is an athlete being
//! answered from a frozen cache.
//!
//! These tests pin both halves: the rule that reads the divergence, and the
//! cross-tenant snapshot that feeds it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_config::environment::ProviderEndpointOverride;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{
    Activity, ActivityBuilder, Athlete, ConnectionStatus, ConnectionType, SportType, Stats,
    TenantId, UserOAuthToken,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_database::repositories::CaptureFreshness;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::admin::diagnostics::partition_stale_captures;
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_providers::request_budget::{
    refused_before_send, ProviderRateLimiter, RequestBudget, ONE_DAY,
};
use pierre_providers::sciotte_error::coach_account_error;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use pierre_tool_runtime::activity_detail_store::store_detail;
use pierre_tool_runtime::activity_fetch::budgeted_read::{
    read_live_window_within_budget, BudgetedRead, OwnedLiveRead,
};
use pierre_tool_runtime::activity_fetch::sync_verdict::{
    record_head_outcome, HeadRead, HeadVerdict,
};
use pierre_tool_runtime::activity_fetch::{fetch_provider_activities, fetch_provider_head};
use pierre_tool_runtime::capture_sweep::capturable;
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use tracing::Level;
use uuid::Uuid;

use crate::common::{self, create_test_server_resources, create_test_user};
use crate::helpers::notify_capture::{capture_logs, named};
use crate::helpers::sciotte_mock::seed_sciotte_session;

/// Build one snapshot row with ages expressed relative to `now`.
fn row(
    provider: &str,
    now: DateTime<Utc>,
    used_hours_ago: Option<i64>,
    fetched_hours_ago: Option<i64>,
) -> CaptureFreshness {
    CaptureFreshness {
        tenant_id: "11111111-1111-1111-1111-111111111111".to_owned(),
        user_id: "22222222-2222-2222-2222-222222222222".to_owned(),
        provider: provider.to_owned(),
        last_used_at: used_hours_ago.map(|h| now - Duration::hours(h)),
        last_fetch_at: fetched_hours_ago.map(|h| now - Duration::hours(h)),
    }
}

/// The incident shape itself: served an hour ago, last real fetch three days
/// back. This is the row that must come out, and it must carry the honest age.
#[test]
fn a_connection_served_from_a_frozen_cache_is_reported_with_its_true_age() {
    let now = Utc::now();
    let snapshot = vec![row("sciotte", now, Some(1), Some(72))];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(judged, 1, "a connection served an hour ago must be judged");
    assert_eq!(
        stale.len(),
        1,
        "72h since a fetch is past the 24h threshold"
    );
    assert_eq!(stale[0].provider, "sciotte");
    let hours = stale[0]
        .hours_since_fetch
        .expect("a connection that has fetched reports its age");
    assert!(
        (hours - 72.0).abs() < 0.1,
        "expected ~72h since the last fetch, got {hours}"
    );
}

/// A healthy connection: served recently, fetched recently. Reporting this
/// would train an operator to ignore the alert.
#[test]
fn a_connection_still_reaching_its_provider_is_not_reported() {
    let now = Utc::now();
    let snapshot = vec![row("strava", now, Some(1), Some(2))];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(judged, 1);
    assert!(
        stale.is_empty(),
        "a fetch 2h old is inside the 24h threshold, got {stale:?}"
    );
}

/// The false positive this design exists to avoid. An athlete who has not opened
/// the app in a week has nothing that SHOULD have fetched, so their ancient
/// `last_fetch_at` is correct rather than alarming — and they must not be judged
/// at all, or every dormant account becomes a page.
#[test]
fn an_athlete_who_has_not_asked_anything_is_never_judged() {
    let now = Utc::now();
    let snapshot = vec![row("whoop", now, Some(24 * 7), Some(24 * 7))];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(
        judged, 0,
        "a connection last served a week ago is outside the 48h activity window"
    );
    assert!(stale.is_empty());
}

/// A connection that has served an athlete without ever recording one successful
/// fetch is the most alarming state available, and it is distinct from a large
/// age — flattening the two would hide which one it is.
#[test]
fn a_connection_that_never_fetched_is_stale_and_says_so_distinctly() {
    let now = Utc::now();
    let snapshot = vec![row("garmin", now, Some(2), None)];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(judged, 1);
    assert_eq!(stale.len(), 1);
    assert!(
        stale[0].hours_since_fetch.is_none(),
        "never-fetched must stay None, not be flattened into a number"
    );
    assert!(stale[0].last_fetch_at.is_none());
}

/// A connection that has never served anyone is not evidence of anything.
#[test]
fn a_connection_that_never_served_is_never_judged() {
    let now = Utc::now();
    let snapshot = vec![row("coros", now, None, None)];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(judged, 0);
    assert!(stale.is_empty());
}

/// The thresholds are the caller's, and moving them must actually move the
/// verdict — that is what lets an operator re-ask with a tighter question
/// without a deploy.
#[test]
fn the_threshold_decides_the_verdict() {
    let now = Utc::now();
    let snapshot = vec![row("sciotte", now, Some(1), Some(6))];

    let (_, lenient) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));
    assert!(lenient.is_empty(), "6h is inside a 24h threshold");

    let (_, strict) =
        partition_stale_captures(&snapshot, now, Duration::hours(4), Duration::hours(48));
    assert_eq!(strict.len(), 1, "6h is outside a 4h threshold");
}

/// A mixed population must be counted honestly: three judged, one of them stale.
#[test]
fn a_mixed_population_reports_honest_counts() {
    let now = Utc::now();
    let snapshot = vec![
        row("sciotte", now, Some(1), Some(72)), // judged, stale
        row("strava", now, Some(3), Some(3)),   // judged, healthy
        row("whoop", now, Some(6), Some(10)),   // judged, healthy
        row("garmin", now, Some(24 * 9), None), // dormant, not judged
        row("coros", now, None, None),          // never served, not judged
    ];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(judged, 3, "three connections served inside the window");
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].provider, "sciotte");
}

/// The snapshot read itself: it must pair a real connection with the fetch mark
/// recorded against it, across the two tables that store their identifiers
/// differently.
#[tokio::test]
async fn the_snapshot_pairs_a_connection_with_its_fetch_mark() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenants = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    let connections = &resources.common.repos.provider_connections;
    let cache = &resources.common.repos.activity_cache;

    // A connection that has served AND fetched.
    connections
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    connections
        .touch_last_used(user_id, tenant, "sciotte")
        .await
        .unwrap();
    let fetched_at = Utc::now() - Duration::hours(50);
    cache
        .record_activity_fetch(user_id, &tenant, "sciotte", fetched_at)
        .await
        .unwrap();

    // A connection that has served but never fetched.
    connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    connections
        .touch_last_used(user_id, tenant, "whoop")
        .await
        .unwrap();

    let snapshot = cache.capture_freshness_snapshot(100).await.unwrap();

    let sciotte = snapshot
        .iter()
        .find(|c| c.provider == "sciotte")
        .expect("the sciotte connection is in the snapshot");
    assert_eq!(sciotte.user_id, user_id.to_string());
    assert_eq!(sciotte.tenant_id, tenant.to_string());
    assert!(
        sciotte.last_used_at.is_some(),
        "a touched connection carries its serve time"
    );
    let mark = sciotte
        .last_fetch_at
        .expect("the recorded fetch mark is joined onto the connection");
    assert!(
        (mark - fetched_at).num_seconds().abs() <= 1,
        "expected the mark at {fetched_at}, got {mark}"
    );

    let whoop = snapshot
        .iter()
        .find(|c| c.provider == "whoop")
        .expect("the whoop connection is in the snapshot");
    assert!(
        whoop.last_fetch_at.is_none(),
        "a connection with no fetch mark must read as never-fetched, not be dropped"
    );
    assert!(
        whoop.last_used_at.is_some(),
        "NULL handling must not swallow a real serve time"
    );

    // And the divergence rule turns that snapshot into exactly one finding: the
    // sciotte row is 50h past its last fetch, the whoop row never fetched at all.
    let (judged, stale) = partition_stale_captures(
        &snapshot,
        Utc::now(),
        Duration::hours(24),
        Duration::hours(48),
    );
    assert_eq!(judged, 2, "both connections were served just now");
    assert_eq!(stale.len(), 2, "one frozen, one never fetched");
}

/// A connection needing re-auth has a KNOWN reason to have stopped fetching and
/// already surfaces through the reconnect path. Leaving it in would bury the
/// silent failures this endpoint exists to find under a pile of loud ones.
#[tokio::test]
async fn a_connection_needing_reauth_is_excluded_from_the_snapshot() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenants = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    let connections = &resources.common.repos.provider_connections;
    connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    connections
        .touch_last_used(user_id, tenant, "strava")
        .await
        .unwrap();

    let before = resources
        .common
        .repos
        .activity_cache
        .capture_freshness_snapshot(100)
        .await
        .unwrap();
    assert_eq!(
        before.iter().filter(|c| c.provider == "strava").count(),
        1,
        "an active connection is in the snapshot to begin with"
    );

    connections
        .mark_needs_reauth(user_id, tenant, "strava", Some("invalid_grant"), Utc::now())
        .await
        .unwrap();

    let after = resources
        .common
        .repos
        .activity_cache
        .capture_freshness_snapshot(100)
        .await
        .unwrap();
    assert_eq!(
        after.iter().filter(|c| c.provider == "strava").count(),
        0,
        "a connection needing re-auth must drop out of the snapshot"
    );
}

/// A push-only provider's fetch mark moves only when it pushes: an athlete on
/// the Garmin API who recorded nothing for days, while chatting, is not a
/// frozen capture. The reader leaves such a connection out before it judges,
/// and still reports the capture beside it that did stop.
#[tokio::test]
async fn a_push_only_connection_is_never_judged_stale() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenants = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    let connections = &resources.common.repos.provider_connections;
    let cache = &resources.common.repos.activity_cache;
    for provider in ["garmin", "whoop"] {
        connections
            .register_connection(user_id, tenant, provider, &ConnectionType::OAuth, None)
            .await
            .unwrap();
        connections
            .touch_last_used(user_id, tenant, provider)
            .await
            .unwrap();
        cache
            .record_activity_fetch(user_id, &tenant, provider, Utc::now() - Duration::hours(72))
            .await
            .unwrap();
    }

    let snapshot = cache.capture_freshness_snapshot(100).await.unwrap();
    let mine = |c: &CaptureFreshness| c.user_id == user_id.to_string();
    assert_eq!(snapshot.iter().filter(|c| mine(c)).count(), 2);

    let (judged_rows, left_out) = capturable(resources.provider_registry(), snapshot);
    assert!(
        left_out >= 1,
        "bug: the Garmin API connection must be left out"
    );
    let (_, stale) = partition_stale_captures(
        &judged_rows,
        Utc::now(),
        Duration::hours(24),
        Duration::hours(48),
    );
    let stale: Vec<&str> = stale
        .iter()
        .filter(|c| c.user_id == user_id.to_string())
        .map(|c| c.provider.as_str())
        .collect();
    assert_eq!(
        stale,
        vec!["whoop"],
        "bug: only the provider read on request can be a frozen capture"
    );
}

/// The tenant the test user was created in.
async fn tenant_of(resources: &Arc<dyn ToolRuntime>, user_id: Uuid) -> TenantId {
    resources
        .repos()
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id
}

/// The line the Slack error layer forwards for a failed head read. Its first
/// 80 characters are the layer's dedup key, so the text is pinned here.
const HEAD_READ_FAILED: &str =
    "Live activity head read failed; the athlete is served from the cache until it recovers";

/// A head read that failed is the one event an operator is paged for: the
/// error layer forwards `error` lines only, and a `warn` is how the 2026-10-30
/// stale serve went unheard (carnet#1278). An authentication failure is not
/// one — the reconnect notice is its page — and a complete read records
/// nothing.
#[tokio::test]
async fn a_failed_head_read_pages_and_an_auth_failure_does_not() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let tenant = tenant_of(&runtime, user_id).await;
    let head = ActivityQueryParams {
        after: Some((Utc::now() - Duration::days(7)).timestamp()),
        before: None,
        limit: Some(50),
        offset: None,
    };

    let (events, _guard) = capture_logs();
    record_head_outcome(
        &runtime,
        user_id,
        tenant,
        "sciotte",
        &head,
        &Err(AppError::provider_auth_required("sciotte")),
    )
    .await;
    record_head_outcome(
        &runtime,
        user_id,
        tenant,
        "sciotte",
        &head,
        &Ok(HeadRead {
            activities: Vec::new(),
            verdict: HeadVerdict::Complete,
            served_by_grant: None,
        }),
    )
    .await;
    assert!(
        named(&events, HEAD_READ_FAILED).is_empty(),
        "bug: an authentication failure or a complete read must not page"
    );

    // A coach account's own calendar, refused before any read once its role
    // is recorded: the account's standing state, never a page — but still a
    // recorded failure, so the backoff spaces the reads.
    record_head_outcome(
        &runtime,
        user_id,
        tenant,
        "sciotte_trainingpeaks",
        &head,
        &Err(coach_account_error(
            "This TrainingPeaks account is a coach account.",
        )),
    )
    .await;
    assert!(
        named(&events, HEAD_READ_FAILED).is_empty(),
        "bug: a coach account's refused own calendar must not page"
    );
    let refused = named(
        &events,
        "Live activity head read refused: a coach account keeps no calendar of its own",
    );
    assert_eq!(refused.len(), 1, "bug: the refusal stays traceable");
    assert_eq!(
        refused[0].level,
        Level::INFO,
        "bug: the refusal is logged at info, below the paging level"
    );
    assert!(
        runtime
            .repos()
            .activity_cache
            .latest_activity_fetch_failure(user_id, &tenant, "sciotte_trainingpeaks")
            .await
            .expect("read failure record")
            .is_some(),
        "bug: the refused read must still be recorded so the backoff spaces it"
    );

    record_head_outcome(
        &runtime,
        user_id,
        tenant,
        "sciotte",
        &head,
        &Err(AppError::external_service(
            "sciotte",
            "500 from the scraper",
        )),
    )
    .await;
    let paged = named(&events, HEAD_READ_FAILED);
    assert_eq!(
        paged.len(),
        1,
        "bug: one failed head read pages exactly once"
    );
    let page = &paged[0];
    assert_eq!(
        page.level,
        Level::ERROR,
        "bug: the page must be at the level the error layer forwards"
    );
    assert_eq!(page.field("provider"), "sciotte");
    assert_eq!(page.field("user_id"), user_id.to_string());
    assert!(
        page.field("cause").contains("500 from the scraper"),
        "bug: the page names what failed, got: {}",
        page.field("cause")
    );
}

/// The 2026-11-10 digest: a chat read refused by the app's spent Strava
/// budget paged "head read failed" once per athlete, beside the limiter's own
/// "Provider request budget spent" page for the same budget. The limiter's
/// refusal is recorded, so the backoff spaces the reads, but logged at
/// `warn`; a `429` Strava itself answered is not the limiter's and still
/// pages.
#[tokio::test]
async fn a_spent_budget_pages_once_through_the_limiter_and_a_provider_429_still_pages() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let tenant = tenant_of(&runtime, user_id).await;
    let head = ActivityQueryParams {
        after: Some((Utc::now() - Duration::days(7)).timestamp()),
        before: None,
        limit: Some(50),
        offset: None,
    };
    let repos = runtime.repos();
    let limiter = Arc::new(ProviderRateLimiter::new(
        Arc::clone(&repos.usage_counters),
        Arc::clone(&repos.provider_connections),
    ));
    limiter.set_budgets("strava", &[(1, ONE_DAY)]);
    let budget = RequestBudget::new(limiter, "digest-app".to_owned(), None);
    budget.admit("strava").await.expect("the day's one request");
    let refusal = budget
        .admit("strava")
        .await
        .expect_err("the spent day refuses a foreground read");
    assert!(
        refused_before_send(&refusal),
        "bug: the limiter's refusal must say it was never sent"
    );

    let (events, _guard) = capture_logs();
    record_head_outcome(&runtime, user_id, tenant, "strava", &head, &Err(refusal)).await;
    assert!(
        named(&events, HEAD_READ_FAILED).is_empty(),
        "bug: a spent budget pages twice, once per athlete beside the limiter's own page"
    );
    let refused = named(
        &events,
        "Live activity head read refused by the provider request budget; the athlete is served from the cache",
    );
    assert_eq!(refused.len(), 1, "bug: the refusal stays traceable");
    assert_eq!(refused[0].level, Level::WARN);
    assert!(
        runtime
            .repos()
            .activity_cache
            .latest_activity_fetch_failure(user_id, &tenant, "strava")
            .await
            .expect("read failure record")
            .is_some(),
        "bug: the refused read must still be recorded so the backoff spaces it"
    );

    let strava_429 = AppError::new(
        ErrorCode::ExternalRateLimited,
        "Strava answered 429 after retries",
    );
    assert!(!refused_before_send(&strava_429));
    record_head_outcome(&runtime, user_id, tenant, "strava", &head, &Err(strava_429)).await;
    assert_eq!(
        named(&events, HEAD_READ_FAILED).len(),
        1,
        "bug: a 429 Strava itself answered is a failed read the operator must hear about"
    );
}

/// A Strava API stand-in whose activity list answers 500: a live read that
/// fails without being an authentication failure, so the elected provider's
/// own cache answers the window.
async fn strava_answering_500() -> String {
    let app = Router::new().route(
        "/athlete/activities",
        get(|| async {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "message": "Internal Server Error" })),
            )
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A window the elected provider's own cache answered, its live read having
/// failed, is still that connection serving the athlete, so it moves
/// `last_used_at`. The active-athlete sweep selects on that mark; touching
/// it on live answers only froze jf's at 2026-10-19 while chat served him
/// from the cache daily (carnet#1278).
#[tokio::test]
async fn a_serve_from_the_elected_providers_own_cache_touches_its_connection() {
    let api_base = strava_answering_500().await;
    let mut config = common::test_server_config();
    common::set_server_app(&mut config, "strava", "test_client", "test_secret");
    common::set_provider_endpoints(
        &mut config,
        "strava",
        ProviderEndpointOverride {
            api_base_url: Some(api_base),
            ..ProviderEndpointOverride::default()
        },
    );
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let (user_id, _user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let tenant = tenant_of(&runtime, user_id).await;
    let repos = &resources.common.repos;

    // A live grant, so the read reaches the API and fails there, not at auth.
    repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let now = Utc::now();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant.to_string(),
            provider: "strava".to_owned(),
            // >= 40 chars and not "at_"-prefixed: the provider's token validation.
            access_token: SecretString::from("strava_access_token_0123456789abcdef0123456789"),
            refresh_token: Some(SecretString::from("strava_refresh")),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + Duration::hours(6)),
            scope: Some("activity:read_all".to_owned()),
            provider_user_id: Some("4242".to_owned()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let ride = ActivityBuilder::new(
        "strava-ride-1",
        "Sortie longue",
        SportType::Ride,
        now - Duration::days(2),
        7_200,
        "strava",
    )
    .distance_meters(80_000.0)
    .build();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", &[ride])
        .await
        .unwrap();
    let fresh = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await
        .unwrap();
    assert!(
        fresh.iter().all(|c| c.last_used_at.is_none()),
        "fixture: a connection that has never served carries no mark"
    );

    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = GetActivitiesTool
        .execute(&runtime, &ctx, json!({ "limit": 10, "mode": "summary" }))
        .await;
    let payload = response
        .structured_content
        .expect("tool result carries structured content");
    assert!(
        payload.get("provider_unavailable").is_some(),
        "fixture: the window must be served from strava's own cache, got: {payload}"
    );
    assert_eq!(
        payload
            .get("activities")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1),
        "the cached ride is what the athlete is served: {payload}"
    );

    let strava = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.provider == "strava")
        .expect("the strava connection");
    assert!(
        strava.last_used_at.is_some(),
        "bug: a serve from the elected provider's own cache must touch its connection"
    );
}

// ---------------------------------------------------------------------------
// Sciotte first, OAuth as fallback (carnet#1494): one fallback, the grant first
// ---------------------------------------------------------------------------

/// The scraper stand-in answers the session's activity list with its ride.
const SESSION_ANSWERS: u8 = 0;
/// A `500`: the scraper failed.
const SESSION_FAILS: u8 = 1;
/// A `401` `session_expired`: Strava no longer honours the session.
const SESSION_DEAD: u8 = 2;
/// A capture that never reached the list head.
const SESSION_INCOMPLETE: u8 = 3;
/// No activities, over a window the cache holds the session's rows in.
const SESSION_EMPTY: u8 = 4;
/// A `500` on the list, while a read by id answers the ride it names, with
/// the device that recorded it.
const SESSION_DETAILS_ONLY: u8 = 5;

/// The id of the ride the Strava API stand-in answers with.
const GRANT_RIDE: &str = "4001";
/// The id of the ride the scraper stand-in answers with.
const SESSION_RIDE: &str = "21550000001";

/// A scraper stand-in whose list answer the test picks, counting its reads.
struct ScraperStandIn {
    answer: AtomicU8,
    reads: AtomicUsize,
    /// Reads by id the stand-in answered.
    detail_reads: AtomicUsize,
}

async fn spawn_scraper(answer: u8) -> (Arc<ScraperStandIn>, String) {
    let scraper = Arc::new(ScraperStandIn {
        answer: AtomicU8::new(answer),
        reads: AtomicUsize::new(0),
        detail_reads: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": "cap-verified-session" })) }),
        )
        .route(
            "/api/activities",
            get(|State(scraper): State<Arc<ScraperStandIn>>| async move {
                scraper.reads.fetch_add(1, Ordering::SeqCst);
                let ride = json!({
                    "id": SESSION_RIDE,
                    "name": "Sortie du matin",
                    "sport_type": "ride",
                    "start_date": (Utc::now() - Duration::days(1)).to_rfc3339(),
                    "duration_seconds": 3600,
                    "provider": "strava",
                    "distance_meters": 30000.0
                });
                let list = |activities: Value, head_complete: bool| {
                    let count = activities.as_array().map_or(0, Vec::len);
                    (
                        StatusCode::OK,
                        Json(json!({
                            "count": count,
                            "activities": activities,
                            "head_complete": head_complete
                        })),
                    )
                };
                match scraper.answer.load(Ordering::SeqCst) {
                    SESSION_FAILS | SESSION_DETAILS_ONLY => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": "the browser closed under the read" })),
                    ),
                    SESSION_DEAD => (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({ "error": "session_expired" })),
                    ),
                    SESSION_INCOMPLETE => list(json!([ride]), false),
                    SESSION_EMPTY => list(json!([]), true),
                    _ => list(json!([ride]), true),
                }
            }),
        )
        .route(
            "/api/activities/{id}",
            get(
                |State(scraper): State<Arc<ScraperStandIn>>, Path(id): Path<String>| async move {
                    // A session Strava no longer honours refuses a read by id as
                    // it refuses its list; any other session finds no such ride,
                    // unless the test lets it read the ride the id names.
                    match scraper.answer.load(Ordering::SeqCst) {
                        SESSION_DEAD => {
                            return (
                                StatusCode::UNAUTHORIZED,
                                Json(json!({ "error": "session_expired" })),
                            );
                        }
                        SESSION_DETAILS_ONLY => {
                            scraper.detail_reads.fetch_add(1, Ordering::SeqCst);
                            return (
                                StatusCode::OK,
                                Json(json!({
                                    "id": id,
                                    "name": "Sortie longue",
                                    "sport_type": "ride",
                                    "start_date": (Utc::now() - Duration::days(1)).to_rfc3339(),
                                    "duration_seconds": 10_800,
                                    "provider": "strava",
                                    "distance_meters": 80_000.0,
                                    "device_name": "Wahoo ELEMNT BOLT"
                                })),
                            );
                        }
                        _ => {}
                    }
                    (
                        StatusCode::NOT_FOUND,
                        Json(json!({ "error": "activity_not_found" })),
                    )
                },
            ),
        )
        .with_state(Arc::clone(&scraper));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (scraper, format!("http://{addr}"))
}

/// A Strava API stand-in serving one ride on its list and by its id, or a
/// `500` on both while `failing`, counting the list reads.
struct StravaStandIn {
    failing: AtomicBool,
    list_reads: AtomicUsize,
}

fn grant_ride() -> Value {
    json!({
        "id": 4001,
        "name": "Sortie longue",
        "type": "Ride",
        "sport_type": "Ride",
        "start_date": (Utc::now() - Duration::days(1)).to_rfc3339(),
        "distance": 80_000.0,
        "elapsed_time": 10_800,
        "moving_time": 10_200,
        "total_elevation_gain": 900.0
    })
}

async fn spawn_strava(failing: bool) -> (Arc<StravaStandIn>, String) {
    let strava = Arc::new(StravaStandIn {
        failing: AtomicBool::new(failing),
        list_reads: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route(
            "/athlete/activities",
            get(|State(strava): State<Arc<StravaStandIn>>| async move {
                strava.list_reads.fetch_add(1, Ordering::SeqCst);
                if strava.failing.load(Ordering::SeqCst) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "message": "Internal Server Error" })),
                    );
                }
                (StatusCode::OK, Json(json!([grant_ride()])))
            }),
        )
        .route(
            "/activities/{id}",
            get(|State(strava): State<Arc<StravaStandIn>>| async move {
                if strava.failing.load(Ordering::SeqCst) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "message": "Internal Server Error" })),
                    );
                }
                (StatusCode::OK, Json(grant_ride()))
            }),
        )
        .with_state(Arc::clone(&strava));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (strava, format!("http://{addr}"))
}

/// One athlete holding a Strava usr/pwd session and a live Strava grant
/// behind it, both pointed at stand-ins.
struct Pair {
    resources: Arc<ServerContext>,
    scraper: Arc<ScraperStandIn>,
    strava: Arc<StravaStandIn>,
    user_id: Uuid,
    tenant: TenantId,
}

impl Pair {
    async fn new(label: &str, session: u8, grant_failing: bool) -> Self {
        let (scraper, scraper_url) = spawn_scraper(session).await;
        let (strava, strava_url) = spawn_strava(grant_failing).await;
        let mut config = common::test_server_config_with_sciotte(&scraper_url);
        common::set_server_app(&mut config, "strava", "test_client", "test_secret");
        common::set_provider_endpoints(
            &mut config,
            "strava",
            ProviderEndpointOverride {
                api_base_url: Some(strava_url),
                ..ProviderEndpointOverride::default()
            },
        );
        let resources = common::create_test_server_resources_with_config(config)
            .await
            .unwrap();
        let email = format!("{label}-{}@example.com", Uuid::new_v4());
        let (user_id, _user, tenant) =
            common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
                .await
                .unwrap();
        let pair = Self {
            resources,
            scraper,
            strava,
            user_id,
            tenant,
        };
        pair.connect().await;
        pair
    }

    fn runtime(&self) -> Arc<dyn ToolRuntime> {
        self.resources.clone()
    }

    /// The session and its connection, as the hosted sign-in writes them,
    /// and the grant and its connection, as the OAuth callback does.
    async fn connect(&self) {
        let repos = &self.resources.common.repos;
        seed_sciotte_session(&self.resources, self.user_id, self.tenant).await;
        repos
            .provider_connections
            .register_connection(
                self.user_id,
                self.tenant,
                "sciotte",
                &ConnectionType::Manual,
                None,
            )
            .await
            .unwrap();
        let now = Utc::now();
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: self.user_id,
                tenant_id: self.tenant.to_string(),
                provider: "strava".to_owned(),
                // >= 40 chars and not "at_"-prefixed: the provider's token validation.
                access_token: SecretString::from("strava_access_token_0123456789abcdef0123456789"),
                refresh_token: Some(SecretString::from("strava_refresh")),
                token_type: "Bearer".to_owned(),
                expires_at: Some(now + Duration::hours(6)),
                scope: Some("activity:read_all".to_owned()),
                provider_user_id: Some("4242".to_owned()),
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(
                self.user_id,
                self.tenant,
                "strava",
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
    }

    fn head() -> ActivityQueryParams {
        ActivityQueryParams {
            after: Some((Utc::now() - Duration::days(7)).timestamp()),
            before: None,
            limit: Some(50),
            offset: None,
        }
    }

    async fn status(&self, provider: &str) -> ConnectionStatus {
        self.resources
            .common
            .repos
            .provider_connections
            .get_for_user(self.user_id, Some(self.tenant))
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.provider == provider)
            .expect("the connection")
            .status
    }

    async fn flag(&self, provider: &str) {
        self.resources
            .common
            .repos
            .provider_connections
            .mark_needs_reauth(
                self.user_id,
                self.tenant,
                provider,
                Some("test_flag"),
                Utc::now(),
            )
            .await
            .unwrap();
    }

    async fn failure_recorded(&self, provider: &str) -> bool {
        self.resources
            .common
            .repos
            .activity_cache
            .latest_activity_fetch_failure(self.user_id, &self.tenant, provider)
            .await
            .unwrap()
            .is_some()
    }

    async fn last_sync(&self, provider: &str) -> Option<DateTime<Utc>> {
        self.resources
            .common
            .repos
            .activity_cache
            .latest_activity_sync(self.user_id, &self.tenant, provider)
            .await
            .unwrap()
    }

    async fn synced(&self, provider: &str) -> bool {
        self.last_sync(provider).await.is_some()
    }

    /// Each cached row of activity `id`, by key, and whether a stored detail
    /// read answers it.
    async fn detail_read(&self, id: &str) -> Vec<(String, bool)> {
        let mut rows: Vec<(String, bool)> = self
            .resources
            .common
            .repos
            .activity_cache
            .get_cached_activity_rows(
                self.user_id,
                &self.tenant,
                Utc::now() - Duration::days(30),
                Utc::now() + Duration::days(1),
                100,
            )
            .await
            .unwrap()
            .into_iter()
            .filter(|row| row.activity.id() == id)
            .map(|row| (row.provider, row.detail_read))
            .collect();
        rows.sort();
        rows
    }

    async fn cache(&self, provider: &str, id: &str, days_ago: i64) {
        let row = ActivityBuilder::new(
            id,
            "Sortie en cache",
            SportType::Ride,
            Utc::now() - Duration::days(days_ago),
            3_600,
            provider,
        )
        .build();
        self.resources
            .common
            .repos
            .activity_cache
            .upsert_activities(self.user_id, &self.tenant, provider, &[row])
            .await
            .unwrap();
    }
}

/// The line logged when the grant answered in the session's place.
const SERVED_BY_GRANT: &str =
    "Live activity head read failed on the session; its grant answered in its place";

fn ids(rows: &[Activity]) -> Vec<&str> {
    rows.iter().map(Activity::id).collect()
}

/// Every read the session did not vouch for is answered by the grant behind
/// it: a scraper that failed, a capture that never reached the head, an
/// empty answer the cache contradicts, and a session Strava no longer
/// honours. The session's failure is recorded under the session, the
/// grant's answer is written under the grant, and nothing pages: the athlete
/// was served.
#[tokio::test]
async fn each_session_read_that_falls_back_is_served_by_the_grant_and_recorded_where_it_failed() {
    for (session, label) in [
        (SESSION_FAILS, "fails"),
        (SESSION_INCOMPLETE, "incomplete"),
        (SESSION_EMPTY, "empty"),
        (SESSION_DEAD, "dead"),
    ] {
        let pair = Pair::new(label, session, false).await;
        let runtime = pair.runtime();
        if session == SESSION_EMPTY {
            pair.cache("sciotte", "21550000009", 2).await;
        }

        let session_synced = pair.last_sync("sciotte").await;
        let (events, _guard) = capture_logs();
        let served = fetch_provider_head(
            &runtime,
            "strava",
            pair.user_id,
            &pair.tenant.to_string(),
            &Pair::head(),
        )
        .await
        .unwrap_or_else(|e| panic!("{label}: the grant must serve the read, got {e}"));

        assert_eq!(
            ids(&served),
            [GRANT_RIDE],
            "{label}: the grant's ride is served"
        );
        assert_eq!(pair.scraper.reads.load(Ordering::SeqCst), 1, "{label}");
        assert_eq!(pair.strava.list_reads.load(Ordering::SeqCst), 1, "{label}");
        assert!(
            pair.synced("strava").await,
            "{label}: the grant's answer is a sync of the grant"
        );
        assert_eq!(
            pair.last_sync("sciotte").await,
            session_synced,
            "{label}: the session's freshness stays where its last good read put it"
        );
        if session == SESSION_DEAD {
            assert_eq!(
                pair.status("sciotte").await,
                ConnectionStatus::NeedsReauth,
                "the dead session is flagged for its sign-in"
            );
        } else {
            assert!(
                pair.failure_recorded("sciotte").await,
                "{label}: the session's failure is recorded under the session"
            );
        }
        assert!(
            !pair.failure_recorded("strava").await,
            "{label}: nothing failed on the grant"
        );
        assert_eq!(pair.status("strava").await, ConnectionStatus::Active);
        assert!(
            named(&events, HEAD_READ_FAILED).is_empty(),
            "{label}: bug: a read the grant served must not page"
        );
        assert_eq!(
            named(&events, SERVED_BY_GRANT).len(),
            1,
            "{label}: the stand-in is traceable"
        );
    }
}

/// A session whose grant cannot answer either is the one case nothing
/// served: it pages once, with the session's cause, and each failure is
/// recorded under the backend that failed.
#[tokio::test]
async fn a_session_and_its_grant_both_failing_page_once() {
    let pair = Pair::new("both-fail", SESSION_FAILS, true).await;
    let runtime = pair.runtime();

    let (events, _guard) = capture_logs();
    let read = fetch_provider_head(
        &runtime,
        "strava",
        pair.user_id,
        &pair.tenant.to_string(),
        &Pair::head(),
    )
    .await;

    assert!(read.is_err(), "nothing served: {read:?}");
    assert_eq!(pair.strava.list_reads.load(Ordering::SeqCst), 1);
    let paged = named(&events, HEAD_READ_FAILED);
    assert_eq!(
        paged.len(),
        1,
        "bug: one read nothing served pages exactly once"
    );
    assert_eq!(paged[0].level, Level::ERROR);
    assert_eq!(paged[0].field("provider"), "sciotte");
    assert!(pair.failure_recorded("sciotte").await);
    assert!(pair.failure_recorded("strava").await);
}

/// A dead session is flagged on the read that finds it dead and the grant
/// serves that read; every read after it goes to the grant without signing
/// in to the dead session again.
#[tokio::test]
async fn a_dead_session_is_flagged_and_its_grant_keeps_serving_without_reading_it() {
    let pair = Pair::new("dead-session", SESSION_DEAD, false).await;
    let runtime = pair.runtime();
    let tenant = pair.tenant.to_string();

    let first = fetch_provider_activities(&runtime, "strava", pair.user_id, &tenant, &Pair::head())
        .await
        .expect("the grant serves the read that found the session dead");
    assert_eq!(ids(&first), [GRANT_RIDE]);
    assert_eq!(pair.status("sciotte").await, ConnectionStatus::NeedsReauth);
    assert_eq!(pair.scraper.reads.load(Ordering::SeqCst), 1);

    let again = fetch_provider_activities(&runtime, "strava", pair.user_id, &tenant, &Pair::head())
        .await
        .expect("the flagged session's grant serves");
    assert_eq!(ids(&again), [GRANT_RIDE]);
    assert_eq!(
        pair.scraper.reads.load(Ordering::SeqCst),
        1,
        "bug: a flagged session was signed in to again"
    );
    assert_eq!(pair.strava.list_reads.load(Ordering::SeqCst), 2);
}

/// A flagged session whose grant is flagged too serves its own cache: only
/// a live grant stands in for it.
#[tokio::test]
async fn a_flagged_session_without_a_live_grant_serves_its_cache() {
    let pair = Pair::new("flagged-alone", SESSION_DEAD, false).await;
    let runtime = pair.runtime();
    pair.flag("strava").await;
    pair.flag("sciotte").await;
    pair.cache("sciotte", SESSION_RIDE, 1).await;

    let served = fetch_provider_activities(
        &runtime,
        "strava",
        pair.user_id,
        &pair.tenant.to_string(),
        &Pair::head(),
    )
    .await
    .expect("the session's cache serves");
    assert_eq!(ids(&served), [SESSION_RIDE]);
    assert_eq!(pair.scraper.reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        pair.strava.list_reads.load(Ordering::SeqCst),
        0,
        "bug: a flagged grant was read in its session's place"
    );
}

/// A read the athlete pinned to Strava is still answered by the Strava grant
/// when the session fails: the grant is the same provider. The window says
/// it came from the grant, and that nothing is missing from it.
#[tokio::test]
async fn a_read_pinned_to_the_provider_reaches_its_grant() {
    let pair = Pair::new("pinned", SESSION_FAILS, false).await;
    let runtime = pair.runtime();
    let ctx = ToolContext::new()
        .with_user(pair.user_id.to_string())
        .with_tenant(pair.tenant.to_string())
        .with_auth_method("jwt_bearer");

    let response = GetActivitiesTool
        .execute(
            &runtime,
            &ctx,
            json!({ "provider": "strava", "limit": 10, "mode": "summary" }),
        )
        .await;
    let payload = response
        .structured_content
        .expect("tool result carries structured content");

    let served: Vec<&str> = payload["activities"]
        .as_array()
        .unwrap_or_else(|| panic!("activities: {payload}"))
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert_eq!(served, [GRANT_RIDE], "{payload}");
    assert_eq!(
        payload["provider_unavailable"]["reason"], "grant",
        "{payload}"
    );
    assert!(payload.get("reconnect_required").is_none(), "{payload}");
}

/// A by-id read the session cannot make is made once more through the
/// Strava grant: both key the activity by Strava's own id.
#[tokio::test]
async fn a_failed_by_id_read_of_the_session_is_read_through_its_grant() {
    let pair = Pair::new("by-id", SESSION_ANSWERS, false).await;
    let provider = AuthService::new(pair.runtime())
        .create_authenticated_provider("strava", pair.user_id, Some(&pair.tenant.to_string()))
        .await
        .expect("the session authenticates");
    assert_eq!(provider.name(), "sciotte", "the session serves the reads");

    let detail = provider
        .get_activity(GRANT_RIDE)
        .await
        .expect("the grant reads the activity the session could not");
    assert_eq!(detail.id(), GRANT_RIDE);
    assert_eq!(detail.provider(), "strava");
    assert_eq!(
        pair.status("sciotte").await,
        ConnectionStatus::Active,
        "a session that only lacked the ride is not flagged"
    );
}

/// A by-id read the session refuses as dead is answered by the grant, and
/// the session is flagged for its sign-in all the same, as a list read that
/// finds it dead flags it: the grant answering never hides the dead sign-in.
#[tokio::test]
async fn a_by_id_read_that_finds_the_session_dead_flags_it_while_its_grant_answers() {
    let pair = Pair::new("by-id-dead", SESSION_DEAD, false).await;
    let provider = AuthService::new(pair.runtime())
        .create_authenticated_provider("strava", pair.user_id, Some(&pair.tenant.to_string()))
        .await
        .expect("the session authenticates");
    assert_eq!(provider.name(), "sciotte", "the session serves the reads");

    let detail = provider
        .get_activity(GRANT_RIDE)
        .await
        .expect("the grant reads the activity the dead session refused");
    assert_eq!(detail.id(), GRANT_RIDE);
    assert_eq!(
        pair.status("sciotte").await,
        ConnectionStatus::NeedsReauth,
        "bug: the grant answered and the dead session was never flagged"
    );
    assert_eq!(
        pair.status("strava").await,
        ConnectionStatus::Active,
        "the grant that answered is not flagged"
    );
}

/// A detail the grant answers in the session's place is Strava API data: it
/// is kept under the grant, never the session, so the grant's revocation
/// purges it as Strava's terms require, and the session's own row keeps
/// nothing the grant read. An activity a synced grant never listed gets
/// the grant's own copy to keep it beside, without making the grant look
/// freshly synced.
#[tokio::test]
async fn a_detail_the_grant_answers_is_kept_under_the_grant_and_purged_with_it() {
    for grant_listed in [true, false] {
        let pair = Pair::new("by-id-kept", SESSION_ANSWERS, false).await;
        pair.cache("sciotte", GRANT_RIDE, 1).await;
        let grant_row = if grant_listed { GRANT_RIDE } else { "4002" };
        pair.cache("strava", grant_row, 3).await;
        let grant_synced = pair.last_sync("strava").await;
        let provider = AuthService::new(pair.runtime())
            .create_authenticated_provider("strava", pair.user_id, Some(&pair.tenant.to_string()))
            .await
            .expect("the session authenticates");
        assert_eq!(provider.name(), "sciotte", "the session serves the reads");
        let detail = provider
            .get_activity_detailed(GRANT_RIDE)
            .await
            .expect("the grant reads the activity the session could not");

        let repos = &pair.resources.common.repos;
        assert!(
            store_detail(
                repos,
                pair.resources.common.live_activities.as_ref(),
                pair.tenant,
                pair.user_id,
                "sciotte",
                &detail
            )
            .await,
            "the grant's answer is kept beside the grant's copy (listed: {grant_listed})"
        );
        assert_eq!(
            pair.detail_read(GRANT_RIDE).await,
            vec![("sciotte".to_owned(), false), ("strava".to_owned(), true)],
            "bug: what the grant answered was kept under the session's key (listed: {grant_listed})"
        );
        assert_eq!(
            pair.last_sync("strava").await,
            grant_synced,
            "bug: a by-id read made the grant look freshly synced (listed: {grant_listed})"
        );

        OAuthService::new(
            pair.resources.data(),
            Arc::clone(&pair.resources.common.config),
        )
        .disconnect_provider(
            pair.user_id,
            "strava",
            Some(pair.tenant.as_uuid()),
            DisconnectReason::ProviderRevoked,
        )
        .await
        .unwrap();
        assert_eq!(
            pair.detail_read(GRANT_RIDE).await,
            vec![("sciotte".to_owned(), false)],
            "bug: the grant's revocation left what it answered (listed: {grant_listed})"
        );
    }
}

/// A session read whose list answers only once the test lets it, and then
/// fails: a scrape that outlives the turn, then errors.
struct SlowFailingSession {
    config: ProviderConfig,
    gate: Arc<Notify>,
}

#[async_trait]
impl FitnessProvider for SlowFailingSession {
    fn name(&self) -> &'static str {
        "sciotte"
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, _credentials: OAuth2Credentials) -> AppResult<()> {
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        true
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        Ok(())
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        Err(AppError::internal("not read by this test"))
    }

    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        self.gate.notified().await;
        Err(AppError::external_service("sciotte", "the browser wedged"))
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Err(AppError::internal("not read by this test"))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        Err(AppError::not_found(id.to_owned()))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        Err(AppError::internal("not read by this test"))
    }
}

/// Past the turn's budget the cached window answers and the grant is not
/// read inline: the session's read runs on, and only when it then fails
/// does its grant answer, detached, for the next turn.
#[tokio::test]
async fn a_turn_past_its_budget_reads_only_one_backend_inline() {
    let pair = Pair::new("budget", SESSION_ANSWERS, false).await;
    let runtime = pair.runtime();
    pair.cache("sciotte", SESSION_RIDE, 3).await;
    let gate = Arc::new(Notify::new());
    let session = Box::new(SlowFailingSession {
        config: ProviderConfig {
            name: "sciotte".to_owned(),
            auth_url: String::new(),
            token_url: String::new(),
            api_base_url: String::new(),
            revoke_url: None,
            default_scopes: Vec::new(),
        },
        gate: Arc::clone(&gate),
    });
    let read = OwnedLiveRead {
        provider_slug: "sciotte".to_owned(),
        user_id: pair.user_id,
        tenant_id: pair.tenant,
        params: Pair::head(),
        attempt_started_at: Utc::now(),
        conversation_id: None,
        ask: None,
    };

    let outcome = timeout(
        StdDuration::from_secs(5),
        read_live_window_within_budget(&runtime, session, read, StdDuration::from_millis(200)),
    )
    .await
    .expect("the turn waited on a read past its budget");
    let BudgetedRead::Cached(rows) = outcome else {
        panic!("a read past the budget answers from the cached window");
    };
    assert_eq!(ids(&rows), [SESSION_RIDE]);
    assert_eq!(
        pair.strava.list_reads.load(Ordering::SeqCst),
        0,
        "bug: the turn read the grant inline beside a session read still running"
    );

    gate.notify_one();
    let landed = timeout(StdDuration::from_secs(10), async {
        while !pair.synced("strava").await {
            sleep(StdDuration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        landed.is_ok(),
        "the detached read falls back to the grant once the session fails"
    );
    assert_eq!(pair.strava.list_reads.load(Ordering::SeqCst), 1);
    assert!(pair.failure_recorded("sciotte").await);
}

/// A ride the grant listed while the session's list fails, detailed by the
/// session's read by id, keeps that detail beside the grant's row it details,
/// as the activity view keeps it: the next turn serves it from the store and
/// scrapes nothing. Before, the session's answer looked for a session row of
/// the ride, found none, and was dropped, so every turn scraped it again.
#[tokio::test]
async fn a_session_detail_of_a_ride_the_grant_listed_is_kept_and_not_scraped_again() {
    let pair = Pair::new("session-detail", SESSION_DETAILS_ONLY, false).await;
    let runtime = pair.runtime();
    let ctx = ToolContext::new()
        .with_user(pair.user_id.to_string())
        .with_tenant(pair.tenant.to_string())
        .with_auth_method("jwt_bearer");

    for turn in 1..=2 {
        let response = GetActivitiesTool
            .execute(&runtime, &ctx, json!({ "limit": 10 }))
            .await;
        let payload = response
            .structured_content
            .expect("tool result carries structured content");
        let ride = payload["activities"]
            .as_array()
            .unwrap_or_else(|| panic!("activities on turn {turn}: {payload}"))
            .iter()
            .find(|a| a["id"].as_str() == Some(GRANT_RIDE))
            .unwrap_or_else(|| panic!("the grant's ride is served on turn {turn}: {payload}"))
            .clone();
        assert_eq!(
            ride["device_name"], "Wahoo ELEMNT BOLT",
            "the session's detail reaches the ride on turn {turn}: {ride}"
        );
    }
    assert_eq!(
        pair.scraper.detail_reads.load(Ordering::SeqCst),
        1,
        "bug: the session's detail of the grant's ride was dropped and scraped again"
    );
    assert_eq!(
        pair.detail_read(GRANT_RIDE).await,
        vec![("strava".to_owned(), true)],
        "bug: the detail is not kept beside the row it details"
    );
}
