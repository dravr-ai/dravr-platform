// ABOUTME: MCP server implementation with tenant isolation and user authentication
// ABOUTME: Handles MCP protocol with per-tenant data isolation and access control
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # MCP Server
//!
//! NOTE: All remaining undocumented `.clone()` calls in this file are Safe - they are
//! necessary for Arc resource sharing in HTTP route handlers and async closures required
//! by the Axum framework for multi-tenant MCP protocol handling.
//! This module provides an MCP server that supports user authentication,
//! secure token storage, and user-scoped data access.

use super::{
    resources::ServerContext,
    tool_handlers::{McpOAuthCredentials, ToolRoutingContext},
};
#[cfg(feature = "provider-strava")]
use crate::constants::oauth::STRAVA_DEFAULT_SCOPES;
use crate::constants::{
    errors::{ERROR_INTERNAL_ERROR, ERROR_INVALID_PARAMS, ERROR_METHOD_NOT_FOUND},
    get_server_config,
    protocol::JSONRPC_VERSION,
};
use chrono::Utc;
use pierre_auth::api_keys::ApiKeyUsage;
use pierre_auth::auth::AuthManager;
use pierre_auth::security::headers::SecurityConfig;
use pierre_auth::tenant::oauth_client::StoreCredentialsRequest;
use pierre_auth::tenant::oauth_manager::TenantOAuthManager;
use pierre_auth::tenant::{TenantContext, TenantOAuthClient};
use pierre_config::environment::ServerConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{OAuthNotification, TenantOAuthCredentials};
use pierre_database::backends::factory::Database;
use pierre_mcp_schema::json_schemas;
use pierre_mcp_schema::{McpError, McpResponse, ProgressNotification};
use pierre_services::oauth_flow::OAuthService;
use pierre_tool_runtime::protocol::types::{CancellationToken, ProgressReporter};
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::protocols::converter::ProtocolConverter;
// Trait methods dispatched through repos.notifications / repos.oauth_tokens
use serde_json::Value;
use std::fmt::Write;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tower_http::LatencyUnit;
use tracing::{debug, error, info, warn, Level};
use uuid::Uuid;

use crate::constants::service_names::PIERRE_MCP_SERVER;
use crate::routes::contremaitre_webhook::routes as contremaitre_webhook_routes;
use crate::routes::oauth_grants::OAuthGrantsRoutes;
#[cfg(feature = "client-settings")]
use crate::routes::{endurance, user_profile::routes as user_profile_routes};
use crate::routes::{onboarding::OnboardingRoutes, viz::VizRoutes};
#[cfg(feature = "client-messaging")]
use crate::services::user_approval_notifier::ApprovalNotifier;
use axum::body::Body;
use axum::middleware;
use axum::response::Response;
#[cfg(feature = "oauth")]
use pierre_auth::oauth2_server::OAuth2RateLimiter;
use pierre_database::backends::UsageRepository;
use pierre_database::{AuthRepos, RepositoryRegistry};
use pierre_llm::health::{LlmHealthSnapshot, LlmHealthState, LlmHealthStatus};
#[cfg(feature = "telemetry")]
use pierre_middleware::telemetry_middleware;
use pierre_middleware::{request_id_middleware, response_failure_log_middleware, setup_cors};
#[cfg(feature = "client-admin-api")]
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
#[cfg(feature = "oauth")]
use pierre_routes_identity::oauth2::OAuth2Context;
use std::any::Any;
use tokio::net::TcpListener;
use tower::layer::util::Identity;

// Constants are now imported from the constants module

/// Connection status for providers
struct ProviderConnectionStatus {
    strava_connected: bool,
    fitbit_connected: bool,
}

/// Helper struct for OAuth provider credential parameters
struct OAuthProviderParams<'a> {
    provider: &'a str,
    client_id: &'a str,
    client_secret: &'a str,
    configured_redirect_uri: Option<&'a String>,
    scopes: &'a [String],
    base_url: &'a str,
}

/// MCP server supporting user authentication and isolated data access
#[derive(Clone)]
pub struct ProviderToolRouter {
    resources: Arc<ServerContext>,
}

impl ProviderToolRouter {
    /// Create a new MCP server with pre-built resources (dependency injection)
    #[must_use]
    pub const fn new(resources: Arc<ServerContext>) -> Self {
        Self { resources }
    }

    /// Get shared reference to server resources
    #[must_use]
    pub fn resources(&self) -> Arc<ServerContext> {
        self.resources.clone()
    }

    /// Initialize security configuration based on environment
    fn setup_security_config(config: &ServerConfig) -> SecurityConfig {
        let security_config =
            SecurityConfig::from_environment(&config.security.headers.environment.to_string());
        info!(
            "Security headers enabled with {} configuration",
            config.security.headers.environment
        );
        security_config
    }

