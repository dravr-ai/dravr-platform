// ABOUTME: Sciotte provider routes for credential-based login and session management
// ABOUTME: Collects credentials via Pierre's UI, runs in-process Chrome login via dravr-sciotte
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
#[cfg(feature = "provider-sciotte")]
use dravr_sciotte::cache::CachedScraper;
#[cfg(feature = "provider-sciotte")]
use dravr_sciotte::error::LoginResult;
#[cfg(feature = "provider-sciotte")]
use dravr_sciotte::models::AuthSession;
#[cfg(feature = "provider-sciotte")]
use dravr_sciotte::scraper::ChromeScraper;
#[cfg(feature = "provider-sciotte")]
use dravr_sciotte::ActivityScraper;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
#[cfg(feature = "provider-sciotte")]
use pierre_providers::sciotte_limiter::{LimiterError, SciotteLimiter, ScrapePermit};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::errors::AppError;
use crate::mcp::resources::ServerContext;
use crate::middleware::provider_link_token::{
    extract_bearer_link_token, verify_link_token, ProviderLinkTokenClaims,
};

// Pending OTP scraper — holds the Chrome browser and its associated scraper
// permit between multi-step login calls. Keyed by `user_id` to prevent
// cross-user interference.
//
// The `ScrapePermit` is parked here for the lifetime of the flow so the
// Cloud Run pod's Chrome budget tracks the real wall-clock duration of the
// OTP / 2FA exchange, not just the first HTTP request. The watchdog drops
// entries whose owning flow has gone silent (see `spawn_pending_watchdog`).
#[cfg(feature = "provider-sciotte")]
struct PendingLogin {
    scraper: CachedScraper<ChromeScraper>,
    provider_name: String,
    link_context: Option<LinkContext>,
    permit: ScrapePermit,
    parked_at: Instant,
}

#[cfg(feature = "provider-sciotte")]
static PENDING_OTP_SCRAPERS: LazyLock<Mutex<HashMap<Uuid, PendingLogin>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Process-global backpressure limiter. Initialised exactly once during
/// server startup via [`init_sciotte_limiter`]; runtime access via
/// [`sciotte_limiter`] panics if that init step was skipped, which is a
/// programmer error in the bootstrap path — operator errors (missing /
/// malformed env vars) surface from `init_sciotte_limiter` as an
/// `AppError` propagated out of `bootstrap_server`.
#[cfg(feature = "provider-sciotte")]
static SCIOTTE_LIMITER: OnceLock<Arc<SciotteLimiter>> = OnceLock::new();

/// Initialise the Sciotte backpressure limiter.
///
/// Reads `PIERRE_SCIOTTE_*` environment variables, constructs a
/// [`SciotteLimiter`], and spawns its watchdog. Must be called exactly
/// once during server startup, before any route that uses the limiter
/// is served.
///
/// # Errors
///
/// Returns [`AppError::internal`] wrapping a `LimiterConfigError` when
/// any required `PIERRE_SCIOTTE_*` variable is missing, malformed, or
/// inconsistent (e.g. `max_queue_depth < max_concurrent`).
#[cfg(feature = "provider-sciotte")]
pub fn init_sciotte_limiter() -> Result<(), AppError> {
    use pierre_providers::sciotte_limiter::LimiterConfig;

    let config = LimiterConfig::from_env()
        .map_err(|e| AppError::internal(format!("Sciotte limiter config invalid: {e}")))?;
    info!(
        max_concurrent = config.max_concurrent,
        max_queue_depth = config.max_queue_depth,
        acquire_timeout_secs = config.acquire_timeout.as_secs(),
        parked_permit_ttl_secs = config.parked_permit_ttl.as_secs(),
        watchdog_interval_secs = config.watchdog_interval.as_secs(),
        retry_after_hint_secs = config.retry_after_hint.as_secs(),
        closed_retry_after_secs = config.closed_retry_after.as_secs(),
        "Sciotte backpressure limiter initialised from environment"
    );
    let limiter = SciotteLimiter::new(config);
    // Dropping a tokio `JoinHandle` detaches the task rather than aborting
    // it, which is the behaviour we want: the watchdog runs for the
    // lifetime of the process.
    drop(spawn_pending_watchdog(&limiter));
    // OnceLock::set returns Err if called twice; swallow because tests may
    // call this init from multiple fixtures in the same process.
    let _ = SCIOTTE_LIMITER.set(limiter);
    Ok(())
}

