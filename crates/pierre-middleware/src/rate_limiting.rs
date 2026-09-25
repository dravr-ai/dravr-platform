// ABOUTME: The per-request budget slot, the gate on it, the X-RateLimit-* headers and the API-key usage row
// ABOUTME: Auth reports into a task-local; one outer layer renders the budget and records the admitted key's real outcome
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Request budget
//!
//! Authentication computes the caller's [`RequestBudget`] deep inside an
//! extractor, a `&HeaderMap` helper or the MCP transport's auth hook, and none
//! of them can reach the response. They report what they decided into a
//! task-local slot at the site that decides it ([`report_request_budget`],
//! [`report_api_key_request`], [`report_request_operation`]), and
//! [`request_budget_middleware`], installed once around the whole router,
//! opens the slot for each request and, once the handler has answered:
//!
//! - renders the budget as `X-RateLimit-Limit`, `X-RateLimit-Remaining` and
//!   `X-RateLimit-Reset`;
//! - writes the admitted API key's `api_key_usage` row with the request's
//!   real outcome: the response status, the milliseconds it took, and the
//!   endpoint it reached. That row is both what the dashboard's usage and
//!   request-log views read and what the key's sliding window counts.
//!
//! Properties:
//!
//! - **Isolation.** The slot belongs to one request's task and is never shared
//!   with another request or cached. It holds the principal's own counters:
//!   per API key, or per user across all of that user's tenants. Nothing is
//!   reported before a credential has validated and passed the account-status
//!   gate.
//! - **Last write wins.** A request that authenticates twice (an exhausted
//!   cookie falling through to a valid `Authorization` header) reports the
//!   credential that was checked last, which is always one the caller
//!   presented and that validated.
//! - **Scope.** Authentication that runs outside the request's task (a spawned
//!   task, stdio, background work) finds no slot: the response then carries no
//!   headers, never another request's, and no usage row is written. Every
//!   API-key authentication the server performs runs inside a request.
//! - **Outcome.** A handler that panics is recorded as the 500 the panic layer
//!   answers with. A request its client abandons before the response exists is
//!   recorded as 499, the "client closed request" status, so leaving early
//!   never takes a call out of the key's window.
//! - **Messaging.** Channel-link authentication never reports: its responses
//!   go to the messaging vendor, and the headers would hand it the athlete's
//!   quota.
//! - **Approximation.** The count is read when a request is admitted and its
//!   row written once it has answered, and the two are not atomic, so
//!   concurrent requests can overshoot the limit by the concurrency level.
//!
//! `Retry-After` is not written here: it belongs on a refusal, and
//! `impl IntoResponse for AppError` renders it from the refusal's own details.

