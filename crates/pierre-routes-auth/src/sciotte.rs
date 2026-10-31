// ABOUTME: Sciotte provider routes for credential-based login and session management
// ABOUTME: Collects credentials via Pierre's UI, drives login on the dedicated dravr-sciotte service
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use secrecy::{ExposeSecret, SecretString};
use std::fmt::{self, Display};
use std::time::Duration;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use dravr_sciotte::models::AuthSession;
use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_core::constants::oauth_providers::SCIOTTE_TRAININGPEAKS;
use pierre_core::models::TenantId;
use pierre_providers::backend_resolver;
use pierre_providers::registry::global_registry;
use pierre_providers::sciotte_provider::SciotteTarget;
use pierre_services::provider_notice::require_notice_accepted;
use pierre_services::scrape_session::{
    persist_sciotte_session, ScrapeSessionInstaller, StoredSession,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use uuid::Uuid;

use dravr_sciotte::client::SciotteClient;
use dravr_sciotte::wire::{LoginMethod, LoginOutcome, LOGIN_CHALLENGE};
use pierre_providers::sciotte_error::{is_login_challenge, shed_retry_after_secs, to_app_error};

#[cfg(feature = "health-sync")]
use crate::oauth::spawn_health_backfill;
use crate::sciotte_prefetch::spawn_activity_prefetch;
use crate::sciotte_session_reuse::{stored_session_use, StoredSessionUse};
use crate::trainingpeaks_account::spawn_login_probe;
use crate::AuthRoutesContext;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::redaction::redact_url;
use pierre_middleware::provider_link_token::{
    extract_bearer_link_token, verify_link_token, ProviderLinkTokenClaims, CONNECT_PROVIDER,
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
    /// carries it: the service refuses a continuation without one
    /// (`flow_id_required`), and resumes whichever browser the id names — so an
    /// id that is not the caller's own would persist another athlete's session
    /// under the caller's account.
    flow_id: String,
    /// Backend provider name (`sciotte`, `sciotte_garmin`,
    /// `sciotte_trainingpeaks`, `sciotte_coros`) carried from the login
    /// request. A *system* failure on an OTP/2FA continuation needs it for the
    /// operator alert + friendly copy, because the outcome only reports the
    /// provider on success — never on the intermediate steps or on an `Err`.
    provider: String,
}

/// How long a remembered flow stays usable.
///
/// Mirrors the pending-login window on the scraper service — one window
/// since dravr-sciotte 0.24.0, `DRAVR_SCIOTTE_PENDING_LOGIN_TTL`, 30 minutes
/// by default (carnet#979). It is the cache entry's TTL, so once the
/// service's reaper has released that browser the entry is gone on its own —
/// the id names nothing, and the continuation is refused with actionable copy
/// instead of being answered with the service's `no_pending_login` 404.
pub const REMOTE_FLOW_TTL_SECS: u64 = 1800;

/// Provider namespace of the cache key a parked flow is filed under.
///
/// The *family*, never the backend variant: which backend the flow belongs to
/// (`sciotte`, `sciotte_garmin`, `sciotte_trainingpeaks`, `sciotte_coros`) is
/// what the entry stores, so a continuation — which does not know it yet —
/// could not use it to find the entry.
const REMOTE_FLOW_KEY_PROVIDER: &str = "sciotte";

/// User-facing copy when a continuation arrives with no live flow of the
/// caller's own. `InvalidInput` renders verbatim (see
/// `AppError::sanitized_message`), and both login UIs put the user back on the
/// credential screen from their error phase, so the copy names the one action
/// that works.
const NO_PENDING_LOGIN_MESSAGE: &str =
    "This sign-in is no longer active. Please start the sign-in again.";

/// `details.reason` of that refusal: what tells a login UI the sign-in lapsed
/// rather than failed, without reading the message.
///
/// The hosted login pages word every failure in the athlete's own language,
/// so they act on this and never print [`NO_PENDING_LOGIN_MESSAGE`].
pub const LOGIN_FLOW_EXPIRED_REASON: &str = "login_flow_expired";

/// `details.reason` of the refusal a sign-in the provider stopped at a
/// challenge (a captcha) gets.
///
/// It is the service's own marker, so the login UIs select on the one word
/// the wire already carries. The hosted login pages word that refusal in the
/// athlete's own language from it, and never print the message.
pub const LOGIN_CHALLENGE_REASON: &str = LOGIN_CHALLENGE;

/// `reason` of an `otp_required` answer to a refused or incomplete code.
///
/// The same sign-in takes another code; the login UIs word the message
/// themselves, never in the provider's own prose.
pub const CODE_REJECTED_REASON: &str = "code_rejected";

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
/// `select-2fa` authenticate the caller, but the scraper service resumes
/// whichever parked browser the id names, and `remote_login_to_response` then
/// persists that session under the caller's `user_id`/`tenant_id`. The id is
/// therefore only ever the caller's own remembered one; a caller with no live
/// entry is refused here, before any call (the service itself refuses a
/// continuation that names no flow).
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
            "Sciotte continuation has no live flow for this caller — refusing before any \
             call to the service"
        );
        let mut refusal = AppError::invalid_input(NO_PENDING_LOGIN_MESSAGE);
        refusal.details = Some(Box::new(
            serde_json::json!({ "reason": LOGIN_FLOW_EXPIRED_REASON }),
        ));
        return Err(refusal);
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
pub(crate) struct SciotteLoginRequest {
    pub email: String,
    pub password: SecretString,
    /// How to sign in: the provider's own form (`email`, the default when the
    /// body names none), `google` or `apple`. Any other value is refused when
    /// the body is read, before a browser is ever started.
    #[serde(default)]
    pub method: LoginMethod,
    /// Target platform: "strava" (default), "garmin", "trainingpeaks" or "coros"
    #[serde(default = "default_target")]
    pub target: String,
    /// The user ticked the TrainingPeaks or COROS exposure notice on this
    /// attempt. Read only for a TrainingPeaks or COROS login, and only while
    /// the account has not already accepted the current notice.
    #[serde(default)]
    pub tos_consent: bool,
}