    /// Route disconnect tool request to appropriate provider handler
    ///
    /// # Errors
    /// Returns an error if the provider is not supported or the operation fails
    #[tracing::instrument(
        skip(ctx, request_id),
        fields(
            provider = %provider_name,
            user_id = %ctx.tenant_context.user_id,
            tenant_id = %ctx.tenant_context.tenant_id,
        )
    )]
    pub async fn route_disconnect_tool(
        provider_name: &str,
        request_id: Value,
        ctx: &ToolRoutingContext<'_>,
    ) -> McpResponse {
        // Tenant context is always available since tool execution requires it
        Self::handle_tenant_disconnect_provider(
            ctx.tenant_context,
            provider_name,
            ctx.resources,
            request_id,
        )
        .await
    }

    /// Route provider-specific tool requests to appropriate handlers
    ///
    /// Tenant context is always available since tool execution requires it.
    #[tracing::instrument(
        skip(args, request_id, ctx),
        fields(
            tool_name = %tool_name,
            user_id = %ctx.tenant_context.user_id,
            tenant_id = %ctx.tenant_context.tenant_id,
        )
    )]
    pub async fn route_provider_tool(
        tool_name: &str,
        args: &Value,
        request_id: Value,
        user_id: Uuid,
        ctx: &ToolRoutingContext<'_>,
    ) -> McpResponse {
        // Tenant context is always available since tool execution requires it
        Self::handle_tenant_tool_with_provider(
            tool_name,
            args,
            request_id,
            ctx.tenant_context,
            ctx.resources,
            user_id,
        )
        .await
    }

    /// Record API key usage for billing and analytics
    ///
    /// # Errors
    ///
    /// Returns an error if the usage cannot be recorded in the database
    pub async fn record_api_key_usage(
        database: &dyn UsageRepository,
        api_key_id: &str,
        tool_name: &str,
        response_time: Duration,
        response: &McpResponse,
    ) -> AppResult<()> {
        let status_code = if response.error.is_some() {
            400 // Error responses
        } else {
            200 // Success responses
        };

        let error_message = response.error.as_ref().map(|e| e.message.clone());

        let usage = ApiKeyUsage {
            id: None,
            api_key_id: api_key_id.to_owned(),
            timestamp: Utc::now(),
            tool_name: tool_name.to_owned(),
            response_time_ms: u32::try_from(response_time.as_millis()).ok(),
            status_code,
            error_message,
            request_size_bytes: None,  // Could be calculated from request
            response_size_bytes: None, // Could be calculated from response
            ip_address: None,          // Would need to be passed from request context
            user_agent: None,          // Would need to be passed from request context
        };

        database
            .record_api_key(&usage)
            .await
            .map_err(|e| AppError::database(format!("Failed to record API key usage: {e}")))?;
        Ok(())
    }

    /// Get database reference for admin API
    #[must_use]
    pub fn database(&self) -> &Database {
        &self.resources.coach.database
    }

    /// Get auth manager reference for admin API
    #[must_use]
    pub fn auth_manager(&self) -> &AuthManager {
        &self.resources.auth.auth_manager
    }

    // === Tenant-Aware Tool Handlers ===

    /// Store user-provided OAuth credentials if supplied
    async fn store_mcp_oauth_credentials(
        tenant_context: &TenantContext,
        oauth_client: &Arc<TenantOAuthClient>,
        repos: &Arc<RepositoryRegistry>,
        credentials: &McpOAuthCredentials<'_>,
        config: &Arc<ServerConfig>,
    ) {
        // Store Strava credentials if provided
        #[cfg(feature = "provider-strava")]
        if let (Some(id), Some(secret)) = (
            credentials.strava_client_id,
            credentials.strava_client_secret,
        ) {
            Self::store_provider_credentials(
                tenant_context,
                oauth_client,
                repos,
                OAuthProviderParams {
                    provider: "strava",
                    client_id: id,
                    client_secret: secret,
                    configured_redirect_uri: config.oauth.strava.redirect_uri.as_ref(),
                    scopes: &Self::get_strava_scopes(),
                    base_url: &config.base_url,
                },
            )
            .await;
        }

        // Store Fitbit credentials if provided
        if let (Some(id), Some(secret)) = (
            credentials.fitbit_client_id,
            credentials.fitbit_client_secret,
        ) {
            Self::store_provider_credentials(
                tenant_context,
                oauth_client,
                repos,
                OAuthProviderParams {
                    provider: "fitbit",
                    client_id: id,
                    client_secret: secret,
                    configured_redirect_uri: config.oauth.fitbit.redirect_uri.as_ref(),
                    scopes: &Self::get_fitbit_scopes(),
                    base_url: &config.base_url,
                },
            )
            .await;
        }
    }

    /// Store OAuth credentials for a specific provider.
    ///
    /// Writes through to both the durable `tenants` repository (the source of
    /// truth — survives restart, visible to other replicas) and the per-process
    /// cache held by `oauth_client` for fast same-process lookups.
    async fn store_provider_credentials(
        tenant_context: &TenantContext,
        oauth_client: &Arc<TenantOAuthClient>,
        repos: &Arc<RepositoryRegistry>,
        params: OAuthProviderParams<'_>,
    ) {
        info!(
            "Storing MCP-provided {} OAuth credentials for tenant {}",
            params.provider, tenant_context.tenant_id
        );

        let redirect_uri = params.configured_redirect_uri.map_or_else(
            || format!("{}/api/oauth/callback/{}", params.base_url, params.provider),
            String::clone,
        );

        Self::persist_provider_credentials(tenant_context, repos, &params, redirect_uri.clone())
            .await;
        Self::cache_provider_credentials(tenant_context, oauth_client, &params, redirect_uri).await;
    }

    /// Persist MCP-provided credentials to the durable `tenants` repository (source of truth).
    async fn persist_provider_credentials(
        tenant_context: &TenantContext,
        repos: &Arc<RepositoryRegistry>,
        params: &OAuthProviderParams<'_>,
        redirect_uri: String,
    ) {
        let credentials = TenantOAuthCredentials {
            tenant_id: tenant_context.tenant_id,
            provider: params.provider.to_owned(),
            client_id: params.client_id.to_owned(),
            client_secret: params.client_secret.to_owned(),
            redirect_uri,
            scopes: params.scopes.to_vec(),
            rate_limit_per_day: TenantOAuthManager::default_rate_limit_for_provider(
                params.provider,
            ),
        };
        if let Err(e) = repos.tenants.store_oauth_credentials(&credentials).await {
            error!(
                "Failed to persist {} OAuth credentials for tenant {}: {}",
                params.provider, tenant_context.tenant_id, e
            );
        }
    }

    /// Populate the per-process credential cache held by `oauth_client`.
    async fn cache_provider_credentials(
        tenant_context: &TenantContext,
        oauth_client: &Arc<TenantOAuthClient>,
        params: &OAuthProviderParams<'_>,
        redirect_uri: String,
    ) {
        let request = StoreCredentialsRequest {
            client_id: params.client_id.to_owned(),
            client_secret: params.client_secret.to_owned(),
            redirect_uri,
            scopes: params.scopes.to_vec(),
            configured_by: tenant_context.user_id,
        };
        if let Err(e) = oauth_client
            .store_credentials(tenant_context.tenant_id, params.provider, request)
            .await
        {
            error!(
                "Failed to cache {} OAuth credentials: {}",
                params.provider, e
            );
        }
    }

    /// Get default Strava OAuth scopes
    #[cfg(feature = "provider-strava")]
    fn get_strava_scopes() -> Vec<String> {
        STRAVA_DEFAULT_SCOPES
            .split(',')
            .map(<str as ToOwned>::to_owned)
            .collect()
    }

    /// Get default Fitbit OAuth scopes
    fn get_fitbit_scopes() -> Vec<String> {
        vec![
            "activity".to_owned(),
            "heartrate".to_owned(),
            "location".to_owned(),
            "nutrition".to_owned(),
            "profile".to_owned(),
            "settings".to_owned(),
            "sleep".to_owned(),
            "social".to_owned(),
            "weight".to_owned(),
        ]
    }

    /// Handle tenant-aware connection status.
    ///
    /// Cross-cuts `AuthRepos` (`oauth_tokens`) and the registry's OAuth
    /// completion `notifications` repository; takes the full registry at
    /// the entry-point to keep the args list under the clippy ceiling.
    /// Helpers below receive narrow views or the rows already read.
    #[tracing::instrument(
        skip(tenant_oauth_client, repos, request_id, credentials, config),
        fields(
            tenant_id = %tenant_context.tenant_id,
            tenant_name = %tenant_context.tenant_name,
            user_id = %tenant_context.user_id,
        )
    )]
    pub async fn handle_tenant_connection_status(
        tenant_context: &TenantContext,
        tenant_oauth_client: &Arc<TenantOAuthClient>,
        repos: &Arc<RepositoryRegistry>,
        request_id: Value,
        credentials: McpOAuthCredentials<'_>,
        http_port: u16,
        config: &Arc<ServerConfig>,
    ) -> McpResponse {
        info!(
            "Checking connection status for tenant {} user {}",
            tenant_context.tenant_name, tenant_context.user_id
        );

        // Store MCP-provided OAuth credentials if supplied
        Self::store_mcp_oauth_credentials(
            tenant_context,
            tenant_oauth_client,
            repos,
            &credentials,
            config,
        )
        .await;

        let auth = repos.auth_repos();
        let base_url = Self::build_oauth_base_url(http_port);
        let connection_status = Self::check_provider_connections(tenant_context, &auth).await;
        let unread_notifications =
            Self::fetch_unread_oauth_notifications(repos, tenant_context.user_id).await;
        let notifications_text = Self::build_notifications_text(&unread_notifications);
        let structured_data = Self::build_structured_connection_data(
            tenant_context,
            &connection_status,
            &base_url,
            &unread_notifications,
        );
        let text_content = Self::build_text_content(
            &connection_status,
            &base_url,
            tenant_context,
            &notifications_text,
        );

        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            result: Some(serde_json::json!({
                "content": [
                    {
                        "type": "text",
                        "text": text_content
                    }
                ],
                "structuredContent": structured_data,
                "isError": false
            })),
            error: None,
            id: Some(request_id),
        }
    }

    /// Build OAuth base URL from server config (respects `BASE_URL` scheme for TLS/proxy)
    fn build_oauth_base_url(http_port: u16) -> String {
        let base = get_server_config().map_or_else(
            || format!("http://localhost:{http_port}"),
            |c| c.base_url.clone(),
        );
        format!("{base}/api/oauth")
    }

    /// Check connection status for all providers
    async fn check_provider_connections(
        tenant_context: &TenantContext,
        auth: &AuthRepos,
    ) -> ProviderConnectionStatus {
        let user_id = tenant_context.user_id;
        let tenant_id_str = tenant_context.tenant_id.to_string();

        // Check Strava connection status
        debug!(
            "Checking Strava token for user_id={}, tenant_id={}, provider=strava",
            user_id, tenant_id_str
        );
        let strava_connected = auth
            .oauth_tokens
            .get_token(user_id, tenant_context.tenant_id, "strava")
            .await
            .map_or_else(
                |e| {
                    warn!("Failed to query Strava OAuth token: {e}");
                    false
                },
                |token| {
                    let connected = token.is_some();
                    debug!("Strava token lookup result: connected={connected}");
                    connected
                },
            );

        // Check Fitbit connection status
        let fitbit_connected = auth
            .oauth_tokens
            .get_token(user_id, tenant_context.tenant_id, "fitbit")
            .await
            .is_ok_and(|token| token.is_some());

        ProviderConnectionStatus {
            strava_connected,
            fitbit_connected,
        }
    }

    /// Read the unread OAuth completion notifications off the registry. A
    /// read failure is logged and reads as none, so the status reply still
    /// goes out.
    async fn fetch_unread_oauth_notifications(
        repos: &RepositoryRegistry,
        user_id: Uuid,
    ) -> Vec<OAuthNotification> {
        repos
            .notifications
            .get_unread(user_id)
            .await
            .unwrap_or_else(|e| {
                warn!(
                    user_id = %user_id,
                    error = %e,
                    "Failed to fetch OAuth notifications for connection status"
                );
                Vec::new()
            })
    }

    /// Build notifications text from unread notifications
    fn build_notifications_text(unread_notifications: &[OAuthNotification]) -> String {
        if unread_notifications.is_empty() {
            String::new()
        } else {
            let mut notifications_msg = String::from("\n\nRecent OAuth Updates:\n");
            for notification in unread_notifications {
                let status_indicator = if notification.success {
                    "[SUCCESS]"
                } else {
                    "[FAILED]"
                };
                writeln!(
                    notifications_msg,
                    "{status_indicator} {}: {}",
                    notification.provider.to_uppercase(),
                    notification.message
                )
                .unwrap_or_else(|_| warn!("Failed to write notification text"));
            }
            notifications_msg
        }
    }

    /// Build structured connection data JSON
    fn build_structured_connection_data(
        tenant_context: &TenantContext,
        connection_status: &ProviderConnectionStatus,
        base_url: &str,
        unread_notifications: &[OAuthNotification],
    ) -> Value {
        serde_json::json!({
            "providers": [
                {
                    "provider": "strava",
                    "connected": connection_status.strava_connected,
                    "tenant_id": tenant_context.tenant_id,
                    "last_sync": null,
                    "connect_url": format!("{base_url}/auth/strava/{}", tenant_context.user_id),
                    "connect_instructions": if connection_status.strava_connected {
                        "Your Strava account is connected and ready to use."
                    } else {
                        "Click this URL to connect your Strava account and authorize access to your fitness data."
                    }
                },
                {
                    "provider": "fitbit",
                    "connected": connection_status.fitbit_connected,
                    "tenant_id": tenant_context.tenant_id,
                    "last_sync": null,
                    "connect_url": format!("{base_url}/auth/fitbit/{}", tenant_context.user_id),
                    "connect_instructions": if connection_status.fitbit_connected {
                        "Your Fitbit account is connected and ready to use."
                    } else {
                        "Click this URL to connect your Fitbit account and authorize access to your fitness data."
                    }
                }
            ],
            "tenant_info": {
                "tenant_id": tenant_context.tenant_id,
                "tenant_name": tenant_context.tenant_name
            },
            "connection_help": serde_json::to_value(json_schemas::ConnectionHelp {
                message: "To connect a fitness provider, click the connect_url for the provider you want to use. You'll be redirected to their website to authorize access, then redirected back to complete the connection.".to_owned(),
                supported_providers: vec!["strava".to_owned(), "fitbit".to_owned()],
                note: "After connecting, you can use fitness tools like get_activities, get_athlete, and get_stats with the connected provider.".to_owned(),
            }).unwrap_or_else(|_| serde_json::json!({})),
            "recent_notifications": unread_notifications.iter().map(|n| {
                json_schemas::NotificationItem {
                    id: n.id.clone(),
                    provider: n.provider.clone(),
                    success: n.success,
                    message: n.message.clone(),
                    created_at: n.created_at,
                }
            }).collect::<Vec<_>>()
        })
    }

    /// Build human-readable text content
    fn build_text_content(
        connection_status: &ProviderConnectionStatus,
        base_url: &str,
        tenant_context: &TenantContext,
        notifications_text: &str,
    ) -> String {
        let strava_status = if connection_status.strava_connected {
            "Connected"
        } else {
            "Not Connected"
        };
        let fitbit_status = if connection_status.fitbit_connected {
            "Connected"
        } else {
            "Not Connected"
        };

        let strava_action = if connection_status.strava_connected {
            "Ready to use fitness tools!".to_owned()
        } else {
            format!(
                "Click to connect: {base_url}/auth/strava/{}",
                tenant_context.user_id
            )
        };

        let fitbit_action = if connection_status.fitbit_connected {
            "Ready to use fitness tools!".to_owned()
        } else {
            format!(
                "Click to connect: {base_url}/auth/fitbit/{}",
                tenant_context.user_id
            )
        };

        let connection_instructions = if !connection_status.strava_connected
            || !connection_status.fitbit_connected
        {
            "To connect a provider:\n\
            1. Click one of the URLs above\n\
            2. You'll be redirected to authorize access\n\
            3. Complete the OAuth flow to connect your account\n\
            4. Start using fitness tools like get_activities, get_athlete, and get_stats"
        } else {
            "All providers connected! You can now use fitness tools like get_activities, get_athlete, and get_stats."
        };

        format!(
            "Fitness Provider Connection Status\n\n\
            Available Providers:\n\n\
            Strava ({strava_status})\n\
            {strava_action}\n\n\
            Fitbit ({fitbit_status})\n\
            {fitbit_action}\n\n\
            {connection_instructions}{notifications_text}"
        )
    }

    /// Disconnect a provider through the domain chokepoint (`OAuthService`):
    /// mirror resolution, lockstep token+row deletion, catalogued notify event.
    async fn handle_tenant_disconnect_provider(
        tenant_context: &TenantContext,
        provider_name: &str,
        resources: &Arc<ServerContext>,
        request_id: Value,
    ) -> McpResponse {
        info!(
            "Tenant {} disconnecting provider {} for user {}",
            tenant_context.tenant_name, provider_name, tenant_context.user_id
        );

        let service = OAuthService::new(
            resources.data(),
            resources.common.config.clone(),
            resources.auth.oauth_notification_sender.clone(),
        );
        let tenant_uuid = Some(tenant_context.tenant_id.as_uuid());
        if let Err(e) = service
            .disconnect_provider(tenant_context.user_id, provider_name, tenant_uuid)
            .await
        {
            error!(
                "Failed to disconnect {} for user {}: {}",
                provider_name, tenant_context.user_id, e
            );
            return McpResponse {
                jsonrpc: JSONRPC_VERSION.to_owned(),
                result: None,
                error: Some(McpError {
                    code: ERROR_INTERNAL_ERROR,
                    message: format!("Failed to disconnect from {provider_name}"),
                    data: None,
                }),
                id: Some(request_id),
            };
        }

        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            result: Some(serde_json::json!({
                "message": format!("Disconnected from {provider_name}"),
                "provider": provider_name,
                "tenant_id": tenant_context.tenant_id,
                "success": true
            })),
            error: None,
            id: Some(request_id),
        }
    }

    /// Create error response for tool execution failure
    fn create_tool_error_response(
        tool_name: &str,
        provider_name: &str,
        response_error: Option<String>,
        request_id: Value,
    ) -> McpResponse {
        let error_msg = response_error
            .unwrap_or_else(|| "Tool execution failed with no error message".to_owned());
        error!(
            "Tool execution failed for {} with provider {}: {} (success=false)",
            tool_name, provider_name, error_msg
        );
        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            result: None,
            error: Some(McpError {
                code: ERROR_INTERNAL_ERROR,
                message: error_msg,
                data: None,
            }),
            id: Some(request_id),
        }
    }

    // Tool routing now uses ToolId::from_name() to validate tools
    // All tools registered in ToolId enum are automatically routed through Universal Protocol

    async fn handle_tenant_tool_with_provider(
        tool_name: &str,
        args: &Value,
        request_id: Value,
        tenant_context: &TenantContext,
        resources: &Arc<ServerContext>,
        user_id: Uuid,
    ) -> McpResponse {
        // Validate tool is known
        if let Some(error_response) =
            Self::validate_known_tool(tool_name, resources, request_id.clone())
        {
            return error_response;
        }

        let params = match serde_json::from_value::<json_schemas::ProviderParams>(args.clone()) {
            Ok(p) => p,
            Err(e) => {
                return McpResponse {
                    jsonrpc: JSONRPC_VERSION.to_owned(),
                    result: None,
                    error: Some(McpError {
                        code: ERROR_INVALID_PARAMS,
                        message: format!("Invalid provider parameters: {e}"),
                        data: None,
                    }),
                    id: Some(request_id),
                };
            }
        };
        let provider_name = params.provider.as_deref().unwrap_or("");

        info!(
            "Executing tenant tool {} with provider {} for tenant {} user {}",
            tool_name, provider_name, tenant_context.tenant_name, tenant_context.user_id
        );

        // Create Universal protocol request
        let universal_request = Self::create_universal_request(
            tool_name,
            args,
            user_id,
            tenant_context,
            resources,
            &request_id,
        );

        // Execute tool through Universal protocol. Thread the session token
        // (JWT `jti`) as the Guardian turn token so taint accumulates across a
        // headless turn's native tool calls (which share one bridge-minted token).
        Self::execute_and_convert_tool(
            universal_request,
            resources,
            tool_name,
            provider_name,
            request_id,
            tenant_context.session_id.clone(),
        )
        .await
    }

    /// Validate that the tool name resolves in the shared [`ToolRegistry`].
    ///
    /// Post-unification (2026-04-18): every tool — MCP protocol or chat
    /// pipeline — lives in `resources.tool_registry`. A lookup miss is the
    /// single source of truth for "unknown tool"; there is no fallback
    /// enum to consult.
    fn validate_known_tool(
        tool_name: &str,
        resources: &Arc<ServerContext>,
        request_id: Value,
    ) -> Option<McpResponse> {
        if resources.mcp.tool_registry.get(tool_name).is_some() {
            None
        } else {
            Some(McpResponse {
                jsonrpc: JSONRPC_VERSION.to_owned(),
                result: None,
                error: Some(McpError {
                    code: ERROR_METHOD_NOT_FOUND,
                    message: format!("Unknown tool: {tool_name}"),
                    data: None,
                }),
                id: Some(request_id),
            })
        }
    }

    /// Create Universal protocol request from tenant tool parameters
    fn create_universal_request(
        tool_name: &str,
        args: &Value,
        user_id: Uuid,
        tenant_context: &TenantContext,
        resources: &Arc<ServerContext>,
        request_id: &Value,
    ) -> UniversalRequest {
        // Create progress reporter if notification sender is available
        let progress_reporter = resources
            .sse
            .progress_notification_sender
            .as_ref()
            .map(|sender| {
                let progress_token = format!("mcp-{request_id}");
                let mut reporter = ProgressReporter::new(progress_token.clone());

                // Set callback to send progress notifications
                let sender_clone = sender.clone();
                reporter.set_callback(move |progress, total, message| {
                    let notification =
                        ProgressNotification::new(progress_token.clone(), progress, total, message);
                    let _ = sender_clone.send(notification);
                });

                reporter
            });

        // Create cancellation token for this operation
        let cancellation_token = Some(CancellationToken::new());

        UniversalRequest {
            tool_name: tool_name.to_owned(),
            parameters: args.clone(),
            user_id: user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(tenant_context.tenant_id.to_string()),
            progress_token: progress_reporter.as_ref().map(|r| r.progress_token.clone()),
            cancellation_token,
            progress_reporter,
        }
    }

    /// Execute Universal protocol tool and convert response to MCP format
    async fn execute_and_convert_tool(
        universal_request: UniversalRequest,
        resources: &Arc<ServerContext>,
        tool_name: &str,
        provider_name: &str,
        request_id: Value,
        turn_token: Option<String>,
    ) -> McpResponse {
        // Register cancellation token if present
        if let (Some(progress_token), Some(cancellation_token)) = (
            &universal_request.progress_token,
            &universal_request.cancellation_token,
        ) {
            resources
                .register_cancellation_token(progress_token.clone(), cancellation_token.clone())
                .await;
        }

        // Guardian turn key only: a `/mcp` caller has no chat turn behind it,
        // and the ACP subprocess that once did now reaches tools in-process.
        let executor = turn_token.map_or_else(
            || UniversalToolExecutor::new(resources.clone()),
            |token| UniversalToolExecutor::new(resources.clone()).with_turn_token(token),
        );

        let result = executor.execute_tool(universal_request.clone()).await;

        // Cleanup cancellation token after execution
        if let Some(progress_token) = &universal_request.progress_token {
            resources.cleanup_cancellation_token(progress_token).await;
        }

        match result {
            Ok(response) => {
                // Convert UniversalResponse to proper MCP ToolResponse format
                let tool_response = ProtocolConverter::universal_to_mcp(response);

                // Serialize ToolResponse to JSON for MCP result field
                match serde_json::to_value(&tool_response) {
                    Ok(result_value) => McpResponse {
                        jsonrpc: JSONRPC_VERSION.to_owned(),
                        result: Some(result_value),
                        error: None,
                        id: Some(request_id),
                    },
                    Err(e) => Self::create_tool_error_response(
                        tool_name,
                        provider_name,
                        Some(format!("Failed to serialize tool response: {e}")),
                        request_id,
                    ),
                }
            }
            Err(e) => Self::create_tool_error_response(
                tool_name,
                provider_name,
                Some(format!("Tool execution error: {e}")),
                request_id,
            ),
        }
    }
}

