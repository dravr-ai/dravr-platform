// ABOUTME: Integration tests for the request-budget layer: X-RateLimit-* headers and the API-key usage row
// ABOUTME: Pins exact header values, the slot's scope rules, and the row's real status, endpoint and latency
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use chrono::{DateTime, Duration, TimeZone, Utc};
use pierre_auth::api_keys::{ApiKey, ApiKeyTier};
use pierre_auth::rate_limiting::RequestBudget;
use pierre_core::models::RequestLog;
use pierre_database::backends::factory::Database;
use pierre_database::backends::UsageRepository;
use pierre_middleware::rate_limiting::{
    create_rate_limit_headers, headers, report_api_key_request, report_request_budget,
    report_request_operation, request_budget_middleware,
};
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};
use tower::ServiceExt;
use tower_http::catch_panic::CatchPanicLayer;
use uuid::Uuid;

fn reset_instant() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()
}

fn header<'a>(map: &'a HeaderMap, name: &str) -> Option<&'a str> {
    map.get(name).and_then(|v| v.to_str().ok())
}

/// The usage repository of a fresh test database.
async fn usage_repository() -> (Arc<Database>, Arc<dyn UsageRepository>) {
    let database = common::create_test_database().await.unwrap();
    let usage = Arc::clone(&database.repositories().usage);
    (database, usage)
}

/// A stored API key the usage rows can reference.
async fn stored_key(database: &Database) -> ApiKey {
    let (user_id, _) = common::create_test_user(database).await.unwrap();
    let api_key = ApiKey {
        id: Uuid::new_v4().to_string(),
        user_id,
        name: "layer test key".to_owned(),
        key_prefix: format!("pk_live_{}", &Uuid::new_v4().simple().to_string()[..8]),
        key_hash: Uuid::new_v4().simple().to_string(),
        description: None,
        tier: ApiKeyTier::Starter,
        rate_limit_requests: 100,
        rate_limit_window_seconds: 3_600,
        is_active: true,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
    };
    database
        .repositories()
        .api_keys
        .create(&api_key)
        .await
        .unwrap();
    api_key
}

/// Every usage row recorded for `api_key_id`.
async fn rows_for(usage: &dyn UsageRepository, api_key_id: &str) -> Vec<RequestLog> {
    usage
        .get_request_logs(None, Some(api_key_id), None, None, None, None)
        .await
        .unwrap()
}

/// The rows for `api_key_id` once there are `expected` of them: a row
/// written from a dropped request lands on its own task.
async fn rows_eventually(
    usage: &dyn UsageRepository,
    api_key_id: &str,
    expected: usize,
) -> Vec<RequestLog> {
    for _ in 0..100 {
        let rows = rows_for(usage, api_key_id).await;
        if rows.len() >= expected {
            return rows;
        }
        sleep(StdDuration::from_millis(20)).await;
    }
    rows_for(usage, api_key_id).await
}

/// `router` wrapped in the request-budget layer, as the server installs it.
fn layered(router: Router, usage: &Arc<dyn UsageRepository>) -> Router {
    router.layer(from_fn_with_state(
        Arc::clone(usage),
        request_budget_middleware,
    ))
}

/// The headers a response carries after going through the layer around
/// `router`, for one GET of `/`.
async fn through_layer(router: Router) -> HeaderMap {
    let (_database, usage) = usage_repository().await;
    let response = layered(router, &usage)
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.headers().clone()
}

/// A handler body that fails the way a bug does.
fn handler_failure() -> StatusCode {
    panic!("handler failure")
}

fn metered(limit: u32, used: u32) -> RequestBudget {
    RequestBudget::Metered {
        limit,
        used,
        resets_at: reset_instant(),
    }
}

#[test]
fn test_metered_budget_renders_exactly_three_headers() {
    let map = create_rate_limit_headers(metered(1000, 249));

    assert_eq!(map.len(), 3, "limit, remaining and reset, nothing else");
    assert_eq!(header(&map, "x-ratelimit-limit"), Some("1000"));
    // 249 counted before this request, and this one: 1000 - 250
    assert_eq!(header(&map, "x-ratelimit-remaining"), Some("750"));
    assert_eq!(
        header(&map, "x-ratelimit-reset"),
        Some(reset_instant().timestamp().to_string().as_str())
    );
    for absent in [
        "retry-after",
        "x-ratelimit-window",
        "x-ratelimit-tier",
        "x-ratelimit-authmethod",
    ] {
        assert!(map.get(absent).is_none(), "{absent} is never sent");
    }
    assert_eq!(headers::X_RATE_LIMIT_LIMIT.as_str(), "x-ratelimit-limit");
    assert_eq!(
        headers::X_RATE_LIMIT_REMAINING.as_str(),
        "x-ratelimit-remaining"
    );
    assert_eq!(headers::X_RATE_LIMIT_RESET.as_str(), "x-ratelimit-reset");
}

