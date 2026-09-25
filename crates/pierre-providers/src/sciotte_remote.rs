// ABOUTME: HTTP client for the dedicated dravr-sciotte scraper service (ADR-021).
// ABOUTME: Reads DRAVR_SCIOTTE_REMOTE_URL + DRAVR_SCIOTTE_AUDIENCE; the sole scrape path since the Phase 4 cutover.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Remote sciotte scraper client.
//!
//! The interactive login **and** the activity/athlete scrape run on a dedicated
//! `dravr-sciotte-server`, isolating the memory-heavy headless Chrome off the
//! multi-tenant API pod (see [[ADR-021]]). Since the Phase 4 cutover this is the
//! only scrape path: `DRAVR_SCIOTTE_REMOTE_URL` must be set or every sciotte call
//! errors — there is no in-process fallback.
//!
//! ## Session-of-record boundary
//!
//! The scraper service holds sessions only transiently (in-memory + ephemeral
//! disk). The **platform stays the session-of-record**: on a successful login it
//! [`export_session`](RemoteSciotteClient::export_session)s the full
//! `AuthSession` and persists it (KMS-encrypted `oauth_tokens`), then
//! [`import_session`](RemoteSciotteClient::import_session)s it back before a
//! scrape when the service has scaled to zero / been redeployed. This keeps the
//! platform's sessions durable — no re-login churn on redeploy.
//!
//! ## Auth
//!
//! Requests carry a Google-signed identity token addressed to
//! `DRAVR_SCIOTTE_AUDIENCE`, which the scraper service verifies (its previous
//! shared-key gate failed open when the key variable was unset — the defect
//! behind registre#36). The token source is metadata-server backed and caches
//! the live token, so minting is a lock read on all but the first call each
//! hour. A scraper on this machine's own loopback is the one exemption: only a
//! developer's own process can be there, so it is called unauthenticated.
//!
//! ## Failures without an answer
//!
//! Every request carries a fresh `x-request-id`, which the service's
//! request guard adopts, echoes and logs the request under, so one id finds a
//! request in both services' logs. A request that ends without a response —
//! the client's own timeout, a service it cannot reach, a connection closed
//! mid-request — and a response in which the service reports it could not
//! finish ([`service_failure_error`](crate::sciotte_remote::service_failure_error))
//! surface as `ExternalServiceUnavailable`, naming the request id, how long
//! the request ran and the transport's own cause. An idempotent `GET` whose
//! connection closed before any response is re-sent once.

use std::convert::identity;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
/// One athlete on a coach account's roster, as the scraper lists it.
pub use dravr_sciotte::models::CoachedAthlete;
/// The scraper's own types for the session a read goes through, whose data it
/// names and what the signed-in account is: callers outside this crate name
/// them through here, since only this crate depends on `dravr-sciotte`.
pub use dravr_sciotte::models::{AccountRole, AthleteId, AthleteProfile, AuthSession};
use dravr_sciotte::models::{
    Activity as SciotteActivity, DailySummary, PlannedWorkout as SciottePlannedWorkout,
};
use dravr_tronc::iam::IdTokenSource;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::Deserialize;
use serde_json::{Map, Value};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::sciotte_transport::{transport_error, TransportFailure, REQUEST_ID_HEADER};

/// Environment variable holding the remote scraper's base URL. Required since
/// the Phase 4 cutover — unset makes `require_from_env` error (no fallback).
pub const ENV_REMOTE_URL: &str = "DRAVR_SCIOTTE_REMOTE_URL";
/// Environment variable naming the audience identity tokens are addressed to.
///
/// Must match what the scraper service accepts, which terraform guarantees by
/// setting both from one local. A stable custom audience rather than the
/// service URL, so neither side depends on a value Cloud Run generates at
/// creation.
pub const ENV_AUDIENCE: &str = "DRAVR_SCIOTTE_AUDIENCE";

/// Value of the `error` field the scraper service sheds with — its
/// `busy_response` answers a saturated Chrome budget with
/// `503 {"error":"scraper_busy","reason":…,"retry_after_secs":N}` plus a
/// `Retry-After` header.
const SHED_ERROR_MARKER: &str = "scraper_busy";

/// Key the shed's wait window travels under.
///
/// It is both the field name in the service's body and the
/// [`AppError::details`] entry [`shed_retry_after_secs`] reads back out, so a
/// route layer can echo it as `Retry-After`.
pub const RETRY_AFTER_SECS_DETAIL: &str = "retry_after_secs";

/// Wait advertised when a shed response carries no `retry_after_secs`.
/// Deliberately short: a shed clears as soon as one in-flight scrape releases
/// its permit, so an over-long hint parks the caller on a service that is
/// already free again.
const SHED_RETRY_AFTER_FALLBACK_SECS: u64 = 30;

/// `error.type` the service's request guard answers a handler panic with, on a
/// `500`: dravr-tronc's `request_guard::HANDLER_PANIC`.
const HANDLER_PANIC_MARKER: &str = "handler_panic";

/// `error.type` the service's request guard answers a request still running at
/// its deadline with, on a `504`: dravr-tronc's `request_guard::REQUEST_TIMEOUT`.
const REQUEST_TIMEOUT_MARKER: &str = "request_timeout";