use std::panic::{resume_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use chrono::{DateTime, Utc};
use futures_util::FutureExt;
use http::{HeaderMap, HeaderValue, StatusCode};
use pierre_auth::rate_limiting::RequestBudget;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::ApiKeyUsage;
use pierre_database::backends::UsageRepository;
use tokio::runtime::Handle;
use tracing::{trace, warn};

/// HTTP header names for the caller's request budget.
///
/// Lower case, as HTTP/2 sends every header name; HTTP/1 clients match them
/// case-insensitively.
pub mod headers {
    use http::HeaderName;

    /// Requests the caller's window admits
    pub const X_RATE_LIMIT_LIMIT: HeaderName = HeaderName::from_static("x-ratelimit-limit");
    /// Requests left in the window once this request is counted
    pub const X_RATE_LIMIT_REMAINING: HeaderName = HeaderName::from_static("x-ratelimit-remaining");
    /// When the window frees capacity, as UTC epoch seconds
    pub const X_RATE_LIMIT_RESET: HeaderName = HeaderName::from_static("x-ratelimit-reset");
}

/// The status recorded for a request whose client went away before the
/// response existed: nginx's "client closed request", which no handler
/// answers with, so it never passes for a real outcome.
const CLIENT_CLOSED_REQUEST: u16 = 499;

/// What authentication reported about the request its task serves.
#[derive(Default)]
struct RequestReport {
    /// The budget of the credential checked last.
    budget: Option<RequestBudget>,
    /// The API key that admitted the request, whose usage row the layer
    /// writes.
    api_key_id: Option<String>,
    /// The operation the request performed, when its route serves many.
    operation: Option<String>,
}

/// One request's report. `Arc` because two owners need it: the task-local
/// the reporters reach, and the layer's [`UsageRecorder`], which must still
/// read it after the handler's future is gone.
type ReportSlot = Arc<Mutex<RequestReport>>;

tokio::task_local! {
    static REQUEST_REPORT: ReportSlot;
}

/// Apply `update` to the current request's report, if there is one.
fn report(update: impl FnOnce(&mut RequestReport)) {
    let reported = REQUEST_REPORT.try_with(|slot| {
        update(&mut slot.lock().unwrap_or_else(PoisonError::into_inner));
    });
    if reported.is_err() {
        trace!("request report outside an HTTP request scope");
    }
}

/// The `X-RateLimit-*` headers for `budget`.
///
/// A metered budget renders its limit, what remains once this request is
/// counted (0 on a refusal), and its reset instant as UTC epoch seconds. An
/// unlimited budget renders nothing: no sentinel value, which an integer
/// parser would choke on.
#[must_use]
pub fn create_rate_limit_headers(budget: RequestBudget) -> HeaderMap {
    let mut map = HeaderMap::new();
    let (
        RequestBudget::Metered {
            limit, resets_at, ..
        },
        Some(remaining),
    ) = (budget, budget.remaining_after_this_request())
    else {
        return map;
    };
    map.insert(headers::X_RATE_LIMIT_LIMIT, HeaderValue::from(limit));
    map.insert(
        headers::X_RATE_LIMIT_REMAINING,
        HeaderValue::from(remaining),
    );
    map.insert(
        headers::X_RATE_LIMIT_RESET,
        HeaderValue::from(resets_at.timestamp()),
    );
    map
}

/// Record the budget of the credential that just authenticated, for the
/// enclosing [`request_budget_middleware`].
///
/// Outside one (stdio, background work, a spawned task) there is no response
/// to carry it, so the report is dropped.
pub fn report_request_budget(budget: RequestBudget) {
    report(|slot| slot.budget = Some(budget));
}

/// Record that `api_key_id` admitted this request.
///
/// The enclosing [`request_budget_middleware`] writes the key's
/// `api_key_usage` row once the request has an outcome. Outside one there is
/// no request to count, and nothing is written.
pub fn report_api_key_request(api_key_id: &str) {
    let api_key_id = api_key_id.to_owned();
    report(|slot| slot.api_key_id = Some(api_key_id));
}

/// Name the operation this request performed, for a route that serves many.
///
/// The usage row otherwise names the route (`GET /api/usage/status`); the MCP
/// endpoint serves every tool from `POST /mcp`, so its auth hook names the
/// tool, or the JSON-RPC method when the request calls none.
pub fn report_request_operation(operation: &str) {
    let operation = operation.to_owned();
    report(|slot| slot.operation = Some(operation));
}

/// Refuse a request over its budget.
///
/// # Errors
///
/// Returns [`AppError::rate_limit_exceeded`] (429) when a metered budget has
/// already admitted its limit, with the seconds until `resets_at` as its
/// retry window (at least one). An unlimited budget is never exceeded.
pub fn enforce_request_budget(budget: RequestBudget, now: DateTime<Utc>) -> AppResult<()> {
    match budget {
        RequestBudget::Metered {
            limit,
            used,
            resets_at,
        } if budget.is_exceeded() => {
            let wait = u64::try_from((resets_at - now).num_seconds()).unwrap_or(0);
            Err(AppError::rate_limit_exceeded(
                i64::from(used),
                i64::from(limit),
                wait,
            ))
        }
        RequestBudget::Metered { .. } | RequestBudget::Unlimited => Ok(()),
    }
}

/// Writes the usage row of the API key that admitted one request, exactly
/// once: with the response's status when there is one, and as
/// [`CLIENT_CLOSED_REQUEST`] when the request is dropped before it has one.
struct UsageRecorder {
    usage: Arc<dyn UsageRepository>,
    slot: ReportSlot,
    /// `METHOD /matched/route`, the endpoint a row names unless the request
    /// reported its operation.
    route: String,
    received_at: DateTime<Utc>,
    started: Instant,
    recorded: bool,
}

impl UsageRecorder {
    /// The row for this request's outcome, or `None` when no API key
    /// admitted it or the row was already taken.
    fn take_row(&mut self, status_code: u16) -> Option<ApiKeyUsage> {
        if self.recorded {
            return None;
        }
        self.recorded = true;
        let report = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        Some(ApiKeyUsage {
            id: None,
            api_key_id: report.api_key_id.clone()?,
            timestamp: self.received_at,
            tool_name: report
                .operation
                .clone()
                .unwrap_or_else(|| self.route.clone()),
            response_time_ms: u32::try_from(self.started.elapsed().as_millis()).ok(),
            status_code,
            error_message: None,
            request_size_bytes: None,
            response_size_bytes: None,
            ip_address: None,
            user_agent: None,
        })
    }

    async fn record(&mut self, status: StatusCode) {
        if let Some(row) = self.take_row(status.as_u16()) {
            write_usage_row(self.usage.as_ref(), &row).await;
        }
    }
}

impl Drop for UsageRecorder {
    /// A request dropped before the layer recorded it: its client went away.
    /// The write outlives the request, so it runs on its own task.
    fn drop(&mut self) {
        let Some(row) = self.take_row(CLIENT_CLOSED_REQUEST) else {
            return;
        };
        let Ok(runtime) = Handle::try_current() else {
            warn!(
                api_key_id = %row.api_key_id,
                endpoint = %row.tool_name,
                "No runtime to record an abandoned API-key request (rate limiting counter impacted)"
            );
            return;
        };
        let usage = Arc::clone(&self.usage);
        runtime.spawn(async move { write_usage_row(usage.as_ref(), &row).await });
    }
}

/// Best-effort: a failed write is logged and the response still goes out.
async fn write_usage_row(usage: &dyn UsageRepository, row: &ApiKeyUsage) {
    if let Err(e) = usage.record_api_key(row).await {
        warn!(
            api_key_id = %row.api_key_id,
            endpoint = %row.tool_name,
            status = row.status_code,
            error = %e,
            "Failed to record api_key_usage (rate limiting counter and usage analytics impacted)"
        );
    }
}

/// The endpoint a request reached, as `METHOD /matched/route`: the route
/// template, never the raw path, so ids stay out of the usage log and one
/// endpoint groups as one row.
fn route_label(request: &Request) -> String {
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| request.uri().path(), MatchedPath::as_str);
    format!("{} {path}", request.method())
}