#[test]
fn test_spent_budget_renders_zero_remaining() {
    let map = create_rate_limit_headers(metered(5, 5));
    assert_eq!(header(&map, "x-ratelimit-limit"), Some("5"));
    assert_eq!(header(&map, "x-ratelimit-remaining"), Some("0"));
}

#[test]
fn test_unlimited_budget_renders_nothing() {
    assert!(create_rate_limit_headers(RequestBudget::Unlimited).is_empty());
}

#[test]
fn test_report_outside_a_request_scope_is_dropped() {
    // No enclosing layer: stdio, background work. Must not panic.
    report_request_budget(metered(5, 1));
    report_api_key_request("no-such-key");
    report_request_operation("tools/list");
}

#[tokio::test]
async fn test_layer_adds_nothing_when_no_one_authenticated() {
    let headers = through_layer(Router::new().route("/", get(|| async { "public" }))).await;
    assert!(headers.get("x-ratelimit-limit").is_none());
    assert!(headers.get("x-ratelimit-remaining").is_none());
    assert!(headers.get("x-ratelimit-reset").is_none());
}

#[tokio::test]
async fn test_layer_renders_the_reported_budget() {
    let headers = through_layer(Router::new().route(
        "/",
        get(|| async {
            report_request_budget(metered(10, 3));
            "authenticated"
        }),
    ))
    .await;
    assert_eq!(header(&headers, "x-ratelimit-limit"), Some("10"));
    assert_eq!(header(&headers, "x-ratelimit-remaining"), Some("6"));
    assert_eq!(
        header(&headers, "x-ratelimit-reset"),
        Some(reset_instant().timestamp().to_string().as_str())
    );
}

#[tokio::test]
async fn test_last_report_wins_within_a_request() {
    let headers = through_layer(Router::new().route(
        "/",
        get(|| async {
            // The slot holds one budget: a later report replaces an earlier
            // one, never renders beside it.
            report_request_budget(metered(10_000, 10_000));
            report_request_budget(metered(5, 1));
            "authenticated twice"
        }),
    ))
    .await;
    assert_eq!(header(&headers, "x-ratelimit-limit"), Some("5"));
    assert_eq!(header(&headers, "x-ratelimit-remaining"), Some("3"));
}

#[tokio::test]
async fn test_layer_replaces_a_header_another_layer_set() {
    let headers = through_layer(Router::new().route(
        "/",
        get(|| async {
            report_request_budget(metered(7, 0));
            ([("x-ratelimit-limit", "999")], "already set").into_response()
        }),
    ))
    .await;
    let values: Vec<_> = headers.get_all("x-ratelimit-limit").iter().collect();
    assert_eq!(values.len(), 1, "insert replaces, never appends");
    assert_eq!(values[0], "7");
}

#[tokio::test]
async fn test_unlimited_report_renders_nothing_through_the_layer() {
    let headers = through_layer(Router::new().route(
        "/",
        get(|| async {
            report_request_budget(RequestBudget::Unlimited);
            "enterprise"
        }),
    ))
    .await;
    assert!(headers.get("x-ratelimit-limit").is_none());
}

#[tokio::test]
async fn test_report_from_a_spawned_task_is_lost_not_misattributed() {
    let headers = through_layer(Router::new().route(
        "/",
        get(|| async {
            tokio::spawn(async { report_request_budget(metered(10, 1)) })
                .await
                .unwrap();
            "spawned"
        }),
    ))
    .await;
    assert!(
        headers.get("x-ratelimit-limit").is_none(),
        "a spawned task has no slot: the headers are missing, never wrong"
    );
}