/// How many times an idempotent `GET` is re-sent after its connection closed
/// before any response arrived.
///
/// One: that failure is the signature of a pooled keep-alive connection the
/// service closed just as the request went out, or of an instance that dropped
/// the request, and a read is safe to repeat. A second consecutive close is no
/// longer a race, and each attempt can hold a browser for a whole scrape.
const CLOSED_CONNECTION_GET_RETRIES: u32 = 1;

/// Outcome of an interactive-login step, mirroring the scraper service's
/// `{status, ...}` login response.
///
/// Continuation variants carry the server-minted `flow_id` naming the parked
/// login flow — echoed back on `submit_otp`/`select_2fa` so a multi-provider,
/// multi-user service resumes the right browser (the server falls back to its
/// sole pending flow when the id is absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteLoginOutcome {
    /// Login complete; the service now holds the session under `session_id`.
    /// The caller exports + persists it, never leaving it only server-side.
    Authenticated {
        /// Server-side session id to export the full session with.
        session_id: String,
        /// Provider the session authenticates against (`"garmin"`, `"strava"`).
        /// The caller maps it to the backend name and persists under it — so
        /// the platform never tracks the provider across the multi-step flow.
        provider: String,
    },
    /// A one-time password / 2FA code is required (`submit_otp`).
    OtpRequired {
        /// Parked flow to resume with the code.
        flow_id: Option<String>,
    },
    /// The user must pick a 2FA method (`select_2fa`); options passed through
    /// verbatim so the caller renders the same choice the in-process path does.
    TwoFactorChoice {
        /// The provider's 2FA method options, verbatim.
        options: Value,
        /// Parked flow to resume with the selection.
        flow_id: Option<String>,
    },
    /// A number-matching challenge — the user approves the number on their phone.
    NumberMatch {
        /// The number the user must confirm on their device.
        number: String,
        /// Parked flow the challenge belongs to.
        flow_id: Option<String>,
    },
    /// The provider rejected the credentials / flow.
    Failed(String),
}

/// What `GET /api/activities` answers with: the scraped rows and whether the
/// service read the list's head.
///
/// `head_complete` is `false` when sciotte's fresh-head fetch failed and the
/// newest rows may be missing from `activities` (carnet#151). A service
/// predating the field omits it; `true` is assumed then, which is what every
/// consumer did before the field existed.
#[derive(Debug, Deserialize)]
pub struct RemoteActivityList {
    /// The scraped rows, newest first.
    pub activities: Vec<SciotteActivity>,
    /// Whether the newest rows the site shows are in `activities`.
    #[serde(default = "head_complete_when_unstated")]
    pub head_complete: bool,
}

const fn head_complete_when_unstated() -> bool {
    true
}

/// Query knobs for a remote activity fetch. Mirrors the `ActivityParams`
/// subset the scraper service honours.
#[derive(Debug, Clone, Default)]
pub struct RemoteActivityQuery {
    /// Max activities to return.
    pub limit: Option<u32>,
    /// Lower bound (epoch seconds) — only activities on/after this instant.
    pub after_epoch: Option<i64>,
    /// Upper bound (epoch seconds) — only activities on/before this instant.
    pub before_epoch: Option<i64>,
    /// Optional sport-type filter.
    pub sport_type: Option<String>,
    /// Whether to enrich each activity with its detail page (N+1, slower),
    /// newest first, up to the scraper's own per-request ceiling.
    pub enrich_details: bool,
    /// Whose activities to read, by the provider's athlete id. `None` reads
    /// the signed-in account's own; a `TrainingPeaks` coach account has none
    /// and answers [`ATHLETE_REQUIRED`].
    pub athlete: Option<AthleteId>,
}

/// What `GET /api/planned-workouts` answers with: the planned workouts,
/// oldest first, and how many the service says it sent.
#[derive(Debug, Deserialize)]
pub struct RemotePlannedWorkoutList {
    /// How many workouts the service put in `planned_workouts`.
    pub count: usize,
    /// The planned workouts, oldest first.
    pub planned_workouts: Vec<SciottePlannedWorkout>,
}

/// Body `error` marker of the scraper's `400` for a read that named no
/// athlete on a coach account, which has no calendar of its own.
pub const ATHLETE_REQUIRED: &str = "athlete_required";

/// Body `error` marker of the scraper's `403` for an athlete the provider
/// refused: outside the signed-in coach's roster, or any athlete at all on a
/// provider that reads only the signed-in account.
pub const ATHLETE_NOT_ACCESSIBLE: &str = "athlete_not_accessible";

/// Body `error` markers of the scraper's `400`s for a query it refused to
/// parse — a malformed athlete id, a missing, malformed or inverted window.
/// The platform builds both, so either one is a platform bug.
const MALFORMED_QUERY_MARKERS: &[&str] = &["invalid_athlete", "invalid_window"];

/// Key of the [`AppError::details`] entry naming which scraper refusal an
/// error carries, read back by [`sciotte_refusal`].
const SCIOTTE_REFUSAL_DETAIL: &str = "sciotte_refusal";

