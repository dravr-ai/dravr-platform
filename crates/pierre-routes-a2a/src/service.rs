// ABOUTME: Service layer for A2A protocol endpoints and client management
// ABOUTME: Implements business logic for A2A authentication, tool execution, and client administration
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A2A HTTP Routes
//!
//! HTTP endpoints for A2A (Agent-to-Agent) protocol management

use pierre_a2a::{
    agent_card::AgentCard,
    auth::A2AAuthenticator,
    client::{
        A2AClientManager, A2ARateLimitStatus, ClientCredentials, ClientRegistrationRequest,
        ClientUsageStats,
    },
    A2AError,
};
use pierre_core::auth_header::extract_bearer_token;
use pierre_core::constants::time::DAY_SECONDS;
use pierre_core::models::a2a::A2AClient;
use pierre_middleware::McpAuthMiddleware;
use pierre_runtime_context::A2ACtx;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tracing::warn;
use uuid::Uuid;

/// A2A dashboard overview statistics
#[derive(Debug, Serialize)]
pub struct A2ADashboardOverview {
    /// Total number of registered A2A clients
    pub total_clients: u32,
    /// Number of active clients
    pub active_clients: u32,
    /// Total number of sessions
    pub total_sessions: u32,
    /// Number of active sessions
    pub active_sessions: u32,
    /// Request count today
    pub requests_today: u32,
    /// Request count this month
    pub requests_this_month: u32,
    /// Most frequently used capability
    pub most_used_capability: Option<String>,
    /// Error rate (0.0-1.0)
    pub error_rate: f64,
    /// Usage breakdown by client tier
    pub usage_by_tier: Vec<A2ATierUsage>,
}

/// Usage statistics for an A2A client tier
#[derive(Debug, Serialize)]
pub struct A2ATierUsage {
    /// Tier name (free, pro, enterprise)
    pub tier: String,
    /// Number of clients in this tier
    pub client_count: u32,
    /// Total requests from this tier
    pub request_count: u32,
    /// Percentage of total usage
    pub percentage: f64,
}

/// Request to register a new A2A client
#[derive(Debug, Deserialize)]
pub struct A2AClientRequest {
    /// Client application name
    pub name: String,
    /// Client description
    pub description: String,
    /// Requested A2A capabilities
    pub capabilities: Vec<String>,
    /// Optional OAuth redirect URIs
    pub redirect_uris: Option<Vec<String>>,
    /// Contact email for client owner
    pub contact_email: String,
    /// Agent software version
    pub agent_version: Option<String>,
    /// URL to client documentation
    pub documentation_url: Option<String>,
}

/// A2A Routes handler
pub struct A2ARoutes {
    /// Narrow runtime-context slice: auth manager, JWKS, repos, base URL.
    ctx: Arc<dyn A2ACtx>,
    /// A2A client manager — registration, lookups, rate-limit status.
    client_manager: Arc<A2AClientManager>,
    /// A2A authenticator constructed from `ctx` + `auth_middleware` +
    /// `client_manager`; held for capability validation and API-key flows.
    authenticator: Arc<A2AAuthenticator>,
}

impl A2ARoutes {
    /// Extract and validate JWT token from Authorization header
    fn extract_jwt_token(auth_header: Option<&str>) -> Result<String, Value> {
        let auth = auth_header.ok_or_else(|| {
            json!({
                "code": -32001,
                "message": "Missing Authorization header"
            })
        })?;

        let token = extract_bearer_token(auth).map_err(|e| {
            warn!(
                error = %e,
                "Failed to extract bearer token from A2A authorization header"
            );
            json!({
                "code": -32001,
                "message": "Invalid authorization header format"
            })
        })?;

        Ok(token.to_owned())
    }

    /// Validate JWT token and return user ID
    fn validate_jwt_and_get_user_id(&self, token: &str) -> Result<String, Value> {
        self.ctx
            .auth_manager()
            .validate_token(token, self.ctx.jwks_manager())
            .map(|claims| claims.sub)
            .map_err(|e| {
                warn!(
                    error = %e,
                    "A2A authentication token validation failed"
                );
                json!({
                    "code": -32001,
                    "message": "Invalid or expired authentication token"
                })
            })
    }