// ============================================================================
// AXUM SERVER ORCHESTRATION
// ============================================================================

impl ProviderToolRouter {
    /// Run HTTP server (convenience method)
    ///
    /// Starts the Axum HTTP server on the specified port using the embedded resources.
    ///
    /// # Errors
    /// Returns an error if server setup or routing configuration fails
    pub async fn run(&self, port: u16) -> AppResult<()> {
        self.run_http_server_with_resources_axum(port, self.resources.clone())
            .await
    }

    /// Panic handler for the outermost [`CatchPanicLayer`].
    ///
    /// A panic in any handler (sqlx column-type mismatch, integer overflow,
    /// `unwrap` on `None`, etc.) used to propagate out of tokio's task and
    /// terminate the Cloud Run container via SIGSEGV/SIGABRT, killing every
    /// in-flight sibling request on the same instance. With this layer:
    ///
    /// - The panicking request returns HTTP 500 with a redacted JSON body.
    /// - The container stays alive; sibling requests complete normally.
    /// - The panic is logged at `ERROR` level with the extracted message so
    ///   the dravr-tronc error-notifier surfaces it on Slack within seconds
    ///   instead of disappearing into Cloud Run logs as a SIGSEGV exit.
    ///
    /// This is defense in depth — the structural fix is the typed-newtype
    /// migration (see [`pierre_core::models::UserId`]) that eliminates the
    /// sqlx panic class at compile time. New panic sources (logic bugs,
    /// overflow, unwrap-on-None) will keep appearing and this catches them.
    fn handle_request_panic(panic_payload: Box<dyn Any + Send + 'static>) -> Response<Body> {
        use axum::http::StatusCode;
        use axum::response::IntoResponse;

        // Consume the Box by attempting downcast::<String>() — Box<dyn Any>'s
        // downcast is the by-value method. Both legs of the chain fall back to
        // a literal label when the panic payload isn't a string. By-value
        // consumption is required by tower_http::catch_panic::CatchPanicLayer's
        // F: Fn(Box<dyn Any + Send + 'static>) -> Response signature, so we
        // consume here explicitly rather than borrow.
        let message = panic_payload
            .downcast::<String>()
            .map(|b| *b)
            .or_else(|payload| payload.downcast::<&'static str>().map(|b| (*b).to_owned()))
            .unwrap_or_else(|_| "non-string panic payload".to_owned());

        error!(
            panic_message = %message,
            "handler panicked — returning 500 and keeping container alive"
        );

        (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({
                "error": "internal_server_error",
                "message": "An internal error occurred. The incident has been recorded.",
            })),
        )
            .into_response()
    }

