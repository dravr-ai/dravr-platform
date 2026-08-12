// ABOUTME: Sciotte provider routes for credential-based login and session management
// ABOUTME: Collects credentials via Pierre's UI, drives login on the dedicated dravr-sciotte service
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Display;
use std::time::Duration;

use axum::extract::State;
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use dravr_sciotte::config::ScraperConfig;
use dravr_sciotte::models::AuthSession;
use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_core::constants::oauth_providers::TOKEN_TYPE_SESSION;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use pierre_providers::sciotte_provider::SciotteTarget;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::{error, info, warn};
use uuid::Uuid;

use pierre_providers::sciotte_remote::{
    shed_retry_after_secs, RemoteLoginOutcome, RemoteSciotteClient, RETRY_AFTER_SECS_DETAIL,
};

use crate::AuthRoutesContext;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::redaction::redact_url;
use pierre_middleware::provider_link_token::{
    extract_bearer_link_token, verify_link_token, ProviderLinkTokenClaims,
};

/// The parked remote-login state the platform remembers between the initial
/// login call and its OTP/2FA continuations. The heavy state — parked browser +
/// permit — lives on the scraper service; the platform keeps only this so a
/// continuation can name the right server-side flow *and* alert operators with
/// the right platform on a system failure.
#[derive(Clone, Serialize, Deserialize)]
struct RemoteFlowState {
    /// Service-minted id that names the parked browser. It is the only binding
    /// between the caller and the login *they* started, so every continuation
    /// carries it: a continuation sent without one lets the service resume its
    /// sole pending flow, which on a busy service is another athlete's browser
    /// — and its exported session would be persisted under the caller's
    /// account.
    flow_id: String,
    /// Backend provider name (`sciotte` / `sciotte_garmin`) carried from the
    /// login request. A *system* failure on an OTP/2FA continuation needs it for
    /// the operator alert + friendly copy, because the outcome only reports the
    /// provider on success — never on the intermediate steps or on an `Err`.
    provider: String,
}

/// How long a remembered flow stays usable.
///
/// Mirrors the parked flow's own lifetime on the scraper service
/// (`DRAVR_SCIOTTE_PARKED_PERMIT_TTL_SECS`, deployed at 600s). It is the cache
/// entry's TTL, so once the service's reaper has released that browser and its
/// permit the entry is gone on its own — the id names nothing, and the
/// continuation is refused with actionable copy instead of being answered with
/// the service's `no_pending_login` 404.
pub const REMOTE_FLOW_TTL_SECS: u64 = 600;

/// Provider namespace of the cache key a parked flow is filed under.
///
/// The *family*, never the backend variant: which backend the flow belongs to
/// (`sciotte` / `sciotte_garmin`) is what the entry stores, so a continuation
/// — which does not know it yet — could not use it to find the entry.
const REMOTE_FLOW_KEY_PROVIDER: &str = "sciotte";

/// User-facing copy when a continuation arrives with no live flow of the
/// caller's own. `InvalidInput` renders verbatim (see
/// `AppError::sanitized_message`), and both login UIs put the user back on the
/// credential screen from their error phase, so the copy names the one action
/// that works.
const NO_PENDING_LOGIN_MESSAGE: &str =
    "This sign-in is no longer active. Please start the sign-in again.";

/// Cache key naming one athlete's parked sciotte login flow.
///
/// [`CacheKey`] carries the tenant alongside the user, so an entry is only ever
/// reachable by a continuation from the same `(tenant, user)` pair.
fn remote_flow_key(tenant_id: Uuid, user_id: Uuid) -> CacheKey {
    CacheKey::new(
        TenantId::from_uuid(tenant_id),
        user_id,
        REMOTE_FLOW_KEY_PROVIDER.to_owned(),
        CacheResource::SciotteLoginFlow,
    )
}