/// Retrieve the process-global limiter, or return an [`AppError::internal`]
/// when `init_sciotte_limiter` was not called during bootstrap. That
/// branch is a programmer error in the startup path, not an operator
/// error — surfacing it as `AppError` lets the route return 500 cleanly
/// instead of panicking the worker thread.
///
/// # Errors
///
/// Returns [`AppError::internal`] when the global limiter has not yet
/// been initialised (i.e. server startup skipped `init_sciotte_limiter`).
#[cfg(feature = "provider-sciotte")]
fn sciotte_limiter() -> Result<&'static Arc<SciotteLimiter>, AppError> {
    SCIOTTE_LIMITER.get().ok_or_else(|| {
        AppError::internal(
            "Sciotte backpressure limiter not initialised — \
             init_sciotte_limiter must be called during bootstrap_server",
        )
    })
}

#[derive(Debug, Deserialize)]
pub struct SciotteLoginRequest {
    pub email: String,
    pub password: String,
    #[serde(default = "default_method")]
    pub method: String,
    /// Target platform: "strava" (default) or "garmin"
    #[serde(default = "default_target")]
    pub target: String,
}

fn default_method() -> String {
    "email".to_owned()
}

fn default_target() -> String {
    "strava".to_owned()
}

/// Create a sciotte scraper configured for the target platform
#[cfg(feature = "provider-sciotte")]
fn create_scraper_for_target(target: &str) -> CachedScraper<ChromeScraper> {
    use dravr_sciotte::config::{CacheConfig, ScraperConfig};
    use dravr_sciotte::provider::ProviderConfig as SciotteProviderConfig;

    let provider_config = match target {
        "garmin" => SciotteProviderConfig::garmin_default(),
        _ => SciotteProviderConfig::strava_default(),
    };
    let scraper = ChromeScraper::new(ScraperConfig::default(), provider_config);
    CachedScraper::new(scraper, &CacheConfig::default())
}

/// Build an HTTP 503 response with a `Retry-After` header from a
/// [`LimiterError`]. Used by every Sciotte login handler when the
/// backpressure queue is saturated.
#[cfg(feature = "provider-sciotte")]
fn busy_response(error: &LimiterError) -> Response {
    let retry_after_secs = error.retry_after_secs();
    let reason = error.to_string();
    warn!(
        reason = %reason,
        retry_after_secs,
        "Sciotte backpressure rejecting request"
    );
    let body = Json(serde_json::json!({
        "error": "sciotte_busy",
        "reason": reason,
        "retry_after_secs": retry_after_secs,
    }));
    let mut response = (StatusCode::SERVICE_UNAVAILABLE, body).into_response();
    if let Ok(value) = HeaderValue::from_str(&retry_after_secs.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

/// Spawn the background watchdog that evicts pending OTP flows whose parked
/// permit has exceeded the configured TTL. This guarantees an abandoned 2FA
/// flow can not permanently hold a Chrome slot. When the `PendingLogin`
/// entry is removed from the map it goes out of scope, and the
/// `ScrapePermit` it owned is dropped — returning the Chrome budget slot to
/// the limiter.
#[cfg(feature = "provider-sciotte")]
fn spawn_pending_watchdog(limiter: &Arc<SciotteLimiter>) -> JoinHandle<()> {
    limiter.spawn_watchdog(move |ttl: Duration| async move {
        let mut guard = PENDING_OTP_SCRAPERS.lock().await;
        let stale: Vec<Uuid> = guard
            .iter()
            .filter_map(|(user_id, pending)| {
                (pending.parked_at.elapsed() > ttl).then_some(*user_id)
            })
            .collect();
        let count = stale.len();
        for user_id in stale {
            if let Some(pending) = guard.remove(&user_id) {
                warn!(
                    %user_id,
                    provider = %pending.provider_name,
                    age_secs = pending.parked_at.elapsed().as_secs(),
                    "Evicting stale pending Sciotte login"
                );
            }
        }
        count
    })
}

/// Get the Pierre provider name for the target
fn provider_name_for_target(target: &str) -> &'static str {
    match target {
        "garmin" => "sciotte_garmin",
        _ => "sciotte",
    }
}

#[derive(Debug, Deserialize)]
pub struct SciotteSelectTwoFactorRequest {
    pub option_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SciotteOtpRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct SciotteConnectRequest {
    pub session_id: String,
}

/// Store a successful sciotte session in Pierre's encrypted DB and register the connection.
/// After storing, spawns a background task to pre-fetch activities into cache so the
/// coach can serve data immediately when the user starts chatting.
#[cfg(feature = "provider-sciotte")]
async fn store_sciotte_session(
    resources: &ServerContext,
    user_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    session: &AuthSession,
    provider_name: &str,
) -> Result<Response, AppError> {
    let session_json = serde_json::to_string(session)
        .map_err(|e| AppError::internal(format!("Failed to serialize session: {e}")))?;

    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: provider_name.to_owned(),
        access_token: session_json.clone(),
        refresh_token: None,
        token_type: "session".to_owned(),
        expires_at: session.expires_at,
        scope: None,
        created_at: now,
        updated_at: now,
    };

    resources.repos.oauth_tokens.upsert_token(&token).await?;

    let tenant = TenantId::from(tenant_id);
    resources
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            provider_name,
            &ConnectionType::Manual,
            None,
        )
        .await
        .map_err(|e| AppError::internal(format!("Failed to register connection: {e}")))?;

    // Pre-fetch activities in background so the cache is warm when the user chats
    spawn_activity_prefetch(resources, user_id, tenant_id, provider_name, &session_json);

    Ok(Json(serde_json::json!({"status": "connected", "provider": provider_name})).into_response())
}

