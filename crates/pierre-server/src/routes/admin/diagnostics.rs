// ABOUTME: Admin diagnostics endpoints for system observability and resource measurement
// ABOUTME: Provides tool schema size estimation and tronc-canary alerting probe
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin diagnostics routes for system observability.
//!
//! Provides endpoints to inspect tool schema sizes, token budgets, and
//! other internal metrics useful for capacity planning. Lives in
//! `pierre-server` rather than `pierre-routes-admin` because the schema
//! size endpoint depends on the [`pierre_tool_runtime::registry::ToolRegistry`]
//! type, which is internal to the composition root.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_database::repositories::CaptureFreshness;
use pierre_database::RepositoryRegistry;
use pierre_routes_admin::auth::middleware::admin_auth_middleware;
use pierre_routes_admin::auth::service::AdminAuthService;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::error;
use uuid::Uuid;

use pierre_core::errors::AppResult;
use pierre_tool_runtime::registry::ToolRegistry;

/// Focused context for the diagnostics sub-routes.
///
/// Carries the [`ToolRegistry`] needed by
/// `/admin/diagnostics/tool-schema-size`. Diagnostic routes do not need
/// the full [`pierre_routes_admin::AdminApiContext`] surface.
#[derive(Clone)]
pub struct DiagnosticsContext {
    /// Tool registry — primary input for schema-size estimation.
    pub tool_registry: Arc<ToolRegistry>,
    /// Repositories — the capture-staleness report reads provider connections
    /// and their fetch-freshness marks.
    pub repos: Arc<RepositoryRegistry>,
}

/// Mount the diagnostics sub-routes behind the admin-token JWT middleware.
///
/// Called from `mcp::multitenant` alongside [`pierre_routes_admin::AdminRoutes::routes`]
/// so the human-facing admin token surface includes the diagnostic
/// endpoints without forcing them through the leaf crate.
pub fn routes(context: DiagnosticsContext, auth_service: AdminAuthService) -> Router {
    let context = Arc::new(context);
    Router::new()
        .route(
            "/admin/diagnostics/tool-schema-size",
            get(handle_tool_schema_size),
        )
        .route("/admin/diagnostics/tronc-canary", post(handle_tronc_canary))
        .route(
            "/admin/diagnostics/capture-staleness",
            get(handle_capture_staleness),
        )
        .with_state(context)
        .layer(middleware::from_fn_with_state(
            auth_service,
            admin_auth_middleware,
        ))
}

/// `GET /admin/diagnostics/tool-schema-size`.
///
/// Returns the estimated token cost of all registered MCP tool schemas,
/// broken down per tool and sorted by token cost descending.
///
/// # Errors
///
/// Returns an error when the caller's admin token lacks
/// [`AdminPermission::ViewConfiguration`].
pub async fn handle_tool_schema_size(
    State(context): State<Arc<DiagnosticsContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let estimate = context.tool_registry.total_schema_token_estimate();

    Ok((StatusCode::OK, Json(estimate)))
}

/// `POST /admin/diagnostics/tronc-canary`.
///
/// Emits a synthetic ERROR-level tracing event tagged with a fresh correlation
/// ID. The dravr-tronc error notification layer listens for `Level::ERROR`
/// events and forwards them to Slack (`SLACK_ERROR_CHANNEL`) and email
/// (`NOTIFY_EMAIL_TO`). A scheduled workflow hits this endpoint every few
/// hours and an operator confirms the canary message lands in the channel.
/// If the canary stops arriving, the alerting pipeline is broken BEFORE the
/// next real production outage surfaces the gap.
///
/// Returns the correlation ID so the caller can grep Cloud Logging or Slack
/// to confirm the event round-tripped.
///
/// # Errors
///
/// Returns an error when the caller's admin token lacks
/// [`AdminPermission::ViewConfiguration`].
pub async fn handle_tronc_canary(
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let correlation_id = Uuid::new_v4();
    error!(
        correlation_id = %correlation_id,
        event = "tronc-canary",
        "Slack alert pipeline health check (synthetic error, no action required)"
    );

    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "emitted",
            "correlation_id": correlation_id,
            "event": "tronc-canary",
            "message": "Synthetic ERROR event emitted — confirm it lands in Slack and email alert channels",
        })),
    ))
}

/// Hours since the last successful provider fetch before a connection counts as
/// stale, when the caller names no threshold. A capture that has not reached its
/// provider in a full day, on a connection that served inside that same day, has
/// stopped for a reason no athlete asked for.
const DEFAULT_STALE_AFTER_HOURS: i64 = 24;

/// How recently a connection must have served to be judged at all, by default.
/// Two days, so a weekend of not opening the app never alarms.
const DEFAULT_ACTIVE_WITHIN_HOURS: i64 = 48;

/// Bound on either threshold. Thirty days: past that the question stops being
/// "has capture stopped" and becomes "is this connection abandoned".
const MAX_THRESHOLD_HOURS: i64 = 720;

/// Cap on connections read in one snapshot, so an operator's curl can never
/// become an unbounded scan of every connection on the platform.
const SNAPSHOT_LIMIT: i64 = 5_000;

/// Query parameters for `/admin/diagnostics/capture-staleness`.
#[derive(Debug, Deserialize)]
pub struct CaptureStalenessQuery {
    /// Hours since the last successful fetch before a connection reads as stale.
    stale_after_hours: Option<i64>,
    /// A connection must have served within this many hours to be judged.
    active_within_hours: Option<i64>,
}