/// Remember the server-side `flow_id` (with its backend provider) reported by a
/// login step that parked a browser.
async fn remember_flow_from_outcome(
    cache: &Cache,
    tenant_id: Uuid,
    user_id: Uuid,
    flow_id: Option<String>,
    provider: &str,
) {
    if let Some(flow_id) = flow_id {
        remember_remote_flow(cache, tenant_id, user_id, flow_id, provider).await;
        return;
    }
    // The service mints an id on every step that parks a browser, so an absent
    // one is a contract break worth an operator's attention. Whatever this user
    // already had is left untouched: on a continuation that is the id the login
    // parked, still naming this same flow, and its remaining TTL is the one
    // that tracks the service's reaper. On the login step itself there is
    // nothing to keep — the handler cleared the user first — so the
    // continuation that follows is refused, which is the safe direction: the
    // platform has no id to bind it with.
    warn!(
        %user_id,
        provider = %provider,
        "Sciotte login outcome carried no flow_id — leaving this user's remembered flow untouched"
    );
}

/// Remember a user's remote login flow for the parked flow's own lifetime.
///
/// The registry is the shared cache, not process-local state: a 2FA
/// continuation can arrive minutes after the login, Cloud Run runs the API on
/// up to three pods with no session affinity, and a revision deploy or a pod
/// restart lands mid-wait — so an entry written while serving the login has to
/// be readable by whichever pod serves the continuation. `pierre_cache` falls
/// back to an in-memory backend when `REDIS_URL` is unset or the `redis`
/// feature is off; that degrades to single-pod behaviour, which is right for
/// local runs and tests and never less safe, because a flow that cannot be
/// recalled is refused rather than resumed blindly.
pub async fn remember_remote_flow(
    cache: &Cache,
    tenant_id: Uuid,
    user_id: Uuid,
    flow_id: String,
    provider: &str,
) {
    remember_remote_flow_for(
        cache,
        tenant_id,
        user_id,
        flow_id,
        provider,
        Duration::from_secs(REMOTE_FLOW_TTL_SECS),
    )
    .await;
}

/// Remember a user's remote login flow for an explicit lifetime.
///
/// Production passes the parked flow's own [`REMOTE_FLOW_TTL_SECS`]; the TTL is
/// what ages the entry out, so it is the whole of the expiry policy.
pub async fn remember_remote_flow_for(
    cache: &Cache,
    tenant_id: Uuid,
    user_id: Uuid,
    flow_id: String,
    provider: &str,
    ttl: Duration,
) {
    let state = RemoteFlowState {
        flow_id,
        provider: provider.to_owned(),
    };
    if let Err(e) = cache
        .set(&remote_flow_key(tenant_id, user_id), &state, ttl)
        .await
    {
        warn!(
            %user_id,
            provider = %provider,
            error = %e,
            "Failed to remember the sciotte login flow — the continuation will be refused"
        );
    }
}