/// Spawn a background task to pre-fetch and cache activities after a
/// successful sciotte login. Uses non-blocking `try_acquire` on the
/// backpressure limiter: if the Chrome budget is saturated, the prefetch is
/// skipped entirely and the user's next interactive request will cold-fetch.
#[cfg(feature = "provider-sciotte")]
fn spawn_activity_prefetch(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &str,
    session_json: &str,
) {
    use pierre_cache::{CacheKey, CacheResource};
    use pierre_providers::core::{ActivityQueryParams, OAuth2Credentials};

    let limiter = match sciotte_limiter() {
        Ok(l) => l,
        Err(err) => {
            warn!(
                user_id = %user_id,
                provider = %provider_name,
                error = %err,
                "Cannot prefetch Sciotte activities — limiter not initialised"
            );
            return;
        }
    };
    let permit = match limiter.try_acquire() {
        Ok(p) => p,
        Err(err) => {
            info!(
                user_id = %user_id,
                provider = %provider_name,
                reason = %err,
                "Skipping Sciotte prefetch — backpressure budget saturated"
            );
            return;
        }
    };

    let registry = resources.provider_registry.clone();
    let cache = resources.cache.clone();
    let provider_name = provider_name.to_owned();
    let session_json = session_json.to_owned();

    tokio::spawn(async move {
        // Keep the permit alive for the full lifetime of the prefetch task.
        // When the task finishes (success or failure), drop releases the slot.
        let prefetch_permit = permit;
        info!(
            user_id = %user_id,
            provider = %provider_name,
            permit_acquired_at = ?prefetch_permit.acquired_at(),
            "Starting background activity pre-fetch"
        );

        let provider = match registry.create_provider(&provider_name) {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "Background pre-fetch: failed to create provider");
                return;
            }
        };

        let credentials = OAuth2Credentials {
            client_id: String::new(),
            client_secret: String::new(),
            access_token: Some(session_json),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
        };
        if let Err(e) = provider.set_credentials(credentials).await {
            warn!(error = %e, "Background pre-fetch: failed to set credentials");
            return;
        }

        let params = ActivityQueryParams {
            limit: Some(30),
            offset: None,
            before: None,
            after: None,
        };

        match provider.get_activities_with_params(&params).await {
            Ok(activities) => {
                let count = activities.len();
                let tenant = TenantId::from(tenant_id);
                let cache_key = CacheKey::new(
                    tenant,
                    user_id,
                    provider_name.clone(),
                    CacheResource::ActivityList {
                        page: 1,
                        per_page: 30,
                        before: None,
                        after: None,
                        sport_type: None,
                    },
                );
                let ttl = Duration::from_mins(15);
                if let Err(e) = cache.set(&cache_key, &activities, ttl).await {
                    warn!(error = %e, "Background pre-fetch: failed to cache activities");
                } else {
                    info!(user_id = %user_id, provider = %provider_name, count, "Background activity pre-fetch complete — cache warm");
                }
            }
            Err(e) => {
                warn!(user_id = %user_id, error = %e, "Background pre-fetch: failed to fetch activities");
            }
        }
    });
}