/// Carry what authentication reported onto the response and the usage log.
///
/// Opens the request's slot, runs the rest of the stack inside it (Axum's
/// `Next::run` awaits the handler and every extractor in this task), then
/// writes the admitted API key's usage row with the response's status and
/// latency, and inserts the `X-RateLimit-*` headers for the budget reported
/// last. `insert` replaces, so a header another layer set is never
/// duplicated. A request that never authenticated gets neither.
///
/// A panicking handler is recorded as a 500 and the panic resumed, so the
/// panic layer outside still answers it.
pub async fn request_budget_middleware(
    State(usage): State<Arc<dyn UsageRepository>>,
    request: Request,
    next: Next,
) -> Response {
    let slot = ReportSlot::default();
    let mut recorder = UsageRecorder {
        usage,
        slot: Arc::clone(&slot),
        route: route_label(&request),
        received_at: Utc::now(),
        started: Instant::now(),
        recorded: false,
    };

    let outcome = REQUEST_REPORT
        .scope(
            Arc::clone(&slot),
            AssertUnwindSafe(next.run(request)).catch_unwind(),
        )
        .await;
    let mut response = match outcome {
        Ok(response) => response,
        Err(panic) => {
            recorder.record(StatusCode::INTERNAL_SERVER_ERROR).await;
            resume_unwind(panic)
        }
    };
    recorder.record(response.status()).await;

    let budget = slot.lock().unwrap_or_else(PoisonError::into_inner).budget;
    if let Some(budget) = budget {
        let target = response.headers_mut();
        for (name, value) in &create_rate_limit_headers(budget) {
            target.insert(name, value.clone());
        }
    }
    response
}
