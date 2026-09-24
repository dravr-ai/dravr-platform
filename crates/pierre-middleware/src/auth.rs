// ABOUTME: MCP authentication middleware for request authentication and authorization
// ABOUTME: Handles JWT tokens and API keys with rate limiting and user context extraction
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::HeaderMap;
use chrono::{Duration, Utc};
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::api_keys::ApiKeyManager;
use pierre_auth::auth::{AuthManager, AuthMethod, AuthResult};
use pierre_auth::config::RateLimitConfig;
use pierre_auth::rate_limiting::UnifiedRateLimitCalculator;
use pierre_auth::security::cookies::get_cookie_value;
use pierre_auth::user_status::enforce_user_status;
use pierre_core::constants::key_prefixes;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{TenantId, User};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::uuid_utils::parse_uuid;
// Trait methods dispatched through repos.api_keys / repos.messaging / repos.tenants / repos.usage / repos.users
use pierre_database::RepositoryRegistry;
use std::sync::Arc;
use tracing::field::Empty;
use tracing::{debug, info, warn};

/// How stale `users.last_active` may grow before an authenticated request
/// writes it again, in minutes.
///
/// The seat-reclaim sweeper counts idle time in days and the admin views show
/// "last seen", so five minutes is fine-grained for both while an athlete
/// whose client calls every second costs one write per window rather than one
/// per request.
const LAST_ACTIVE_REFRESH_MINUTES: i64 = 5;

/// What a request path does with the scopes of the credential it accepts.
///
/// A delegated OAuth grant — an access token the authorization server minted
/// for a third-party application, as narrow as the athlete consented to — is
/// only a limit where something reads it. The MCP and A2A tool dispatch read
/// it; a REST handler does not, and acts with the athlete's whole authority.
/// Accepted there, a `fitness:read` consent would become everything the
/// athlete can do: minting a full-grant API key that outlives the grant,
/// driving a chat turn that runs under the self grant, reaching the admin
/// console when the athlete is an operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GrantPolicy {
    /// The athlete's own credential only: a session, an API key. A delegated
    /// grant is refused with 403.
    DirectOnly,
    /// A delegated grant too, because the caller enforces its scopes.
    ScopesEnforced,
}

/// The refusal a delegated grant meets on a path that reads no scope.
///
/// `PermissionDenied`, so it answers 403 and its sentence reaches the client:
/// the credential is genuine and the application holding it is told where it
/// is accepted, rather than being sent back to re-authenticate by a 401.
fn delegated_grant_refused() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "This access token was delegated to an application and is accepted only by the MCP and A2A endpoints",
    )
}

/// Middleware for `MCP` protocol authentication
///
/// Holds the full `RepositoryRegistry` because authentication and rate-
/// limiting cross-cut `AuthRepos` (`users`, `api_keys`, `oauth_tokens`)
/// and `UsageRepos` (`api-key` + JWT usage counters). Narrowing here
/// would require two stored views and two separate constructor params
/// with no material benefit — the middleware is a long-lived singleton.
#[derive(Clone)]
pub struct McpAuthMiddleware {
    auth_manager: AuthManager,
    api_key_manager: ApiKeyManager,
    rate_limit_calculator: UnifiedRateLimitCalculator,
    repos: Arc<RepositoryRegistry>,
    jwks_manager: Arc<JwksManager>,
}

impl McpAuthMiddleware {
    /// Create new `MCP` auth middleware
    #[must_use]
    pub fn new(
        auth_manager: AuthManager,
        repos: Arc<RepositoryRegistry>,
        jwks_manager: Arc<JwksManager>,
        rate_limit_config: RateLimitConfig,
    ) -> Self {
        Self {
            auth_manager,
            api_key_manager: ApiKeyManager::new(),
            rate_limit_calculator: UnifiedRateLimitCalculator::new_with_config(rate_limit_config),
            repos,
            jwks_manager,
        }
    }