/// Session context bundle for a Sciotte login step — groups user/tenant/provider
/// identifiers and the optional channel link context. Used to keep the
/// `login_result_to_response` signature readable.
#[cfg(feature = "provider-sciotte")]
struct LoginSessionContext<'a> {
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &'a str,
    log_prefix: &'a str,
    link_context: Option<LinkContext>,
}

/// Convert a `LoginResult` into an HTTP response, storing the scraper +
/// permit for follow-up calls when the flow is still in progress. The
/// `permit` is moved into `PENDING_OTP_SCRAPERS` on continuation branches
/// and dropped on terminal outcomes (success or failure).
#[cfg(feature = "provider-sciotte")]
async fn login_result_to_response(
    result: LoginResult,
    resources: &ServerContext,
    scraper: CachedScraper<ChromeScraper>,
    permit: ScrapePermit,
    ctx: LoginSessionContext<'_>,
) -> Result<Response, AppError> {
    let LoginSessionContext {
        user_id,
        tenant_id,
        provider_name,
        log_prefix,
        link_context,
    } = ctx;

    match result {
        LoginResult::Success(session) => {
            info!(user_id = %user_id, "{log_prefix} successful");
            if let Some(link) = &link_context {
                emit_provider_linked_webhook(user_id, tenant_id, provider_name, link);
            }
            let response =
                store_sciotte_session(resources, user_id, tenant_id, &session, provider_name).await;
            drop(permit);
            response
        }
        LoginResult::OtpRequired => {
            park_pending_login(user_id, scraper, provider_name, link_context, permit).await;
            Ok(Json(serde_json::json!({"status": "otp_required"})).into_response())
        }
        LoginResult::TwoFactorChoice(options) => {
            park_pending_login(user_id, scraper, provider_name, link_context, permit).await;
            let options_json: Vec<serde_json::Value> = options
                .iter()
                .map(|o| serde_json::json!({"id": o.id, "label": o.label}))
                .collect();
            Ok(
                Json(serde_json::json!({"status": "two_factor_choice", "options": options_json}))
                    .into_response(),
            )
        }
        LoginResult::NumberMatch(number) => {
            park_pending_login(user_id, scraper, provider_name, link_context, permit).await;
            Ok(Json(serde_json::json!({
                "status": "number_match",
                "number": number,
            }))
            .into_response())
        }
        LoginResult::Failed(reason) => {
            drop(permit);
            Ok(Json(serde_json::json!({"status": "failed", "error": reason})).into_response())
        }
    }
}

/// Move a running login flow (scraper + permit) into the pending map so the
/// next step in the multi-request exchange can reclaim it.
#[cfg(feature = "provider-sciotte")]
async fn park_pending_login(
    user_id: Uuid,
    scraper: CachedScraper<ChromeScraper>,
    provider_name: &str,
    link_context: Option<LinkContext>,
    permit: ScrapePermit,
) {
    let replaced = PENDING_OTP_SCRAPERS.lock().await.insert(
        user_id,
        PendingLogin {
            scraper,
            provider_name: provider_name.to_owned(),
            link_context,
            permit,
            parked_at: Instant::now(),
        },
    );
    if replaced.is_some() {
        warn!(
            %user_id,
            "Overwriting in-progress Sciotte login flow — previous permit released"
        );
    }
}

