// ABOUTME: OAuth route handlers for provider authorization, token exchange, and connection management
// ABOUTME: Thin HTTP layer delegating business logic to pierre_services::oauth_flow::OAuthService
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};
// Both name the sync notifier handed to `RefreshService`, which only exists
// under `health-sync`.
#[cfg(feature = "health-sync")]
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use tracing::{error, field, field::Empty, info, warn, Span};

use crate::AuthRoutesContext;
use pierre_auth::oauth2_client::{OAuthClientState, PkceParams};
use pierre_auth::tenant::TenantContext;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::TenantId;
use pierre_mcp_transport::oauth_flow_manager::OAuthTemplateRenderer;
use pierre_providers::backend_resolver;
use pierre_providers::ProviderDescriptor;
use pierre_services::oauth_flow::{
    categorize_oauth_error, extract_tenant_id, get_user_for_oauth, parse_user_id, OAuthService,
};
use pierre_services::oauth_redirects;
use pierre_services::provider_refresh::RefreshService;
#[cfg(feature = "health-sync")]
use pierre_services::provider_refresh::SyncNotifier;

use pierre_auth::dto::auth::{OAuthStatus, ProviderStatus, ProvidersStatusResponse};
use pierre_auth::strava_pool;
use pierre_core::constants::oauth::providers as oauth_providers;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Axum handler functions — called from AuthRoutes::routes() in mod.rs
// ---------------------------------------------------------------------------

