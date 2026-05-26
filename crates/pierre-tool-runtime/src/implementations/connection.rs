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

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_auth::oauth2_client::OAuthClientState;
use pierre_auth::tenant::{TenantContext, TenantRole};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::TenantId;
use serde_json::{json, Map, Value};
use tracing::{error, info, warn};

use crate::context::ToolExecutionContext;
use crate::traits::{McpTool, ToolCapabilities};
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
             4. Once connected, you can access your {} data through Pierre",
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
impl McpTool for ConnectProviderTool {
    fn name(&self) -> &'static str {
        "connect_provider"
    }

    fn description(&self) -> &'static str {
        "Initiate OAuth connection flow to connect a fitness data provider like Strava, Fitbit, or Garmin"
    }

    fn input_schema(&self) -> JsonSchema {
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

        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["provider".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::REQUIRES_TENANT
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(open_world_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
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

        // If the user has already opted into the sciotte mirror backend
        // for this provider, refuse to mint an OAuth URL.
        if let Some(mirror) = backend_resolver::mirror_backend_for(provider) {
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
        let tenant_name = repos
            .tenants
            .get_by_id(tenant_id)
            .await
            .map_or_else(|_| "Unknown Tenant".to_owned(), |t| t.name);
        let tctx = TenantContext {
            tenant_id,
            user_id: user_uuid,
            tenant_name,
            user_role: TenantRole::Member,
        };

        let state = build_oauth_state(user_uuid, redirect_url);

        match ctx
            .resources
            .tenant_oauth_client()
            .get_authorization_url(
                &tctx,
                provider,
                &state,
                ctx.resources.repos().tenants.as_ref(),
                ctx.resources.repos().oauth_tokens.as_ref(),
            )
            .await
        {
            Ok(url) => {
                let now = Utc::now();
                let base_url = env::var("BASE_URL").unwrap_or_else(|_| {
                    format!("http://localhost:{}", ctx.resources.config().http_port)
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
                    return Ok(oauth_error_result(
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
}

// ============================================================================
// GetConnectionStatusTool - Check OAuth connection status
// ============================================================================

/// Tool for checking the connection status of fitness providers.
///
/// Can check a single provider's status or all supported providers.
pub struct GetConnectionStatusTool;

#[async_trait]
impl McpTool for GetConnectionStatusTool {
    fn name(&self) -> &'static str {
        "get_connection_status"
    }

    fn description(&self) -> &'static str {
        "Check the connection status of fitness data providers. If no provider is specified, returns status for all supported providers."
    }

    fn input_schema(&self) -> JsonSchema {
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

        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let user_uuid = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

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

            let status = if is_connected {
                "connected"
            } else {
                "disconnected"
            };

            Ok(ToolResult::ok(json!({
                "provider": specific_provider,
                "status": status,
                "connected": is_connected,
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

            Ok(ToolResult::ok(json!({
                "providers": providers_status
            })))
        }
    }
}

// ============================================================================
// DisconnectProviderTool - Disconnect OAuth provider
// ============================================================================

/// Tool for disconnecting from a fitness provider by removing OAuth tokens.
pub struct DisconnectProviderTool;

#[async_trait]
impl McpTool for DisconnectProviderTool {
    fn name(&self) -> &'static str {
        "disconnect_provider"
    }

    fn description(&self) -> &'static str {
        "Disconnect from a fitness data provider by removing stored OAuth tokens"
    }

    fn input_schema(&self) -> JsonSchema {
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

        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["provider".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(destructive_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
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

        match ctx
            .resources
            .repos()
            .oauth_tokens
            .delete_token(user_uuid, tenant_id, provider)
            .await
        {
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
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all connection tools for registration
#[must_use]
pub fn create_connection_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(ConnectProviderTool),
        Box::new(GetConnectionStatusTool),
        Box::new(DisconnectProviderTool),
    ]
}