fn default_target() -> String {
    "strava".to_owned()
}

#[derive(Debug, Deserialize)]
pub(crate) struct SciotteSelectTwoFactorRequest {
    pub option_id: String,
}

#[derive(Deserialize)]
pub(crate) struct SciotteOtpRequest {
    pub code: String,
}

impl fmt::Debug for SciotteOtpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteOtpRequest")
            .field("code", &"[REDACTED]")
            .finish()
    }
}

/// notify: scrape session established. One helper for both session-write
/// sites so the emit cannot drift between them. The event carries the
/// user-facing provider name ("strava"/"garmin"/"trainingpeaks"/"coros") so
/// the connect/disconnect pair is measured on the same axis as the OAuth
/// surface (the disconnect chokepoint also emits user-facing names for mirror
/// backends); `backend` distinguishes the scrape cohort. `user_id`/`tenant_id`
/// ride inline because the `PostHog` sink drops events it cannot attribute.
fn notify_sciotte_connected(user_id: Uuid, tenant_id: Uuid, backend: &str) {
    info!(
        target: "notify",
        event = "provider.connected",
        provider = %backend_resolver::user_facing_name(backend),
        backend = %backend,
        user_id = %user_id,
        tenant_id = %tenant_id,
        "user connected fitness provider (scrape session)"
    );
}