/// The user's remembered remote flow state, when the cache still holds one.
///
/// Expiry is the entry's TTL, matching the scraper service's parked-flow
/// lifetime: past it the id names a browser the reaper has already released, so
/// the entry is gone and counts as no entry at all — sending its id would
/// resume nothing and surface the service's `no_pending_login` 404 as a
/// platform failure. A cache read that *fails* is treated the same way, since
/// refusing a continuation the platform cannot bind is the safe direction.
async fn recall_remote_flow(
    cache: &Cache,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Option<RemoteFlowState> {
    match cache
        .get::<RemoteFlowState>(&remote_flow_key(tenant_id, user_id))
        .await
    {
        Ok(state) => state,
        Err(e) => {
            warn!(
                %user_id,
                error = %e,
                "Failed to read the remembered sciotte login flow — treating it as absent"
            );
            None
        }
    }
}

/// The `flow_id` + backend provider of the caller's own parked flow, or a
/// client error naming the recovery.
///
/// The id is the whole of the caller-to-flow binding: `submit-otp` /
/// `select-2fa` authenticate the caller, but the scraper service resumes its
/// *sole* pending flow when a continuation carries no id — so continuing
/// without one hands the caller whichever browser is parked, and
/// `remote_login_to_response` then persists that session under the caller's
/// `user_id`/`tenant_id`. A caller with no live entry is refused here instead.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] when the caller has no live flow —
/// they never started a login, or they let the flow lapse.
pub async fn require_remote_flow(
    cache: &Cache,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Result<(String, String), AppError> {
    let Some(flow) = recall_remote_flow(cache, tenant_id, user_id).await else {
        warn!(
            %user_id,
            "Sciotte continuation has no live flow for this caller — refusing instead of \
             letting the service resume whichever browser is parked"
        );
        return Err(AppError::invalid_input(NO_PENDING_LOGIN_MESSAGE));
    };
    Ok((flow.flow_id, flow.provider))
}

/// Drop the user's remembered remote flow state (the attempt is over, whether
/// it authenticated, was rejected, errored, or was superseded by a new login).
pub async fn forget_remote_flow(cache: &Cache, tenant_id: Uuid, user_id: Uuid) {
    if let Err(e) = cache.invalidate(&remote_flow_key(tenant_id, user_id)).await {
        warn!(
            %user_id,
            error = %e,
            "Failed to drop the remembered sciotte login flow"
        );
    }
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
async fn store_sciotte_session(
    resources: &AuthRoutesContext,
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
        token_type: TOKEN_TYPE_SESSION.to_owned(),
        expires_at: session.expires_at,
        scope: None,
        provider_user_id: None,
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    };

    resources.repos.oauth_tokens.upsert_token(&token).await?;

    let tenant = TenantId::from_uuid(tenant_id);
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
/// successful sciotte login, so the coach has warm data on the first chat.
/// Backpressure now lives on the dedicated service (its own concurrency
/// limiter), so the platform just fires the scrape — no in-pod permit
/// (ADR-021 Phase 4 cutover).
fn spawn_activity_prefetch(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &str,
    session_json: &str,
) {
    use pierre_providers::core::{ActivityQueryParams, OAuth2Credentials};

    let registry = resources.provider_registry.clone();
    let cache = resources.cache.clone();
    let provider_name = provider_name.to_owned();
    let session_json = session_json.to_owned();

    tokio::spawn(async move {
        info!(
            user_id = %user_id,
            provider = %provider_name,
            "Starting background activity pre-fetch (remote scrape)"
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
                let tenant = TenantId::from_uuid(tenant_id);
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

/// Map a [`RemoteLoginOutcome`] from the dedicated scraper service to an HTTP
/// response.
///
/// The service holds the interactive browser state across the multi-step 2FA
/// flow (keyed by `flow_id`), so the platform parks nothing. On success the
/// full session is exported from the service and persisted, keeping the
/// platform session-of-record (ADR-021).
async fn remote_login_to_response(
    outcome: RemoteLoginOutcome,
    remote: &RemoteSciotteClient,
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    provider: &str,
    link_context: Option<&LinkContext>,
) -> Result<Response, AppError> {
    match outcome {
        RemoteLoginOutcome::Authenticated {
            session_id,
            provider,
        } => {
            // The service reports the provider the session authenticates against
            // ("garmin"/"strava"); map it to the backend name the platform
            // persists under. The platform never has to remember the provider
            // across the multi-step 2FA flow — only the flow_id string.
            forget_remote_flow(&resources.cache, tenant_id, user_id).await;
            let provider_name = SciotteTarget::from_target_param(&provider).provider_name();
            info!(user_id = %user_id, provider = %provider_name, "Sciotte remote login successful");
            let session = remote.export_session(&session_id).await?;
            if let Some(link) = link_context {
                emit_provider_linked_webhook(user_id, tenant_id, provider_name, link);
            }
            store_sciotte_session(resources, user_id, tenant_id, &session, provider_name).await
        }
        RemoteLoginOutcome::OtpRequired { flow_id } => {
            remember_flow_from_outcome(&resources.cache, tenant_id, user_id, flow_id, provider)
                .await;
            Ok(Json(serde_json::json!({"status": "otp_required"})).into_response())
        }
        RemoteLoginOutcome::TwoFactorChoice { options, flow_id } => {
            remember_flow_from_outcome(&resources.cache, tenant_id, user_id, flow_id, provider)
                .await;
            Ok(
                Json(serde_json::json!({"status": "two_factor_choice", "options": options}))
                    .into_response(),
            )
        }
        RemoteLoginOutcome::NumberMatch { number, flow_id } => {
            remember_flow_from_outcome(&resources.cache, tenant_id, user_id, flow_id, provider)
                .await;
            Ok(Json(serde_json::json!({
                "status": "number_match",
                "number": number,
            }))
            .into_response())
        }
        RemoteLoginOutcome::Failed(reason) => {
            forget_remote_flow(&resources.cache, tenant_id, user_id).await;
            info!(
                user_id = %user_id,
                "Sciotte remote login rejected by provider (user credentials)"
            );
            Ok(Json(serde_json::json!({"status": "failed", "error": reason})).into_response())
        }
    }
}

/// Emit a fire-and-forget webhook announcing the successful provider connection.
/// The channel link context tells the downstream bot where to post the confirmation.
fn emit_provider_linked_webhook(
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &str,
    ctx: &LinkContext,
) {
    use crate::provider_link_webhook::{spawn_emit, ProviderLinkedEvent};
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
pub struct LinkContext {
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
    resources: &AuthRoutesContext,
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
    resources: &AuthRoutesContext,
    headers: &HeaderMap,
) -> Option<ProviderLinkTokenClaims> {
    let header_value = headers.get("authorization").and_then(|h| h.to_str().ok());
    let token = extract_bearer_link_token(header_value)?;
    verify_link_token(token, &resources.admin_jwt_secret, "sciotte").ok()
}

/// Short-circuit the login when the user already has a *usable* stored Sciotte
/// session for this provider — turns a double-tap / refresh-after-connect into a
/// zero-work operation. Since the Phase 4 cutover there is no in-pod Chrome to
/// probe cookies with, so this leans on two cheap local signals: the blob must
/// deserialise, and any `expires_at` it carries must still be in the future. A
/// corrupt or known-expired session falls through to a fresh login instead of
/// handing back a "connected" the next scrape would immediately reject — which
/// would strand the user in a "connected but keeps asking me to reconnect" loop.
/// The dedicated service still re-validates the cookies (and triggers re-auth)
/// when the next scrape imports them.
///
/// Returns `Ok(Some(response))` when the short-circuit fires, `Ok(None)` when
/// the caller must proceed with a fresh login (no token, an unparseable blob,
/// or a known-expired session), and `Err(_)` only for a hard DB failure.
async fn try_reuse_existing_session(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    provider_name: &str,
) -> Result<Option<Response>, AppError> {
    let tenant = TenantId::from_uuid(tenant_id);
    let Some(token) = resources
        .repos
        .oauth_tokens
        .get_token(user_id, tenant, provider_name)
        .await?
    else {
        return Ok(None);
    };

    // Guard on deserializability so a corrupt blob falls through to a fresh
    // login rather than short-circuiting into an unusable "connected" state.
    let Ok(session) = serde_json::from_str::<AuthSession>(&token.access_token) else {
        warn!(
            %user_id,
            provider = %provider_name,
            "Stored Sciotte session blob is not deserialisable — falling through to fresh login"
        );
        return Ok(None);
    };

    // A session whose expiry is already in the past is certainly dead. Without
    // Chrome we can't probe the cookies, but this cheap local check stops us
    // returning "connected" for a session the next scrape would reject (the
    // reconnect-loop trap). A `None` expiry is "unknown, assume usable" — the
    // service re-auths on import if the cookies turn out stale.
    if let Some(expires_at) = session.expires_at {
        if expires_at <= Utc::now() {
            info!(
                %user_id,
                provider = %provider_name,
                "Stored Sciotte session is expired — falling through to fresh login"
            );
            return Ok(None);
        }
    }

    info!(
        %user_id,
        provider = %provider_name,
        "Reusing stored Sciotte session — short-circuiting login"
    );
    Ok(Some(
        Json(serde_json::json!({
            "status": "connected",
            "provider": provider_name,
            "short_circuit": true,
        }))
        .into_response(),
    ))
}

/// Client-facing sciotte login configuration. Lets the login UI size its
/// "this may take up to N" progress copy from the server's real timeout
/// budget instead of a hardcoded constant, so the displayed wait always
/// matches the deployed `DRAVR_SCIOTTE_LOGIN_TIMEOUT` (set per-environment
/// in Cloud Run).
#[derive(Serialize)]
pub struct SciotteConfigResponse {
    /// Overall credential-login budget in seconds — the longest the UI
    /// should tell the user to wait before the server gives up.
    pub login_timeout_secs: u64,
}

/// Return the sciotte login timeout budget for the login UI.
///
/// Reads the same `ScraperConfig::default()` the scraper itself uses, so the
/// value reflects `DRAVR_SCIOTTE_LOGIN_TIMEOUT` from the environment (Cloud
/// Run) with the crate default as the single fallback — no duplicated magic
/// number on the client.
pub async fn handle_sciotte_config() -> Json<SciotteConfigResponse> {
    Json(SciotteConfigResponse {
        login_timeout_secs: ScraperConfig::default().login_timeout_secs,
    })
}

/// Log + alert a sciotte login *system* failure and build the user-facing error.
///
/// A *system* failure is an `Err` from the dedicated scraper service (transport
/// fault, service 5xx, browser-launch failure), as opposed to a
/// `RemoteLoginOutcome::Failed` credential rejection (a *user* failure surfaced
/// in `remote_login_to_response`). `stage` names where in the flow it failed
/// (`credential_login` / `two_factor` / `otp`).
///
/// Splits the failure between two audiences so neither is served the wrong
/// thing:
/// * **Operators** — the full technical reason is logged at `error!`, which
///   `dravr-tronc` forwards to the `#dev-dravr-errors` Slack channel next to
///   the other server-fault alerts (a browser-launch failure blocks *every*
///   credential login on the revision, so it belongs there, not the
///   `#dravr-events` firehose the `sync.failed` business event lands in). The
///   reason names the last page reached (sciotte v0.7.12); `redact_url`
///   scrubs any embedded `user:pass@` credentials before it leaves the
///   process. The catalogued `sync.failed` notify event is still emitted so
///   the `PostHog` analytics signal is preserved.
/// * **The end user** — gets a friendly, generic message carrying no browser
///   stderr, internal paths, or stack detail. `InvalidInput` exposes the
///   message verbatim (see `AppError::sanitized_message`), so the friendly
///   copy is exactly what the login modal / hosted login page render.
pub fn report_login_system_failure<E: Display>(
    user_id: Uuid,
    tenant_id: Uuid,
    provider: &str,
    stage: &str,
    error: &E,
) -> AppError {
    let reason = redact_url(&error.to_string());
    error!(
        user_id = %user_id,
        provider = %provider,
        stage = %stage,
        reason = %reason,
        "Sciotte login failed (system error)"
    );
    info!(
        target: "notify",
        event = "sync.failed",
        user_id = %user_id,
        tenant_id = %tenant_id,
        provider = %provider,
        trigger = %stage,
        reason = %reason,
        "sciotte login failed"
    );
    AppError::invalid_input(friendly_login_failure_message(provider))
}

/// Friendly, provider-aware copy shown to the user on a sciotte login *system* failure.
///
/// Deliberately carries no technical detail — the actionable specifics live in
/// the `error!` log + `#dev-dravr-errors` Slack alert emitted by
/// `report_login_system_failure`.
#[must_use]
pub fn friendly_login_failure_message(provider: &str) -> String {
    let display = if provider.contains("garmin") {
        "Garmin"
    } else {
        "Strava"
    };
    format!(
        "We couldn't sign you in to {display} right now. \
         This is usually temporary — please try again in a few minutes."
    )
}

/// Turn an `Err` from the dedicated scraper service into what the athlete gets
/// back: a retryable shed response, or the operator-alerting system failure.
///
/// A load-shed is the service's designed answer to a saturated Chrome budget
/// (`503` + `Retry-After`), so it is deliberately routed *around*
/// [`report_login_system_failure`]. That path emits an `error!` — which
/// `dravr-tronc` forwards to the `#dev-dravr-errors` Slack channel — plus a
/// `sync.failed` business event, and both would fire once per shed request at
/// exactly the moment the service is busiest, turning healthy backpressure into
/// an operator page storm. Everything else still alerts.
///
/// # Errors
///
/// Returns the system-failure [`AppError`] (already logged + alerted) for any
/// error that is not a shed.
pub fn login_failure_response(
    user_id: Uuid,
    tenant_id: Uuid,
    provider: &str,
    stage: &str,
    error: &AppError,
) -> Result<Response, AppError> {
    let Some(retry_after_secs) = shed_retry_after_secs(error) else {
        return Err(report_login_system_failure(
            user_id, tenant_id, provider, stage, error,
        ));
    };
    warn!(
        %user_id,
        provider = %provider,
        stage = %stage,
        retry_after_secs,
        "Sciotte login shed by scraper backpressure — retryable, not a system failure"
    );
    Ok(shed_response(provider, retry_after_secs))
}

/// The athlete-facing answer to a scraper-service load-shed: the service's own
/// `503` status, the wait it computed both as a `Retry-After` header and in
/// `details`, and the same friendly copy a login failure carries.
///
/// [`ErrorCode::ExternalRateLimited`] is the code that renders as `503` *and*
/// whose message `AppError::sanitized_message` passes through verbatim, so the
/// login modal shows the copy rather than a canned description. The header is
/// also what `response_failure_log_middleware` keys on to record the `503` at
/// `WARN` instead of `ERROR`, so the shed leaves one backpressure line in the
/// logs and nothing in the ops channel.
fn shed_response(provider: &str, retry_after_secs: u64) -> Response {
    let mut error = AppError::new(
        ErrorCode::ExternalRateLimited,
        friendly_login_failure_message(provider),
    );
    let mut details = Map::new();
    details.insert(
        RETRY_AFTER_SECS_DETAIL.to_owned(),
        Value::from(retry_after_secs),
    );
    error.details = Some(Box::new(Value::Object(details)));

    let mut response = error.into_response();
    if let Ok(value) = HeaderValue::from_str(&retry_after_secs.to_string()) {
        response.headers_mut().insert(RETRY_AFTER, value);
    }
    response
}

/// Credential-based login via the dedicated dravr-sciotte scraper service (ADR-021)
pub async fn handle_sciotte_login(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Json(request): Json<SciotteLoginRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, link_context) = authenticate(&resources, &headers).await?;

    if request.email.is_empty() || request.password.is_empty() {
        return Err(AppError::invalid_input("Email and password are required"));
    }

    // This login supersedes whatever the user left parked: the service mints a
    // fresh flow_id for it, and an id inherited from an abandoned 2FA would
    // name a flow the service has already reaped.
    forget_remote_flow(&resources.cache, tenant_id, user_id).await;

    let target = &request.target;
    let provider = SciotteTarget::from_target_param(target).provider_name();

    // Session short-circuit: if the user already has a stored Sciotte session
    // for this provider, return 200 without a fresh login. The dedicated
    // service validates the cookies (and re-auths) on the next scrape, so a
    // cheap DB-presence check is the right short-circuit for a double-tap or a
    // refresh-after-connect.
    if let Some(response) =
        try_reuse_existing_session(&resources, user_id, tenant_id, provider).await?
    {
        return Ok(response);
    }

    // ADR-021: the login runs on the dedicated dravr-sciotte service (there is
    // no in-process fallback since the Phase 4 cutover). The service owns the
    // Chrome budget and the interactive 2FA parking; the platform stays
    // session-of-record (it exports + persists the session, keyed by flow_id).
    // A transport/service fault (not a credential rejection, which comes back
    // as `Failed`, nor a load-shed, which comes back as retryable backpressure)
    // is a *system* failure: alert operators + emit `sync.failed`.
    let remote = RemoteSciotteClient::require_from_env()?;
    info!(user_id = %user_id, target = %target, "Starting sciotte credential login (remote service)");
    let outcome = match remote
        .login_with_credentials(
            &request.email,
            &request.password,
            &request.method,
            SciotteTarget::from_target_param(target).scraper_provider_name(),
        )
        .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            return login_failure_response(user_id, tenant_id, provider, "credential_login", &e)
        }
    };
    remote_login_to_response(
        outcome,
        &remote,
        &resources,
        user_id,
        tenant_id,
        provider,
        link_context.as_ref(),
    )
    .await
}

/// Select a 2FA method for a pending login
pub async fn handle_sciotte_select_2fa(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Json(request): Json<SciotteSelectTwoFactorRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, link_context) = authenticate(&resources, &headers).await?;

    // ADR-021: the parked browser lives on the dedicated service; resume the
    // flow by its server-minted flow_id, which is also the only thing tying
    // that browser to this caller (see `require_remote_flow`). The remembered
    // provider names the platform for a *system* failure alert.
    let (flow_id, provider) = require_remote_flow(&resources.cache, tenant_id, user_id).await?;
    let remote = RemoteSciotteClient::require_from_env()?;
    info!(user_id = %user_id, option = %request.option_id, "Selecting sciotte 2FA method (remote service)");
    let outcome = match remote
        .select_2fa(&request.option_id, Some(flow_id.as_str()))
        .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            return match login_failure_response(user_id, tenant_id, &provider, "two_factor", &e) {
                // A shed never reached the parked browser, so the flow is still
                // there for the retry the `Retry-After` invites — keep it.
                Ok(shed) => Ok(shed),
                // Both login UIs send the user back to the credential screen
                // from their error phase, so a system failure ends the attempt:
                // dropping the entry stops the next login from inheriting an id
                // the service reaps.
                Err(failure) => {
                    forget_remote_flow(&resources.cache, tenant_id, user_id).await;
                    Err(failure)
                }
            };
        }
    };
    remote_login_to_response(
        outcome,
        &remote,
        &resources,
        user_id,
        tenant_id,
        &provider,
        link_context.as_ref(),
    )
    .await
}

/// Submit OTP code for a pending login
pub async fn handle_sciotte_submit_otp(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Json(request): Json<SciotteOtpRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, link_context) = authenticate(&resources, &headers).await?;

    if request.code.is_empty() {
        return Err(AppError::invalid_input("Verification code is required"));
    }

    // ADR-021: the parked browser lives on the dedicated service, which
    // continues the login and reports the provider on success. The platform
    // holds only the flow_id + provider — no parked browser, no permit. The
    // flow_id is what binds that browser to this caller (see
    // `require_remote_flow`); the provider names the platform for a *system*
    // failure alert.
    let (flow_id, provider) = require_remote_flow(&resources.cache, tenant_id, user_id).await?;
    let remote = RemoteSciotteClient::require_from_env()?;
    info!(user_id = %user_id, "Submitting sciotte OTP (remote service)");
    let outcome = match remote
        .submit_otp(&request.code, Some(flow_id.as_str()))
        .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            return match login_failure_response(user_id, tenant_id, &provider, "otp", &e) {
                // A shed never reached the parked browser, so the flow is still
                // there for the retry the `Retry-After` invites — keep it.
                Ok(shed) => Ok(shed),
                // Both login UIs send the user back to the credential screen
                // from their error phase, so a system failure ends the attempt:
                // dropping the entry stops the next login from inheriting an id
                // the service reaps.
                Err(failure) => {
                    forget_remote_flow(&resources.cache, tenant_id, user_id).await;
                    Err(failure)
                }
            };
        }
    };
    remote_login_to_response(
        outcome,
        &remote,
        &resources,
        user_id,
        tenant_id,
        &provider,
        link_context.as_ref(),
    )
    .await
}

