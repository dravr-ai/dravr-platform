// ABOUTME: A2A (Agent-to-Agent) protocol route handlers for inter-agent communication
// ABOUTME: Provides endpoints for agent registration, messaging, and protocol management
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre A2A Routes
//!
//! HTTP routes for the A2A (Agent-to-Agent) protocol:
//!
//! - Client-management surface (`/a2a/clients/*`, `/a2a/dashboard/*`,
//!   `/a2a/status`, `/.well-known/agent-card.json`) implemented as
//!   free-function handlers attached to [`A2ARoutesState`] — the JWT-auth
//!   gate is supplied by [`pierre_middleware::AuthenticatedUser`].
//!
//! - Service-style protocol surface ([`service::A2ARoutes`]) used by the
//!   transport layer to dispatch JSON-RPC requests (`/a2a/auth`,
//!   `/a2a/message`, etc.) — wraps the client manager, the authenticator,
//!   and the universal tool executor.
//!
//! ## Composition root state
//!
//! Because [`pierre_a2a`] already depends on [`pierre_runtime_context`] (for
//! the [`A2ACtx`] trait), the reverse dependency cannot be encoded as new
//! methods on `A2ACtx`. Instead this crate exposes [`A2ARoutesState`]: a
//! generic struct that pairs an `Arc<C>` (where `C: MiddlewareCtx + A2ACtx`)
//! with the concrete [`A2AClientManager`] handle, the [`McpAuthMiddleware`]
//! handle, and the [`ToolRuntime`] trait object. The composition root in
//! `pierre-server` constructs the state from its `Arc<ServerContext>`
//! (which implements every trait and holds every handle) and hands it to
//! [`A2ARoutes::routes`]. A local [`A2AAuth`] extractor mirrors
//! `pierre_middleware::AuthenticatedUser` against this larger state type
//! (Rust's orphan rules forbid adding the foreign `FromRequestParts` impl
//! against the foreign extractor type directly).

/// Service layer for A2A protocol operations (JSON-RPC dispatch surface).
pub mod service;

use axum::{
    extract::{FromRequestParts, Path, State},
    http::{header::AUTHORIZATION, request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use pierre_a2a::{
    agent_card::AgentCard,
    client::{A2AClientManager, ClientRegistrationRequest},
    A2ARequest, A2AServer,
};
use pierre_auth::auth::AuthResult;
use pierre_core::auth_header::extract_bearer_token;
use pierre_core::errors::AppError;
use pierre_middleware::{extract_auth_from_headers, McpAuthMiddleware};
use pierre_runtime_context::{A2ACtx, MiddlewareCtx};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::Arc;
use tokio::task;
use tracing::{error, info};

/// Local axum extractor that authenticates the request against the
/// composition-root context held inside [`A2ARoutesState`].
///
/// `pierre_middleware::AuthenticatedUser` implements
/// `FromRequestParts<Arc<C>>` directly, so it can only run against handlers
/// whose state is exactly `Arc<C>`. The routes in this crate use a richer
/// state type, so we wrap the same `extract_auth_from_headers` helper in a
/// local extractor implemented over [`A2ARoutesState<C>`] — Rust's orphan
/// rules forbid adding the impl to the foreign extractor directly.
pub struct A2AAuth(pub AuthResult);

impl A2AAuth {
    /// Consume the extractor and return the inner [`AuthResult`].
    #[must_use]
    pub fn into_inner(self) -> AuthResult {
        self.0
    }
}

impl<C: MiddlewareCtx + A2ACtx> FromRequestParts<A2ARoutesState<C>> for A2AAuth {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &A2ARoutesState<C>,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let headers = parts.headers.clone();
        let ctx = state.ctx.clone(); // Safe: Arc clone for async block
        async move {
            let auth_result = extract_auth_from_headers(&headers, &ctx).await?;
            Ok(Self(auth_result))
        }
    }
}

/// Shared state passed to the A2A client-management route handlers.
///
/// Generic over the composition-root context `C`, which must satisfy
/// both [`MiddlewareCtx`] (so the local [`A2AAuth`] extractor can run
/// against it) and [`A2ACtx`] (so the handlers can read `repos.a2a.*` and
/// the configured base URL). Holds the disparate handles `pierre-a2a` and
/// `pierre-tool-runtime` define — these intentionally do not live on
/// `A2ACtx` because the corresponding types would close a dependency
/// cycle through `pierre-runtime-context`.
pub struct A2ARoutesState<C: MiddlewareCtx + A2ACtx> {
    /// Composition-root context (auth manager, JWKS, repos, CSRF, base URL).
    pub ctx: Arc<C>,
    /// A2A client manager — registration, credential lookups, rate-limit status.
    pub client_manager: Arc<A2AClientManager>,
    /// MCP auth middleware — required when constructing the
    /// [`pierre_a2a::auth::A2AAuthenticator`] for the API-key flow.
    pub auth_middleware: Arc<McpAuthMiddleware>,
    /// Tool runtime — backs the universal tool executor used by the
    /// JSON-RPC `tools.execute` dispatch path in [`service::A2ARoutes`].
    pub tool_runtime: Arc<dyn ToolRuntime>,
}

// Manual `Clone` impl: derive(Clone) would require `C: Clone`, but every
// field is already an `Arc`, so cloning each one is the cheap correct
// operation.
impl<C: MiddlewareCtx + A2ACtx> Clone for A2ARoutesState<C> {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx.clone(), // Safe: Arc clone for shared context
            client_manager: self.client_manager.clone(), // Safe: Arc clone for shared client manager
            auth_middleware: self.auth_middleware.clone(), // Safe: Arc clone for shared middleware
            tool_runtime: self.tool_runtime.clone(),     // Safe: Arc clone of trait object
        }
    }
}