    /// Run HTTP server with Axum framework
    ///
    /// This method provides the Axum-based server implementation.
    ///
    /// # Errors
    /// Returns an error if server setup or routing configuration fails
    pub async fn run_http_server_with_resources_axum(
        &self,
        port: u16,
        resources: Arc<ServerContext>,
    ) -> AppResult<()> {
        info!("HTTP server (Axum) starting on port {}", port);

        // Build the main router with all routes
        let app = Self::setup_axum_router(&resources);

        // Apply middleware layers (order matters - applied bottom-up).
        // CatchPanicLayer is added LAST so it wraps every other layer; a
        // panic in TraceLayer span machinery, the request-id middleware, or
        // any handler converts to a 500 response instead of unwinding past
        // tokio and killing the container.
        // OTLP request metrics + inbound W3C trace-context extraction. Applied
        // innermost so it runs inside the TraceLayer span — `Span::current()`
        // is then the per-request span that `set_parent` chains to the caller's
        // trace. Inert unless the `telemetry` feature is active. `MatchedPath`
        // (low-cardinality route label) is still visible here via Router::layer.
        #[cfg(feature = "telemetry")]
        let app = app.layer(middleware::from_fn(telemetry_middleware));
        let app = app
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(
                        DefaultMakeSpan::new()
                            .level(Level::INFO)
                            .include_headers(false),
                    )
                    .on_response(
                        DefaultOnResponse::new()
                            .level(Level::INFO)
                            .latency_unit(LatencyUnit::Millis),
                    )
                    // Silence tower-http's default failure logger: its event
                    // knows only latency + status, so the forwarded ops alert
                    // never named the failing endpoint. `response_failure_log`
                    // (applied just outside this layer) is the single failure
                    // logger, carrying method + path and routing designed
                    // backpressure (Retry-After 503) to WARN instead of ERROR.
                    .on_failure(()),
            )
            .layer(middleware::from_fn(response_failure_log_middleware))
            .layer(middleware::from_fn(request_id_middleware))
            .layer(setup_cors(&resources.common.config.cors.allowed_origins))
            .layer(Self::create_security_headers_layer(
                &resources.common.config,
            ))
            .layer(CatchPanicLayer::custom(Self::handle_request_panic));

