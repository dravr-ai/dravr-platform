// ABOUTME: Tenant route handler for multi-tenant sessions
// ABOUTME: Provides the REST endpoint that switches a session's active tenant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tenant routes
//!
//! Users can belong to multiple tenants (like Slack workspaces or GitHub organizations).
//! The active tenant for a session is determined by the `active_tenant_id` claim in the JWT.
//! Use the POST /tenants/switch endpoint to change the active tenant and receive a new JWT.
//! The handler requires valid JWT authentication.

use crate::mcp::resources::ServerContext;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_middleware::AuthenticatedUser;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

/// Request body for switching active tenant
#[derive(Debug, Deserialize)]
pub struct SwitchTenantRequest {
    /// UUID of the tenant to switch to
    pub tenant_id: String,
}

/// Response after successful tenant switch
#[derive(Debug, Serialize)]
pub struct SwitchTenantResponse {
    /// New JWT token with the `active_tenant_id` claim set
    #[serde(serialize_with = "pierre_core::secret::serialize_exposed")]
    pub token: SecretString,
    /// Tenant ID that is now active
    pub active_tenant_id: String,
    /// Tenant name for display
    pub tenant_name: String,
    /// User's role in this tenant
    pub role: String,
    /// Token expiration time in seconds
    pub expires_in: u64,
}

/// Tenant routes
#[derive(Debug)]
pub struct TenantRoutes;

impl TenantRoutes {
    /// Create the tenant routes
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/tenants/switch", post(Self::handle_switch_tenant))
            .with_state(resources)
    }

    /// Handle switching active tenant
    ///
    /// Validates that the user belongs to the target tenant, then returns a new JWT
    /// with the `active_tenant_id` claim set to the specified tenant.
    async fn handle_switch_tenant(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Json(request): Json<SwitchTenantRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        // The switch mints a token for the new tenant: never from an API key,
        // and an impersonation stays an impersonation.
        let impersonation = auth.auth_method.impersonation().cloned();
        if impersonation.is_none() {
            auth.auth_method.refuse_session_upgrade()?;
        }

        info!(
            user_id = %auth.user_id,
            target_tenant = %request.tenant_id,
            "Processing tenant switch request"
        );

        // Parse the target tenant ID
        let tenant_id = TenantId::parse_str(&request.tenant_id).map_err(|e| {
            warn!(tenant_id = %request.tenant_id, error = %e, "Invalid tenant ID format");
            AppError::invalid_input(format!("Invalid tenant ID format: {e}"))
        })?;

        // Verify user belongs to this tenant via tenant_users table
        let role_str = resources
            .common
            .repos
            .tenants
            .get_user_role(auth.user_id, tenant_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to check tenant membership: {e}")))?
            .ok_or_else(|| {
                warn!(
                    user_id = %auth.user_id,
                    tenant_id = %tenant_id,
                    "User attempted to switch to tenant they don't belong to"
                );
                AppError::auth_invalid(format!("User does not belong to tenant {tenant_id}"))
            })?;

        // Get tenant details
        let tenant = resources
            .common
            .repos
            .tenants
            .get_by_id(tenant_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get tenant: {e}")))?;

        // SECURITY: Global lookup — tenant JWT refresh, user verified via auth middleware
        let user = resources
            .common
            .repos
            .users
            .get_global(auth.user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user: {e}")))?
            .ok_or_else(|| AppError::not_found("User"))?;

        // Generate new JWT with active_tenant_id set: the same impersonation
        // (admin, session, no later expiry) for an impersonation token,
        // else a session still naming the client it was signed in to
        let auth_manager = &resources.auth.auth_manager;
        let jwks_manager = &resources.auth.jwks_manager;
        let active_tenant = Some(tenant_id.to_string());
        let token = match &impersonation {
            Some(grant) => {
                auth_manager.reissue_impersonation_token(&user, grant, jwks_manager, active_tenant)
            }
            None => auth_manager.generate_client_session_token(
                &user,
                jwks_manager,
                active_tenant,
                auth.client_id.as_deref(),
            ),
        }
        .map_err(|e| AppError::internal(format!("Failed to generate token: {e}")))?;
        // A reissued impersonation token expires when the one it replaces
        // does, which is never past its hour; a session lasts the default day
        let expires_in = impersonation.as_ref().map_or(86400, |grant| {
            u64::try_from(grant.expires_at - Timestamp::now().epoch_seconds()).unwrap_or(0)
        });

        info!(
            user_id = %auth.user_id,
            tenant_id = %tenant_id,
            tenant_name = %tenant.name,
            "Successfully switched tenant context"
        );

        Ok((
            StatusCode::OK,
            Json(SwitchTenantResponse {
                token: SecretString::from(token),
                active_tenant_id: tenant_id.to_string(),
                tenant_name: tenant.name,
                role: role_str,
                expires_in,
            }),
        )
            .into_response())
    }
}