/// Response for A2A client list
#[derive(Debug, Serialize)]
pub struct A2AClientResponse {
    /// Unique client identifier
    pub id: String,
    /// Human-readable client name
    pub name: String,
    /// Description of the client application
    pub description: String,
    /// Public key for identification
    pub public_key: String,
    /// List of capabilities this client can access
    pub capabilities: Vec<String>,
    /// List of permissions granted to this client
    pub permissions: Vec<String>,
    /// Whether this client is active
    pub is_active: bool,
    /// When this client was created
    pub created_at: String,
    /// When this client was last updated
    pub updated_at: String,
    /// Rate limit for requests per window
    pub rate_limit_requests: u32,
    /// Rate limit window in seconds
    pub rate_limit_window_seconds: u32,
}

/// Request to create a new A2A client
#[derive(Debug, Deserialize)]
struct CreateA2AClientRequest {
    /// Name of the client application
    name: String,
    /// Description of the client's purpose
    description: String,
    /// List of agent capabilities this client provides
    capabilities: Vec<String>,
    /// `OAuth2` redirect URIs for authorization flows (optional)
    #[serde(default)]
    redirect_uris: Vec<String>,
    /// Contact email for the client administrator
    contact_email: String,
}

/// Response for created A2A client with credentials
#[derive(Debug, Serialize)]
struct CreateA2AClientResponse {
    /// Unique client identifier
    client_id: String,
    /// Client secret for authentication
    client_secret: String,
    /// API key for direct API access
    api_key: String,
    /// Ed25519 public key for signature verification
    public_key: String,
    /// Ed25519 private key for signing
    private_key: String,
    /// Key type identifier
    key_type: String,
}

/// A2A routes implementation
pub struct A2ARoutes;

impl A2ARoutes {
    /// Create all A2A routes
    ///
    /// Routes match frontend API expectations:
    /// - /a2a/status - Basic A2A protocol status
    /// - /a2a/clients - List A2A clients
    /// - /a2a/clients/{id} - Get/delete A2A client
    /// - /a2a/dashboard/overview - Dashboard overview for A2A clients
    /// - /a2a/dashboard/analytics - Usage analytics for A2A clients
    /// - /.well-known/agent-card.json - Agent card discovery
    pub fn routes<C: MiddlewareCtx + A2ACtx>(state: A2ARoutesState<C>) -> Router {
        Router::new()
            // Public routes (no auth required)
            .route("/a2a/status", get(Self::handle_status))
            .route(
                "/.well-known/agent-card.json",
                get(Self::handle_agent_card_discovery::<C>),
            )
            // JSON-RPC transport endpoint advertised by the agent card.
            // Per-method auth is enforced inside `A2AServer::handle_request`,
            // so the route itself is not behind the `A2AAuth` extractor.
            .route("/a2a/jsonrpc", post(Self::handle_jsonrpc::<C>))
            // Client management routes (auth required)
            .route(
                "/a2a/clients",
                get(Self::handle_list_clients::<C>).post(Self::handle_create_client::<C>),
            )
            .route(
                "/a2a/clients/{client_id}",
                get(Self::handle_get_client::<C>),
            )
            .route(
                "/a2a/clients/{client_id}",
                delete(Self::handle_delete_client::<C>),
            )
            .route(
                "/a2a/clients/{client_id}/usage",
                get(Self::handle_client_usage::<C>),
            )
            .route(
                "/a2a/clients/{client_id}/rate-limit",
                get(Self::handle_client_rate_limit::<C>),
            )
            // Dashboard routes
            .route(
                "/a2a/dashboard/overview",
                get(Self::handle_dashboard_overview::<C>),
            )
            .route(
                "/a2a/dashboard/analytics",
                get(Self::handle_dashboard_analytics::<C>),
            )
            .with_state(state)
    }

