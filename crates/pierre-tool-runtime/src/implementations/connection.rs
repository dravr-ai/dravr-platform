// ABOUTME: Connection management tools implementing the McpTool trait.
// ABOUTME: Provides connect_provider, get_connection_status, disconnect_provider tools.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Connection Management Tools
//!
//! This module contains tools for managing provider connections:
//! - `ConnectProviderTool` - Initiate OAuth flow for a provider
//! - `GetConnectionStatusTool` - Check provider connection status
//! - `DisconnectProviderTool` - Disconnect and revoke OAuth tokens

use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_auth::oauth2_client::OAuthClientState;
use pierre_auth::tenant::TenantContext;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionStatus, TenantId};
use serde_json::{json, Map, Value};
use tracing::{error, info};

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_config::constants::oauth_config::AUTHORIZATION_EXPIRES_MINUTES;
use pierre_core::errors::AppResult;
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_providers::backend_resolver::{self, BackendKind};
use pierre_tools_core::ToolResult;

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

/// Canonicalise a provider name into its static user-facing entry from
/// `USER_FACING_PROVIDERS`, or `None` if the name is not a recognised
/// user-facing provider.
fn user_facing_canonical(name: &str) -> Option<&'static str> {
    USER_FACING_PROVIDERS.iter().copied().find(|p| *p == name)
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

/// Build successful OAuth connection payload
fn build_oauth_success_payload(provider: &str, authorization_url: &str, state: &str) -> Value {
    json!({
        "provider": provider,
        "authorization_url": authorization_url,
        "state": state,
        "instructions": format!(
            "To connect your {} account:\n\
             1. Visit the authorization URL\n\
             2. Log in to {} and approve the connection\n\
             3. You will be redirected back to complete the connection\n\
             4. Once connected, you can access your {} data through Dravr",
            provider, provider, provider
        ),
        "expires_in_minutes": AUTHORIZATION_EXPIRES_MINUTES,
        "status": "pending_authorization"
    })
}

/// Build OAuth error payload merged into a `ToolResult` error.
fn oauth_error_result(provider: &str, error: &str) -> ToolResult {
    ToolResult::error(json!({
        "error": format!(
            "Failed to generate authorization URL: {error}. \
             Please check that OAuth credentials are configured for provider '{provider}'."
        ),
        "error_type": "oauth_configuration_error",
        "provider": provider,
    }))
}

/// Mint a provider OAuth authorization URL and persist its CSRF state row.
///
/// Shared by `connect_provider` (interactive connect) and the chat auth-recovery stage
/// (reconnect a dead OAuth provider). Builds the opaque `state`, asks the tenant OAuth
/// client for the provider's authorization URL, and stores an [`OAuthClientState`] row so
/// the callback can validate the round-trip. Returns `(authorization_url, state)`.
///
/// # Errors
///
/// Returns an error if the tenant has no OAuth client for the provider, the authorization
/// URL cannot be generated, or the CSRF state row cannot be persisted.
pub async fn mint_oauth_authorize_url(
    resources: &dyn ToolRuntime,
    user_id: uuid::Uuid,
    tenant_id: TenantId,
    provider: &str,
    redirect_url: Option<&str>,
) -> AppResult<(String, String)> {
    let tenant_name = resources
        .repos()
        .tenants
        .get_by_id(tenant_id)
        .await
        .map_or_else(|_| "Unknown Tenant".to_owned(), |t| t.name);
    // Minting an authorize URL for a caller that already holds both ids; no
    // membership lookup happened, so no role is asserted.
    let tenant_context =
        TenantContext::for_tenant_scoped_operation(tenant_id, tenant_name, user_id);

    let state = build_oauth_state(user_id, redirect_url);

    let url = resources
        .tenant_oauth_client()
        .get_authorization_url(
            &tenant_context,
            provider,
            &state,
            resources.repos().tenants.as_ref(),
            resources.repos().oauth_tokens.as_ref(),
        )
        .await?;

    let now = Utc::now();
    let base_url = env::var("BASE_URL")
        .unwrap_or_else(|_| format!("http://localhost:{}", resources.config().http_port));
    let oauth_callback_uri = format!("{base_url}/api/oauth/callback/{provider}");
    let client_state = OAuthClientState {
        state: state.clone(),
        provider: provider.to_owned(),
        user_id: Some(user_id),
        tenant_id: Some(tenant_id.to_string()),
        redirect_uri: oauth_callback_uri,
        scope: None,
        pkce_code_verifier: None,
        oauth_app_client_id: None,
        created_at: now,
        expires_at: now + Duration::minutes(i64::from(AUTHORIZATION_EXPIRES_MINUTES)),
        used: false,
    };
    resources
        .repos()
        .oauth_client_state
        .store_oauth_client_state(&client_state)
        .await?;

    Ok((url, state))
}