/// Store a successful sciotte session in Pierre's encrypted DB and register the connection.
/// After storing, raises `provider.connected` and warms the account's data from
/// the session (see [`warm_from_session`]).
async fn store_sciotte_session(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    session: &AuthSession,
    provider_name: &str,
) -> Result<Response, AppError> {
    let stored = persist_sciotte_session(
        &resources.repos,
        &resources.cache,
        user_id,
        TenantId::from_uuid(tenant_id),
        session,
        provider_name,
    )
    .await?;

    notify_sciotte_connected(user_id, tenant_id, provider_name);
    warm_from_session(
        resources,
        user_id,
        tenant_id,
        session,
        &stored,
        provider_name,
    );

    Ok(Json(serde_json::json!({"status": "connected", "provider": provider_name})).into_response())
}

/// Start what a freshly stored session owes the account: an activity
/// pre-fetch so the agent can serve data immediately, then the health
/// backfill. A `TrainingPeaks` session is first probed for its account role,
/// which decides whether there is a calendar to prefetch at all. A session
/// the provider rotates on those reads is written back over `stored`'s row.
fn warm_from_session(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    session: &AuthSession,
    stored: &StoredSession,
    provider_name: &str,
) {
    // Pre-fetch activities in background so the cache is warm when the user
    // chats. A TrainingPeaks coach account has no calendar of its own, so its
    // role is read first and decides whether there is anything to prefetch.
    // Registered before anything else reads the session: a list read that
    // arrives meanwhile (the onboarding proposal) waits for this one instead
    // of scraping the fresh session beside it (carnet#736).
    let prefetch = if provider_name == SCIOTTE_TRAININGPEAKS {
        spawn_login_probe(resources, user_id, tenant_id, session, stored)
    } else {
        spawn_activity_prefetch(resources, user_id, tenant_id, provider_name, stored)
    };

    // The session also feeds the health sync (Garmin's and COROS's nights,
    // resting heart rate, HRV, body metrics): read its window now rather than
    // at the next scheduled cycle — once the pre-fetch has finished with the
    // session, so the two never hold two of the scraper's browsers at once.
    #[cfg(feature = "health-sync")]
    spawn_health_backfill(
        resources,
        &user_id.to_string(),
        backend_resolver::user_facing_name(provider_name),
        prefetch,
    );
    #[cfg(not(feature = "health-sync"))]
    drop(prefetch);
}

/// The super admin clone's install: the same store and warm-up as a login,
/// with no `provider.connected` event, since a super admin copying a session
/// onto a test account is not an athlete connecting and must not count as one.
#[async_trait::async_trait]
impl ScrapeSessionInstaller for AuthRoutesContext {
    async fn install_session(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        backend: &str,
        session: &AuthSession,
    ) -> AppResult<()> {
        let stored = persist_sciotte_session(
            &self.repos,
            &self.cache,
            user_id,
            tenant_id,
            session,
            backend,
        )
        .await?;
        warm_from_session(
            self,
            user_id,
            tenant_id.as_uuid(),
            session,
            &stored,
            backend,
        );
        Ok(())
    }
}

/// Which step of a sign-in produced the outcome being answered.
#[derive(Clone, Copy)]
enum LoginStep<'a> {
    /// The credential login, which starts a flow.
    Login {
        /// Backend provider name the login targets
        provider: &'a str,
    },
    /// A 2FA pick or a code, continuing the flow the platform holds.
    Continuation {
        /// Backend provider name the held flow was started for
        provider: &'a str,
        /// The held flow this continuation named to the service
        flow_id: &'a str,
    },
}

impl LoginStep<'_> {
    /// Backend provider name the step belongs to.
    const fn provider(&self) -> &str {
        match self {
            Self::Login { provider } | Self::Continuation { provider, .. } => provider,
        }
    }
}

/// Keep the flow a code step names.
///
/// A refused code leaves the service's browser parked under the same id, on
/// the age it already had; the held entry ages with it, so writing it again
/// would outlive the browser it names. Any other code step (or a refusal
/// naming a flow not held) is remembered.
async fn hold_code_step(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    step: LoginStep<'_>,
    flow_id: String,
    refused: bool,
) {
    let held = matches!(step, LoginStep::Continuation { flow_id: named, .. } if named == flow_id);
    if refused && held {
        info!(
            user_id = %user_id,
            provider = %step.provider(),
            "Sciotte code refused by the provider; the flow stays pending"
        );
    } else {
        remember_remote_flow(
            &resources.cache,
            tenant_id,
            user_id,
            flow_id,
            step.provider(),
        )
        .await;
    }
}