/// Handle OAuth callback
#[tracing::instrument(
    skip(resources, params),
    fields(
        route = "oauth_callback",
        provider = %provider,
        user_id = Empty,
        success = Empty,
    )
)]
pub async fn handle_oauth_callback(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let oauth_routes = OAuthService::new(resources.data.clone(), resources.config.clone());

    // Check if we should redirect to a separate frontend URL
    let frontend_url = &resources.config.frontend_url.clone();

    // The user cancelled, or the provider returned an error: there is no `code`
    // (e.g. Strava sends `?error=access_denied`). Redirect back to the frontend
    // callback page (or mobile deep link) with a friendly error instead of
    // surfacing a raw `AuthInvalid` JSON body to the browser.
    if !params.contains_key("code") {
        let error_msg = params.get("error").map_or("cancelled", String::as_str);
        let state = params.get("state").map_or("", String::as_str);
        let mobile_redirect_url = oauth_redirects::extract_mobile_redirect_from_state(
            state,
            &resources.config.base_url,
            &resources.config.security.allowed_mobile_redirect_origins,
        );
        if let Some(mobile_url) = mobile_redirect_url {
            let redirect_url = oauth_redirects::oauth_return_url(
                mobile_url.trim_end_matches('/'),
                &provider,
                false,
                Some(error_msg),
            );
            return Ok((StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response());
        }
        if let Some(url) = frontend_url {
            let base = format!("{}/oauth-callback", url.trim_end_matches('/'));
            let redirect_url =
                oauth_redirects::oauth_return_url(&base, &provider, false, Some(error_msg));
            return Ok((StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response());
        }
        return Err(AppError::auth_invalid("OAuth authorization was cancelled"));
    }

    let code = params
        .get("code")
        .ok_or_else(|| AppError::auth_invalid("Missing OAuth code parameter"))?;

    let state = params
        .get("state")
        .ok_or_else(|| AppError::auth_invalid("Missing OAuth state parameter"))?;

    match oauth_routes.handle_callback(code, state, &provider).await {
        Ok(response) => {
            #[cfg(feature = "health-sync")]
            spawn_health_backfill(&resources, &response.user_id, &response.provider);

            // Completion reaches the client through the durable path:
            // `handle_callback` has already written the notification row, and
            // `append_oauth_notifications_to_response` attaches every unread one
            // to the next tool response. The push that used to run here fanned
            // out over session-keyed MCP protocol streams, which revision
            // 2026-07-28 removed along with protocol sessions.

            // Priority: mobile redirect URL > frontend URL > render template
            // Mobile apps pass redirect URL through OAuth state for deep linking
            if let Some(mobile_url) = &response.mobile_redirect_url {
                let redirect_url = oauth_redirects::oauth_return_url(
                    mobile_url.trim_end_matches('/'),
                    &provider,
                    true,
                    None,
                );
                // Don't log the full URL — a hosted-connect return carries the
                // connect link-token in its query string.
                info!(provider = %provider, "Redirecting OAuth success to embedded return URL");
                return Ok(
                    (StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response()
                );
            }

            // If frontend URL is configured, redirect to frontend with success params
            if let Some(url) = frontend_url {
                let base = format!("{}/oauth-callback", url.trim_end_matches('/'));
                let redirect_url = oauth_redirects::oauth_return_url(&base, &provider, true, None);
                info!(provider = %provider, "Redirecting OAuth success to frontend");
                return Ok(
                    (StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response()
                );
            }

            // Otherwise serve the success page directly (same-origin production)
            let html = OAuthTemplateRenderer::render_success_template(&provider, &response);

            Ok((StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html).into_response())
        }
        Err(e) => {
            error!("OAuth callback failed: {}", e);

            // Determine error message and description based on error type
            let (error_msg, description) = categorize_oauth_error(&e);

            // For errors, we need to parse the state to check for mobile redirect URL
            // since handle_callback failed and didn't return the parsed state
            let config = &resources.config;
            let mobile_redirect_url = oauth_redirects::extract_mobile_redirect_from_state(
                state,
                &config.base_url,
                &config.security.allowed_mobile_redirect_origins,
            );

            // Priority: mobile redirect URL > frontend URL > render template
            if let Some(mobile_url) = mobile_redirect_url {
                let redirect_url = oauth_redirects::oauth_return_url(
                    mobile_url.trim_end_matches('/'),
                    &provider,
                    false,
                    Some(error_msg),
                );
                // Don't log the full URL — a hosted-connect return carries the
                // connect link-token in its query string.
                info!(provider = %provider, "Redirecting OAuth error to embedded return URL");
                return Ok(
                    (StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response()
                );
            }

            // If frontend URL is configured, redirect to frontend with error params
            if let Some(url) = frontend_url {
                let base = format!("{}/oauth-callback", url.trim_end_matches('/'));
                let redirect_url =
                    oauth_redirects::oauth_return_url(&base, &provider, false, Some(error_msg));
                info!(provider = %provider, "Redirecting OAuth error to frontend");
                return Ok(
                    (StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response()
                );
            }

            let html =
                OAuthTemplateRenderer::render_error_template(&provider, error_msg, description);

            Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/html")],
                html,
            )
                .into_response())
        }
    }
}

/// Handle OAuth status check
#[tracing::instrument(
    skip(resources, headers),
    fields(
        route = "oauth_status",
        user_id = Empty,
    )
)]
pub async fn handle_oauth_status(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate using middleware (supports both cookies and Authorization header)
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;

    // Check OAuth provider connection status for the user (cross-tenant view).
    // A repository failure propagates as a 5xx: an all-disconnected payload is
    // reserved for the user who genuinely holds no tokens, so the client can
    // tell "you are not connected" apart from "we could not find out".
    let tokens = resources
        .repos
        .oauth_tokens
        .get_tokens(user_id, None)
        .await?;

    // Convert tokens to status objects
    let mut provider_statuses = vec![];
    let mut providers_seen = HashSet::new();

    for token in tokens {
        if providers_seen.insert(token.provider.clone()) {
            provider_statuses.push(OAuthStatus {
                provider: token.provider,
                connected: true,
                last_sync: Some(token.created_at.to_rfc3339()),
            });
        }
    }

    // Add default providers if not connected
    for provider in ["strava", "fitbit"] {
        if !providers_seen.contains(provider) {
            provider_statuses.push(OAuthStatus {
                provider: provider.to_owned(),
                connected: false,
                last_sync: None,
            });
        }
    }

    Ok((StatusCode::OK, Json(provider_statuses)).into_response())
}

/// Get all providers with connection status
///
/// Returns all available providers from the registry with their connection status.
/// Uses `provider_connections` table as the single source of truth for connectivity.
pub async fn handle_providers_status(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate using middleware
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let response = compute_providers_status(&resources, auth_result.user_id).await;
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Is this card's provider connected, counting either backend that serves it?
///
/// The Strava and Garmin cards are named for their MIRROR backends (`sciotte`,
/// `sciotte_garmin`) because the scrape is the supported way in, and the raw
/// `strava` / `garmin` cards are withheld — `garmin` is skipped outright below,
/// and both clients filter `strava` out. But a connection made through OAuth is
/// stored under the user-facing name. One athlete and one provider therefore
/// have two possible row names, so a card lights up when EITHER is present.
///
/// Both clients used to do this themselves, and only for Strava: each carried a
/// copy of "if a connected `strava` row exists, mark the `sciotte` card
/// connected". Garmin had no such copy, so an athlete with a `garmin` row was
/// told no provider was connected — by the connect screen, the connect banner
/// and the chat header alike, while `/mcp` said connected (carnet#255). Deciding
/// it here means the two clients cannot disagree with each other or with the
/// server, and Garmin is covered by the same rule rather than a third copy.
fn card_is_connected(card: &str, rows: &HashSet<String>) -> bool {
    rows.contains(card)
        || rows.contains(backend_resolver::user_facing_name(card))
        || backend_resolver::mirror_backend_for(card).is_some_and(|mirror| rows.contains(mirror))
}

/// Compute the provider catalogue + connection status for a user.
///
/// Shared by the JWT-gated `/api/providers` handler and the channel-initiated
/// hosted connect page, which authenticates via a provider link-token rather
/// than a session cookie. Infallible: a repo failure degrades to an empty
/// connection set / "no seats left" rather than erroring the page.
pub async fn compute_providers_status(
    resources: &AuthRoutesContext,
    user_id: Uuid,
) -> ProvidersStatusResponse {
    use pierre_providers::registry::global_registry;

    // Get all supported providers from the registry
    let registry = global_registry();
    let supported_providers = registry.supported_providers();

    // Get user's provider connections (cross-tenant view, single source of truth)
    let connections = resources
        .repos
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap_or_default();

    // A connection whose lifecycle status is NeedsReauth/Revoked still has a row
    // (so it reads as "connected") but its session no longer works — a dead
    // sciotte scrape or a non-recoverable OAuth refresh. Track those separately so
    // the screen can show "Reconnect needed" rather than a healthy "Connected".
    let reauth_providers: HashSet<String> = connections
        .iter()
        .filter(|c| c.status.requires_reauth())
        .map(|c| c.provider.clone())
        .collect();

    let connected_providers: HashSet<String> =
        connections.into_iter().map(|c| c.provider).collect();

    // Seat availability across the shared-app OAuth POOL: the env-default
    // STRAVA_CLIENT_ID app plus any enabled DB pool apps. Strava enforces a
    // per-application athlete cap not exposed via any API header, so onboarding a
    // NEW Strava connection prefers OAuth while ANY pool seat remains and falls
    // back to the Sciotte mirror once every app is full. A summary failure is
    // treated as "no seats left" so we degrade to the always-available mirror
    // rather than pushing the user into an OAuth flow Strava would reject.
    let strava_seats_left = strava_pool::strava_seat_summary(resources.repos.oauth_tokens.as_ref())
        .await
        .map_or(0, |s| s.left());

    // Build provider status list
    let mut provider_statuses = Vec::new();

    for provider_name in supported_providers {
        // Garmin's OAuth API is uncredentialed/unsupported — never advertise it so
        // the UI can't offer a "Garmin Connect" card that 500s on connect (the
        // `sciotte_garmin` scrape card is the supported Garmin path).
        if provider_name == oauth_providers::GARMIN {
            continue;
        }
        // Get provider descriptor from registry
        if let Some(descriptor) = registry.get_descriptor(provider_name) {
            let caps = descriptor.capabilities();
            let requires_oauth = caps.requires_oauth();

            // Determine connection status from the provider_connections table,
            // coalescing the card's two possible backends (see `card_is_connected`).
            let connected = card_is_connected(provider_name, &connected_providers);
            // A connected provider whose session died (dead scrape / failed OAuth
            // refresh) is flagged so the UI prompts a reconnect instead of showing
            // a healthy "Connected". Only meaningful while `connected` is true.
            let needs_reauth = connected && card_is_connected(provider_name, &reauth_providers);

            // Always show all providers regardless of connection status.
            // Non-OAuth providers (like synthetic) appear with connected=false
            // so users can activate them from the provider modal.

            // Build capabilities list from bitflags
            let mut capabilities = Vec::new();
            if caps.supports_activities() {
                capabilities.push("activities".to_owned());
            }
            if caps.supports_sleep() {
                capabilities.push("sleep".to_owned());
            }
            if caps.supports_recovery() {
                capabilities.push("recovery".to_owned());
            }
            if caps.supports_health() {
                capabilities.push("health".to_owned());
            }

            // The Sciotte entry is the user-facing "Strava" card. Tell the
            // frontend which Strava backend a new connection should use:
            // shared-app OAuth while seats remain, else the Sciotte mirror.
            let (recommended_backend, seats_left) = if provider_name == oauth_providers::SCIOTTE {
                let backend = if strava_seats_left > 0 {
                    "oauth"
                } else {
                    "mirror"
                };
                (Some(backend.to_owned()), Some(strava_seats_left))
            } else {
                (None, None)
            };

            provider_statuses.push(ProviderStatus {
                provider: provider_name.to_owned(),
                display_name: descriptor.display_name().to_owned(),
                requires_oauth,
                connected,
                needs_reauth,
                capabilities,
                recommended_backend,
                seats_left,
            });
        }
    }

    // Sort providers in a consistent display order
    let provider_order: HashMap<&str, usize> = [
        "synthetic",
        "synthetic_sleep",
        "sciotte",
        "sciotte_garmin",
        "strava",
        "garmin",
        "fitbit",
        "whoop",
        "coros",
        "terra",
    ]
    .iter()
    .enumerate()
    .map(|(i, &name)| (name, i))
    .collect();
    provider_statuses.sort_by_key(|p| {
        provider_order
            .get(p.provider.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });

    ProvidersStatusResponse {
        providers: provider_statuses,
    }
}

/// Handle OAuth authorization initiation
///
/// Requires authentication and verifies that the authenticated user matches
/// the `user_id` in the path to prevent unauthorized OAuth flow initiation.
#[tracing::instrument(
    skip(resources, headers),
    fields(
        route = "oauth_auth_initiate",
        provider = %provider,
        user_id = %user_id_str,
        tenant_id = Empty,
    )
)]
pub async fn handle_oauth_auth_initiate(
    State(resources): State<AuthRoutesContext>,
    Path((provider, user_id_str)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate the request before proceeding
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = parse_user_id(&user_id_str)?;

    // Verify authenticated user matches the requested user_id
    if auth_result.user_id != user_id {
        warn!(
            "OAuth auth initiate: authenticated user {} does not match path user_id {}",
            auth_result.user_id, user_id
        );
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Cannot initiate OAuth flow for a different user",
        ));
    }

    info!(
        "OAuth authorization initiation for provider: {} user: {}",
        provider, user_id_str
    );

    // Verify user exists
    get_user_for_oauth(resources.repos.users.as_ref(), user_id).await?;
    let tenant_id = extract_tenant_id(auth_result.active_tenant_id.map(TenantId::from_uuid))?;

    let oauth_service = OAuthService::new(resources.data.clone(), resources.config.clone());

    let auth_response = oauth_service
        .get_auth_url(user_id, tenant_id, &provider)
        .await
        .map_err(|e| {
            error!(
                "Failed to generate OAuth URL for {} user {}: {}",
                provider, user_id, e
            );
            AppError::internal(format!("Failed to generate OAuth URL for {provider}: {e}"))
        })?;

    info!(
        "Generated OAuth URL for {} user {} (state issued)",
        provider, user_id
    );

    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, auth_response.authorization_url)],
    )
        .into_response())
}