    /// Handle A2A status (public endpoint)
    async fn handle_status() -> Json<serde_json::Value> {
        // Yield to scheduler for cooperative multitasking
        task::yield_now().await;
        Json(serde_json::json!({
            "status": "active"
        }))
    }

    /// Handle agent card discovery endpoint (public endpoint)
    async fn handle_agent_card_discovery<C: MiddlewareCtx + A2ACtx>(
        State(state): State<A2ARoutesState<C>>,
    ) -> Json<AgentCard> {
        // Yield to scheduler for cooperative multitasking
        task::yield_now().await;
        Json(AgentCard::with_base_url(state.ctx.base_url()))
    }

    /// Handle the A2A JSON-RPC transport endpoint (`/a2a/jsonrpc`).
    ///
    /// This is the endpoint advertised by the agent card's `jsonrpc`
    /// transport. It deserializes a JSON-RPC `A2ARequest` body and dispatches
    /// it to [`A2AServer::handle_request`], which enforces per-method
    /// authentication. A bearer token supplied via the `Authorization` header
    /// is folded into the request's `auth` field when the body omits it, so
    /// discovering agents can authenticate the standard HTTP way.
    async fn handle_jsonrpc<C: MiddlewareCtx + A2ACtx>(
        State(state): State<A2ARoutesState<C>>,
        headers: HeaderMap,
        Json(mut request): Json<A2ARequest>,
    ) -> Response {
        // Fold the Authorization-header bearer into the request body when the
        // JSON-RPC payload did not carry its own `auth` field.
        if request.auth_token.is_none() {
            if let Some(token) = headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|header| extract_bearer_token(header).ok())
            {
                request.auth_token = Some(token.to_owned());
            }
        }

        let ctx: Arc<dyn A2ACtx> = state.ctx.clone(); // Safe: Arc clone coerced into trait object
        let server = A2AServer::new_with_resources(ctx, state.tool_runtime.clone()); // Safe: Arc clone of trait object
        let response = server.handle_request(request).await;