/// Emit a fire-and-forget webhook announcing the successful provider connection.
/// The channel link context tells the downstream bot where to post the confirmation.
#[cfg(feature = "provider-sciotte")]
fn emit_provider_linked_webhook(
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &str,
    ctx: &LinkContext,
) {
    use crate::routes::auth::provider_link_webhook::{spawn_emit, ProviderLinkedEvent};
    let event = ProviderLinkedEvent::new(
        user_id,
        tenant_id,
        provider_name.to_owned(),
        ctx.target.clone(),
        ctx.channel.clone(),
        ctx.channel_thread.clone(),
    );
    spawn_emit(event);
}

/// Channel context carried by a provider link-token — used to emit the
/// post-back webhook after a successful connection.
#[derive(Debug, Clone)]
pub(super) struct LinkContext {
    /// Channel slug the token was minted for (e.g. "slack")
    pub channel: String,
    /// Optional channel thread/DM id so the bot can reply in the original thread
    pub channel_thread: Option<String>,
    /// Target platform ("strava" / "garmin") from the signed claims
    pub target: String,
}

/// Extract authenticated `user_id` and `tenant_id` from request headers.
///
/// Accepts two authentication methods, tried in order:
/// 1. A channel-initiated provider link-token (`Authorization: Bearer <link-token>`),
///    minted by `POST /api/channels/provider/sciotte/link-token`. This is the path
///    used by the hosted Sciotte login page embedded in chat channels.
/// 2. A standard Pierre session (cookie or Bearer JWT) — used by the web/mobile
///    `SciotteLoginModal` component.
///
/// When the link-token path is taken, the returned `LinkContext` carries the
/// channel metadata needed to emit a post-back webhook on success.
async fn authenticate(
    resources: &ServerContext,
    headers: &HeaderMap,
) -> Result<(Uuid, Uuid, Option<LinkContext>), AppError> {
    if let Some(claims) = try_verify_link_token_from_headers(resources, headers) {
        let user_id = Uuid::parse_str(&claims.sub).map_err(|_| {
            AppError::auth_invalid("Provider link-token has invalid user_id in sub claim")
        })?;
        let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| {
            AppError::auth_invalid("Provider link-token has invalid tenant_id in tid claim")
        })?;
        let link_context = LinkContext {
            channel: claims.channel,
            channel_thread: claims.channel_thread,
            target: claims.tgt,
        };
        return Ok((user_id, tenant_id, Some(link_context)));
    }

    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(headers)
        .await?;
    let tenant_id = auth_result
        .active_tenant_id
        .ok_or_else(|| AppError::invalid_input("No active tenant"))?;
    Ok((auth_result.user_id, tenant_id, None))
}

/// If an `Authorization: Bearer ...` header contains a Sciotte-scoped link-token,
/// verify it against the server's admin JWT secret and return the claims.
/// Returns None when the header is absent, not a Bearer token, or fails verification
/// as a link-token (so the caller falls through to normal auth).
fn try_verify_link_token_from_headers(
    resources: &ServerContext,
    headers: &HeaderMap,
) -> Option<ProviderLinkTokenClaims> {
    let header_value = headers.get("authorization").and_then(|h| h.to_str().ok());
    let token = extract_bearer_link_token(header_value)?;
    verify_link_token(token, &resources.admin_jwt_secret, "sciotte").ok()
}

/// Take the pending login (scraper + provider name + link context + permit)
/// for a user, or return an error if no flow is in progress.
#[cfg(feature = "provider-sciotte")]
async fn take_pending_login(user_id: Uuid) -> Result<PendingLogin, AppError> {
    PENDING_OTP_SCRAPERS
        .lock()
        .await
        .remove(&user_id)
        .ok_or_else(|| AppError::invalid_input("No pending login session — please log in again"))
}

