// ABOUTME: Connection management handlers for OAuth providers
// ABOUTME: Handle connection status, disconnection, and connection initiation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::constants::oauth_config::AUTHORIZATION_EXPIRES_MINUTES;
use crate::models::TenantId;
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::providers::backend_resolver::{self, BackendKind};
use crate::utils::uuid::parse_user_id_for_protocol;
use chrono::{Duration, Utc};
use pierre_auth::oauth2_client::OAuthClientState;
use pierre_auth::tenant::{TenantContext, TenantRole};
use pierre_core::constants::oauth::providers as oauth_providers;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::pin::Pin;
use tracing::{error, info, warn};

/// User-facing provider names reported by `get_connection_status`.
///
/// Mirror backends (`sciotte`, `sciotte_garmin`) never appear here — they
/// are coalesced into the user-facing provider they serve (`strava`,
/// `garmin`). Any user-facing provider added to `ProviderRegistry` must
/// also be added here so the LLM sees it.
const USER_FACING_PROVIDERS: &[&str] = &[
    oauth_providers::STRAVA,
    oauth_providers::FITBIT,
    oauth_providers::GARMIN,
    oauth_providers::WHOOP,
    oauth_providers::TERRA,
    oauth_providers::COROS,
];

/// Handle `get_connection_status` tool - check OAuth connection status
#[must_use]
pub fn handle_get_connection_status(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        // Check cancellation at start
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "handle_get_connection_status cancelled by user".to_owned(),
                ));
            }
        }

        // Parse user ID from request
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Parse tenant_id once; None if the caller didn't supply one or it
        // isn't a valid UUID. A missing tenant means we cannot check any
        // token row, so we report everything as disconnected.
        let tenant_id_parsed: Option<TenantId> = request
            .tenant_id
            .as_deref()
            .and_then(|t| t.parse::<TenantId>().ok());

        // Check if a specific provider is requested
        if let Some(specific_provider) = request.parameters.get("provider").and_then(Value::as_str)
        {
            // Mirror backends are internal-only — refuse to expose them.
            // The LLM should always ask in terms of user-facing providers.
            if backend_resolver::is_mirror_backend(specific_provider) {
                return Ok(UniversalResponse {
                    success: false,
                    result: None,
                    error: Some(format!(
                        "Unknown provider '{specific_provider}'. Use 'strava' or 'garmin' instead.",
                    )),
                    metadata: None,
                });
            }

            let (is_connected, backend_kind) =
                match (tenant_id_parsed, user_facing_canonical(specific_provider)) {
                    (Some(tid), Some(canonical)) => {
                        let status = backend_resolver::coalesced_status(
                            &executor.resources.repos,
                            user_uuid,
                            tid,
                            canonical,
                        )
                        .await;
                        (status.connected, status.backend_kind)
                    }
                    _ => (false, BackendKind::None),
                };

            let status = if is_connected {
                "connected"
            } else {
                "disconnected"
            };

            Ok(UniversalResponse {
                success: true,
                result: Some(json!({
                    "provider": specific_provider,
                    "status": status,
                    "connected": is_connected,
                    "backend": backend_kind.as_str()
                })),
                error: None,
                metadata: Some({
                    let mut map = HashMap::new();
                    map.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
                    map.insert(
                        "provider".to_owned(),
                        Value::String(specific_provider.to_owned()),
                    );
                    map.insert(
                        "tenant_id".to_owned(),
                        request.tenant_id.map_or(Value::Null, Value::String),
                    );
                    map
                }),
            })
        } else {
            // Multi-provider mode - report only user-facing providers.
            // sciotte/sciotte_garmin are coalesced into strava/garmin so the
            // LLM cannot see the internal backend name and cannot offer it
            // to end users as a separate option.
            let mut providers_status = Map::new();

            for user_facing in USER_FACING_PROVIDERS {
                let status = match tenant_id_parsed {
                    Some(tid) => {
                        backend_resolver::coalesced_status(
                            &executor.resources.repos,
                            user_uuid,
                            tid,
                            user_facing,
                        )
                        .await
                    }
                    None => backend_resolver::CoalescedStatus {
                        user_facing,
                        connected: false,
                        backend_kind: BackendKind::None,
                    },
                };

                let status_str = if status.connected {
                    "connected"
                } else {
                    "disconnected"
                };

                providers_status.insert(
                    (*user_facing).to_owned(),
                    json!({
                        "connected": status.connected,
                        "status": status_str,
                        "backend": status.backend_kind.as_str()
                    }),
                );
            }

            Ok(UniversalResponse {
                success: true,
                result: Some(json!({
                    "providers": providers_status
                })),
                error: None,
                metadata: Some({
                    let mut map = HashMap::new();
                    map.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
                    map.insert(
                        "tenant_id".to_owned(),
                        request.tenant_id.map_or(Value::Null, Value::String),
                    );
                    map
                }),
            })
        }
    })
}