    /// Authenticate request using headers (supports cookies and Authorization header)
    ///
    /// Only the athlete's own credential authenticates here; a delegated OAuth
    /// grant is refused, as on [`Self::authenticate_request`].
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Authentication credentials missing (no cookie or header)
    /// - The credential is a delegated OAuth grant (403 `PermissionDenied`)
    /// - JWT token validation fails
    /// - API key validation fails
    /// - Database queries fail
    /// - Rate limit calculations fail
    /// - User lookup fails
    #[tracing::instrument(
        skip(self, headers),
        fields(
            auth_method = Empty,
            user_id = Empty,
            tenant_id = Empty,
            success = Empty,
        )
    )]
    pub async fn authenticate_request_with_headers(
        &self,
        headers: &HeaderMap,
    ) -> AppResult<AuthResult> {
        debug!("=== AUTH MIDDLEWARE AUTHENTICATE_REQUEST_WITH_HEADERS START ===");

        // Try cookie authentication first (preferred for web clients)
        if let Some(jwt_token) = get_cookie_value(headers, "auth_token") {
            debug!("Found JWT in httpOnly cookie, attempting authentication");
            tracing::Span::current().record("auth_method", "JWT_COOKIE");
            match self
                .authenticate_jwt_token(&jwt_token, GrantPolicy::DirectOnly)
                .await
            {
                Ok(result) => {
                    let span = tracing::Span::current();
                    span.record("user_id", result.user_id.to_string())
                        .record("success", true);
                    if let Some(tid) = result.active_tenant_id {
                        span.record("tenant_id", tid.to_string());
                    }
                    info!(
                        "JWT cookie authentication successful for user: {}",
                        result.user_id
                    );
                    return Ok(result);
                }
                Err(e) => {
                    // Cookie auth failed — fall through to Authorization header
                    // instead of returning immediately. This handles cases where a
                    // stale/invalid cookie is present alongside a valid header.
                    debug!(
                        "JWT cookie authentication failed ({}), trying Authorization header",
                        e
                    );
                }
            }
        }

        // Fall back to Authorization header for API clients
        let auth_header = headers.get("authorization").and_then(|h| h.to_str().ok());

        self.authenticate_request(auth_header).await
    }

    /// Authenticate a request on a path that reads no scope — every REST route
    /// — and extract user context with rate limiting.
    ///
    /// Only the athlete's own credential authenticates: a session JWT, an API
    /// key. A delegated OAuth grant is refused, because the handler behind this
    /// acts with the athlete's whole authority and would ignore how narrow the
    /// grant is. The MCP transport, which enforces the grant at tool dispatch,
    /// uses [`Self::authenticate_scoped_request`] instead.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Authentication header is missing or malformed
    /// - JWT token validation fails
    /// - The token is a delegated OAuth grant (403 `PermissionDenied`)
    /// - API key validation fails
    /// - Database queries fail
    /// - Rate limit calculations fail
    /// - User lookup fails
    pub async fn authenticate_request(&self, auth_header: Option<&str>) -> AppResult<AuthResult> {
        self.authenticate_header(auth_header, GrantPolicy::DirectOnly)
            .await
    }

    /// Authenticate a request on a path that enforces the credential's scopes
    /// itself — the MCP transport, whose tool dispatch refuses any tool the
    /// grant does not cover.
    ///
    /// Accepts everything [`Self::authenticate_request`] does, plus a delegated
    /// OAuth grant, whose narrowed scopes ride out on [`AuthResult::scopes`].
    /// A caller that does not read those scopes must not use this.
    ///
    /// # Errors
    ///
    /// As [`Self::authenticate_request`], less the delegated-grant refusal.
    pub async fn authenticate_scoped_request(
        &self,
        auth_header: Option<&str>,
    ) -> AppResult<AuthResult> {
        self.authenticate_header(auth_header, GrantPolicy::ScopesEnforced)
            .await
    }

    /// Authenticate an `Authorization` header value under `policy`.
    #[tracing::instrument(
        skip(self, auth_header),
        fields(
            auth_method = Empty,
            user_id = Empty,
            tenant_id = Empty,
            success = Empty,
        )
    )]
    async fn authenticate_header(
        &self,
        auth_header: Option<&str>,
        policy: GrantPolicy,
    ) -> AppResult<AuthResult> {
        debug!("=== AUTH MIDDLEWARE AUTHENTICATE_REQUEST START ===");
        debug!("Auth header provided: {}", auth_header.is_some());

        let auth_str = if let Some(header) = auth_header {
            // Security: Do not log auth header content to prevent token leakage
            debug!(
                "Authentication attempt with header type: {}",
                if header.starts_with(key_prefixes::LIVE) || header.starts_with(key_prefixes::TRIAL)
                {
                    "API_KEY"
                } else if header.starts_with("Bearer ") {
                    "JWT_TOKEN"
                } else {
                    "UNKNOWN"
                }
            );
            header
        } else {
            warn!("Authentication failed: Missing authorization header");
            return Err(AppError::auth_invalid("Authentication failed: Missing authorization header - Request authentication requires Authorization header with Bearer token or API key"));
        };

        // Try API key authentication first (starts with pk_live_ or pk_trial_)
        if auth_str.starts_with(key_prefixes::LIVE) || auth_str.starts_with(key_prefixes::TRIAL) {
            tracing::Span::current().record("auth_method", "API_KEY");
            debug!("Attempting API key authentication");
            match self.authenticate_api_key(auth_str).await {
                Ok(result) => {
                    let span = tracing::Span::current();
                    span.record("user_id", result.user_id.to_string())
                        .record("success", true);
                    if let Some(tid) = result.active_tenant_id {
                        span.record("tenant_id", tid.to_string());
                    }
                    info!(
                        "API key authentication successful for user: {}",
                        result.user_id
                    );
                    Ok(result)
                }
                Err(e) => {
                    tracing::Span::current().record("success", false);
                    warn!("API key authentication failed: {}", e);
                    Err(e)
                }
            }
        }
        // Then try Bearer token authentication
        else if let Some(token) = auth_str.strip_prefix("Bearer ") {
            tracing::Span::current().record("auth_method", "JWT_TOKEN");
            debug!("Attempting JWT token authentication");
            match self.authenticate_jwt_token(token, policy).await {
                Ok(result) => {
                    let span = tracing::Span::current();
                    span.record("user_id", result.user_id.to_string())
                        .record("success", true);
                    if let Some(tid) = result.active_tenant_id {
                        span.record("tenant_id", tid.to_string());
                    }
                    info!("JWT authentication successful for user: {}", result.user_id);
                    Ok(result)
                }
                Err(e) => {
                    tracing::Span::current().record("success", false);
                    warn!("JWT authentication failed: {}", e);
                    Err(e)
                }
            }
        } else {
            tracing::Span::current()
                .record("auth_method", "INVALID")
                .record("success", false);
            warn!("Authentication failed: Invalid authorization header format (expected 'Bearer ...' or 'pk_live_...')");
            Err(AppError::auth_invalid("Invalid authorization header format - must be 'Bearer <token>' or 'pk_live_<api_key>'"))
        }
    }

    /// Authenticate using `API` key
    async fn authenticate_api_key(&self, api_key: &str) -> AppResult<AuthResult> {
        // Validate key format
        self.api_key_manager.validate_key_format(api_key)?;

        // Extract prefix and hash the key
        let key_prefix = self.api_key_manager.extract_key_prefix(api_key);
        let key_hash = self.api_key_manager.hash_key(api_key);

        // Look up the API key in database
        let db_key = self
            .repos
            .api_keys
            .get_by_prefix(&key_prefix, &key_hash)
            .await?
            .ok_or_else(|| {
                AppError::auth_invalid(format!("API key not found or invalid: {key_prefix}"))
            })?;

        // Validate key status
        self.api_key_manager.is_key_valid(&db_key)?;

        // SECURITY: Enforce account-status policy on the API-key path.
        // A Pending or Suspended user's API key must not authenticate, same as
        // their JWT cookie / channel link cannot. Single source of truth lives
        // in services::user_status_gate.
        let user = self
            .repos
            .users
            .get_global(db_key.user_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("User {} for API key", db_key.user_id)))?;
        enforce_user_status(user.user_status).inspect_err(|e| {
            warn!(
                user_id = %db_key.user_id,
                status = ?user.user_status,
                error = %e,
                "API key access denied by user-status gate"
            );
        })?;

        // Get current usage for rate limiting
        let current_usage = self.repos.usage.get_api_key_current(&db_key.id).await?;
        let rate_limit = self
            .rate_limit_calculator
            .calculate_api_key_rate_limit(&db_key, current_usage);

        // Check rate limit. A breach is a 429 with a retry window — it used to
        // surface as `ExternalServiceError` (HTTP 502), which read as "server
        // broken" to clients and misfired their backoff (registre#10).
        if rate_limit.is_rate_limited {
            let retry_after = rate_limit.reset_at.map_or(3600, |dt| {
                let now = chrono::Utc::now().timestamp();
                u64::try_from((dt.timestamp() - now).max(0)).unwrap_or(3600)
            });
            return Err(AppError::rate_limit_exceeded(
                i64::from(current_usage),
                i64::from(rate_limit.limit.unwrap_or(0)),
                retry_after,
            ));
        }

        // Update last used timestamp
        self.repos.api_keys.update_last_used(&db_key.id).await?;
        self.note_activity(&user).await;

        // Resolve user's default tenant — API keys are single-tenant by design
        let active_tenant_id = self
            .repos
            .tenants
            .list_for_user(db_key.user_id)
            .await
            .map_err(|e| {
                AppError::database(format!(
                    "Failed to resolve tenant for API key user {}: {e}",
                    db_key.user_id
                ))
            })?
            .first()
            .map(|t| t.id.as_uuid());

        Ok(AuthResult {
            user_id: db_key.user_id,
            auth_method: AuthMethod::ApiKey {
                key_id: db_key.id,
                tier: format!("{:?}", db_key.tier).to_lowercase(),
            },
            rate_limit,
            active_tenant_id,
            // The athlete acting directly, not a third party acting for them,
            // so the credential is not a narrowed delegation. The role gate
            // still decides admin independently.
            scopes: OAuthScope::self_grant(),
            session_id: None,
        })
    }

    /// Authenticate an inbound messaging webhook turn via the channel link.
    ///
    /// Messaging counterpart to [`Self::authenticate_jwt_token`] — same shape
    /// (`AuthResult`), same gates (user existence, status, rate limit, active
    /// tenant), same single source of truth. `services::messaging_ingress`
    /// calls this for every inbound `Telegram` / `WhatsApp` / `Discord` /
    /// `Slack` / `Messenger` message so the chat pipeline, command
    /// dispatcher, and tool execution layer downstream cannot distinguish
    /// messaging callers from web/mobile callers.
    ///
    /// `tenant_id` is the bot's tenant (resolved from the channel config at
    /// the webhook boundary); `active_tenant_id` in the returned
    /// [`AuthResult`] is the user's own first-membership tenant so tool
    /// execution (`OAuth`, activities) stays inside the user's data scope
    /// even when the bot lives in a different tenant.
    ///
    /// # Errors
    ///
    /// - [`AppError::auth_invalid`] — channel link missing or malformed
    ///   (sender not linked, or DB row corrupted).
    /// - [`AppError::not_found`] — link references a deleted user.
    /// - [`AppError::account_pending`] / [`AppError::account_suspended`] —
    ///   linked user is not active (callers map this to the localized
    ///   `KEY_ACCOUNT_PENDING` / `KEY_ACCOUNT_SUSPENDED` reply).
    /// - [`AppError::rate_limit_exceeded`] — the user's request budget is
    ///   breached (callers map this to the localized `KEY_RATE_LIMITED` reply).
    /// - [`AppError::database`] — DB lookup failures.
    pub async fn authenticate_channel(
        &self,
        tenant_id: TenantId,
        channel: &str,
        channel_user_id: &str,
    ) -> AppResult<AuthResult> {
        let link = self
            .repos
            .messaging
            .get_channel_link(tenant_id, channel, channel_user_id)
            .await?
            .ok_or_else(|| {
                AppError::auth_invalid(format!(
                    "No channel link for {channel}:{channel_user_id} under tenant {tenant_id}"
                ))
            })?;

        let user_id_str = link["user_id"]
            .as_str()
            .ok_or_else(|| AppError::internal("Channel link missing user_id"))?;
        let user_id = parse_uuid(user_id_str).map_err(|_| {
            AppError::internal(format!("Channel link user_id not a UUID: {user_id_str}"))
        })?;

        // SECURITY: Global lookup — channel-link validation, no tenant context yet.
        let user = self
            .repos
            .users
            .get_global(user_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("User {user_id}")))?;

        // Single source of truth for the approval policy — same call as the
        // JWT path so an admin flipping Pending → Active takes effect on every
        // transport simultaneously.
        enforce_user_status(user.user_status).inspect_err(|e| {
            warn!(
                user_id = %user_id,
                status = ?user.user_status,
                channel = %channel,
                error = %e,
                "Channel access denied by user-status gate"
            );
        })?;

        // Resolve the user's own tenant for tool execution (OAuth, activities).
        // Webhook tenant is the bot's tenant; the user may belong to a
        // different one. Falling back to the bot's tenant keeps the request
        // routable when the user has zero memberships (defensive — the OTP
        // link flow already rejects users with empty membership lists).
        let active_tenant_id = self
            .repos
            .tenants
            .list_for_user(user_id)
            .await
            .map_err(|e| {
                AppError::database(format!(
                    "Failed to resolve tenant for messaging user {user_id}: {e}"
                ))
            })?
            .first()
            .map_or_else(|| Some(tenant_id.as_uuid()), |t| Some(t.id.as_uuid()));

        // Rate limit on the user-tier policy (JWT calculator — channel links
        // are long-lived user credentials, same authority as a session token).
        let current_usage = self.repos.usage.get_jwt_current_usage(user_id).await?;
        let rate_limit = self
            .rate_limit_calculator
            .calculate_jwt_rate_limit(&user, current_usage);

        // A breach is a rate-limit error, not an external-service fault: the
        // messaging denial arm matches `ErrorCode::RateLimitExceeded` to send
        // the localized "slow down" reply, and the old `ExternalServiceError`
        // shape fell through to the operator-error catch-all instead — the
        // rate-limited user got silence and on-call got paged (registre#8).
        if rate_limit.is_rate_limited {
            let retry_after = rate_limit.reset_at.map_or(3600, |dt| {
                let now = chrono::Utc::now().timestamp();
                u64::try_from((dt.timestamp() - now).max(0)).unwrap_or(3600)
            });
            warn!(
                user_id = %user_id,
                channel = %channel,
                current_usage,
                limit = rate_limit.limit.unwrap_or(0),
                "Channel rate limit exceeded; refusing the turn"
            );
            return Err(AppError::rate_limit_exceeded(
                i64::from(current_usage),
                i64::from(rate_limit.limit.unwrap_or(0)),
                retry_after,
            ));
        }

        Ok(AuthResult {
            user_id,
            auth_method: AuthMethod::ChannelLink {
                channel: channel.to_owned(),
                channel_user_id: channel_user_id.to_owned(),
                tier: format!("{:?}", user.tier).to_lowercase(),
            },
            rate_limit,
            active_tenant_id,
            // The athlete acting directly, not a third party acting for them,
            // so the credential is not a narrowed delegation. The role gate
            // still decides admin independently.
            scopes: OAuthScope::self_grant(),
            session_id: None,
        })
    }

    /// Authenticate using RS256 JWT token
    ///
    /// Under [`GrantPolicy::DirectOnly`] a token whose grant is narrower than
    /// the self grant is refused before anything is read or written for it —
    /// no usage row, no activity — since the request it carries is not served.
    async fn authenticate_jwt_token(
        &self,
        token: &str,
        policy: GrantPolicy,
    ) -> AppResult<AuthResult> {
        let claims = self
            .auth_manager
            .validate_token_detailed(token, &self.jwks_manager)
            .map_err(|e| AppError::auth_invalid(format!("JWT validation failed: {e}")))?;

        let user_id = parse_uuid(&claims.sub)
            .map_err(|_| AppError::auth_invalid("Invalid user ID in token"))?;

        // Whatever the token was minted with. A first-party session was minted
        // with the full self grant; a delegated OAuth token stays as narrow as
        // the athlete consented to. A token minted before the `scope` claim
        // existed parses to the empty grant and is refused every tool that
        // reads or writes — deliberate, and the reason this shipped with a
        // re-authentication rather than a compatibility path.
        let scopes = OAuthScope::parse_granted(&claims.scope);
        if policy == GrantPolicy::DirectOnly && !OAuthScope::is_self_grant(&scopes) {
            warn!(
                user_id = %user_id,
                granted = %OAuthScope::render_granted(&scopes),
                "Delegated OAuth grant refused on a path that does not enforce scopes"
            );
            return Err(delegated_grant_refused());
        }

        // Extract active_tenant_id from JWT claims (multi-tenant user tenant selection)
        let active_tenant_id = claims.active_tenant_id.as_deref().and_then(|tid| {
            parse_uuid(tid)
                .inspect_err(|e| {
                    warn!(
                        tenant_id = %tid,
                        error = %e,
                        "Invalid active_tenant_id format in JWT claims, will use default tenant"
                    );
                })
                .ok()
        });

        // SECURITY: Global lookup — JWT validation, no tenant context yet
        let user = self
            .repos
            .users
            .get_global(user_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("User {user_id}")))?;

        // SECURITY: Enforce account status on every API request.
        // Login endpoints let pending users authenticate (so the frontend can show
        // the "pending approval" page), but API routes must not serve data.
        // Same gate runs in services::messaging_ingress so Telegram / WhatsApp /
        // Discord / Slack / Messenger channel traffic can't bypass approval.
        enforce_user_status(user.user_status).inspect_err(|e| {
            warn!(user_id = %user_id, status = ?user.user_status, error = %e, "API access denied by user-status gate");
        })?;

        // Get current usage for rate limiting
        let current_usage = self.repos.usage.get_jwt_current_usage(user_id).await?;
        let rate_limit = self
            .rate_limit_calculator
            .calculate_jwt_rate_limit(&user, current_usage);

        // Check rate limit. A 429 with a retry window, not a 401: "slow down"
        // and "bad credentials" demand opposite client reactions, and the old
        // auth_invalid shape sent rate-limited clients into re-login loops
        // (registre#10).
        if rate_limit.is_rate_limited {
            let retry_after = rate_limit.reset_at.map_or(3600, |dt| {
                let now = chrono::Utc::now().timestamp();
                u64::try_from((dt.timestamp() - now).max(0)).unwrap_or(3600)
            });
            return Err(AppError::rate_limit_exceeded(
                i64::from(current_usage),
                i64::from(rate_limit.limit.unwrap_or(0)),
                retry_after,
            ));
        }

        // Record JWT usage so the *next* request sees an accurate
        // `get_jwt_current_usage` and the rate-limit gate actually fires.
        // Without this write the counter stays at zero across all users and
        // every transport that goes through the JWT calculator (HTTP cookie,
        // Bearer header, `MCP`, and — via `authenticate_channel` — every
        // messaging webhook). Same posture as
        // `services::messaging_ingress::record_channel_usage`.
        record_jwt_usage_for_request(&self.repos, user_id, "http:jwt", "AUTH").await;
        self.note_activity(&user).await;

        Ok(AuthResult {
            user_id,
            auth_method: AuthMethod::JwtToken {
                tier: format!("{:?}", user.tier).to_lowercase(),
            },
            rate_limit,
            active_tenant_id,
            scopes,
            // The Guardian turn token: the `jti` only for a per-turn (ACP) token;
            // `None` for a reused session token so a stateless MCP client is keyed
            // per-call, not across its whole session (#2).
            session_id: claims.guardian_turn_token(),
        })
    }

    /// Get reference to the auth manager for testing purposes
    #[must_use]
    pub const fn auth_manager(&self) -> &AuthManager {
        &self.auth_manager
    }

    /// Record that `user` is using Dravr.
    ///
    /// `users.last_active` is how the admin "last seen" views and the Strava
    /// seat reclaimer tell an athlete who uses Dravr from one who left, and an
    /// athlete who only reaches it through an MCP client (a Claude or
    /// `ChatGPT` connector's `OAuth2` token, or an API key) never logs in again, so
    /// every request authenticated here is activity. The row is written only
    /// once the value this authentication already read is older than
    /// [`LAST_ACTIVE_REFRESH_MINUTES`]. Best-effort, like the usage row: a
    /// failed write is logged and the request proceeds.
    async fn note_activity(&self, user: &User) {
        if Utc::now() - user.last_active < Duration::minutes(LAST_ACTIVE_REFRESH_MINUTES) {
            return;
        }
        if let Err(e) = self.repos.users.update_last_active(user.id).await {
            warn!(
                user_id = %user.id,
                error = %e,
                "Failed to update last_active on an authenticated request (activity tracking impacted)"
            );
        }
    }
}

