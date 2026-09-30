// ABOUTME: HTTP REST endpoints for admin configuration management
// ABOUTME: Provides endpoints for viewing, updating, and auditing runtime configuration
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin Configuration Routes
//!
//! HTTP endpoints for managing runtime configuration parameters through
//! an admin API. Supports viewing the full catalog, updating values,
//! resetting to defaults, and viewing audit history.

use crate::config::admin::{AdminConfigService, UpdateConfigContext};
use crate::mcp::resources::ServerContext;
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use pierre_auth::security::cookies::{auth_cookie_name, get_cookie_value};
use pierre_config::admin_types::{ConfigScope, ResetConfigRequest, UpdateConfigRequest};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_middleware::{require_admin, PeerAddress};
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_runtime_context::ConfigLookupScope;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

/// Shared state for admin config routes
#[derive(Clone)]
pub struct AdminConfigState {
    /// Admin configuration service
    pub service: Arc<AdminConfigService>,
    /// Server resources for authentication
    pub resources: Arc<ServerContext>,
    /// Validator for admin tokens — what `pierre-cli` holds after `auth login`.
    pub admin_auth: AdminAuthService,
}

impl AdminConfigState {
    /// Create new admin config state
    #[must_use]
    pub const fn new(
        service: Arc<AdminConfigService>,
        resources: Arc<ServerContext>,
        admin_auth: AdminAuthService,
    ) -> Self {
        Self {
            service,
            resources,
            admin_auth,
        }
    }

    /// Authenticate the caller, requiring admin privileges.
    ///
    /// Two credentials reach these routes. The admin console sends the
    /// operator's own session (a user JWT, in the header or the web session
    /// cookie named by [`auth_cookie_name`]). `pierre-cli` sends the super-admin *admin token* its device
    /// login minted — accepted only when its stored row names the approving
    /// super-admin ([`ValidatedAdminToken::operator_user_id`], written by the
    /// device grant alone), the user every write is audited as. A service token minted by `token
    /// generate` names no operator and is refused, whatever its permissions:
    /// `admin_config_overrides.created_by` references `users`.
    ///
    /// Any `Admin`-or-higher account passes: the admin console is a global
    /// operator model, and the operator account is a plain `Admin`, so the
    /// configuration surface — reads and writes, every scope — is theirs
    /// without a further permission or super-admin gate.
    async fn authenticate_admin(&self, headers: &HeaderMap) -> Result<AdminAuthInfo, AppError> {
        let auth_value =
            if let Some(auth_header) = headers.get("authorization").and_then(|h| h.to_str().ok()) {
                auth_header.to_owned()
            } else if let Some(token) = get_cookie_value(headers, &auth_cookie_name()) {
                format!("Bearer {token}")
            } else {
                return Err(AppError::auth_invalid(
                    "Missing authorization header or cookie",
                ));
            };

        let user_id = match self
            .resources
            .auth
            .auth_middleware
            .authenticate_request(Some(&auth_value))
            .await
        {
            Ok(auth) => auth.user_id,
            // A delegated OAuth grant is a genuine user credential refused
            // here, not an admin token to try next: keep its 403.
            Err(delegated) if delegated.code == ErrorCode::PermissionDenied => {
                return Err(delegated)
            }
            // Not an admin token either: the user credential's refusal
            // stands, a spent budget as its 429.
            Err(user_jwt_error) => self
                .device_login_operator(&auth_value)
                .await?
                .ok_or_else(|| user_jwt_error.into_auth_refusal("Authentication failed"))?,
        };

        // Verify admin privileges using centralized guard
        let user = require_admin(user_id, &self.resources.common.repos.users).await?;

        Ok(AdminAuthInfo {
            user_id: user_id.to_string(),
            email: user.email,
        })
    }