/// Annotations for tools that interact with external OAuth services
fn open_world_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        open_world_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for read-only connection status checks
fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for destructive operations like disconnect
fn destructive_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(true),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

// ============================================================================
// ConnectProviderTool - Initiate OAuth connection flow
// ============================================================================

/// Tool for initiating OAuth connection flow with a fitness provider.
///
/// Generates an authorization URL that the user can visit to authenticate
/// with the provider. Supports optional redirect URL for mobile app flows.
pub struct ConnectProviderTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ConnectProviderTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Provider to connect (e.g., 'strava', 'fitbit', 'garmin')".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "redirect_url".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional redirect URL for mobile app OAuth flows (supports pierre://, exp://, http://localhost, https://)".to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["provider".to_owned()]),
        };

        tool_definition(
            "connect_provider",
            "Initiate OAuth connection flow to connect a fitness data provider like Strava, Fitbit, or Garmin",
            schema,
            Some(open_world_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::REQUIRES_TENANT)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
        let user_uuid = ctx.user_id;
        let registry = &ctx.resources.provider_registry();
        let repos = &ctx.resources.repos();

        let Some(provider) = args.get("provider").and_then(Value::as_str) else {
            let supported = USER_FACING_PROVIDERS.join(", ");
            return Ok(ToolResult::error(json!({
                "error": format!(
                    "Missing required 'provider' parameter. Supported providers: {supported}"
                )
            })));
        };

        if backend_resolver::is_mirror_backend(provider) {
            return Ok(ToolResult::error(json!({
                "error": format!(
                    "Unknown provider '{provider}'. Use 'strava' or 'garmin' instead.",
                )
            })));
        }
        if !registry.is_supported(provider) {
            let supported = USER_FACING_PROVIDERS.join(", ");
            return Ok(ToolResult::error(json!({
                "error": format!(
                    "Provider '{provider}' is not supported. Supported providers: {supported}"
                )
            })));
        }

        let tenant_id = TenantId::from(ctx.require_tenant()?);

        // If the user has already opted into the sciotte mirror backend for
        // this provider, refuse to mint an OAuth URL — they must re-authenticate
        // through the mirror flow.
        //
        // Strava is exempt: it is migrating to its OAuth API, so a sciotte-Strava
        // user is allowed to authorize OAuth (after which resolve_backend routes
        // them to the API and the mirror goes dormant). Garmin keeps the block —
        // its official API is partner-gated, so the mirror is its only backend.
        let mirror = if provider == oauth_providers::STRAVA {
            None
        } else {
            backend_resolver::mirror_backend_for(provider)
        };
        if let Some(mirror) = mirror {
            if let Ok(Some(_)) = repos
                .oauth_tokens
                .get_token(user_uuid, tenant_id, mirror)
                .await
            {
                info!(
                    user_id = %user_uuid,
                    provider = provider,
                    mirror = mirror,
                    "Refusing OAuth connect: user has mirror backend active, \
                     they must re-authenticate through the mirror flow"
                );
                return Ok(ToolResult::error(json!({
                    "provider": provider,
                    "backend": "mirror",
                    "requires_mirror_reauth": true,
                    "message": format!(
                        "Your {provider} connection uses a direct login (email + password), \
                         not OAuth. If it has stopped working, re-authenticate through the \
                         same flow — do not propose a fresh OAuth connection."
                    ),
                    "error": format!(
                        "{provider} is already connected via direct login; \
                         OAuth reconnection is blocked."
                    ),
                })));
            }
        }

        let redirect_url = args.get("redirect_url").and_then(Value::as_str);
        if let Some(url) = redirect_url {
            if !validate_redirect_url_scheme(url) {
                return Ok(ToolResult::error(json!({
                    "error": "Invalid redirect_url scheme. Allowed: pierre://, exp://, http://localhost, https://"
                })));
            }
        }

        // SECURITY: Global lookup — connection handler, tenant resolved from user's membership
        match repos.users.get_global(user_uuid).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Ok(ToolResult::error(json!({
                    "error": format!("User {user_uuid} not found")
                })))
            }
            Err(e) => {
                return Ok(ToolResult::error(json!({
                    "error": format!("Database error: {e}")
                })))
            }
        }

        // Security: always verify membership before using request.tenant_id.
        let tenants = repos
            .tenants
            .list_for_user(user_uuid)
            .await
            .unwrap_or_default();
        if !tenants.iter().any(|t| t.id == tenant_id) {
            return Ok(ToolResult::error(json!({
                "error": format!(
                    "User {user_uuid} is not a member of tenant {tenant_id}"
                )
            })));
        }
        match mint_oauth_authorize_url(
            ctx.resources.as_ref(),
            user_uuid,
            tenant_id,
            provider,
            redirect_url,
        )
        .await
        {
            Ok((url, state)) => {
                let flow_type = if redirect_url.is_some() {
                    " (mobile flow)"
                } else {
                    ""
                };
                info!(
                    "Generated OAuth URL for user {} provider {}{}",
                    user_uuid, provider, flow_type
                );
                Ok(ToolResult::ok(build_oauth_success_payload(
                    provider, &url, &state,
                )))
            }
            Err(e) => {
                error!("OAuth URL generation failed for {}: {}", provider, e);
                Ok(oauth_error_result(provider, &e.to_string()))
            }
        }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetConnectionStatusTool - Check OAuth connection status