    /// Authenticate the auth header and return the user ID as a UUID.
    /// Validates JWT token from the Authorization header.
    fn authenticate_and_get_user_id(&self, auth_header: Option<&str>) -> Result<Uuid, A2AError> {
        let token = Self::extract_jwt_token(auth_header).map_err(|e| {
            A2AError::AuthenticationFailed(format!("Missing or invalid authorization: {e}"))
        })?;
        let user_id_str = self
            .validate_jwt_and_get_user_id(&token)
            .map_err(|e| A2AError::AuthenticationFailed(format!("Token validation failed: {e}")))?;
        Uuid::parse_str(&user_id_str)
            .map_err(|e| A2AError::InternalError(format!("Invalid user ID format: {e}")))
    }

    /// Verify that the authenticated user owns the specified client.
    /// Returns an error if the client does not exist or the user does not own it.
    async fn verify_client_ownership(
        &self,
        client_id: &str,
        user_id: &Uuid,
    ) -> Result<(), A2AError> {
        let client = self
            .client_manager
            .get_client(client_id)
            .await?
            .ok_or_else(|| A2AError::ResourceNotFound(format!("Client {client_id}")))?;

        if client.user_id != *user_id {
            return Err(A2AError::ResourceNotFound(format!("Client {client_id}")));
        }

        Ok(())
    }

    /// Creates a new A2A routes instance.
    ///
    /// The composition root supplies each handle explicitly so this crate
    /// does not need to depend on `pierre-server::mcp::resources`:
    ///
    /// - `ctx` — narrow runtime-context slice (auth manager, JWKS, repos, base URL).
    /// - `client_manager` — A2A client lifecycle and credential storage.
    /// - `auth_middleware` — MCP auth middleware required to construct the
    ///   internal [`A2AAuthenticator`] (used for API-key authentication).
    #[must_use]
    pub fn new(
        ctx: Arc<dyn A2ACtx>,
        client_manager: Arc<A2AClientManager>,
        auth_middleware: Arc<McpAuthMiddleware>,
    ) -> Self {
        let authenticator = Arc::new(A2AAuthenticator::new(
            ctx.clone(),            // Safe: Arc clone of trait object for authenticator
            auth_middleware,        // Safe: Arc clone for shared middleware
            client_manager.clone(), // Safe: Arc clone for shared client manager
        ));

        Self {
            ctx,
            client_manager,
            authenticator,
        }
    }

    /// Get A2A agent card
    ///
    /// # Errors
    ///
    /// Returns `A2AError` if the agent card cannot be created
    pub fn get_agent_card(&self) -> Result<AgentCard, A2AError> {
        Ok(AgentCard::with_base_url(self.ctx.base_url()))
    }

    /// Get A2A dashboard overview (scoped to authenticated user's clients)
    ///
    /// # Errors
    ///
    /// Returns `A2AError` if:
    /// - Authentication fails
    /// - Database operations fail
    /// - Client list cannot be retrieved
    pub async fn get_dashboard_overview(
        &self,
        auth_header: Option<&str>,
    ) -> Result<A2ADashboardOverview, A2AError> {
        // Authenticate and scope to user's clients
        let user_id = self.authenticate_and_get_user_id(auth_header)?;

        let clients = self
            .client_manager
            .list_clients_for_user(&user_id)
            .await
            .map_err(|e| A2AError::DatabaseError(e.to_string()))?;

        let total_clients = u32::try_from(clients.len()).unwrap_or(u32::MAX);
        let active_clients =
            u32::try_from(clients.iter().filter(|c| c.is_active).count()).unwrap_or(0);

        // Sessions and usage stats based on database queries
        // These would need proper session tracking implementation
        let total_sessions = 0; // No session tracking implemented yet
        let active_sessions = 0; // No session tracking implemented yet
        let requests_today = 0; // No usage logging implemented yet
        let requests_this_month = 0; // No usage logging implemented yet
        let most_used_capability = None; // No usage tracking implemented yet
        let error_rate = 0.0; // No error tracking implemented yet

        // Create tier structure based on user subscription level
        let usage_tiers = if active_clients > 0 {
            vec![A2ATierUsage {
                tier: "basic".into(),
                client_count: active_clients,
                request_count: 0, // No usage tracking yet
                percentage: 100.0,
            }]
        } else {
            vec![]
        };

        let overview = A2ADashboardOverview {
            total_clients,
            active_clients,
            total_sessions,
            active_sessions,
            requests_today,
            requests_this_month,
            most_used_capability,
            error_rate,
            usage_by_tier: usage_tiers,
        };

        Ok(overview)
    }