        (StatusCode::OK, Json(response)).into_response()
    }

    /// List all A2A clients for authenticated user
    async fn handle_list_clients<C: MiddlewareCtx + A2ACtx>(
        State(state): State<A2ARoutesState<C>>,
        auth: A2AAuth,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let user_id = auth.user_id;

        let clients = A2ACtx::repos(state.ctx.as_ref())
            .a2a
            .list_clients(&user_id)
            .await?;

        let response: Vec<A2AClientResponse> = clients
            .into_iter()
            .map(|c| A2AClientResponse {
                id: c.id,
                name: c.name,
                description: c.description,
                public_key: c.public_key,
                capabilities: c.capabilities,
                permissions: c.permissions,
                is_active: c.is_active,
                created_at: c.created_at.to_rfc3339(),
                updated_at: c.updated_at.to_rfc3339(),
                rate_limit_requests: c.rate_limit_requests,
                rate_limit_window_seconds: c.rate_limit_window_seconds,
            })
            .collect();

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Create a new A2A client
    async fn handle_create_client<C: MiddlewareCtx + A2ACtx>(
        State(state): State<A2ARoutesState<C>>,
        auth: A2AAuth,
        Json(request): Json<CreateA2AClientRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        info!(
            user_id = %auth.user_id,
            client_name = %request.name,
            "Creating new A2A client"
        );

        // Convert to the A2A client registration request format
        let registration_request = ClientRegistrationRequest {
            name: request.name,
            description: request.description,
            capabilities: request.capabilities,
            redirect_uris: request.redirect_uris,
            contact_email: request.contact_email,
        };

        // Register the client using the A2A client manager with the authenticated user's ID
        let credentials = state
            .client_manager
            .register_client(registration_request, auth.user_id)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to register A2A client");
                AppError::internal(format!("Failed to register A2A client: {e}"))
            })?;

        let response = CreateA2AClientResponse {
            client_id: credentials.client_id,
            client_secret: credentials.client_secret,
            api_key: credentials.api_key,
            public_key: credentials.public_key,
            private_key: credentials.private_key,
            key_type: credentials.key_type,
        };

        info!(
            client_id = %response.client_id,
            "A2A client created successfully"
        );

        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// Get a specific A2A client
    async fn handle_get_client<C: MiddlewareCtx + A2ACtx>(
        State(state): State<A2ARoutesState<C>>,
        auth: A2AAuth,
        Path(client_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let user_id = auth.user_id;

        let client = A2ACtx::repos(state.ctx.as_ref())
            .a2a
            .get_client(&client_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("A2A client {client_id}")))?;

        // Check ownership
        if client.user_id != user_id {
            return Err(AppError::not_found(format!("A2A client {client_id}")));
        }

        let response = A2AClientResponse {
            id: client.id,
            name: client.name,
            description: client.description,
            public_key: client.public_key,
            capabilities: client.capabilities,
            permissions: client.permissions,
            is_active: client.is_active,
            created_at: client.created_at.to_rfc3339(),
            updated_at: client.updated_at.to_rfc3339(),
            rate_limit_requests: client.rate_limit_requests,
            rate_limit_window_seconds: client.rate_limit_window_seconds,
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Delete (deactivate) an A2A client
    async fn handle_delete_client<C: MiddlewareCtx + A2ACtx>(
        State(state): State<A2ARoutesState<C>>,
        auth: A2AAuth,
        Path(client_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let user_id = auth.user_id;

        // Verify ownership
        let client = A2ACtx::repos(state.ctx.as_ref())
            .a2a
            .get_client(&client_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("A2A client {client_id}")))?;

        if client.user_id != user_id {
            return Err(AppError::not_found(format!("A2A client {client_id}")));
        }

        // Deactivate client
        A2ACtx::repos(state.ctx.as_ref())
            .a2a
            .deactivate_client(&client_id)
            .await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "message": "Client deactivated successfully",
                "client_id": client_id
            })),
        )
            .into_response())
    }

    /// Get A2A client usage statistics
    async fn handle_client_usage<C: MiddlewareCtx + A2ACtx>(
        State(state): State<A2ARoutesState<C>>,
        auth: A2AAuth,
        Path(client_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let user_id = auth.user_id;

        // Verify ownership
        let client = A2ACtx::repos(state.ctx.as_ref())
            .a2a
            .get_client(&client_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("A2A client {client_id}")))?;

        if client.user_id != user_id {
            return Err(AppError::not_found(format!("A2A client {client_id}")));
        }

        // Get current usage count
        let current_usage = A2ACtx::repos(state.ctx.as_ref())
            .a2a
            .get_client_current_usage(&client_id)
            .await
            .unwrap_or(0);

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "client_id": client_id,
                "total_requests": current_usage,
                "requests_today": 0,
                "daily_usage": []
            })),
        )
            .into_response())
    }

    /// Get A2A client rate limit status
    async fn handle_client_rate_limit<C: MiddlewareCtx + A2ACtx>(
        State(state): State<A2ARoutesState<C>>,
        auth: A2AAuth,
        Path(client_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let user_id = auth.user_id;

        // Verify ownership and get client
        let client = A2ACtx::repos(state.ctx.as_ref())
            .a2a
            .get_client(&client_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("A2A client {client_id}")))?;

        if client.user_id != user_id {
            return Err(AppError::not_found(format!("A2A client {client_id}")));
        }

        let current_usage = A2ACtx::repos(state.ctx.as_ref())
            .a2a
            .get_client_current_usage(&client_id)
            .await
            .unwrap_or(0);

        let limit = client.rate_limit_requests;
        let remaining = limit.saturating_sub(current_usage);

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "client_id": client_id,
                "rate_limit_requests": limit,
                "rate_limit_window_seconds": client.rate_limit_window_seconds,
                "current_usage": current_usage,
                "remaining": remaining,
                "reset_at": Utc::now().to_rfc3339()
            })),
        )
            .into_response())
    }

    /// Handle A2A dashboard overview
    async fn handle_dashboard_overview<C: MiddlewareCtx + A2ACtx>(
        State(state): State<A2ARoutesState<C>>,
        auth: A2AAuth,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let user_id = auth.user_id;

        let clients = A2ACtx::repos(state.ctx.as_ref())
            .a2a
            .list_clients(&user_id)
            .await?;
        let active_count = clients.iter().filter(|c| c.is_active).count();

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "total_clients": clients.len(),
                "active_clients": active_count,
                "total_requests": 0,
                "requests_today": 0,
                "status": "active"
            })),
        )
            .into_response())
    }

    /// Handle A2A dashboard analytics
    async fn handle_dashboard_analytics<C: MiddlewareCtx + A2ACtx>(
        State(_state): State<A2ARoutesState<C>>,
        _: A2AAuth,
    ) -> Result<Response, AppError> {
        // Yield to scheduler for cooperative multitasking
        task::yield_now().await;
        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "daily_requests": [],
                "top_clients": [],
                "request_types": {},
                "period_days": 30
            })),
        )
            .into_response())
    }
}