/// The structured backpressure error for a scraper-service load-shed, or
/// `None` when the response is a genuine failure.
///
/// The service sheds a saturated Chrome budget with `503` +
/// `{"error":"scraper_busy","reason":…,"retry_after_secs":N}` and a
/// `Retry-After` header. That body carries no login `status` field, so
/// recognising it *before* any status match is what keeps designed
/// load-shedding out of the catch-all that reports a system failure — one
/// operator alert plus one `sync.failed` business event per shed request,
/// precisely when the service is saturated.
///
/// [`ErrorCode::ResourceUnavailable`] renders as `503` and keeps the technical
/// reason out of the client's reach; the wait the service computed rides in
/// `details` so a route layer can hand it back as `Retry-After`.
#[must_use]
pub fn backpressure_error(http_status: StatusCode, body: &Value) -> Option<AppError> {
    let shed = http_status == StatusCode::SERVICE_UNAVAILABLE
        || body.get("error").and_then(Value::as_str) == Some(SHED_ERROR_MARKER);
    if !shed {
        return None;
    }

    let retry_after_secs = body
        .get(RETRY_AFTER_SECS_DETAIL)
        .and_then(Value::as_u64)
        .unwrap_or(SHED_RETRY_AFTER_FALLBACK_SECS);
    let reason = body
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or(SHED_ERROR_MARKER);

    let mut error = AppError::resource_unavailable(format!(
        "sciotte shed the request ({reason}); retry after {retry_after_secs}s"
    ));
    let mut details = Map::new();
    details.insert(
        RETRY_AFTER_SECS_DETAIL.to_owned(),
        Value::from(retry_after_secs),
    );
    error.details = Some(Box::new(Value::Object(details)));
    Some(error)
}

/// The wait window a load-shed advertises, or `None` when `error` is anything
/// else.
///
/// Callers branch on this to answer a shed with the service's own
/// `Retry-After` instead of routing it through the system-failure path.
#[must_use]
pub fn shed_retry_after_secs(error: &AppError) -> Option<u64> {
    if !matches!(error.code, ErrorCode::ResourceUnavailable) {
        return None;
    }
    error
        .details
        .as_ref()?
        .get(RETRY_AFTER_SECS_DETAIL)?
        .as_u64()
}

/// Body `error` markers on a scraper-service `401` that mean the ATHLETE's
/// session is gone or dead: `session_not_found` (no session held) and
/// `session_expired` (the upstream provider rejected the session's cookies).
/// Deliberately NOT `unauthorized` — that is the service's rejected-identity-
/// token answer (audience mismatch, expired token), an operator
/// misconfiguration that must keep alerting as an internal fault instead of
/// sending the athlete on a pointless re-login.
const SESSION_AUTH_ERROR_MARKERS: &[&str] = &[SESSION_NOT_FOUND_MARKER, "session_expired"];

/// Body `error` marker of the scraper's `401` for a session id it holds no
/// session under.
const SESSION_NOT_FOUND_MARKER: &str = "session_not_found";

/// The auth-shaped error for a scraper-service `401` whose body carries a
/// session-death marker, or `None` for anything else.
///
/// A session-shaped `401` means the athlete must re-login, so it surfaces
/// as [`ErrorCode::ProviderAuthRequired`] — the chat pipeline's
/// auth-recovery stage turns that into a hosted-login reconnect link, where
/// an `internal` error would have left the agent apologising with nothing
/// actionable (live gap behind the 2026-08-11 "problème de connexion"
/// incident). The provider slug here is the generic `sciotte`; the provider
/// layer re-tags it with the owning backend name so a `sciotte_garmin`
/// session reconnects to Garmin, not Strava.
#[must_use]
pub fn auth_required_error(http_status: StatusCode, body: &Value) -> Option<AppError> {
    if http_status != StatusCode::UNAUTHORIZED {
        return None;
    }
    let marker = body.get("error").and_then(Value::as_str)?;
    SESSION_AUTH_ERROR_MARKERS
        .contains(&marker)
        .then(|| AppError::provider_auth_required("sciotte"))
}

/// The typed error for a scraper refusal of the athlete a read named, or
/// `None` for anything else.
///
/// Neither refusal is a dead session, so neither may become
/// [`ErrorCode::ProviderAuthRequired`]: that code sends the athlete through a
/// re-login, and signing in again changes nothing about which account they
/// hold or whose roster an athlete is on.
///
/// - `400` [`ATHLETE_REQUIRED`]: the session is a coach account, which has no
///   calendar or activities of its own. An [`ErrorCode::InvalidInput`], whose
///   message reaches the reader: a link to each athlete, made from a group
///   the coach coaches, is what reads their workouts. The provider layer
///   re-words it with its brand name.
/// - `403` [`ATHLETE_NOT_ACCESSIBLE`]: the named athlete is not one this
///   session may read. An [`ErrorCode::PermissionDenied`].
/// - `400` `invalid_athlete` / `invalid_window`: the query the platform built
///   was malformed, so an internal error — no action of the athlete's fixes it.
///
/// Each of the first two carries its marker under the details key
/// [`sciotte_refusal`] reads, so a caller can branch on the kind.
#[must_use]
pub fn athlete_refusal_error(http_status: StatusCode, body: &Value) -> Option<AppError> {
    let marker = body.get("error").and_then(Value::as_str)?;
    match (http_status, marker) {
        (StatusCode::BAD_REQUEST, ATHLETE_REQUIRED) => Some(with_refusal_marker(
            AppError::invalid_input(
                "This account is a coach account, and a coach account has no training \
                 calendar of its own. To read an athlete's workouts, link each athlete from a \
                 group you coach; the athlete confirms the link.",
            ),
            ATHLETE_REQUIRED,
        )),
        (StatusCode::FORBIDDEN, ATHLETE_NOT_ACCESSIBLE) => Some(athlete_not_accessible()),
        (StatusCode::BAD_REQUEST, malformed) if MALFORMED_QUERY_MARKERS.contains(&malformed) => {
            let detail = body.get("message").and_then(Value::as_str).unwrap_or("");
            Some(AppError::internal(format!(
                "sciotte refused the query the platform built as {malformed}: {detail}"
            )))
        }
        _ => None,
    }
}