/// Canonicalise a provider name into its static user-facing entry from
/// `USER_FACING_PROVIDERS`, or `None` if the name is not a recognised
/// user-facing provider.
///
/// `coalesced_status` requires a `&'static str` so the returned struct
/// outlives the request; matching against the static list gives us one.
/// Unknown names return `None` so callers can report them as disconnected
/// instead of silently aliasing into a different provider's status.
fn user_facing_canonical(name: &str) -> Option<&'static str> {
    USER_FACING_PROVIDERS.iter().copied().find(|p| *p == name)
}

/// Handle `disconnect_provider` tool - disconnect user from OAuth provider
#[must_use]
pub fn handle_disconnect_provider(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        // Check cancellation at start
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "handle_disconnect_provider cancelled by user".to_owned(),
                ));
            }
        }

        // Parse user ID from request
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

        // Extract provider from parameters (required)
        let Some(provider) = request.parameters.get("provider").and_then(Value::as_str) else {
            let supported = executor
                .resources
                .provider_registry
                .supported_providers()
                .join(", ");
            return Ok(connection_error(format!(
                "Missing required 'provider' parameter. Supported providers: {supported}"
            )));
        };

        // Resolve tenant ID: prefer request.tenant_id (user's selected tenant from JWT),
        // falling back to user's first tenant for clients without active_tenant_id.
        let tenant_id: TenantId = if let Some(tid) = request.tenant_id.as_deref() {
            tid.parse::<TenantId>()
                .map_err(|_| ProtocolError::InvalidRequest(format!("Invalid tenant_id: {tid}")))?
        } else {
            // No active tenant in request - fall back to user's first tenant
            let tenants = executor
                .resources
                .repos
                .tenants
                .list_for_user(user_uuid)
                .await
                .unwrap_or_default();
            match tenants.first() {
                Some(t) => t.id,
                None => {
                    return Ok(connection_error(
                        "Cannot disconnect: user does not belong to any tenant",
                    ));
                }
            }
        };

        // Disconnect by deleting the token
        match executor
            .resources
            .repos
            .oauth_tokens
            .delete_token(user_uuid, tenant_id, provider)
            .await
        {
            Ok(()) => Ok(UniversalResponse {
                success: true,
                result: Some(json!({
                    "provider": provider,
                    "status": "disconnected",
                    "message": format!("Successfully disconnected from {provider}")
                })),
                error: None,
                metadata: Some({
                    let mut map = HashMap::new();
                    map.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
                    map.insert("provider".to_owned(), Value::String(provider.to_owned()));
                    map.insert(
                        "tenant_id".to_owned(),
                        request.tenant_id.map_or(Value::Null, Value::String),
                    );
                    map
                }),
            }),
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to disconnect from {provider}: {e}")),
                metadata: Some({
                    let mut map = HashMap::new();
                    map.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
                    map.insert("provider".to_owned(), Value::String(provider.to_owned()));
                    map.insert(
                        "tenant_id".to_owned(),
                        request.tenant_id.map_or(Value::Null, Value::String),
                    );
                    map
                }),
            }),
        }
    })
}

/// Build successful OAuth connection response
fn build_oauth_success_response(
    user_uuid: uuid::Uuid,
    tenant_id: uuid::Uuid,
    provider: &str,
    authorization_url: &str,
    state: &str,
) -> UniversalResponse {
    UniversalResponse {
        success: true,
        result: Some(json!({
            "provider": provider,
            "authorization_url": authorization_url,
            "state": state,
            "instructions": format!(
                "To connect your {} account:\n\
                 1. Visit the authorization URL\n\
                 2. Log in to {} and approve the connection\n\
                 3. You will be redirected back to complete the connection\n\
                 4. Once connected, you can access your {} data through Pierre",
                provider, provider, provider
            ),
            "expires_in_minutes": AUTHORIZATION_EXPIRES_MINUTES,
            "status": "pending_authorization"
        })),
        error: None,
        metadata: Some({
            let mut map = HashMap::new();
            map.insert("user_id".to_owned(), Value::String(user_uuid.to_string()));
            map.insert("tenant_id".to_owned(), Value::String(tenant_id.to_string()));
            map.insert("provider".to_owned(), Value::String(provider.to_owned()));
            map
        }),
    }
}