/// Map a [`LoginOutcome`] from the dedicated scraper service to an HTTP
/// response.
///
/// The service holds the interactive browser state across the multi-step 2FA
/// flow (keyed by `flow_id`), so the platform parks nothing. On success the
/// full session is exported from the service and persisted, keeping the
/// platform session-of-record (ADR-021).
async fn remote_login_to_response(
    outcome: LoginOutcome,
    remote: &SciotteClient,
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    step: LoginStep<'_>,
    link_context: Option<&LinkContext>,
) -> Result<Response, AppError> {
    let provider = step.provider();
    match outcome {
        LoginOutcome::Authenticated {
            session_id,
            provider,
            ..
        } => {
            // The service reports the provider the session authenticates against
            // ("strava"/"garmin"/"trainingpeaks"/"coros"); map it to the backend
            // name the platform persists under. The platform never has to
            // remember the provider across the multi-step 2FA flow — only the
            // flow_id string.
            forget_remote_flow(&resources.cache, tenant_id, user_id).await;
            let provider_name = SciotteTarget::from_target_param(&provider).provider_name();
            info!(user_id = %user_id, provider = %provider_name, "Sciotte remote login successful");
            let session = remote
                .export_session(&session_id)
                .await
                .map_err(to_app_error)?;
            if let Some(link) = link_context {
                emit_provider_linked_webhook(user_id, tenant_id, provider_name, link);
            }
            store_sciotte_session(resources, user_id, tenant_id, &session, provider_name).await
        }
        LoginOutcome::OtpRequired {
            flow_id, rejected, ..
        } => {
            let refused = rejected.is_some();
            hold_code_step(resources, user_id, tenant_id, step, flow_id, refused).await;
            let body = if refused {
                serde_json::json!({"status": "otp_required", "reason": CODE_REJECTED_REASON})
            } else {
                serde_json::json!({"status": "otp_required"})
            };
            Ok(Json(body).into_response())
        }
        LoginOutcome::TwoFactorChoice {
            options, flow_id, ..
        } => {
            remember_remote_flow(&resources.cache, tenant_id, user_id, flow_id, provider).await;
            Ok(
                Json(serde_json::json!({"status": "two_factor_choice", "options": options}))
                    .into_response(),
            )
        }
        LoginOutcome::NumberMatch {
            number, flow_id, ..
        } => {
            remember_remote_flow(&resources.cache, tenant_id, user_id, flow_id, provider).await;
            Ok(Json(serde_json::json!({
                "status": "number_match",
                "number": number,
            }))
            .into_response())
        }
        LoginOutcome::Failed { reason } => {
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
pub(crate) struct LinkContext {
    /// Channel slug the token was minted for (e.g. "slack")
    pub channel: String,
    /// Optional channel thread/DM id so the bot can reply in the original thread
    pub channel_thread: Option<String>,
    /// Target platform (strava / garmin / trainingpeaks / coros) from the claims
    pub target: String,
}

/// Extract authenticated `user_id` and `tenant_id` from request headers.
///
/// Accepts two authentication methods, tried in order:
/// 1. A channel-initiated provider link-token (`Authorization: Bearer <link-token>`),
///    minted in-process by the messaging pipeline or the backfill notifier. This
///    is the path used by the hosted Sciotte login page embedded in chat
///    channels, and it reaches only the login, 2FA and OTP routes.
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

/// Log + alert a sciotte login *system* failure and build the user-facing error.
///
/// A *system* failure is an `Err` from the dedicated scraper service (transport
/// fault, service 5xx, browser-launch failure), as opposed to a
/// `LoginOutcome::Failed` credential rejection (a *user* failure surfaced
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
    let display = backend_resolver::brand_name(&global_registry(), provider)
        .unwrap_or("your fitness account");
    format!(
        "We couldn't sign you in to {display} right now. \
         This is usually temporary — please try again in a few minutes."
    )
}

/// Friendly, provider-aware copy shown to the user when the provider stopped a
/// sign-in at a challenge (a captcha) the scraper service could not answer.
///
/// The English fallback of `hosted.common.signInChallenged`; the hosted login
/// pages show the catalogue's copy in the athlete's locale instead.
#[must_use]
pub fn friendly_login_challenge_message(provider: &str) -> String {
    let display = backend_resolver::brand_name(&global_registry(), provider)
        .unwrap_or("Your fitness account");
    format!(
        "{display} asked for an extra security check we can't complete automatically \
         right now. Please try again in a little while."
    )
}

/// The athlete-facing refusal of a sign-in the provider stopped at a
/// challenge: the friendly copy, verbatim under `InvalidInput` (see
/// `AppError::sanitized_message`), and [`LOGIN_CHALLENGE_REASON`] under
/// `details.reason` for the login UIs to select on.
///
/// A `4xx` on purpose: `response_failure_log_middleware` records a `5xx` that
/// carries no `Retry-After` at `ERROR`, which pages operators, and the service
/// names no wait for a challenge.
fn login_challenge_refusal(provider: &str) -> AppError {
    let mut refusal = AppError::invalid_input(friendly_login_challenge_message(provider));
    refusal.details = Some(Box::new(
        serde_json::json!({ "reason": LOGIN_CHALLENGE_REASON }),
    ));
    refusal
}

/// Turn an `Err` from the dedicated scraper service into what the athlete gets
/// back: a retryable shed response, the challenge refusal, or the
/// operator-alerting system failure.
///
/// A load-shed is the service's designed answer to a saturated Chrome budget
/// (`503` + `Retry-After`), so it is deliberately routed *around*
/// [`report_login_system_failure`]. That path emits an `error!` — which
/// `dravr-tronc` forwards to the `#dev-dravr-errors` Slack channel — plus a
/// `sync.failed` business event, and both would fire once per shed request at
/// exactly the moment the service is busiest, turning healthy backpressure into
/// an operator page storm.
///
/// A sign-in the provider stopped at a challenge (a captcha) goes around it
/// too: it is the provider's bot check, which no operator can answer, and the
/// athlete is told so in [`friendly_login_challenge_message`]'s words. Unlike
/// a shed it ends the attempt — the service stopped the login — so it is an
/// `Err`, which the continuation handlers answer by dropping the flow.
///
/// Everything else still alerts.
///
/// # Errors
///
/// Returns the challenge refusal for a challenge, and the system-failure
/// [`AppError`] (already logged + alerted) for any other error that is not a
/// shed.
pub fn login_failure_response(
    user_id: Uuid,
    tenant_id: Uuid,
    provider: &str,
    stage: &str,
    error: &AppError,
) -> Result<Response, AppError> {
    if is_login_challenge(error) {
        warn!(
            %user_id,
            provider = %provider,
            stage = %stage,
            "Sciotte login stopped at a provider challenge — the athlete's to retry, not a \
             system failure"
        );
        return Err(login_challenge_refusal(provider));
    }
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
/// `details` (one value, attached once, which `impl IntoResponse for AppError`
/// renders as the header), and the same friendly copy a login failure carries.
///
/// [`ErrorCode::ExternalRateLimited`] is the code that renders as `503` *and*
/// whose message `AppError::sanitized_message` passes through verbatim, so the
/// login modal shows the copy rather than a canned description. The header is
/// also what `response_failure_log_middleware` keys on to record the `503` at
/// `WARN` instead of `ERROR`, so the shed leaves one backpressure line in the
/// logs and nothing in the ops channel.
fn shed_response(provider: &str, retry_after_secs: u64) -> Response {
    AppError::new(
        ErrorCode::ExternalRateLimited,
        friendly_login_failure_message(provider),
    )
    .with_retry_after(retry_after_secs)
    .into_response()
}

/// Refuse a login to a backend whose exposure notice is in force for this
/// account until it has accepted the current version, recording the
/// acceptance this attempt carries. A backend with no notice, or an account
/// the `provider_exposure_notice` flag leaves off, passes.
///
/// TrainingPeaks and COROS offer no API Pierre can use, so the scraper reads
/// through the user's own signed-in session — access their terms prohibit for
/// third parties, putting the account at risk. Every connect surface (web,
/// mobile, the hosted page, the channel picker) states this before the
/// credentials field; this is where that statement becomes a precondition
/// rather than copy, so no surface can skip it. An account that already
/// accepted this notice version is not asked again, including on a reconnect
/// after a disconnect.
async fn require_provider_terms(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    target: SciotteTarget,
    accepted_now: bool,
) -> Result<(), AppError> {
    require_notice_accepted(
        &resources.repos,
        tenant_id,
        user_id,
        target.provider_name(),
        target.brand(),
        accepted_now,
    )
    .await
}

/// Credential-based login via the dedicated dravr-sciotte scraper service (ADR-021)
pub(crate) async fn handle_sciotte_login(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Json(request): Json<SciotteLoginRequest>,
) -> Result<Response, AppError> {
    let (user_id, tenant_id, link_context) = authenticate(&resources, &headers).await?;

    // A provider-specific link-token signs in only to the platform it was
    // minted for — it must not accept another platform's notice or store a
    // session under it. The connect picker's token leaves the choice to the
    // page, so it carries no platform of its own.
    if let Some(link) = link_context.as_ref() {
        let minted_for = SciotteTarget::from_target_param(&link.target).provider_name();
        let requested = SciotteTarget::from_target_param(&request.target).provider_name();
        if link.target != CONNECT_PROVIDER && minted_for != requested {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "This login link is for another provider; ask for a fresh link for this one",
            ));
        }
    }

    if request.email.is_empty() || request.password.expose_secret().is_empty() {
        return Err(AppError::invalid_input("Email and password are required"));
    }

    let target = &request.target;
    // Before anything else can happen with these credentials: a login to a
    // provider with an exposure notice is refused until the account accepted it.
    require_provider_terms(
        &resources,
        user_id,
        tenant_id,
        SciotteTarget::from_target_param(target),
        request.tos_consent,
    )
    .await?;

    // This login supersedes whatever the user left parked: the service mints a
    // fresh flow_id for it, and an id inherited from an abandoned 2FA would
    // name a flow the service has already reaped.
    forget_remote_flow(&resources.cache, tenant_id, user_id).await;

    let provider = SciotteTarget::from_target_param(target).provider_name();

    // Session short-circuit: a stored Sciotte session written within the reuse
    // window answers a double tap or a refresh-after-connect without a second
    // login; anything older is replaced by signing in with these credentials.
    // One the platform has observed dead (its expiry is past, or a refused
    // read flagged this caller's connection to the provider `needs_reauth` or
    // `revoked`) seeds the login browser with its cookies so the provider
    // sees the device it trusted last time; one not observed dead is never
    // sent, since its live cookies could sign in to the account a coach is
    // switching away from. The lookups are by this caller's (user, tenant,
    // provider), so no other athlete's session or connection can seed it.
    let prior_session = match stored_session_use(&resources, user_id, tenant_id, provider).await? {
        StoredSessionUse::Reused(response) => return Ok(response),
        StoredSessionUse::SignIn { prior_session } => prior_session,
    };

    // ADR-021: the login runs on the dedicated dravr-sciotte service (there is
    // no in-process fallback since the Phase 4 cutover). The service owns the
    // Chrome budget and the interactive 2FA parking; the platform stays
    // session-of-record (it exports + persists the session, keyed by flow_id).
    // A transport/service fault (not a credential rejection, which comes back
    // as `Failed`, nor a load-shed, which comes back as retryable backpressure)
    // is a *system* failure: alert operators + emit `sync.failed`.
    let remote = resources.provider_registry.require_sciotte_client()?;
    info!(
        user_id = %user_id,
        target = %target,
        with_prior_session = prior_session.is_some(),
        "Starting sciotte credential login (remote service)"
    );
    let outcome = match remote
        .login_with_credentials(
            &request.email,
            request.password.expose_secret(),
            request.method,
            SciotteTarget::from_target_param(target).scraper_provider_name(),
            prior_session.as_ref(),
        )
        .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            return login_failure_response(
                user_id,
                tenant_id,
                provider,
                "credential_login",
                &to_app_error(e),
            );
        }
    };
    remote_login_to_response(
        outcome,
        remote,
        &resources,
        user_id,
        tenant_id,
        LoginStep::Login { provider },
        link_context.as_ref(),
    )
    .await
}