/// Build an HTTP 409 response signalling that the same user already has a
/// Sciotte login flow in flight. The frontend should either wait for that
/// flow to complete or cancel it with a disconnect before retrying. Used to
/// reject double-click and refresh-storm amplification before a Chrome
/// permit is ever acquired.
#[cfg(feature = "provider-sciotte")]
fn login_already_in_progress_response(user_id: Uuid) -> Response {
    warn!(
        %user_id,
        "Rejecting Sciotte login — flow already in progress for this user"
    );
    let body = Json(serde_json::json!({
        "error": "login_already_in_progress",
        "reason": "A Sciotte login is already in progress for this user. \
                   Complete or cancel the existing flow before starting a new one."
    }));
    (StatusCode::CONFLICT, body).into_response()
}

/// Check whether the user already has a valid Sciotte session persisted in
/// `user_oauth_tokens` for this provider. When found, probe the stored
/// cookies via `is_authenticated` (lightweight, no Chrome) and, if still
/// valid, short-circuit the login with HTTP 200. This turns every double-tap
/// or refresh-after-success into a zero-Chrome operation.
///
/// Returns `Ok(Some(response))` when the short-circuit fires,
/// `Ok(None)` when the caller must proceed with a real Chrome login, and
/// `Err(_)` only for hard DB / deserialisation failures.
#[cfg(feature = "provider-sciotte")]
async fn try_reuse_existing_session(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: Uuid,
    target: &str,
    provider_name: &str,
) -> Result<Option<Response>, AppError> {
    let tenant = TenantId::from(tenant_id);
    let existing = resources
        .repos
        .oauth_tokens
        .get_token(user_id, tenant, provider_name)
        .await?;
    let Some(token) = existing else {
        return Ok(None);
    };

    let session: AuthSession = match serde_json::from_str(&token.access_token) {
        Ok(s) => s,
        Err(e) => {
            warn!(
                %user_id,
                provider = %provider_name,
                error = %e,
                "Stored Sciotte session blob is not deserialisable — falling through to fresh login"
            );
            return Ok(None);
        }
    };

    // `is_authenticated` is a cheap cookie-state probe on the scraper; it
    // never launches Chrome and never holds a backpressure permit.
    let probe = create_scraper_for_target(target);
    if probe.is_authenticated(&session).await {
        info!(
            %user_id,
            provider = %provider_name,
            "Reusing valid Sciotte session — short-circuiting login without Chrome"
        );
        let body = Json(serde_json::json!({
            "status": "connected",
            "provider": provider_name,
            "short_circuit": true,
        }));
        return Ok(Some(body.into_response()));
    }

    info!(
        %user_id,
        provider = %provider_name,
        "Stored Sciotte session is stale — proceeding with fresh login"
    );
    Ok(None)
}

/// Credential-based login via in-process dravr-sciotte
#[cfg(feature = "provider-sciotte")]
pub(super) async fn handle_sciotte_login(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(request): Json<SciotteLoginRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, link_context) = authenticate(&resources, &headers).await?;

    if request.email.is_empty() || request.password.is_empty() {
        return Err(AppError::invalid_input("Email and password are required"));
    }

    let target = &request.target;
    let provider = provider_name_for_target(target);

    // Per-user dedup: if this user already has a Sciotte login flow parked
    // (OTP / 2FA continuation), a concurrent login would clobber the parked
    // permit and orphan the first flow's Chrome. Reject with 409 so the
    // client can retry after the first flow finishes. This also kills
    // double-click / refresh-storm amplification before a Chrome permit is
    // ever acquired.
    if PENDING_OTP_SCRAPERS.lock().await.contains_key(&user_id) {
        return Ok(login_already_in_progress_response(user_id));
    }

    // Session short-circuit: if the user already has a valid Sciotte session
    // for this provider, return 200 without launching Chrome. Exercises a
    // cheap cookie probe via `is_authenticated` and bypasses the permit
    // budget entirely. A stale / missing / unparseable session falls
    // through to the full login path.
    if let Some(response) =
        try_reuse_existing_session(&resources, user_id, tenant_id, target, provider).await?
    {
        return Ok(response);
    }

    // Acquire a permit before launching Chrome. Under overload the limiter
    // fast-rejects with 503 + Retry-After so the pod doesn't accumulate an
    // unbounded backlog of pending Chrome processes.
    let permit = match sciotte_limiter()?.acquire().await {
        Ok(p) => p,
        Err(err) => return Ok(busy_response(&err)),
    };

    info!(user_id = %user_id, target = %target, "Starting sciotte credential login");

    let cached = create_scraper_for_target(target);

    let result = match cached
        .credential_login(&request.email, &request.password, &request.method)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Sciotte credential login failed");
            drop(permit);
            return Err(AppError::invalid_input(format!("Login failed: {e}")));
        }
    };

    login_result_to_response(
        result,
        &resources,
        cached,
        permit,
        LoginSessionContext {
            user_id,
            tenant_id,
            provider_name: provider,
            log_prefix: "Sciotte credential login",
            link_context,
        },
    )
    .await
}