/// The refusal of an athlete the session may not read.
///
/// It carries the [`ATHLETE_NOT_ACCESSIBLE`] marker: the scraper's `403` maps
/// to it, and a delegated read answers it when the coach's roster no longer
/// lists the athlete it names.
#[must_use]
pub fn athlete_not_accessible() -> AppError {
    with_refusal_marker(
        AppError::new(
            ErrorCode::PermissionDenied,
            "That athlete is not on this coach account's roster",
        ),
        ATHLETE_NOT_ACCESSIBLE,
    )
}

/// `error` with `marker` recorded under the refusal details key.
fn with_refusal_marker(mut error: AppError, marker: &str) -> AppError {
    let mut details = Map::new();
    details.insert(
        SCIOTTE_REFUSAL_DETAIL.to_owned(),
        Value::String(marker.to_owned()),
    );
    error.details = Some(Box::new(Value::Object(details)));
    error
}

/// The scraper refusal `error` carries ([`ATHLETE_REQUIRED`],
/// [`ATHLETE_NOT_ACCESSIBLE`]), or `None` for any other error.
#[must_use]
pub fn sciotte_refusal(error: &AppError) -> Option<&str> {
    error
        .details
        .as_ref()?
        .get(SCIOTTE_REFUSAL_DETAIL)?
        .as_str()
}

/// The transient-unavailability error for a response in which the service, or
/// the gateway in front of it, says the request could not be finished — or
/// `None` for any other response:
///
/// - `504`: the request outlived a deadline — the service's request guard
///   (`request_timeout`) or the gateway's own;
/// - `502`: the gateway got no usable answer from the service;
/// - `500` with a `handler_panic` body: a service handler panicked and the
///   request guard answered for it.
///
/// None of these is the athlete's doing, and the same request can succeed a
/// moment later, so each is [`ErrorCode::ExternalServiceUnavailable`] — the
/// provider is temporarily unavailable. Every other `500` keeps its internal
/// classification: a scraper error the service chose to report.
///
/// The message names `request_id`, the `x-request-id` the request was
/// sent under and the service logged it under.
#[must_use]
pub fn service_failure_error(
    http_status: StatusCode,
    body: &Value,
    request_id: &str,
) -> Option<AppError> {
    let guard_type = body
        .get("error")
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str);
    let what = match (http_status, guard_type) {
        (StatusCode::GATEWAY_TIMEOUT, Some(REQUEST_TIMEOUT_MARKER)) => {
            "outlived the service's request deadline"
        }
        (StatusCode::GATEWAY_TIMEOUT, _) => "outlived the gateway's deadline",
        (StatusCode::BAD_GATEWAY, _) => "got no usable answer through the gateway",
        (StatusCode::INTERNAL_SERVER_ERROR, Some(HANDLER_PANIC_MARKER)) => {
            "failed in a service handler that panicked"
        }
        _ => return None,
    };
    let detail = body
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("no detail");
    Some(AppError::new(
        ErrorCode::ExternalServiceUnavailable,
        format!(
            "sciotte request ({REQUEST_ID_HEADER} {request_id}) {what} ({http_status}): {detail}"
        ),
    ))
}

/// The unavailability error for a `401 session_not_found` answering a read
/// that was re-sent after its first connection closed, or `None` for any other
/// answer.
///
/// The platform imports the session before every read, so a service that does
/// not hold it right after a connection closed mid-request lost it along with
/// that connection: the instance went down — a crash, an out-of-memory kill —
/// and the re-send reached its replacement. The athlete's session is intact and
/// the next read imports it again, so this is the provider being unavailable
/// for a moment, never the re-login [`auth_required_error`] would send the
/// athlete through.
fn session_lost_with_connection(
    http_status: StatusCode,
    body: &Value,
    request_id: &str,
) -> Option<AppError> {
    let marker = body.get("error").and_then(Value::as_str)?;
    (http_status == StatusCode::UNAUTHORIZED && marker == SESSION_NOT_FOUND_MARKER).then(|| {
        AppError::new(
            ErrorCode::ExternalServiceUnavailable,
            format!(
                "sciotte no longer held the session when the read was re-sent \
                 ({REQUEST_ID_HEADER} {request_id}): the instance that held it went down \
                 with the connection that closed"
            ),
        )
    })
}

/// Map a non-success scrape response to an error, classifying the service's
/// designed load-shed as retryable backpressure, a request it could not finish
/// as [`service_failure_error`] names, a session lost with a closed connection
/// as [`session_lost_with_connection`] names, its session-death `401` as an
/// auth-shaped failure, and its athlete refusals as the typed errors
/// [`athlete_refusal_error`] names, rather than any of them as a system
/// failure. Backpressure wins over the status match: a shed carries its own
/// body marker and must not be misread as anything else.
///
/// `operation` names the endpoint in the internal message only — the
/// internal message never reaches a client verbatim.
async fn scrape_failure(operation: &str, sent: Sent) -> AppError {
    let status = sent.response.status();
    let body = sent.response.json::<Value>().await.unwrap_or_default();
    if let Some(shed) = backpressure_error(status, &body) {
        return shed;
    }
    let lost_session = if sent.resent {
        session_lost_with_connection(status, &body, &sent.request_id)
    } else {
        None
    };
    lost_session
        .or_else(|| service_failure_error(status, &body, &sent.request_id))
        .or_else(|| auth_required_error(status, &body))
        .or_else(|| athlete_refusal_error(status, &body))
        .unwrap_or_else(|| {
            AppError::internal(format!(
                "sciotte {operation} returned {status} ({REQUEST_ID_HEADER} {})",
                sent.request_id
            ))
        })
}