/// Handle mobile OAuth initiation
///
/// Returns OAuth URL in JSON format for mobile apps to use with in-app browsers.
/// Accepts optional `redirect_url` query parameter for deep linking back to the app.
#[tracing::instrument(
    skip(resources, headers, query),
    fields(
        route = "mobile_oauth_init",
        provider = %provider,
        user_id = Empty,
    )
)]
pub async fn handle_mobile_oauth_init(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    // Authenticate using middleware
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;
    info!(
        "Mobile OAuth initiation for provider: {} user: {}",
        provider, user_id
    );

    // Get optional redirect_uri from query parameters (mobile app's deep link)
    let redirect_url = query.get("redirect_uri");

    // Validate redirect URL against allowlist to prevent open-redirect attacks
    if let Some(url) = redirect_url {
        let base_url = &resources.config.base_url;
        let extra_origins = &resources.config.security.allowed_mobile_redirect_origins;
        if !oauth_redirects::is_allowed_redirect_url(url, base_url, extra_origins) {
            return Err(AppError::invalid_input(
                "Invalid redirect_url. Must use dravr://, exp://, http://localhost, or an HTTPS origin matching the server's base_url.",
            ));
        }
    }

    // Verify user exists
    get_user_for_oauth(resources.repos.users.as_ref(), user_id).await?;
    let tenant_id = extract_tenant_id(auth_result.active_tenant_id.map(TenantId::from_uuid))?;

    // Build OAuth state with optional redirect URL
    let state = redirect_url.map_or_else(
        || format!("{}:{}", user_id, uuid::Uuid::new_v4()),
        |url| {
            let encoded_url = URL_SAFE_NO_PAD.encode(url.as_bytes());
            format!("{}:{}:{}", user_id, uuid::Uuid::new_v4(), encoded_url)
        },
    );

    // Generate OAuth URL using the state with embedded redirect URL
    let tenant_name = resources
        .repos
        .tenants
        .get_by_id(tenant_id)
        .await
        .map_or_else(|_| "Unknown Tenant".to_owned(), |t| t.name);

    // Generating an OAuth authorize URL for an already-identified user and
    // tenant; no membership lookup happened, so no role is asserted.
    let ctx = TenantContext::for_tenant_scoped_operation(tenant_id, tenant_name, user_id);

    // Check if the provider supports PKCE for enhanced security
    let use_pkce = resources
        .provider_registry
        .get_descriptor(&provider)
        .and_then(ProviderDescriptor::oauth_params)
        .is_some_and(|p| p.use_pkce);

    let pkce = if use_pkce {
        Some(PkceParams::generate())
    } else {
        None
    };

    let authorization_url = if let Some(ref pkce_params) = pkce {
        resources
            .tenant_oauth_client
            .get_authorization_url_with_pkce(
                &ctx,
                &provider,
                &state,
                pkce_params,
                resources.repos.tenants.as_ref(),
                resources.repos.oauth_tokens.as_ref(),
            )
            .await
    } else {
        resources
            .tenant_oauth_client
            .get_authorization_url(
                &ctx,
                &provider,
                &state,
                resources.repos.tenants.as_ref(),
                resources.repos.oauth_tokens.as_ref(),
            )
            .await
    }
    .map_err(|e| {
        error!(
            "Failed to generate OAuth URL for {} user {}: {}",
            provider, user_id, e
        );
        AppError::internal(format!("Failed to generate OAuth URL for {provider}: {e}"))
    })?;

    // Build redirect URI for state storage
    let oauth_redirect_uri = format!(
        "{}/api/oauth/callback/{provider}",
        resources.config.base_url
    );

    // Store state server-side for CSRF protection with 10-minute TTL.
    // The pkce_code_verifier is stored alongside the state for PKCE token exchange.
    let now = chrono::Utc::now();
    let client_state = OAuthClientState {
        state: state.clone(),
        provider: provider.clone(),
        user_id: Some(user_id),
        tenant_id: Some(tenant_id.to_string()),
        redirect_uri: oauth_redirect_uri,
        scope: None,
        pkce_code_verifier: pkce.as_ref().map(|p| p.code_verifier.clone()),
        oauth_app_client_id: None,
        created_at: now,
        expires_at: now + chrono::Duration::minutes(10),
        used: false,
    };

    resources
        .repos
        .oauth_client_state
        .store_oauth_client_state(&client_state)
        .await
        .map_err(|e| {
            error!("Failed to store OAuth state for CSRF protection: {}", e);
            AppError::internal("Failed to initiate OAuth flow")
        })?;

    info!(
        "Generated mobile OAuth URL for {} user {} (state issued){}",
        provider,
        user_id,
        if redirect_url.is_some() {
            " (with redirect)"
        } else {
            ""
        }
    );

    // Return JSON response with OAuth URL (mobile apps need this for in-app browsers)
    // State is returned so mobile apps can correlate the callback
    Ok((
        StatusCode::OK,
        Json(json!({
            "authorization_url": authorization_url,
            "provider": provider,
            "state": state,
            "message": format!("Visit the authorization URL to connect your {} account", provider)
        })),
    )
        .into_response())
}