/// Select a 2FA method for a pending login
#[cfg(feature = "provider-sciotte")]
pub(super) async fn handle_sciotte_select_2fa(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(request): Json<SciotteSelectTwoFactorRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, _) = authenticate(&resources, &headers).await?;
    let pending = take_pending_login(user_id).await?;
    let PendingLogin {
        scraper,
        provider_name,
        link_context,
        permit,
        ..
    } = pending;

    info!(user_id = %user_id, option = %request.option_id, "Selecting 2FA method");

    let result = match scraper.select_two_factor(&request.option_id).await {
        Ok(r) => r,
        Err(e) => {
            drop(permit);
            return Err(AppError::invalid_input(format!(
                "2FA verification failed: {e}"
            )));
        }
    };

    login_result_to_response(
        result,
        &resources,
        scraper,
        permit,
        LoginSessionContext {
            user_id,
            tenant_id,
            provider_name: &provider_name,
            log_prefix: "Sciotte 2FA login",
            link_context,
        },
    )
    .await
}

/// Submit OTP code for a pending login
#[cfg(feature = "provider-sciotte")]
pub(super) async fn handle_sciotte_submit_otp(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(request): Json<SciotteOtpRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, _) = authenticate(&resources, &headers).await?;

    if request.code.is_empty() {
        return Err(AppError::invalid_input("Verification code is required"));
    }

    let pending = take_pending_login(user_id).await?;
    let PendingLogin {
        scraper,
        provider_name,
        link_context,
        permit,
        ..
    } = pending;

    info!(user_id = %user_id, "Submitting OTP code");

    let result = match scraper.submit_otp(&request.code).await {
        Ok(r) => r,
        Err(e) => {
            drop(permit);
            return Err(AppError::invalid_input(format!(
                "Code verification failed: {e}"
            )));
        }
    };

    login_result_to_response(
        result,
        &resources,
        scraper,
        permit,
        LoginSessionContext {
            user_id,
            tenant_id,
            provider_name: &provider_name,
            log_prefix: "Sciotte OTP login",
            link_context,
        },
    )
    .await
}

/// Connect with a pre-existing serialized session (used by external sciotte CLI)
pub(super) async fn handle_sciotte_connect(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(request): Json<SciotteConnectRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, _) = authenticate(&resources, &headers).await?;

    if request.session_id.is_empty() {
        return Err(AppError::invalid_input("session_id is required"));
    }

    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: "sciotte".to_owned(),
        access_token: request.session_id,
        refresh_token: None,
        token_type: "session".to_owned(),
        expires_at: None,
        scope: None,
        created_at: now,
        updated_at: now,
    };

    resources.repos.oauth_tokens.upsert_token(&token).await?;

    let tenant = TenantId::from(tenant_id);
    resources
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::Manual, None)
        .await
        .map_err(|e| AppError::internal(format!("Failed to register connection: {e}")))?;

    info!(user_id = %user_id, "Sciotte session connected");

    Ok(Json(serde_json::json!({"status": "connected", "provider": "sciotte"})).into_response())
}

/// Disconnect the sciotte session
pub(super) async fn handle_sciotte_disconnect(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, _) = authenticate(&resources, &headers).await?;
    let tenant = TenantId::from(tenant_id);

    resources
        .repos
        .oauth_tokens
        .delete_token(user_id, tenant, "sciotte")
        .await?;

    info!(user_id = %user_id, "Sciotte session disconnected");

    Ok(StatusCode::NO_CONTENT.into_response())
}