#[tokio::test]
async fn test_admitted_key_row_carries_the_real_status_route_and_latency() {
    let (database, usage) = usage_repository().await;
    let key = stored_key(&database).await;
    let key_id = key.id.clone();
    let app = layered(
        Router::new().route(
            "/items/{id}",
            get(move || {
                let key_id = key_id.clone();
                async move {
                    report_api_key_request(&key_id);
                    sleep(StdDuration::from_millis(30)).await;
                    StatusCode::NOT_FOUND
                }
            }),
        ),
        &usage,
    );

    let before = Utc::now();
    let response = app
        .oneshot(Request::get("/items/42").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let rows = rows_for(usage.as_ref(), &key.id).await;
    assert_eq!(rows.len(), 1, "one row per admitted request: {rows:?}");
    let row = &rows[0];
    assert_eq!(row.status_code, 404, "the handler's status, not a 200");
    assert_eq!(
        row.tool_name, "GET /items/{id}",
        "the route template, never the raw path"
    );
    assert!(
        row.response_time_ms.is_some_and(|ms| ms >= 30),
        "the request's latency, got {:?}",
        row.response_time_ms
    );
    assert!(
        (row.timestamp - before).num_seconds().abs() <= 2,
        "stamped when the request arrived"
    );
}

#[tokio::test]
async fn test_reported_operation_names_the_row() {
    let (database, usage) = usage_repository().await;
    let key = stored_key(&database).await;
    let key_id = key.id.clone();
    let app = layered(
        Router::new().route(
            "/mcp",
            get(move || {
                let key_id = key_id.clone();
                async move {
                    report_api_key_request(&key_id);
                    report_request_operation("get_activities");
                    "tool ran"
                }
            }),
        ),
        &usage,
    );

    let response = app
        .oneshot(Request::get("/mcp").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let rows = rows_for(usage.as_ref(), &key.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tool_name, "get_activities");
    assert_eq!(rows[0].status_code, 200);
}

#[tokio::test]
async fn test_no_row_without_an_admitted_key() {
    let (database, usage) = usage_repository().await;
    let key = stored_key(&database).await;
    let key_id = key.id.clone();
    let app = layered(
        Router::new()
            .route("/public", get(|| async { "no credential" }))
            .route(
                "/spawned",
                get(move || {
                    let key_id = key_id.clone();
                    async move {
                        tokio::spawn(async move { report_api_key_request(&key_id) })
                            .await
                            .unwrap();
                        "spawned"
                    }
                }),
            ),
        &usage,
    );

    for uri in ["/public", "/spawned"] {
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
    }
    assert!(
        rows_for(usage.as_ref(), &key.id).await.is_empty(),
        "no admitted key in the request's own task, no row"
    );
}

#[tokio::test]
async fn test_panicking_handler_is_recorded_as_the_500_it_answers() {
    let (database, usage) = usage_repository().await;
    let key = stored_key(&database).await;
    let key_id = key.id.clone();
    let app = layered(
        Router::new().route(
            "/boom",
            get(move || {
                let key_id = key_id.clone();
                async move {
                    report_api_key_request(&key_id);
                    handler_failure()
                }
            }),
        ),
        &usage,
    )
    .layer(CatchPanicLayer::new());

    let response = app
        .oneshot(Request::get("/boom").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "the panic still reaches the panic layer"
    );

    let rows = rows_for(usage.as_ref(), &key.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status_code, 500);
    assert_eq!(rows[0].tool_name, "GET /boom");
}

#[tokio::test]
async fn test_abandoned_request_is_still_counted_as_499() {
    let (database, usage) = usage_repository().await;
    let key = stored_key(&database).await;
    let key_id = key.id.clone();
    let (admitted_tx, admitted_rx) = oneshot::channel::<()>();
    let admitted_tx = Arc::new(Mutex::new(Some(admitted_tx)));
    let app = layered(
        Router::new().route(
            "/slow",
            get(move || {
                let key_id = key_id.clone();
                let admitted_tx = Arc::clone(&admitted_tx);
                async move {
                    report_api_key_request(&key_id);
                    let sender = admitted_tx.lock().unwrap().take();
                    if let Some(tx) = sender {
                        tx.send(()).unwrap();
                    }
                    sleep(StdDuration::from_secs(60)).await;
                    "never answered"
                }
            }),
        ),
        &usage,
    );

    // The client goes away once the key has been admitted.
    let request = app.oneshot(Request::get("/slow").body(Body::empty()).unwrap());
    let abandoned = timeout(StdDuration::from_secs(5), async {
        tokio::select! {
            _ = request => panic!("the slow handler answered"),
            _ = admitted_rx => {}
        }
    })
    .await;
    assert!(abandoned.is_ok(), "the handler admitted the key in time");

    let rows = rows_eventually(usage.as_ref(), &key.id, 1).await;
    assert_eq!(
        rows.len(),
        1,
        "leaving early never takes a call out of the window"
    );
    assert_eq!(rows[0].status_code, 499, "client closed request");
    assert_eq!(rows[0].tool_name, "GET /slow");
    assert!(rows[0].timestamp > Utc::now() - Duration::minutes(1));
}