/// Select a 2FA method for a pending login
pub(crate) async fn handle_sciotte_select_2fa(
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
    let remote = resources.provider_registry.require_sciotte_client()?;
    info!(user_id = %user_id, option = %request.option_id, "Selecting sciotte 2FA method (remote service)");
    let outcome = match remote.select_2fa(&request.option_id, &flow_id).await {
        Ok(outcome) => outcome,
        Err(e) => {
            return match login_failure_response(
                user_id,
                tenant_id,
                &provider,
                "two_factor",
                &to_app_error(e),
            ) {
                // A shed never reached the parked browser, so the flow is still
                // there for the retry the `Retry-After` invites — keep it.
                Ok(shed) => Ok(shed),
                // Both login UIs send the user back to the credential screen
                // from their error phase, so a system failure or a provider
                // challenge ends the attempt: dropping the entry stops the next
                // login from inheriting an id the service reaps.
                Err(failure) => {
                    forget_remote_flow(&resources.cache, tenant_id, user_id).await;
                    Err(failure)
                }
            };
        }
    };
    remote_login_to_response(
        outcome,
        remote,
        &resources,
        user_id,
        tenant_id,
        LoginStep::Continuation {
            provider: &provider,
            flow_id: &flow_id,
        },
        link_context.as_ref(),
    )
    .await
}