/// Record a `JwtUsage` row for a successful JWT/cookie authentication.
///
/// [`UnifiedRateLimitCalculator::calculate_jwt_rate_limit`] reads
/// `repos.usage.get_jwt_current_usage`; without a paired write the counter
/// stays at zero and the rate-limit gate never fires across any transport
/// that uses this calculator (HTTP cookie, Bearer header, `MCP`, and — via
/// [`McpAuthMiddleware::authenticate_channel`] — every messaging webhook).
///
/// `endpoint` is rendered like `http:jwt` / `messaging:telegram` so admin
/// usage reports can disaggregate by transport. `status_code = 200`
/// reflects the auth result, not the eventual handler outcome — the rate-
/// limit policy intentionally counts all authenticated requests, including
/// those that go on to fail downstream.
///
/// Best-effort: a write failure is logged but does not block the request.
/// The symmetry partner on the messaging side is
/// `services::messaging_ingress::record_channel_usage`.
pub async fn record_jwt_usage_for_request(
    repos: &RepositoryRegistry,
    user_id: uuid::Uuid,
    endpoint: &str,
    method: &str,
) {
    use pierre_core::models::usage::JwtUsage;

    let usage = JwtUsage {
        id: None,
        user_id,
        timestamp: chrono::Utc::now(),
        endpoint: endpoint.to_owned(),
        method: method.to_owned(),
        status_code: 200,
        response_time_ms: None,
        request_size_bytes: None,
        response_size_bytes: None,
        ip_address: None,
        user_agent: None,
    };

    if let Err(e) = repos.usage.record_jwt_usage(&usage).await {
        warn!(
            user_id = %user_id,
            endpoint = %endpoint,
            error = %e,
            "Failed to record jwt_usage (rate limiting counter impacted)"
        );
    }
}