    /// The super-admin behind a device-login admin token, or `None` when the
    /// bearer is not an admin token at all (so the user-JWT error stands).
    ///
    /// # Errors
    ///
    /// An admin token that validates but is not super-admin, or names no
    /// operator, is refused outright rather than falling through: the caller
    /// did hold a real credential, and the message says what it lacks.
    async fn device_login_operator(&self, auth_value: &str) -> AppResult<Option<Uuid>> {
        let Some(token) = auth_value.strip_prefix("Bearer ") else {
            return Ok(None);
        };
        let Ok(validated) = self.admin_auth.authenticate(token, None).await else {
            return Ok(None);
        };
        if !validated.is_super_admin {
            return Err(AppError::auth_invalid(
                "Admin token is not super-admin; admin config needs a super-admin device login",
            ));
        }
        let Some(operator_id) = validated.operator_user_id else {
            return Err(AppError::auth_invalid(
                "Admin token names no operator; admin config writes are audited per user, so sign in with `pierre-cli auth login`",
            ));
        };
        let operator = self
            .resources
            .common
            .repos
            .users
            .get_global(operator_id)
            .await?
            .ok_or_else(|| {
                AppError::auth_invalid(
                    "The super-admin who approved this device login no longer exists",
                )
            })?;
        Ok(Some(operator.id))
    }
}

/// Authenticated admin info for audit logging
struct AdminAuthInfo {
    user_id: String,
    email: String,
}

/// Client address recorded on the audit row: the client the trusted proxy
/// chain reports (`TrustedProxies::client_address`), the same address the
/// `OAuth2` rate limits key on. An `X-Forwarded-For` entry a client wrote
/// never becomes it. `None` when the request carries no TCP peer.
///
/// PII — it is stored as an audit field and must not be logged.
fn audit_client_ip(
    state: &AdminConfigState,
    peer: PeerAddress,
    headers: &HeaderMap,
) -> Option<String> {
    let trusted = &state.resources.common.config.rate_limiting.trusted_proxies;
    peer.0
        .map(|peer| trusted.client_address(peer, headers).to_string())
}

/// Client user agent recorded on the audit row.
///
/// PII — it is stored as an audit field and must not be logged.
fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|agent| !agent.is_empty())
}

// ============================================================================
// Request/Response Types
// ============================================================================

/// Scope selector shared by every config endpoint — catalog, update and
/// reset all take the same two dimensions.
///
/// Naming both `user_id` and `tenant_id` is rejected rather than silently
/// picking one: a caller that meant "this user" and a caller that meant
/// "this tenant" must not be served the same row.
#[derive(Debug, Deserialize)]
pub struct ConfigScopeQuery {
    /// Optional tenant ID for tenant-specific changes
    pub tenant_id: Option<String>,
    /// Optional user ID for per-user overrides — the narrowest scope
    pub user_id: Option<String>,
}

impl ConfigScopeQuery {
    /// Resolve the query into the read scope: naming both a user and a tenant
    /// is meaningful here — it asks "what does this user see inside this
    /// tenant", which is what enforcement resolves.
    #[must_use]
    pub fn lookup(&self) -> ConfigLookupScope<'_> {
        ConfigLookupScope {
            user_id: self.user_id.as_deref(),
            tenant_id: self.tenant_id.as_deref(),
        }
    }

    /// Resolve the query into exactly one write scope.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::invalid_input`] when both a user and a tenant are
    /// named, since a stored row belongs to one scope only.
    pub fn scope(&self) -> AppResult<ConfigScope<'_>> {
        match (self.user_id.as_deref(), self.tenant_id.as_deref()) {
            (Some(_), Some(_)) => Err(AppError::invalid_input(
                "Specify user_id or tenant_id, not both — an override row belongs to one scope",
            )),
            (Some(user_id), None) => Ok(ConfigScope::User(user_id)),
            (None, Some(tenant_id)) => Ok(ConfigScope::Tenant(tenant_id)),
            (None, None) => Ok(ConfigScope::Global),
        }
    }
}

/// Generic success response
#[derive(Debug, Serialize)]
pub struct AdminConfigApiResponse<T> {
    /// Whether the operation succeeded
    pub success: bool,
    /// Response data
    pub data: T,
}

// ============================================================================
// Route Handlers
// ============================================================================

/// Get the full configuration catalog with all parameters and metadata
///
/// `GET /api/admin/config/catalog`
///
/// Returns all configuration categories, parameters, current values,
/// defaults, validation rules, and metadata.
///
/// # Errors
///
/// Returns an error if the catalog cannot be retrieved from the database.
pub async fn get_catalog(
    State(state): State<Arc<AdminConfigState>>,
    headers: HeaderMap,
    Query(query): Query<ConfigScopeQuery>,
) -> AppResult<impl IntoResponse> {
    let auth = state.authenticate_admin(&headers).await?;
    info!(
        user_id = %auth.user_id,
        tenant_id = ?query.tenant_id,
        user_id_scope = ?query.user_id,
        "Admin fetching configuration catalog"
    );

    let catalog = state.service.get_catalog(query.lookup()).await?;

    Ok(Json(AdminConfigApiResponse {
        success: true,
        data: catalog,
    }))
}

