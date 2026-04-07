// ABOUTME: OAuth route handlers for provider authorization, token exchange, and connection management
// ABOUTME: Thin HTTP layer delegating business logic to services::oauth_flow::OAuthService
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use tracing::{error, field::Empty, info, warn};
use urlencoding::encode;

use crate::{
    context::ServerContext,
    errors::{AppError, ErrorCode},
    mcp::{oauth_flow_manager::OAuthTemplateRenderer, resources::ServerResources},
    models::TenantId,
    providers::ProviderDescriptor,
    services::oauth_flow::{
        self as oauth_flow_service, categorize_oauth_error, extract_tenant_id, get_user_for_oauth,
        parse_user_id, OAuthService,
    },
};
use pierre_auth::oauth2_client::{OAuthClientState, PkceParams};
use pierre_auth::tenant::{TenantContext, TenantRole};

use std::collections::HashSet;

use super::types::{OAuthStatus, ProviderStatus, ProvidersStatusResponse};

/// OAuth routes - alias for OAuth service to match test expectations
pub type OAuthRoutes = OAuthService;

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
pub(super) async fn handle_oauth_callback(
    State(resources): State<Arc<ServerResources>>,
    Path(provider): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let server_context = ServerContext::from(resources.as_ref());
    let oauth_routes = OAuthService::new(
        server_context.data().clone(),
        server_context.config().clone(),
        server_context.notification().clone(),
    );

    let code = params
        .get("code")
        .ok_or_else(|| AppError::auth_invalid("Missing OAuth code parameter"))?;

    let state = params
        .get("state")
        .ok_or_else(|| AppError::auth_invalid("Missing OAuth state parameter"))?;

    // Check if we should redirect to a separate frontend URL
    let frontend_url = server_context.config().config().frontend_url.clone();

    match oauth_routes.handle_callback(code, state, &provider).await {
        Ok(response) => {
            #[cfg(feature = "health-sync")]
            spawn_health_backfill(&resources, &response.user_id, &response.provider);

            // Priority: mobile redirect URL > frontend URL > render template
            // Mobile apps pass redirect URL through OAuth state for deep linking
            if let Some(mobile_url) = &response.mobile_redirect_url {
                let redirect_url = format!(
                    "{}?provider={}&success=true",
                    mobile_url.trim_end_matches('/'),
                    encode(&provider)
                );
                info!("Redirecting OAuth success to mobile app: {}", redirect_url);
                return Ok(
                    (StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response()
                );
            }

            // If frontend URL is configured, redirect to frontend with success params
            if let Some(url) = frontend_url {
                let redirect_url = format!(
                    "{}/oauth-callback?provider={}&success=true",
                    url.trim_end_matches('/'),
                    encode(&provider)
                );
                info!("Redirecting OAuth success to frontend: {}", redirect_url);
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
            let config = server_context.config().config();
            let mobile_redirect_url = oauth_flow_service::extract_mobile_redirect_from_state(
                state,
                &config.base_url,
                &config.security.allowed_mobile_redirect_origins,
            );

            // Priority: mobile redirect URL > frontend URL > render template
            if let Some(mobile_url) = mobile_redirect_url {
                let redirect_url = format!(
                    "{}?provider={}&success=false&error={}",
                    mobile_url.trim_end_matches('/'),
                    encode(&provider),
                    encode(error_msg)
                );
                info!("Redirecting OAuth error to mobile app: {}", redirect_url);
                return Ok(
                    (StatusCode::FOUND, [(header::LOCATION, redirect_url)], "").into_response()
                );
            }

            // If frontend URL is configured, redirect to frontend with error params
            if let Some(url) = frontend_url {
                let redirect_url = format!(
                    "{}/oauth-callback?provider={}&success=false&error={}",
                    url.trim_end_matches('/'),
                    encode(&provider),
                    encode(error_msg)
                );
                info!("Redirecting OAuth error to frontend: {}", redirect_url);
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
pub(super) async fn handle_oauth_status(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate using middleware (supports both cookies and Authorization header)
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;

    // Check OAuth provider connection status for the user (cross-tenant view)
    let provider_statuses = resources
        .repos
        .oauth_tokens
        .get_tokens(user_id, None)
        .await
        .map_or_else(
            |_| {
                vec![
                    OAuthStatus {
                        provider: "strava".to_owned(),
                        connected: false,
                        last_sync: None,
                    },
                    OAuthStatus {
                        provider: "fitbit".to_owned(),
                        connected: false,
                        last_sync: None,
                    },
                ]
            },
            |tokens| {
                // Convert tokens to status objects
                let mut statuses = vec![];
                let mut providers_seen = HashSet::new();

                for token in tokens {
                    if providers_seen.insert(token.provider.clone()) {
                        statuses.push(OAuthStatus {
                            provider: token.provider,
                            connected: true,
                            last_sync: Some(token.created_at.to_rfc3339()),
                        });
                    }
                }

                // Add default providers if not connected
                for provider in ["strava", "fitbit"] {
                    if !providers_seen.contains(provider) {
                        statuses.push(OAuthStatus {
                            provider: provider.to_owned(),
                            connected: false,
                            last_sync: None,
                        });
                    }
                }

                statuses
            },
        );

    Ok((StatusCode::OK, Json(provider_statuses)).into_response())
}

/// Get all providers with connection status
///
/// Returns all available providers from the registry with their connection status.
/// Uses `provider_connections` table as the single source of truth for connectivity.
pub(super) async fn handle_providers_status(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    use crate::providers::registry::global_registry;

    // Authenticate using middleware
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;

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

    let connected_providers: HashSet<String> =
        connections.into_iter().map(|c| c.provider).collect();

    // Build provider status list
    let mut provider_statuses = Vec::new();

    for provider_name in supported_providers {
        // Get provider descriptor from registry
        if let Some(descriptor) = registry.get_descriptor(provider_name) {
            let caps = descriptor.capabilities();
            let requires_oauth = caps.requires_oauth();

            // Determine connection status from the provider_connections table
            let connected = connected_providers.contains(provider_name);

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

            provider_statuses.push(ProviderStatus {
                provider: provider_name.to_owned(),
                display_name: descriptor.display_name().to_owned(),
                requires_oauth,
                connected,
                capabilities,
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

    let response = ProvidersStatusResponse {
        providers: provider_statuses,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
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
pub(super) async fn handle_oauth_auth_initiate(
    State(resources): State<Arc<ServerResources>>,
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
    let tenant_id = extract_tenant_id(auth_result.active_tenant_id.map(TenantId::from))?;

    let server_context = ServerContext::from(resources.as_ref());
    let oauth_service = OAuthService::new(
        server_context.data().clone(),
        server_context.config().clone(),
        server_context.notification().clone(),
    );

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
pub(super) async fn handle_mobile_oauth_init(
    State(resources): State<Arc<ServerResources>>,
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
        if !oauth_flow_service::is_allowed_redirect_url(url, base_url, extra_origins) {
            return Err(AppError::invalid_input(
                "Invalid redirect_url. Must use pierre://, exp://, http://localhost, or an HTTPS origin matching the server's base_url.",
            ));
        }
    }

    // Verify user exists
    get_user_for_oauth(resources.repos.users.as_ref(), user_id).await?;
    let tenant_id = extract_tenant_id(auth_result.active_tenant_id.map(TenantId::from))?;

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

    let ctx = TenantContext {
        tenant_id,
        user_id,
        tenant_name,
        user_role: TenantRole::Member,
    };

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
pub(super) async fn handle_disconnect_provider_rest(
    State(resources): State<Arc<ServerResources>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate using middleware (supports both cookies and Authorization header)
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;
    info!("Disconnecting provider {} for user {}", provider, user_id);

    // Create OAuthService instance and call existing disconnect logic
    let server_context = ServerContext::from(resources.as_ref());
    let oauth_service = OAuthService::new(
        server_context.data().clone(),
        server_context.config().clone(),
        server_context.notification().clone(),
    );
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
pub(super) async fn handle_sync_provider(
    State(resources): State<Arc<ServerResources>>,
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

    info!(
        user_id = %user_id,
        provider = %provider,
        wait = %wait,
        "REST-triggered provider sync"
    );

    let refresh_service = crate::services::provider_refresh::RefreshService::new(
        resources.repos.clone(),
        #[cfg(feature = "health-sync")]
        resources.sync_orchestrator.clone(),
        resources.sse_manager.clone(),
    );

    let result = refresh_service
        .refresh_provider(user_id, TenantId::from(tenant_id), &provider, wait)
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
fn spawn_health_backfill(resources: &ServerResources, user_id: &str, provider: &str) {
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