// ============================================================================

/// Tool for checking the connection status of fitness providers.
///
/// Can check a single provider's status or all supported providers.
pub struct GetConnectionStatusTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetConnectionStatusTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional: specific provider to check (e.g., 'strava'). If omitted, checks all providers.".to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };

        tool_definition(
            "get_connection_status",
            "Check the connection status of fitness data providers. If no provider is specified, returns status for all supported providers.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_uuid = ctx.user_id;
            let tenant_id = TenantId::from(ctx.require_tenant()?);

            // Lifecycle status per connected provider (best-effort). Lets us report a
            // connected-but-dead provider as `needs_reauth` — a token row still exists, but a
            // non-recoverable refresh failure means the user must reconnect before data flows.
            let status_by_provider: HashMap<String, ConnectionStatus> = ctx
                .resources
                .repos()
                .provider_connections
                .get_for_user(user_uuid, Some(tenant_id))
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|c| (c.provider, c.status))
                .collect();

            if let Some(specific_provider) = args.get("provider").and_then(Value::as_str) {
                // Mirror backends are internal-only.
                if backend_resolver::is_mirror_backend(specific_provider) {
                    return Ok(ToolResult::ok(json!({
                        "provider": specific_provider,
                        "status": "disconnected",
                        "connected": false,
                        "backend": "none",
                        "note": "Unknown provider. Use 'strava' or 'garmin' instead."
                    })));
                }

                let auth_repos = ctx.resources.repos().auth_repos();
                let (is_connected, backend_kind) = match user_facing_canonical(specific_provider) {
                    Some(canonical) => {
                        let status = backend_resolver::coalesced_status(
                            &auth_repos,
                            user_uuid,
                            tenant_id,
                            canonical,
                        )
                        .await;
                        (status.connected, status.backend_kind)
                    }
                    None => (false, BackendKind::None),
                };

                let needs_reauth = status_by_provider
                    .get(specific_provider)
                    .copied()
                    .is_some_and(|s| s.requires_reauth());
                let status = if needs_reauth {
                    "needs_reauth"
                } else if is_connected {
                    "connected"
                } else {
                    "disconnected"
                };

                Ok(ToolResult::ok(json!({
                    "provider": specific_provider,
                    "status": status,
                    "connected": is_connected,
                    "needs_reauth": needs_reauth,
                    "backend": backend_kind.as_str()
                })))
            } else {
                let mut providers_status = Map::new();

                let auth_repos = ctx.resources.repos().auth_repos();
                for user_facing in USER_FACING_PROVIDERS {
                    let status = backend_resolver::coalesced_status(
                        &auth_repos,
                        user_uuid,
                        tenant_id,
                        user_facing,
                    )
                    .await;

                    let needs_reauth = status_by_provider
                        .get(*user_facing)
                        .copied()
                        .is_some_and(|s| s.requires_reauth());
                    let status_str = if needs_reauth {
                        "needs_reauth"
                    } else if status.connected {
                        "connected"
                    } else {
                        "disconnected"
                    };

                    providers_status.insert(
                        (*user_facing).to_owned(),
                        json!({
                            "connected": status.connected,
                            "status": status_str,
                            "needs_reauth": needs_reauth,
                            "backend": status.backend_kind.as_str()
                        }),
                    );
                }

                Ok(ToolResult::ok(json!({
                    "providers": providers_status
                })))
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// DisconnectProviderTool - Disconnect OAuth provider
// ============================================================================

/// Tool for disconnecting from a fitness provider by removing OAuth tokens.
pub struct DisconnectProviderTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for DisconnectProviderTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Provider to disconnect (e.g., 'strava', 'fitbit', 'garmin')".to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["provider".to_owned()]),
        };

        tool_definition(
            "disconnect_provider",
            "Disconnect from a fitness data provider by removing stored OAuth tokens",
            schema,
            Some(destructive_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_uuid = ctx.user_id;

            let Some(provider) = args.get("provider").and_then(Value::as_str) else {
                let supported = ctx
                    .resources
                    .provider_registry()
                    .supported_providers()
                    .join(", ");
                return Ok(ToolResult::error(json!({
                    "error": format!(
                        "Missing required 'provider' parameter. Supported providers: {supported}"
                    )
                })));
            };

            let tenant_id = TenantId::from(ctx.require_tenant()?);

            // Resolve the user-facing name ("garmin") to the backend that actually
            // holds the session ("sciotte_garmin"). Deleting the raw name would
            // miss the mirror row entirely — "disconnect garmin" would delete a
            // non-existent `garmin` token and leave the live sciotte_garmin
            // session + connection untouched.
            let backend = backend_resolver::resolve_backend(
                &ctx.resources.repos().auth_repos(),
                user_uuid,
                Some(tenant_id),
                provider,
            )
            .await;

            // Delete the token and the connection row in lockstep. They are
            // separate sources of truth (oauth_tokens → resolve_backend + scrape
            // session; provider_connections → the "connected" badge + coaching
            // fetch enumeration); an orphaned connection row shows "Connected" for
            // a dead session and routes later fetches to a backend with no token.
            let token_res = ctx
                .resources
                .repos()
                .oauth_tokens
                .delete_token(user_uuid, tenant_id, &backend)
                .await;
            let conn_res = ctx
                .resources
                .repos()
                .provider_connections
                .remove_connection(user_uuid, tenant_id, &backend)
                .await;

            match token_res.and(conn_res) {
                // Report the user-facing name — the mirror backend is internal.
                Ok(()) => Ok(ToolResult::ok(json!({
                    "provider": provider,
                    "status": "disconnected",
                    "message": format!("Successfully disconnected from {provider}")
                }))),
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to disconnect from {provider}: {e}")
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all connection tools for registration
#[must_use]
pub fn create_connection_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(ConnectProviderTool),
        Box::new(GetConnectionStatusTool),
        Box::new(DisconnectProviderTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(DisconnectProviderTool => IRREVERSIBLE);
crate::declare_security!(ConnectProviderTool => empty);
crate::declare_security!(GetConnectionStatusTool => empty);