/// Get current configuration values
///
/// `GET /api/admin/config`
///
/// Returns the current effective configuration values (defaults + overrides)
///
/// # Errors
///
/// Returns an error if the configuration cannot be retrieved from the database.
pub async fn get_config(
    State(state): State<Arc<AdminConfigState>>,
    headers: HeaderMap,
    Query(query): Query<ConfigScopeQuery>,
) -> AppResult<impl IntoResponse> {
    let auth = state.authenticate_admin(&headers).await?;
    info!(
        user_id = %auth.user_id,
        tenant_id = ?query.tenant_id,
        user_id_scope = ?query.user_id,
        "Admin fetching current configuration"
    );

    let catalog = state.service.get_catalog(query.lookup()).await?;

    Ok(Json(AdminConfigApiResponse {
        success: true,
        data: catalog,
    }))
}

/// Update configuration values
///
/// `PUT /api/admin/config`
///
/// Updates one or more configuration parameters
///
/// # Errors
///
/// Returns an error if the update fails due to validation or database errors.
pub async fn update_config(
    State(state): State<Arc<AdminConfigState>>,
    peer: PeerAddress,
    headers: HeaderMap,
    Query(query): Query<ConfigScopeQuery>,
    Json(request): Json<UpdateConfigRequest>,
) -> AppResult<impl IntoResponse> {
    let auth = state.authenticate_admin(&headers).await?;
    let client_ip = audit_client_ip(&state, peer, &headers);
    let scope = query.scope()?;
    let user_id = auth.user_id;
    let user_email = &auth.email;
    info!(
        user_id = %user_id,
        tenant_id = ?query.tenant_id,
        user_id_scope = ?query.user_id,
        parameter_count = request.parameters.len(),
        "Admin updating configuration"
    );

    let response = state
        .service
        .update_config(
            &request,
            UpdateConfigContext {
                admin_user_id: &user_id,
                admin_email: user_email,
                scope,
                ip_address: client_ip.as_deref(),
                user_agent: user_agent(&headers),
            },
        )
        .await?;

    let status = if response.success {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };

    Ok((
        status,
        Json(AdminConfigApiResponse {
            success: response.success,
            data: response,
        }),
    ))
}

/// Reset configuration to defaults
///
/// `POST /api/admin/config/reset`
///
/// Resets configuration parameters to their default values
///
/// # Errors
///
/// Returns an error if the reset operation fails.
pub async fn reset_config(
    State(state): State<Arc<AdminConfigState>>,
    peer: PeerAddress,
    headers: HeaderMap,
    Query(query): Query<ConfigScopeQuery>,
    Json(request): Json<ResetConfigRequest>,
) -> AppResult<impl IntoResponse> {
    let auth = state.authenticate_admin(&headers).await?;
    let client_ip = audit_client_ip(&state, peer, &headers);
    let scope = query.scope()?;
    let user_id = auth.user_id;
    let user_email = &auth.email;
    info!(
        user_id = %user_id,
        tenant_id = ?query.tenant_id,
        user_id_scope = ?query.user_id,
        category = ?request.category,
        "Admin resetting configuration"
    );

    let response = state
        .service
        .reset_config(
            &request,
            UpdateConfigContext {
                admin_user_id: &user_id,
                admin_email: user_email,
                scope,
                ip_address: client_ip.as_deref(),
                user_agent: user_agent(&headers),
            },
        )
        .await?;

    Ok(Json(AdminConfigApiResponse {
        success: response.success,
        data: response,
    }))
}

// ============================================================================
// Router Builder
// ============================================================================

/// Build the admin configuration router
///
/// This creates the router for `/api/admin/config/*` endpoints
pub fn admin_config_router(state: Arc<AdminConfigState>) -> axum::Router {
    use axum::routing::{get, post, put};

    axum::Router::new()
        .route("/catalog", get(get_catalog))
        .route("/", get(get_config))
        .route("/", put(update_config))
        .route("/reset", post(reset_config))
        .with_state(state)
}