/// Map a non-success response from a session-transfer endpoint (export,
/// import): a request the service could not finish as
/// [`service_failure_error`] names, anything else as an internal fault.
async fn transfer_failure(operation: &str, sent: Sent) -> AppError {
    let status = sent.response.status();
    let body = sent.response.json::<Value>().await.unwrap_or_default();
    service_failure_error(status, &body, &sent.request_id).unwrap_or_else(|| {
        AppError::internal(format!(
            "sciotte {operation} returned {status} ({REQUEST_ID_HEADER} {})",
            sent.request_id
        ))
    })
}

/// A response from the service, with the request id it was sent under.
struct Sent {
    response: Response,
    request_id: String,
    /// Whether this answers a re-send: the first attempt's connection closed
    /// before any response.
    resent: bool,
}

/// Client for the dedicated `dravr-sciotte-server`.
///
/// Cheap to clone (wraps a connection-pooled [`reqwest::Client`]); construct via
/// [`require_from_env`](Self::require_from_env) at the call site — the scrape runs
/// only on the service since the Phase 4 cutover.
#[derive(Clone)]
pub struct RemoteSciotteClient {
    http: Client,
    base_url: String,
    /// Mints the identity token the scraper service requires, or `None` when
    /// the service is on this machine's own loopback (a developer's process,
    /// called unauthenticated).
    ///
    /// `Arc` because the client is cloned per call site and the source caches a
    /// token behind a lock — cloning the source itself would give each clone
    /// its own cache and mint a token per call instead of per hour.
    tokens: Option<Arc<IdTokenSource>>,
}