/// Build OAuth error response
fn build_oauth_error_response(provider: &str, error: &str) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: None,
        error: Some(format!(
            "Failed to generate authorization URL: {error}. \
             Please check that OAuth credentials are configured for provider '{provider}'."
        )),
        metadata: Some({
            let mut map = HashMap::new();
            map.insert(
                "error_type".to_owned(),
                Value::String("oauth_configuration_error".to_owned()),
            );
            map.insert("provider".to_owned(), Value::String(provider.to_owned()));
            map
        }),
    }
}

/// Create error response for connection operations
#[inline]
fn connection_error(message: impl Into<String>) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: None,
        error: Some(message.into()),
        metadata: None,
    }
}

/// Validate redirect URL scheme for OAuth mobile flows
fn validate_redirect_url_scheme(url: &str) -> bool {
    url.starts_with("pierre://")
        || url.starts_with("exp://")
        || url.starts_with("http://localhost")
        || url.starts_with("https://")
}

/// Build OAuth state string with optional redirect URL
fn build_oauth_state(user_uuid: uuid::Uuid, redirect_url: Option<&str>) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    redirect_url.map_or_else(
        || format!("{}:{}", user_uuid, uuid::Uuid::new_v4()),
        |url| {
            let encoded_url = URL_SAFE_NO_PAD.encode(url.as_bytes());
            format!("{}:{}:{}", user_uuid, uuid::Uuid::new_v4(), encoded_url)
        },
    )
}