/// Connect with a pre-existing serialized session (used by external sciotte CLI)
pub async fn handle_sciotte_connect(
    State(resources): State<AuthRoutesContext>,
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
        token_type: TOKEN_TYPE_SESSION.to_owned(),
        expires_at: None,
        scope: None,
        provider_user_id: None,
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    };

    resources.repos.oauth_tokens.upsert_token(&token).await?;

    let tenant = TenantId::from_uuid(tenant_id);
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
pub async fn handle_sciotte_disconnect(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, _) = authenticate(&resources, &headers).await?;
    let tenant = TenantId::from_uuid(tenant_id);

    resources
        .repos
        .oauth_tokens
        .delete_token(user_id, tenant, "sciotte")
        .await?;

    // Remove the connection row in lockstep with the token. The two tables are
    // separate sources of truth (provider_connections drives the "connected"
    // badge + coaching fetch enumeration; oauth_tokens drives resolve_backend +
    // the scrape session); leaving an orphaned connection row makes the UI show
    // "Connected" for a session that no longer exists and routes later fetches
    // to a dead backend.
    resources
        .repos
        .provider_connections
        .remove_connection(user_id, tenant, "sciotte")
        .await?;

    info!(user_id = %user_id, "Sciotte session disconnected");

    Ok(StatusCode::NO_CONTENT.into_response())
}