    /// Register new A2A client
    ///
    /// # Errors
    ///
    /// Returns `A2AError` if:
    /// - Authentication fails or no valid auth header
    /// - Client registration fails
    /// - Database operations fail
    pub async fn register_client(
        &self,
        auth_header: Option<&str>,
        request: A2AClientRequest,
    ) -> Result<ClientCredentials, A2AError> {
        // Extract and validate JWT to get the authenticated user's ID
        let token = Self::extract_jwt_token(auth_header)
            .map_err(|e| A2AError::AuthenticationFailed(format!("JWT extraction failed: {e}")))?;
        let user_id_str = self
            .validate_jwt_and_get_user_id(&token)
            .map_err(|e| A2AError::AuthenticationFailed(format!("JWT validation failed: {e}")))?;
        let user_id = Uuid::parse_str(&user_id_str)
            .map_err(|e| A2AError::InternalError(format!("Invalid user ID format: {e}")))?;

        let registration = ClientRegistrationRequest {
            name: request.name,
            description: request.description,
            capabilities: request.capabilities,
            redirect_uris: request.redirect_uris.unwrap_or_default(),
            contact_email: request.contact_email,
        };

        self.client_manager
            .register_client(registration, user_id)
            .await
    }

    /// List A2A clients scoped to the authenticated user
    ///
    /// # Errors
    ///
    /// Returns `A2AError` if:
    /// - Authentication fails
    /// - Database operations fail
    /// - Client list cannot be retrieved
    pub async fn list_clients(
        &self,
        auth_header: Option<&str>,
    ) -> Result<Vec<A2AClient>, A2AError> {
        let user_id = self.authenticate_and_get_user_id(auth_header)?;
        self.client_manager.list_clients_for_user(&user_id).await
    }

    /// Get A2A client usage statistics (requires ownership)
    ///
    /// # Errors
    ///
    /// Returns `A2AError` if:
    /// - Authentication fails
    /// - Client does not exist or caller does not own it
    /// - Database operations fail
    pub async fn get_client_usage(
        &self,
        auth_header: Option<&str>,
        client_id: &str,
    ) -> Result<ClientUsageStats, A2AError> {
        let user_id = self.authenticate_and_get_user_id(auth_header)?;
        self.verify_client_ownership(client_id, &user_id).await?;
        self.client_manager.get_client_usage(client_id).await
    }

    /// Get A2A client rate limit status (requires ownership)
    ///
    /// # Errors
    ///
    /// Returns `A2AError` if:
    /// - Authentication fails
    /// - Client does not exist or caller does not own it
    /// - Database operations fail
    pub async fn get_client_rate_limit(
        &self,
        auth_header: Option<&str>,
        client_id: &str,
    ) -> Result<A2ARateLimitStatus, A2AError> {
        let user_id = self.authenticate_and_get_user_id(auth_header)?;
        self.verify_client_ownership(client_id, &user_id).await?;
        self.client_manager
            .get_client_rate_limit_status(client_id)
            .await
    }

    /// Deactivate A2A client (requires ownership)
    ///
    /// # Errors
    ///
    /// Returns `A2AError` if:
    /// - Authentication fails
    /// - Client does not exist or caller does not own it
    /// - Database operations fail
    pub async fn deactivate_client(
        &self,
        auth_header: Option<&str>,
        client_id: &str,
    ) -> Result<(), A2AError> {
        let user_id = self.authenticate_and_get_user_id(auth_header)?;
        self.verify_client_ownership(client_id, &user_id).await?;
        self.client_manager.deactivate_client(client_id).await
    }