/// Handle `connect_provider` tool - initiate OAuth connection flow
///
/// Accepts optional `redirect_url` parameter for mobile app OAuth flows.
/// When provided, the redirect URL is base64 encoded and included in the OAuth state,
/// allowing the server to redirect back to the mobile app after OAuth completes.
#[must_use]
pub fn handle_connect_provider(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        if let Some(token) = &request.cancellation_token {
            if token.is_cancelled().await {
                return Err(ProtocolError::OperationCancelled(
                    "handle_connect_provider cancelled by user".to_owned(),
                ));
            }
        }
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let registry = &executor.resources.provider_registry;
        let repos = &executor.resources.repos;

        // Extract and validate provider parameter
        let Some(provider) = request.parameters.get("provider").and_then(|v| v.as_str()) else {
            let supported = USER_FACING_PROVIDERS.join(", ");
            return Ok(connection_error(format!(
                "Missing required 'provider' parameter. Supported providers: {supported}"
            )));
        };
        // Mirror backends are internal — the LLM should never be able to
        // initiate an OAuth-style "connect" flow against them.
        if backend_resolver::is_mirror_backend(provider) {
            return Ok(connection_error(format!(
                "Unknown provider '{provider}'. Use 'strava' or 'garmin' instead.",
            )));
        }
        if !registry.is_supported(provider) {
            let supported = USER_FACING_PROVIDERS.join(", ");
            return Ok(connection_error(format!(
                "Provider '{provider}' is not supported. Supported providers: {supported}"
            )));
        }

        // If the user has already opted into the sciotte mirror backend
        // for this provider, refuse to mint an OAuth URL. Re-login must
        // happen through the same sciotte flow — switching to OAuth would
        // silently strand the existing mirror session and contradict the
        // user's stated preference.
        if let Some(mirror) = backend_resolver::mirror_backend_for(provider) {
            if let Some(tenant_str) = request.tenant_id.as_deref() {
                if let Ok(tid) = tenant_str.parse::<TenantId>() {
                    if let Ok(Some(_)) = repos.oauth_tokens.get_token(user_uuid, tid, mirror).await
                    {
                        info!(
                            user_id = %user_uuid,
                            provider = provider,
                            mirror = mirror,
                            "Refusing OAuth connect: user has mirror backend active, \
                             they must re-authenticate through the mirror flow"
                        );
                        return Ok(UniversalResponse {
                            success: false,
                            result: Some(json!({
                                "provider": provider,
                                "backend": "mirror",
                                "requires_mirror_reauth": true,
                                "message": format!(
                                    "Your {provider} connection uses a direct login (email + password), \
                                     not OAuth. If it has stopped working, re-authenticate through the \
                                     same flow — do not propose a fresh OAuth connection."
                                )
                            })),
                            error: Some(format!(
                                "{provider} is already connected via direct login; \
                                 OAuth reconnection is blocked."
                            )),
                            metadata: Some({
                                let mut map = HashMap::new();
                                map.insert(
                                    "user_id".to_owned(),
                                    Value::String(user_uuid.to_string()),
                                );
                                map.insert(
                                    "provider".to_owned(),
                                    Value::String(provider.to_owned()),
                                );
                                map.insert(
                                    "backend".to_owned(),
                                    Value::String("mirror".to_owned()),
                                );
                                map
                            }),
                        });
                    }
                }
            }
        }

        // Extract and validate optional redirect_url for mobile OAuth flows
        let redirect_url = request
            .parameters
            .get("redirect_url")
            .and_then(Value::as_str);
        if let Some(url) = redirect_url {
            if !validate_redirect_url_scheme(url) {
                return Ok(connection_error(
                    "Invalid redirect_url scheme. Allowed: pierre://, exp://, http://localhost, https://",
                ));
            }
        }

        // SECURITY: Global lookup — connection handler, tenant resolved from user's membership
        match repos.users.get_global(user_uuid).await {
            Ok(Some(_)) => {}
            Ok(None) => return Ok(connection_error(format!("User {user_uuid} not found"))),
            Err(e) => return Ok(connection_error(format!("Database error: {e}"))),
        }

        // Get tenant context: prefer request.tenant_id (user's selected tenant from JWT),
        // falling back to user's first tenant for clients without active_tenant_id.
        // Security: always verify membership before using request.tenant_id,
        // as it would allow a caller to use another tenant's OAuth credentials/rate limits.
        let tenants = repos
            .tenants
            .list_for_user(user_uuid)
            .await
            .unwrap_or_default();
        let tenant_id: TenantId = request
            .tenant_id
            .as_ref()
            .and_then(|t| t.parse::<TenantId>().ok())
            .map_or_else(
                // No tenant_id in request; use first membership or user_uuid
                || {
                    tenants
                        .first()
                        .map_or_else(|| TenantId::from(user_uuid), |t| t.id)
                },
                |requested_tid| {
                    // Verify the user is a member of the requested tenant
                    if tenants.iter().any(|t| t.id == requested_tid) {
                        requested_tid
                    } else {
                        // User is not a member of the requested tenant
                        tenants
                            .first()
                            .map_or_else(|| TenantId::from(user_uuid), |t| t.id)
                    }
                },
            );
        let tenant_name = repos
            .tenants
            .get_by_id(tenant_id)
            .await
            .map_or_else(|_| "Unknown Tenant".to_owned(), |t| t.name);
        let ctx = TenantContext {
            tenant_id,
            user_id: user_uuid,
            tenant_name,
            user_role: TenantRole::Member,
        };

        let state = build_oauth_state(user_uuid, redirect_url);

        match executor
            .resources
            .tenant_oauth_client
            .get_authorization_url(
                &ctx,
                provider,
                &state,
                executor.resources.repos.tenants.as_ref(),
                executor.resources.repos.oauth_tokens.as_ref(),
            )
            .await
        {
            Ok(url) => {
                // Store state server-side for CSRF protection with 10-minute TTL
                let now = Utc::now();
                let base_url = env::var("BASE_URL").unwrap_or_else(|_| {
                    format!("http://localhost:{}", executor.resources.config.http_port)
                });
                let oauth_callback_uri = format!("{base_url}/api/oauth/callback/{provider}");
                let client_state = OAuthClientState {
                    state: state.clone(),
                    provider: provider.to_owned(),
                    user_id: Some(user_uuid),
                    tenant_id: Some(tenant_id.to_string()),
                    redirect_uri: oauth_callback_uri,
                    scope: None,
                    pkce_code_verifier: None,
                    created_at: now,
                    expires_at: now + Duration::minutes(i64::from(AUTHORIZATION_EXPIRES_MINUTES)),
                    used: false,
                };

                if let Err(e) = repos
                    .oauth_client_state
                    .store_oauth_client_state(&client_state)
                    .await
                {
                    warn!("Failed to store OAuth state for CSRF protection: {}", e);
                    return Ok(build_oauth_error_response(
                        provider,
                        &format!("Failed to initiate OAuth flow: {e}"),
                    ));
                }

                let flow_type = if redirect_url.is_some() {
                    " (mobile flow)"
                } else {
                    ""
                };
                info!(
                    "Generated OAuth URL for user {} provider {}{}",
                    user_uuid, provider, flow_type
                );
                Ok(build_oauth_success_response(
                    user_uuid,
                    tenant_id.as_uuid(),
                    provider,
                    &url,
                    &state,
                ))
            }
            Err(e) => {
                error!("OAuth URL generation failed for {}: {}", provider, e);
                Ok(build_oauth_error_response(provider, &e.to_string()))
            }
        }
    })
}