/// REST endpoint to disconnect a provider
///
/// DELETE /api/oauth/providers/:provider/disconnect
///
/// Disconnects a fitness provider (e.g., Strava, Fitbit) by deleting the stored OAuth tokens.
/// Requires valid JWT authentication via cookie or Authorization header.
#[tracing::instrument(
    skip(resources, headers),
    fields(
        route = "oauth_disconnect",
        provider = %provider,
        user_id = Empty,
        tenant_id = Empty,
    )
)]
pub async fn handle_disconnect_provider_rest(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate using middleware (supports both cookies and Authorization header)
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;

    // Record IDs on the span for request tracing.
    let span = Span::current();
    span.record("user_id", field::display(&user_id));
    if let Some(tenant_id) = auth_result.active_tenant_id {
        span.record("tenant_id", field::display(&tenant_id));
    }

    info!("Disconnecting provider {} for user {}", provider, user_id);

    // The service is the domain chokepoint: it deletes the token + connection
    // row and emits the `provider.disconnected` notify event, so this route
    // (like the chat and /mcp tool paths) emits nothing itself.
    let oauth_service = OAuthService::new(resources.data.clone(), resources.config.clone());
    oauth_service
        .disconnect_provider(user_id, &provider, auth_result.active_tenant_id)
        .await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Trigger a data sync for a specific provider.
///
/// POST /api/providers/{provider}/sync
///
/// Starts a background sync and returns immediately with status.
/// Optionally blocks if `?wait=true` query parameter is provided.
#[tracing::instrument(
    skip(resources, headers, params),
    fields(
        route = "provider_sync",
        provider = %provider,
        user_id = Empty,
        tenant_id = Empty,
    )
)]
pub async fn handle_sync_provider(
    State(resources): State<AuthRoutesContext>,
    Path(provider): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;
    let tenant_id = auth_result
        .active_tenant_id
        .ok_or_else(|| AppError::auth_invalid("Tenant context required for sync"))?;
    let wait = params.get("wait").is_some_and(|v| v == "true");

    // Record IDs on the span so the NotifyLayer can attribute the
    // provider.fetch_started event without re-passing fields.
    let span = Span::current();
    span.record("user_id", field::display(&user_id));
    span.record("tenant_id", field::display(&tenant_id));

    info!(
        user_id = %user_id,
        provider = %provider,
        wait = %wait,
        "REST-triggered provider sync"
    );

    // notify: explicit REST sync is unambiguously user_initiated.
    info!(
        target: "notify",
        event = "provider.fetch_started",
        provider = %provider,
        trigger = "user_initiated",
        "user-triggered provider sync starting"
    );

    #[cfg(feature = "health-sync")]
    let notifier: Arc<dyn SyncNotifier> = resources.sync_notifier.clone();
    let auth_repos = resources.repos.auth_repos();
    let refresh_service = RefreshService::new(
        &auth_repos,
        resources.repos.activity_cache.clone(),
        #[cfg(feature = "health-sync")]
        resources.sync_orchestrator.clone(),
        #[cfg(feature = "health-sync")]
        notifier,
    );

    let result = refresh_service
        .refresh_provider(user_id, TenantId::from_uuid(tenant_id), &provider, wait)
        .await;

    Ok(Json(json!({
        "provider": result.provider,
        "success": result.success,
        "message": result.message,
        "records_synced": result.records_synced,
    }))
    .into_response())
}

/// Spawn background health data backfill after a successful OAuth connection.
///
/// Triggers a 30-day backfill via the sync orchestrator so the user gets
/// historical data immediately after connecting a wearable provider.
#[cfg(feature = "health-sync")]
fn spawn_health_backfill(resources: &AuthRoutesContext, user_id: &str, provider: &str) {
    const BACKFILL_DAYS: u32 = 30;

    let Some(orchestrator) = resources.sync_orchestrator.clone() else {
        return;
    };
    let user_id = user_id.to_owned();
    let provider = provider.to_owned();
    tokio::spawn(async move {
        info!(
            user_id = %user_id,
            provider = %provider,
            days = BACKFILL_DAYS,
            "Triggering initial health data backfill after OAuth connection"
        );
        if let Err(e) = orchestrator
            .backfill(&user_id, &provider, BACKFILL_DAYS)
            .await
        {
            tracing::error!(
                error = %e,
                user_id = %user_id,
                provider = %provider,
                "Initial health data backfill failed"
            );
        }
    });
}