/// Submit OTP code for a pending login
pub(crate) async fn handle_sciotte_submit_otp(
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
    let remote = resources.provider_registry.require_sciotte_client()?;
    info!(user_id = %user_id, "Submitting sciotte OTP (remote service)");
    let outcome = match remote.submit_otp(&request.code, &flow_id).await {
        Ok(outcome) => outcome,
        Err(e) => {
            return match login_failure_response(
                user_id,
                tenant_id,
                &provider,
                "otp",
                &to_app_error(e),
            ) {
                // A shed never reached the parked browser, so the flow is still
                // there for the retry the `Retry-After` invites — keep it.
                Ok(shed) => Ok(shed),
                // Both login UIs send the user back to the credential screen
                // from their error phase, so a system failure or a provider
                // challenge ends the attempt: dropping the entry stops the next
                // login from inheriting an id the service reaps.
                Err(failure) => {
                    forget_remote_flow(&resources.cache, tenant_id, user_id).await;
                    Err(failure)
                }
            };
        }
    };
    remote_login_to_response(
        outcome,
        remote,
        &resources,
        user_id,
        tenant_id,
        LoginStep::Continuation {
            provider: &provider,
            flow_id: &flow_id,
        },
        link_context.as_ref(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sciotte_otp_request_debug_hides_code() {
        let sentinel = "sentinel-otp-4b7d";
        let request = SciotteOtpRequest {
            code: sentinel.to_owned(),
        };
        let printed = format!("{request:?}");
        assert!(!printed.contains(sentinel), "{printed}");
        assert!(printed.contains("SciotteOtpRequest"), "{printed}");
    }
}