/// Whether a scraper URL points at this machine's own loopback.
///
/// Only a developer's own process can be there — a Cloud Run service never is,
/// from the platform's point of view — so a loopback scraper is the one case
/// where skipping the ID token cannot widen anything.
fn is_loopback(url: &str) -> bool {
    let host = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split(['/', ':'])
        .next()
        .unwrap_or_default();
    matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

impl RemoteSciotteClient {
    /// Build a client from the environment, or `None` when `DRAVR_SCIOTTE_REMOTE_URL`
    /// is unset or empty. Scrape-path callers use
    /// [`require_from_env`](Self::require_from_env), which turns the unset case into
    /// an error — since the Phase 4 cutover the service is the only scrape path, so
    /// a missing URL is a deployment fault, not a fallback signal.
    ///
    /// # Errors
    ///
    /// Returns an error only when the HTTP client itself cannot be built.
    pub fn from_env() -> AppResult<Option<Self>> {
        let Ok(base_url) = env::var(ENV_REMOTE_URL) else {
            return Ok(None);
        };
        let base_url = base_url.trim_end_matches('/').to_owned();
        if base_url.is_empty() {
            return Ok(None);
        }
        let http = Client::builder()
            // The 2FA flow parks a live browser across this one request; the HTTP
            // timeout must outlast the scraper's own login window (300s) plus
            // headroom so the platform doesn't cut the call before the scraper does.
            // It also outlasts the service's REST request deadline
            // (DRAVR_SCIOTTE_REQUEST_TIMEOUT, 320s by default), so a request that
            // overruns arrives as the service's `request_timeout` 504, which
            // `service_failure_error` classifies, rather than as this timeout.
            .timeout(Duration::from_secs(330))
            .build()
            .map_err(|e| AppError::internal(format!("build sciotte http client: {e}")))?;

        // Both or neither, with loopback as the one exemption. A remote URL
        // without an audience would send unsigned requests the scraper refuses,
        // so it is treated as "not configured" — one warning at construction
        // instead of a 401 on every scrape that reads as a session problem.
        let audience = env::var(ENV_AUDIENCE).ok().filter(|a| !a.is_empty());
        let tokens = if is_loopback(&base_url) {
            debug!(base_url = %base_url, "sciotte on loopback; calling it unauthenticated (development)");
            None
        } else if let Some(audience) = audience {
            Some(Arc::new(IdTokenSource::new(audience, http.clone())))
        } else {
            warn!(
                "{ENV_REMOTE_URL} is set but {ENV_AUDIENCE} is not; disabling the \
                 remote sciotte client rather than sending requests it will refuse"
            );
            return Ok(None);
        };

        debug!(base_url = %base_url, "Remote sciotte client enabled");
        Ok(Some(Self {
            http,
            base_url,
            tokens,
        }))
    }

    /// Build a client, erroring when `DRAVR_SCIOTTE_REMOTE_URL` is unset.
    ///
    /// Since the Phase 4 cutover the sciotte scrape runs *only* on the
    /// dedicated service ([[ADR-021]]) — there is no in-process fallback — so a
    /// missing URL is a deployment misconfiguration, surfaced as an internal
    /// error rather than silently doing nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when the URL is unset/empty or the HTTP client can't be built.
    pub fn require_from_env() -> AppResult<Self> {
        Self::from_env()?.ok_or_else(|| {
            AppError::internal(
                "DRAVR_SCIOTTE_REMOTE_URL must be set (with DRAVR_SCIOTTE_AUDIENCE \
                 for a non-loopback service) — the in-process sciotte scrape path \
                 was removed in the ADR-021 Phase 4 cutover",
            )
        })
    }

    /// Build a request carrying the identity token the scraper requires.
    ///
    /// `None` in `tokens` is the loopback case and nothing else — the
    /// constructor refuses a non-loopback URL without an audience — so having
    /// no source means a developer's scraper on 127.0.0.1, which takes the
    /// request unauthenticated.
    async fn request(&self, method: Method, path: &str) -> AppResult<RequestBuilder> {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base_url));
        if let Some(tokens) = &self.tokens {
            let token = tokens.token().await.map_err(|e| {
                AppError::new(
                    ErrorCode::ExternalServiceError,
                    format!("could not mint an identity token for sciotte: {e}"),
                )
            })?;
            req = req.bearer_auth(token);
        }
        Ok(req)
    }

    /// Send one request to the service under a fresh [`REQUEST_ID_HEADER`],
    /// `decorate` adding its headers, query and body.
    ///
    /// A `GET` whose connection closed before any response is re-sent under a
    /// new id, [`CLOSED_CONNECTION_GET_RETRIES`] time(s). Nothing else is
    /// retried: a timeout has already spent the client's whole budget, and a
    /// `POST` — a login step, a session import — may have acted on the
    /// service before the connection went.
    ///
    /// # Errors
    ///
    /// [`transport_error`] when no response arrives; the token error when the
    /// identity token cannot be minted.
    async fn send<F>(
        &self,
        operation: &str,
        method: Method,
        path: &str,
        decorate: F,
    ) -> AppResult<Sent>
    where
        F: Fn(RequestBuilder) -> RequestBuilder + Send + Sync,
    {
        let retries = if method == Method::GET {
            CLOSED_CONNECTION_GET_RETRIES
        } else {
            0
        };
        let mut attempt = 0;
        loop {
            let request_id = Uuid::new_v4().to_string();
            let request = decorate(self.request(method.clone(), path).await?)
                .header(REQUEST_ID_HEADER, request_id.as_str());
            let started = Instant::now();
            let error = match request.send().await {
                Ok(response) => {
                    return Ok(Sent {
                        response,
                        request_id,
                        resent: attempt > 0,
                    })
                }
                Err(error) => error,
            };
            let failure = TransportFailure::of(&error);
            let error = transport_error(operation, &request_id, started, failure, error);
            if failure != TransportFailure::ClosedBeforeResponse || attempt >= retries {
                return Err(error);
            }
            attempt += 1;
            warn!(
                operation,
                request_id = %request_id,
                error = %error.message,
                "sciotte closed the connection before responding; re-sending the GET once"
            );
        }
    }

    /// POST `/auth/login-with-credentials` — start an interactive login on
    /// `provider` (`"garmin"`, `"strava"`): the multi-provider service needs
    /// it named on every new flow.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure or an unparseable response.
    pub async fn login_with_credentials(
        &self,
        email: &str,
        password: &str,
        method: &str,
        provider: &str,
    ) -> AppResult<RemoteLoginOutcome> {
        let body = serde_json::json!({
            "email": email,
            "password": password,
            "method": method,
            "provider": provider,
        });
        let sent = self
            .send(
                "login",
                Method::POST,
                "/auth/login-with-credentials",
                |req| req.json(&body),
            )
            .await?;
        Self::parse_login_outcome(sent).await
    }

    /// POST `/auth/submit-otp` — continue an interactive login with an OTP/2FA
    /// code. `flow_id` names the parked flow (from the continuation outcome);
    /// `None` lets the server resume its sole pending flow.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure or an unparseable response.
    pub async fn submit_otp(
        &self,
        code: &str,
        flow_id: Option<&str>,
    ) -> AppResult<RemoteLoginOutcome> {
        let body = serde_json::json!({ "code": code, "flow_id": flow_id });
        let sent = self
            .send("submit-otp", Method::POST, "/auth/submit-otp", |req| {
                req.json(&body)
            })
            .await?;
        Self::parse_login_outcome(sent).await
    }

    /// POST `/auth/select-2fa` — pick a 2FA method during an interactive login.
    /// `flow_id` names the parked flow; `None` lets the server resume its sole
    /// pending flow.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure or an unparseable response.
    pub async fn select_2fa(
        &self,
        option_id: &str,
        flow_id: Option<&str>,
    ) -> AppResult<RemoteLoginOutcome> {
        let body = serde_json::json!({ "option_id": option_id, "flow_id": flow_id });
        let sent = self
            .send("select-2fa", Method::POST, "/auth/select-2fa", |req| {
                req.json(&body)
            })
            .await?;
        Self::parse_login_outcome(sent).await
    }

    /// GET `/auth/sessions/{id}/export` — retrieve the full `AuthSession` so the
    /// platform can persist it as the durable session-of-record. The service
    /// returns `{provider, session}`; the provider is dropped here because the
    /// caller already knows it from the `Authenticated` outcome.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, a non-success status, or a body
    /// that does not deserialize into the export shape.
    pub async fn export_session(&self, session_id: &str) -> AppResult<AuthSession> {
        #[derive(Deserialize)]
        struct ExportResponse {
            session: AuthSession,
        }
        let sent = self
            .send(
                "export",
                Method::GET,
                &format!("/auth/sessions/{session_id}/export"),
                identity,
            )
            .await?;
        if !sent.response.status().is_success() {
            return Err(transfer_failure("export", sent).await);
        }
        Ok(sent
            .response
            .json::<ExportResponse>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte export decode: {e}")))?
            .session)
    }

    /// POST `/auth/import-session` — re-hydrate the service's transient store from
    /// the platform's durable session before a scrape, tagged with the provider
    /// (`"garmin"`, `"strava"`) so the multi-provider service routes its scrapes.
    /// Idempotent; returns the session id the scrape endpoints key on via
    /// `X-Session-Id`.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure or a non-success status.
    pub async fn import_session(&self, session: &AuthSession, provider: &str) -> AppResult<String> {
        #[derive(Deserialize)]
        struct ImportResponse {
            session_id: String,
        }
        let body = serde_json::json!({ "provider": provider, "session": session });
        let sent = self
            .send("import", Method::POST, "/auth/import-session", |req| {
                req.json(&body)
            })
            .await?;
        if !sent.response.status().is_success() {
            return Err(transfer_failure("import", sent).await);
        }
        Ok(sent
            .response
            .json::<ImportResponse>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte import decode: {e}")))?
            .session_id)
    }

    /// GET `/api/activities` — scrape the activity list for `session_id`.
    ///
    /// Returns the raw upstream [`SciotteActivity`] rows and whether the
    /// service read the list's head; the caller maps the rows with
    /// `convert_activity`, so there is no duplicate DTO logic.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, a non-success status (e.g. the
    /// service no longer holds the session — the caller re-imports and retries,
    /// or the service shed the request under backpressure), or an unparseable
    /// body.
    pub async fn get_activities(
        &self,
        session_id: &str,
        query: &RemoteActivityQuery,
    ) -> AppResult<RemoteActivityList> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(l) = query.limit {
            params.push(("limit", l.to_string()));
        }
        if let Some(a) = query.after_epoch {
            params.push(("after", a.to_string()));
        }
        if let Some(b) = query.before_epoch {
            params.push(("before", b.to_string()));
        }
        if let Some(s) = &query.sport_type {
            params.push(("sport_type", s.clone()));
        }
        if query.enrich_details {
            // The scraper takes the pass by its scope and bounds it by its own
            // ceiling (DRAVR_SCIOTTE_MAX_DETAIL_NAVIGATIONS); any other value is
            // a 400 for the whole list read.
            params.push(("detail", "every".to_owned()));
        }
        if let Some(athlete) = &query.athlete {
            params.push(("athlete", athlete.as_str().to_owned()));
        }
        let sent = self
            .send("activities", Method::GET, "/api/activities", |req| {
                let req = req.header("X-Session-Id", session_id);
                if params.is_empty() {
                    req
                } else {
                    req.query(&params)
                }
            })
            .await?;
        if !sent.response.status().is_success() {
            return Err(scrape_failure("activities", sent).await);
        }
        sent.response
            .json::<RemoteActivityList>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte activities decode: {e}")))
    }

    /// GET `/api/planned-workouts` — read the workouts planned on the
    /// calendar of `session_id` over `[after, before]`, inclusive days.
    ///
    /// `athlete` names whose calendar, by the provider's athlete id; `None`
    /// reads the signed-in account's own. Returns the raw upstream rows,
    /// oldest first; the caller converts them.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, a non-success status (classified
    /// by `scrape_failure`: a shed, a dead session, an athlete refusal, or an
    /// internal fault), an unparseable body, or a body whose `count` disagrees
    /// with the rows it carries.
    pub async fn get_planned_workouts(
        &self,
        session_id: &str,
        after: NaiveDate,
        before: NaiveDate,
        athlete: Option<&AthleteId>,
    ) -> AppResult<Vec<SciottePlannedWorkout>> {
        let mut params = vec![
            ("after", after.format("%Y-%m-%d").to_string()),
            ("before", before.format("%Y-%m-%d").to_string()),
        ];
        if let Some(athlete) = athlete {
            params.push(("athlete", athlete.as_str().to_owned()));
        }
        let sent = self
            .send(
                "planned-workouts",
                Method::GET,
                "/api/planned-workouts",
                |req| req.header("X-Session-Id", session_id).query(&params),
            )
            .await?;
        if !sent.response.status().is_success() {
            return Err(scrape_failure("planned-workouts", sent).await);
        }
        let list = sent
            .response
            .json::<RemotePlannedWorkoutList>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte planned-workouts decode: {e}")))?;
        // A body that disagrees with itself was cut or mangled on the way; a
        // plan read from it could be missing days the coach filled.
        if list.count != list.planned_workouts.len() {
            return Err(AppError::internal(format!(
                "sciotte planned-workouts announced {} workouts and carried {}",
                list.count,
                list.planned_workouts.len()
            )));
        }
        Ok(list.planned_workouts)
    }

    /// GET `/api/athlete` — scrape the profile of the account `session_id`
    /// signed in with: its names, its id, whether it trains or coaches, and a
    /// coach account's roster.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, a non-success status, or an
    /// unparseable body.
    pub async fn get_athlete(&self, session_id: &str) -> AppResult<AthleteProfile> {
        let sent = self
            .send("athlete", Method::GET, "/api/athlete", |req| {
                req.header("X-Session-Id", session_id)
            })
            .await?;
        if !sent.response.status().is_success() {
            return Err(scrape_failure("athlete", sent).await);
        }
        sent.response
            .json::<AthleteProfile>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte athlete decode: {e}")))
    }

    /// GET `/api/activities/{id}` — scrape a single activity's detail for `session_id`.
    ///
    /// Returns the raw upstream [`SciotteActivity`]; the caller reuses the same
    /// `convert_activity` mapping the list path uses.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, a non-success status, or an
    /// unparseable body.
    pub async fn get_activity(
        &self,
        session_id: &str,
        activity_id: &str,
    ) -> AppResult<SciotteActivity> {
        let sent = self
            .send(
                "activity",
                Method::GET,
                &format!("/api/activities/{activity_id}"),
                |req| req.header("X-Session-Id", session_id),
            )
            .await?;
        if !sent.response.status().is_success() {
            return Err(scrape_failure("activity", sent).await);
        }
        sent.response
            .json::<SciotteActivity>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte activity decode: {e}")))
    }

    /// GET `/api/daily-summary` — scrape the day's health summary for
    /// `session_id`: what the session's provider records for `date` (sleep,
    /// resting heart rate, HRV, body metrics, `VO2max`, as far as it keeps them).
    ///
    /// A day the provider holds nothing for is a summary with every metric
    /// absent, not an error.
    ///
    /// # Errors
    ///
    /// Returns an error on transport failure, a non-success status (classified
    /// by `scrape_failure`: a shed, a dead session, or an internal fault), or an
    /// unparseable body.
    pub async fn get_daily_summary(
        &self,
        session_id: &str,
        date: NaiveDate,
    ) -> AppResult<DailySummary> {
        let day = [("date", date.format("%Y-%m-%d").to_string())];
        let sent = self
            .send("daily-summary", Method::GET, "/api/daily-summary", |req| {
                req.header("X-Session-Id", session_id).query(&day)
            })
            .await?;
        if !sent.response.status().is_success() {
            return Err(scrape_failure("daily-summary", sent).await);
        }
        sent.response
            .json::<DailySummary>()
            .await
            .map_err(|e| AppError::internal(format!("sciotte daily-summary decode: {e}")))
    }

    /// Parse a login-step response body into a [`RemoteLoginOutcome`], or into
    /// the retryable backpressure error a load-shed maps to (see
    /// [`backpressure_error`]), or the unavailability a request the service
    /// could not finish maps to (see [`service_failure_error`]).
    async fn parse_login_outcome(sent: Sent) -> AppResult<RemoteLoginOutcome> {
        let http_status = sent.response.status();
        let request_id = sent.request_id;
        let body = match sent.response.json::<Value>().await {
            Ok(body) => body,
            // A gateway's own 502/503/504 need not be JSON; it is still the
            // service shedding or failing to finish, not a login the platform
            // misread.
            Err(e) => {
                return Err(backpressure_error(http_status, &Value::Null)
                    .or_else(|| service_failure_error(http_status, &Value::Null, &request_id))
                    .unwrap_or_else(|| {
                        AppError::internal(format!(
                            "sciotte login decode (HTTP {http_status}, \
                             {REQUEST_ID_HEADER} {request_id}): {e}"
                        ))
                    }));
            }
        };
        // A load-shed body carries no login `status` at all, so it has to be
        // recognised ahead of the match below or it lands in the catch-all and
        // is reported as a system failure. So does a request the service
        // could not finish — a handler panic, a deadline.
        if let Some(error) = backpressure_error(http_status, &body)
            .or_else(|| service_failure_error(http_status, &body, &request_id))
        {
            return Err(error);
        }
        let status = body.get("status").and_then(Value::as_str).unwrap_or("");
        let flow_id = body
            .get("flow_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        match status {
            "authenticated" => {
                let session_id = body
                    .get("session_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        AppError::internal("sciotte authenticated response missing session_id")
                    })?
                    .to_owned();
                let provider = body
                    .get("provider")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        AppError::internal("sciotte authenticated response missing provider")
                    })?
                    .to_owned();
                Ok(RemoteLoginOutcome::Authenticated {
                    session_id,
                    provider,
                })
            }
            "otp_required" => Ok(RemoteLoginOutcome::OtpRequired { flow_id }),
            "two_factor_choice" => Ok(RemoteLoginOutcome::TwoFactorChoice {
                options: body.get("options").cloned().unwrap_or(Value::Null),
                flow_id,
            }),
            "number_match" => Ok(RemoteLoginOutcome::NumberMatch {
                number: body
                    .get("number")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                flow_id,
            }),
            // Only an explicit "failed" is a credential rejection.
            "failed" => Ok(RemoteLoginOutcome::Failed(
                body.get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("login rejected")
                    .to_owned(),
            )),
            // No recognizable login status → a transport/auth/server fault, NOT a
            // login rejection. Surfacing it as an error (rather than Failed) keeps
            // a rejected identity token (401) from masquerading as bad
            // credentials — which is exactly what bit the first e2e run.
            _ => Err(AppError::internal(format!(
                "sciotte returned HTTP {http_status} with no recognizable login status \
                 ({REQUEST_ID_HEADER} {request_id}, body: {body}); check \
                 DRAVR_SCIOTTE_AUDIENCE / IAM / service health"
            ))),
        }
    }
}