    /// Authenticate A2A request
    ///
    /// # Errors
    ///
    /// Returns `A2AError` if:
    /// - Required fields are missing from the request
    /// - Client authentication fails
    /// - Session creation fails
    pub async fn authenticate(&self, request: Value) -> Result<Value, A2AError> {
        // Parse authentication request
        let client_id = request
            .get("client_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| A2AError::InvalidRequest("Missing client_id".into()))?;

        let client_secret = request
            .get("client_secret")
            .and_then(|v| v.as_str())
            .ok_or_else(|| A2AError::InvalidRequest("Missing client_secret".into()))?;

        let explicitly_requested_scopes =
            request.get("scopes").and_then(|v| v.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_owned)
                    .collect::<Vec<String>>()
            });

        // Verify client exists and credentials are valid
        let client = self
            .client_manager
            .get_client(client_id)
            .await?
            .ok_or_else(|| A2AError::AuthenticationFailed("Invalid client_id".into()))?;

        if !client.is_active {
            return Err(A2AError::AuthenticationFailed(
                "Client is deactivated".into(),
            ));
        }

        // Verify client secret BEFORE scope validation to prevent unauthenticated scope probing
        let credentials = self
            .client_manager
            .get_client_credentials(client_id)
            .await?
            .ok_or_else(|| A2AError::AuthenticationFailed("Invalid credentials".into()))?;

        // Hash the provided secret to compare against stored hash
        let provided_hash = format!("{:x}", Sha256::digest(client_secret.as_bytes()));

        // Use constant-time comparison to prevent timing attacks
        let expected_secret = credentials.client_secret.as_bytes();
        let provided_secret = provided_hash.as_bytes();

        // Both secrets must be the same length and content for authentication to succeed
        let secrets_match = expected_secret.len() == provided_secret.len()
            && expected_secret.ct_eq(provided_secret).into();

        if !secrets_match {
            return Err(A2AError::AuthenticationFailed(
                "Invalid client_secret".into(),
            ));
        }

        // Determine granted scopes (only after successful authentication):
        // - If scopes are explicitly requested, validate against client's registered permissions
        // - If no scopes requested, grant the client's full registered permissions
        let granted_scopes = if let Some(requested) = explicitly_requested_scopes {
            let allowed_permissions = &client.permissions;
            if !allowed_permissions.is_empty() {
                for scope in &requested {
                    if !allowed_permissions.contains(scope) {
                        return Err(A2AError::InsufficientPermissions(format!(
                            "Scope '{scope}' is not in client's allowed permissions"
                        )));
                    }
                }
            }
            requested
        } else {
            // No scopes requested: grant all client permissions
            client.permissions.clone()
        };

        // Issue a JWT token (compatible with all handlers that use validate_jwt_and_get_user_id)
        // instead of a session token that would fail JWT validation downstream
        let jwt_token = self
            .ctx
            .auth_manager()
            .generate_client_credentials_token(
                self.ctx.jwks_manager(),
                client_id,
                &granted_scopes,
                None,
            )
            .map_err(|e| {
                A2AError::InternalError(format!("Failed to generate access token: {e}"))
            })?;

        // Also create an A2A session for tracking purposes
        self.ctx
            .repos()
            .a2a
            .create_session(client_id, None, &granted_scopes, 24)
            .await
            .map_err(|e| A2AError::InternalError(format!("Failed to create session: {e}")))?;

        Ok(json!({
            "status": "authenticated",
            "access_token": jwt_token,
            "expires_in": DAY_SECONDS,
            "token_type": "Bearer",
            "scope": granted_scopes.join(" ")
        }))
    }
}

impl Clone for A2ARoutes {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx.clone(),                       // Safe: Arc clone of trait object
            client_manager: self.client_manager.clone(), // Safe: Arc clone for A2A context
            authenticator: self.authenticator.clone(),   // Safe: Arc clone for A2A context
        }
    }
}