/// One connection being served from cache by a provider that has stopped
/// answering.
///
/// Carries ids, never an email or any other athlete-identifying field. This
/// repo is public and its CI logs are world-readable, so the monitor that reads
/// this endpoint cannot leak an athlete by echoing a response it was handed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleCapture {
    /// Owning tenant.
    pub tenant_id: String,
    /// Connection owner.
    pub user_id: String,
    /// Provider slug that has gone quiet.
    pub provider: String,
    /// When this connection last served the athlete.
    pub last_used_at: Option<DateTime<Utc>>,
    /// When a fetch last reached the provider — `None` when none ever has.
    pub last_fetch_at: Option<DateTime<Utc>>,
    /// Hours between `last_fetch_at` and the moment of the check. `None` when no
    /// fetch has ever succeeded, which is a distinct state from a large number
    /// and is deliberately not flattened into one.
    pub hours_since_fetch: Option<f64>,
}

/// The full report `/admin/diagnostics/capture-staleness` returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureStalenessReport {
    /// When the snapshot was taken.
    pub checked_at: DateTime<Utc>,
    /// The stale threshold actually applied, after clamping.
    pub stale_after_hours: i64,
    /// The activity window actually applied, after clamping.
    pub active_within_hours: i64,
    /// Active connections in the snapshot, before the activity window.
    pub connections_examined: usize,
    /// Those that served inside the activity window — the judged population.
    pub connections_judged: usize,
    /// The judged connections whose provider has gone quiet.
    pub stale: Vec<StaleCapture>,
}

/// Split a connection snapshot into the judged population and the stale subset.
///
/// The rule is a DIVERGENCE, not an age. `last_used_at` is touched at the serve
/// chokepoint on every serve — including one the durable cache answered — while
/// `last_fetch_at` advances only when a fetch genuinely reached the provider. A
/// connection that served recently and has not fetched in `stale_after` is
/// therefore one whose athlete is being answered from cache while their provider
/// has stopped responding, which is precisely the state that hid for days in
/// carnet#149.
///
/// Requiring recent use is what keeps this quiet: an athlete who has not opened
/// the app has nothing that should have fetched, so their old `last_fetch_at` is
/// correct rather than alarming, and they are excluded from the judged
/// population entirely.
///
/// `pub` so the decision is exercisable by the integration test suite, like
/// `historical_depth_covered` and `before_bounds_a_closed_window` in
/// `pierre_tool_runtime::activity_fetch`.
#[must_use]
pub fn partition_stale_captures(
    snapshot: &[CaptureFreshness],
    now: DateTime<Utc>,
    stale_after: Duration,
    active_within: Duration,
) -> (usize, Vec<StaleCapture>) {
    let judged: Vec<&CaptureFreshness> = snapshot
        .iter()
        .filter(|c| {
            c.last_used_at
                .is_some_and(|used| now - used <= active_within)
        })
        .collect();

    let stale = judged
        .iter()
        .filter(|c| {
            // A connection that has NEVER fetched is stale by definition: it has
            // been serving an athlete without one successful provider read.
            c.last_fetch_at
                .is_none_or(|fetched| now - fetched > stale_after)
        })
        .map(|c| StaleCapture {
            tenant_id: c.tenant_id.clone(),
            user_id: c.user_id.clone(),
            provider: c.provider.clone(),
            last_used_at: c.last_used_at,
            last_fetch_at: c.last_fetch_at,
            hours_since_fetch: c.last_fetch_at.map(|fetched| {
                // `num_seconds` over a fixed 3600, never a computed divisor.
                (now - fetched).num_seconds() as f64 / 3600.0
            }),
        })
        .collect();

    (judged.len(), stale)
}

/// `GET /admin/diagnostics/capture-staleness`.
///
/// Reports every live provider connection that is being served from cache while
/// its provider has stopped answering fetches.
///
/// The table this reads, `activity_fetch_freshness`, has held the honest answer
/// since it was added — it was simply never read by anything but a per-user
/// freshness report. On 2026-08-28 one athlete's sciotte capture stopped and was
/// still stopped two days later; three real activities never landed and the coach
/// kept answering from a training log that had frozen. `176ab975c` removed the
/// mask that made the freshness mark lie; this is the reader that makes it
/// matter (carnet#149, which cites that fix by its pre-squash SHA `5c3c405ce` —
/// a hash that never landed on main).
///
/// Both thresholds are clamped to `1..=720` hours and the applied values are
/// echoed in the response, so a caller can always see which question was
/// actually answered.
///
/// # Errors
///
/// Returns an error when the caller's admin token lacks
/// [`AdminPermission::ViewConfiguration`], or when the snapshot read fails.
pub async fn handle_capture_staleness(
    State(context): State<Arc<DiagnosticsContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<CaptureStalenessQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let stale_after_hours = params
        .stale_after_hours
        .unwrap_or(DEFAULT_STALE_AFTER_HOURS)
        .clamp(1, MAX_THRESHOLD_HOURS);
    let active_within_hours = params
        .active_within_hours
        .unwrap_or(DEFAULT_ACTIVE_WITHIN_HOURS)
        .clamp(1, MAX_THRESHOLD_HOURS);

    let snapshot = context
        .repos
        .activity_cache
        .capture_freshness_snapshot(SNAPSHOT_LIMIT)
        .await?;

    let now = Utc::now();
    let (connections_judged, stale) = partition_stale_captures(
        &snapshot,
        now,
        Duration::hours(stale_after_hours),
        Duration::hours(active_within_hours),
    );

    Ok((
        StatusCode::OK,
        Json(CaptureStalenessReport {
            checked_at: now,
            stale_after_hours,
            active_within_hours,
            connections_examined: snapshot.len(),
            connections_judged,
            stale,
        }),
    ))
}