        // Create server address using host from config (defaults to localhost, can be 0.0.0.0 for network access)
        let host = &resources.common.config.host;
        let addr: SocketAddr = format!("{host}:{port}")
            .parse()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], port)));
        info!("HTTP server (Axum) listening on http://{}", addr);

        // Start the Axum server with ConnectInfo for IP extraction (rate limiting)
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| AppError::internal(format!("Transport error: {e}")))?;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .map_err(|e| AppError::internal(format!("Transport error: {e}")))?;

        Ok(())
    }

    /// Setup complete Axum router with all route modules
    ///
    /// Routes are conditionally compiled based on feature flags to support
    /// modular server configurations. See Cargo.toml for feature definitions.
    ///
    /// Note: This function is intentionally long due to the conditional route
    /// registration pattern. Splitting it would fragment related route setup
    /// logic and make the code harder to follow. Each section is clearly
    /// documented and the structure follows the feature flag hierarchy.
    fn setup_axum_router(resources: &Arc<ServerContext>) -> axum::Router {
        use axum::{middleware::from_fn_with_state, Router};

        use pierre_middleware::csrf_protection_layer;

        // ═══════════════════════════════════════════════════════════════
        // CONDITIONAL IMPORTS - Based on feature flags
        // ═══════════════════════════════════════════════════════════════

        #[cfg(feature = "client-api-keys")]
        use crate::routes::api_keys::ApiKeyRoutes;
        #[cfg(feature = "client-messaging")]
        use crate::routes::commands::CommandRoutes;
        #[cfg(feature = "protocol-mcp")]
        use crate::routes::mcp::McpRoutes;
        #[cfg(all(feature = "client-chat", feature = "client-messaging"))]
        use crate::routes::surfaces::SurfaceRoutes;
        #[cfg(feature = "client-tenants")]
        use crate::routes::tenants::TenantRoutes;
        #[cfg(feature = "client-mcp-tokens")]
        use crate::routes::user_mcp_tokens::UserMcpTokenRoutes;
        #[cfg(feature = "client-chat")]
        use crate::routes::{chat::ChatRoutes, usage::UsageRoutes};
        #[cfg(feature = "client-settings")]
        use crate::routes::{
            configuration::ConfigurationRoutes, fitness::FitnessConfigurationRoutes,
            health_data::HealthDataRoutes,
        };
        use crate::routes::{i18n::I18nRoutes, memory::MemoryRoutes, personas::PersonasRoutes};
        #[cfg(feature = "protocol-a2a")]
        use pierre_routes_a2a::{A2ARoutes, A2ARoutesState};
        #[cfg(feature = "client-llm-settings")]
        use pierre_routes_admin::llm_settings::LlmSettingsRoutes;
        #[cfg(feature = "client-admin-api")]
        use pierre_routes_admin::AdminRoutes;
        #[cfg(feature = "client-impersonation")]
        use pierre_routes_admin::ImpersonationRoutes;
        #[cfg(feature = "client-chat")]
        use pierre_routes_admin::LlmConsumptionRoutes;
        #[cfg(feature = "protocol-rest")]
        use pierre_routes_auth::AuthRoutes;
        #[cfg(feature = "client-dashboard")]
        use pierre_routes_dashboard::DashboardRoutes;
        #[cfg(feature = "oauth")]
        use pierre_routes_identity::OAuth2Routes;
        #[cfg(feature = "client-oauth-apps")]
        use pierre_routes_identity::UserOAuthAppRoutes;
        #[cfg(feature = "client-admin-ui")]
        use pierre_routes_web_admin::WebAdminRoutes;
        #[cfg(feature = "transport-sse")]
        use pierre_sse::SseRoutes;

        #[cfg(feature = "client-admin-api")]
        use crate::config::routes::{admin_config_router, AdminConfigState};

        // ═══════════════════════════════════════════════════════════════
        // HEALTH ROUTES - Always enabled
        // ═══════════════════════════════════════════════════════════════

        let health_routes =
            Self::create_axum_health_routes(Arc::clone(&resources.common.llm_health));
        let app = Router::new().merge(health_routes);

        // ═══════════════════════════════════════════════════════════════
        // CLIENT-ADMIN-API ROUTES
        // ═══════════════════════════════════════════════════════════════

        #[cfg(feature = "client-admin-api")]
        let app = {
            use crate::routes::admin::diagnostics::{
                routes as diagnostics_routes, DiagnosticsContext,
            };
            use pierre_routes_admin::tool_selection::{ToolSelectionContext, ToolSelectionRoutes};
            let admin_api_key_limit = resources
                .common
                .config
                .rate_limiting
                .admin_provisioned_api_key_monthly_limit;
            let admin_token_cache_ttl = resources.common.config.auth.admin_token_cache_ttl_secs;
            let mut admin_context = AdminApiContext::new(AdminApiContextInit {
                database: resources.coach.database.clone(),
                repos: resources.common.repos.clone(),
                jwt_secret: resources.auth.admin_jwt_secret.to_string(),
                auth_manager: resources.auth.auth_manager.clone(),
                jwks_manager: resources.auth.jwks_manager.clone(),
                admin_api_key_monthly_limit: admin_api_key_limit,
                admin_token_cache_ttl_secs: admin_token_cache_ttl,
                harness_config_registry: resources.fitness.harness_config_registry.clone(),
                guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
                prompt_registry: resources.mcp.prompt_registry.clone(),
                tool_description_registry: resources.mcp.tool_description_registry.clone(),
                evidence_registry: resources.mcp.evidence_registry.clone(),
                messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
                cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
                persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
                contremaitre_config: resources.mcp.contremaitre_config.clone(),
            });
            admin_context
                .email_service
                .clone_from(&resources.common.email_service);
            admin_context
                .frontend_url
                .clone_from(&resources.common.config.frontend_url);
            #[cfg(feature = "client-messaging")]
            {
                admin_context.approval_notifier = Some(ApprovalNotifier::from_context(resources));
            }

            // Tool-selection and diagnostic sub-routes use pierre-server-internal
            // types (`ToolSelectionService`, `ToolRegistry`) and so are
            // mounted alongside the admin route group rather than baked into it.
            let auth_service = admin_context.auth_service.clone();
            let tool_selection_routes = ToolSelectionRoutes::routes(ToolSelectionContext {
                tool_selection: resources.mcp.tool_selection.clone(),
            })
            .layer(middleware::from_fn_with_state(
                auth_service.clone(),
                pierre_routes_admin::admin_auth_middleware,
            ));
            let diagnostics_ctx = DiagnosticsContext {
                tool_registry: resources.mcp.tool_registry.clone(),
                runtime: resources.clone(), // Safe: Arc clone coerced into the trait object
            };
            let diagnostics_router = diagnostics_routes(diagnostics_ctx, auth_service);

            let cookie_admin_routes =
                AdminRoutes::cookie_admin_routes::<ServerContext>(admin_context.clone(), resources);
            let admin_routes = AdminRoutes::routes(admin_context);

            let admin_config_routes = resources.coach.admin_config.as_ref().map_or_else(
                || {
                    tracing::warn!(
                        "Admin config service not available - admin config API disabled"
                    );
                    Router::new()
                },
                |admin_config| {
                    let admin_config_state = Arc::new(AdminConfigState::new(
                        Arc::clone(admin_config),
                        Arc::clone(resources),
                    ));
                    admin_config_router(admin_config_state)
                },
            );

            app.merge(admin_routes)
                .merge(tool_selection_routes)
                .merge(diagnostics_router)
                .merge(cookie_admin_routes)
                .nest("/api/admin/config", admin_config_routes)
        };

        // ═══════════════════════════════════════════════════════════════
        // PROTOCOL ROUTES
        // ═══════════════════════════════════════════════════════════════

        #[cfg(feature = "protocol-rest")]
        let app = app.merge(AuthRoutes::routes(resources.auth_routes_context()));

        #[cfg(feature = "oauth")]
        let app = {
            let oauth2_context = OAuth2Context {
                database: resources.coach.database.clone(),
                oauth2_server: resources.common.repos.oauth2_server.clone(),
                tenants: resources.common.repos.tenants.clone(),
                users: resources.common.repos.users.clone(),
                auth_manager: resources.auth.auth_manager.clone(),
                jwks_manager: resources.auth.jwks_manager.clone(),
                // pierre-routes-identity owns only the OAuth 2.0 server slice of
                // ServerConfig — narrowing the route group's config dependency to
                // OAuth2ServerConfig so the leaf crate has no pierre-server import.
                config: Arc::new(resources.common.config.oauth2_server.clone()),
                rate_limiter: Arc::new(OAuth2RateLimiter::from_rate_limit_config(
                    resources.common.config.rate_limiting.clone(),
                )),
            };
            app.merge(OAuth2Routes::routes(oauth2_context))
        };

        #[cfg(feature = "protocol-mcp")]
        let app = app.merge(McpRoutes::routes(Arc::clone(resources)));

        #[cfg(feature = "protocol-a2a")]
        let app = {
            // A2ARoutesState combines the composition-root context (which
            // implements both MiddlewareCtx + A2ACtx) with the concrete
            // A2AClientManager / auth-middleware / tool-runtime handles
            // pierre-a2a and pierre-tool-runtime own. The state struct
            // sidesteps the pierre-runtime-context ↔ pierre-a2a cycle that
            // would arise from adding `a2a_client_manager()` /
            // `tool_runtime()` accessors to A2ACtx directly.
            use pierre_tool_runtime::runtime::ToolRuntime;
            let tool_runtime: Arc<dyn ToolRuntime> = resources.clone(); // Safe: Arc clone coerced into trait object
            let a2a_state = A2ARoutesState {
                ctx: Arc::clone(resources),
                client_manager: resources.a2a.a2a_client_manager.clone(), // Safe: Arc clone for shared client manager
                auth_middleware: resources.auth.auth_middleware.clone(), // Safe: Arc clone for shared middleware
                tool_runtime,
            };
            app.merge(A2ARoutes::routes(a2a_state))
        };

        // ═══════════════════════════════════════════════════════════════
        // TRANSPORT ROUTES
        // ═══════════════════════════════════════════════════════════════

        #[cfg(feature = "transport-sse")]
        let app = {
            // Upcast Arc<ServerContext> → Arc<dyn SseCtx>. `Arc::clone` cannot
            // do the coercion in argument position, so use the From impl.
            let sse_ctx: Arc<dyn pierre_runtime_context::SseCtx> = Arc::clone(resources) as _;
            app.merge(SseRoutes::routes(
                Arc::clone(&resources.sse.sse_manager),
                sse_ctx,
            ))
        };

        // ═══════════════════════════════════════════════════════════════
        // CLIENT-WEB ROUTES
        // ═══════════════════════════════════════════════════════════════

        #[cfg(feature = "client-dashboard")]
        let app =
            app.merge(DashboardRoutes::routes::<ServerContext>().with_state(Arc::clone(resources)));

        #[cfg(feature = "client-settings")]
        let app = app
            .merge(ConfigurationRoutes::routes(Arc::clone(resources)))
            .merge(FitnessConfigurationRoutes::routes(Arc::clone(resources)))
            .merge(HealthDataRoutes::routes(Arc::clone(resources)))
            .merge(pierre_routes_billing::billing_routes().with_state(Arc::clone(resources)))
            .merge(endurance::endurance_routes().with_state(Arc::clone(resources)))
            .merge(user_profile_routes().with_state(Arc::clone(resources)));

        // Webhook routes for provider-pushed health data (WHOOP, Garmin, Oura)
        #[cfg(feature = "health-sync")]
        let app = {
            use crate::routes::webhooks::WebhookRoutes;
            app.merge(WebhookRoutes::routes(Arc::clone(resources)))
        };

        // Contremaitre prompt hot-reload: webhook + admin routes
        // Contremaitre admin write-back routes (`/api/admin/contremaitre/*`)
        // are mounted by `AdminRoutes::cookie_admin_routes` above. Only the
        // webhook handler stays here.
        let app = app.merge(contremaitre_webhook_routes(Arc::clone(resources)));

        // Connected-apps management: list/revoke a user's MCP OAuth client grants.
        let app = app.merge(OAuthGrantsRoutes::routes(Arc::clone(resources)));

        #[cfg(feature = "client-chat")]
        let app = app
            .merge(ChatRoutes::routes(Arc::clone(resources)))
            .merge(UsageRoutes::routes(Arc::clone(resources)))
            .merge(LlmConsumptionRoutes::routes(Arc::clone(resources)));

        // The surface-capability catalogue the shared-constants generator
        // reads. Public and stateless: compiled-in product capabilities, the
        // same bytes for every caller.
        #[cfg(all(feature = "client-chat", feature = "client-messaging"))]
        let app = app.merge(SurfaceRoutes::routes());

        // Memory facts (list / forget), persona cards from the live contract, and
        // the live string catalogue both clients overlay (public, ETag-revalidated).
        let app = app
            .merge(MemoryRoutes::routes(Arc::clone(resources)))
            .merge(PersonasRoutes::routes(Arc::clone(resources)));
        let catalogue = Arc::clone(&resources.mcp.messaging_strings_registry);
        let app = app.merge(I18nRoutes::routes(catalogue));

        // The slash commands this caller may actually run. Resolved per caller
        // through the same availability predicates `/help` asks, so the in-app
        // palette advertises exactly what the messaging `/help` would list.
        #[cfg(feature = "client-messaging")]
        let app = app.merge(CommandRoutes::routes(Arc::clone(resources)));

        // Runtime feature flags — self-read endpoint. The admin CRUD
        // endpoints come in through `AdminRoutes::cookie_admin_routes`
        // above so they share its cookie-admin middleware mount.
        let app = app.merge(pierre_routes_admin::FeatureFlagsRoutes::routes::<
            ServerContext,
        >(Arc::clone(resources)));

        // Onboarding state: cheap self-read web + mobile use to gate routing.
        let app = app.merge(OnboardingRoutes::routes(Arc::clone(resources)));
        let app = app.merge(VizRoutes::routes(Arc::clone(resources)));

        #[cfg(feature = "client-coaches")]
        let app = app
            .merge(
                pierre_routes_coaches::build_coaches_router::<ServerContext>()
                    .with_state(Arc::clone(resources)),
            )
            .nest(
                "/api/admin",
                pierre_routes_coaches::build_coaches_admin_router::<ServerContext>()
                    .with_state(Arc::clone(resources)),
            );

        #[cfg(feature = "client-store")]
        let app = app.merge(
            pierre_routes_coaches::build_store_router::<ServerContext>()
                .with_state(Arc::clone(resources)),
        );

        #[cfg(feature = "client-oauth-apps")]
        let app = app
            .merge(UserOAuthAppRoutes::routes::<ServerContext>().with_state(Arc::clone(resources)));

        #[cfg(feature = "client-groups")]
        let app = {
            use pierre_routes_groups::group_analytics::GroupAnalyticsRoutes;
            use pierre_routes_groups::GroupRoutes;
            app.merge(GroupRoutes::routes(Arc::clone(resources)))
                .merge(GroupAnalyticsRoutes::routes(Arc::clone(resources)))
        };

        // ═══════════════════════════════════════════════════════════════
        // CLIENT-ADMIN ROUTES
        // ═══════════════════════════════════════════════════════════════

        #[cfg(feature = "client-admin-ui")]
        let app = app.merge(WebAdminRoutes::routes(resources.web_admin_context()));

        #[cfg(feature = "client-api-keys")]
        let app = app.merge(ApiKeyRoutes::routes(Arc::clone(resources)));

        #[cfg(feature = "client-tenants")]
        let app = app.merge(TenantRoutes::routes(Arc::clone(resources)));

        #[cfg(feature = "client-impersonation")]
        let app = app.merge(ImpersonationRoutes::routes(Arc::clone(resources)));

        #[cfg(feature = "client-llm-settings")]
        let app = app.merge(LlmSettingsRoutes::routes::<ServerContext>(Arc::clone(
            resources,
        )));

        // Coach-athlete roster routes — gated by users.manages_roster=true
        // (or is_admin=true). Always mounted; the permission check lives
        // inside the handler so a user without the bit gets a 403 here
        // rather than a 404 from a missing route.
        let app = app.merge(
            pierre_routes_coaches::build_roster_router::<ServerContext>()
                .with_state(Arc::clone(resources)),
        );

        // ═══════════════════════════════════════════════════════════════
        // OTHER CLIENT ROUTES
        // ═══════════════════════════════════════════════════════════════

        #[cfg(feature = "client-mcp-tokens")]
        let app = app.merge(UserMcpTokenRoutes::routes(Arc::clone(resources)));

        #[cfg(feature = "client-messaging")]
        let app = {
            use crate::routes::messaging::MessagingRoutes;
            app.merge(MessagingRoutes::routes(Arc::clone(resources)))
        };

        #[cfg(feature = "client-notifications")]
        let app = {
            use pierre_routes_groups::NotificationRoutes;
            app.merge(NotificationRoutes::routes(Arc::clone(resources)))
        };

        // ═══════════════════════════════════════════════════════════════
        // CSRF PROTECTION LAYER
        // ═══════════════════════════════════════════════════════════════
        // Applied globally but only activates for cookie-authenticated
        // state-changing requests (POST/PUT/DELETE/PATCH). Bearer token
        // and API key requests pass through without CSRF validation.
        app.layer(from_fn_with_state(
            Arc::clone(resources),
            csrf_protection_layer,
        ))
    }

    /// Create health check routes for Axum.
    ///
    /// Three endpoints:
    ///
    /// * `/health` — cheap liveness probe (always 200 when the process is
    ///   up). This is what the Docker `HEALTHCHECK` and Cloud Run's
    ///   default startup probe hit; it must stay free of dependency
    ///   round-trips so a slow LLM never restarts a healthy server.
    /// * `/ready` — readiness probe gated on the LLM startup probe.
    ///   Returns 200 while the probe is in flight (status `Unknown`) or
    ///   has succeeded, and 503 once the probe has reported `Unhealthy`.
    ///   Operators who want a hard gate on LLM availability can point
    ///   Cloud Run's startup probe at this path instead of `/health`.
    /// * `/health/llm` — JSON snapshot of the latest LLM probe outcome
    ///   for operator introspection (provider, error, timestamp).
    fn create_axum_health_routes(llm_health: Arc<LlmHealthState>) -> axum::Router {
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use axum::{routing::get, Json, Router};

        async fn health_handler() -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "status": "ok",
                "service": PIERRE_MCP_SERVER
            }))
        }

        async fn plugins_health_handler() -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "status": "ok",
                "plugins": []
            }))
        }

        async fn ready_handler(State(state): State<Arc<LlmHealthState>>) -> impl IntoResponse {
            let snapshot = state.snapshot().await;
            let code = match snapshot.status {
                LlmHealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
                LlmHealthStatus::Healthy | LlmHealthStatus::Unknown => StatusCode::OK,
            };
            (
                code,
                Json(serde_json::json!({
                    "status": snapshot.status.to_string(),
                    "service": PIERRE_MCP_SERVER,
                    "llm": snapshot,
                })),
            )
        }

        async fn llm_health_handler(
            State(state): State<Arc<LlmHealthState>>,
        ) -> Json<LlmHealthSnapshot> {
            Json(state.snapshot().await)
        }

        Router::new()
            .route("/health", get(health_handler))
            .route("/health/plugins", get(plugins_health_handler))
            .route("/ready", get(ready_handler))
            .route("/health/llm", get(llm_health_handler))
            .with_state(llm_health)
    }

    /// Create security headers layer for Axum
    ///
    /// Validates security headers configuration and returns Identity layer.
    /// Security headers are validated at startup to catch configuration errors early.
    /// Response header injection happens via response interceptor middleware.
    fn create_security_headers_layer(config: &Arc<ServerConfig>) -> Identity {
        // Validate security headers configuration at startup
        let security_config = Self::setup_security_config(config);
        let headers = security_config.to_headers();

        // Validate all headers can be parsed - this catches configuration errors early
        for (header_name, header_value) in headers {
            if http::HeaderName::from_bytes(header_name.as_bytes()).is_err()
                || http::HeaderValue::from_str(header_value).is_err()
            {
                warn!(
                    "Invalid security header in config: {} = {}",
                    header_name, header_value
                );
            }
        }

        // Return identity layer - headers are applied via CORS middleware and response interceptors
        Identity::new()
    }
}
