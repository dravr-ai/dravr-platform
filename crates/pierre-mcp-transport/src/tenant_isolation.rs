// ABOUTME: Tenant isolation and multi-tenancy management for MCP server
// ABOUTME: Handles user validation, tenant context extraction, and access control
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use http::HeaderMap;
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::{AuthManager, Claims};
use pierre_auth::tenant::{TenantContext, TenantRole};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::models::User;
use pierre_core::uuid_utils::parse_uuid;
use pierre_runtime_context::McpDispatchCtx;
// Trait methods dispatched through repos.tenants / repos.users / repos.oauth_tokens
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

/// Manages tenant isolation and multi-tenancy for the MCP server
pub struct TenantIsolation {
    resources: Arc<dyn McpDispatchCtx>,
}

impl TenantIsolation {
    /// Create a new tenant isolation manager
    #[must_use]
    pub fn new(resources: Arc<dyn McpDispatchCtx>) -> Self {
        Self { resources }
    }

    /// Validate JWT token and extract tenant context
    ///
    /// The active tenant is determined from the JWT claims `active_tenant_id` field.
    /// If no active tenant is specified, the user's default tenant is used.
    ///
    /// # Errors
    /// Returns an error if JWT validation fails or tenant information cannot be retrieved
    pub async fn validate_tenant_access(&self, jwt_token: &str) -> AppResult<TenantContext> {
        let claims = self
            .resources
            .auth_manager()
            .validate_token(jwt_token, self.resources.jwks_manager())
            .map_err(|e| AppError::auth_invalid(format!("Failed to validate token: {e}")))?;

        // Parse user ID from claims
        let user_id = parse_uuid(&claims.sub).map_err(|e| {
            warn!(sub = %claims.sub, error = %e, "Invalid user ID in JWT token claims");
            AppError::auth_invalid("Invalid user ID in token")
        })?;

        // Get tenant ID from JWT claims (active_tenant_id) or fall back to default tenant
        let tenant_id = self
            .extract_tenant_from_claims_or_default(&claims, user_id)
            .await?;
        let tenant_name = self.get_tenant_name(tenant_id).await;
        let user_role = self.get_user_role_for_tenant(user_id, tenant_id).await?;

        // The JWT `jti` is the Guardian turn token (the ACP bridge mints one
        // token per turn, so all of a turn's native tool calls share it).
        Ok(
            TenantContext::from_verified_membership(tenant_id, tenant_name, user_id, user_role)
                .with_session_id(Some(claims.jti.clone())),
        )
    }

    /// Extract tenant ID from JWT claims or get user's default tenant
    ///
    /// # Errors
    /// Returns an error if user has no tenant memberships
    async fn extract_tenant_from_claims_or_default(
        &self,
        claims: &Claims,
        user_id: Uuid,
    ) -> AppResult<TenantId> {
        // Check if active_tenant_id is specified in JWT claims
        if let Some(tenant_id_str) = claims.active_tenant_id.as_deref() {
            let tenant_id: TenantId = tenant_id_str.parse().map_err(|e| {
                warn!(tenant_id = %tenant_id_str, error = %e, "Invalid tenant ID format in JWT claims");
                AppError::invalid_input("Invalid tenant ID format in token")
            })?;

            // Verify user actually belongs to this tenant
            self.verify_user_tenant_membership(user_id, tenant_id)
                .await?;

            return Ok(tenant_id);
        }

        // No active tenant in claims - get user's default tenant
        self.get_user_default_tenant(user_id).await
    }

    /// Verify user belongs to a tenant via `tenant_users` table
    ///
    /// # Errors
    /// Returns an error if user does not belong to the tenant
    pub async fn verify_user_tenant_membership(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<()> {
        let role = self
            .resources
            .repos()
            .tenants
            .get_user_role(user_id, tenant_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to check tenant membership: {e}")))?;

        if role.is_none() {
            return Err(AppError::auth_invalid(format!(
                "User {user_id} does not belong to tenant {tenant_id}"
            )));
        }

        Ok(())
    }

    /// Get user's default tenant (first tenant they belong to)
    ///
    /// # Errors
    /// Returns an error if user has no tenant memberships
    pub async fn get_user_default_tenant(&self, user_id: Uuid) -> AppResult<TenantId> {
        let tenants = self
            .resources
            .repos()
            .tenants
            .list_for_user(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenants: {e}")))?;

        tenants
            .first()
            .map(|t| t.id)
            .ok_or_else(|| AppError::auth_invalid("User does not belong to any tenant"))
    }

    /// Get user with tenant information
    ///
    /// # Errors
    /// Returns an error if user lookup fails
    pub async fn get_user_with_tenant(&self, user_id: Uuid) -> AppResult<User> {
        // SECURITY: Global lookup — tenant isolation resolves user to find their tenant
        self.resources
            .repos()
            .users
            .get_global(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user: {e}")))?
            .ok_or_else(|| AppError::not_found("User"))
    }

    /// Get user's default tenant ID
    ///
    /// This method looks up the user's tenant memberships in the `tenant_users` table
    /// and returns the first tenant (typically the oldest membership).
    ///
    /// # Errors
    /// Returns an error if user has no tenant memberships
    pub async fn extract_tenant_id_for_user(&self, user: &User) -> AppResult<TenantId> {
        self.get_user_default_tenant(user.id).await
    }

    /// Get tenant name by ID
    pub async fn get_tenant_name(&self, tenant_id: TenantId) -> String {
        match self.resources.repos().tenants.get_by_id(tenant_id).await {
            Ok(tenant) => tenant.name,
            Err(e) => {
                warn!(
                    "Failed to get tenant {}: {}, using default name",
                    tenant_id, e
                );
                "Unknown Tenant".to_owned()
            }
        }
    }

    /// Get user's role in a tenant
    ///
    /// Uses the `tenant_users` junction table to determine the user's role.
    /// This is the source of truth for multi-tenant membership.
    ///
    /// # Errors
    /// Returns an error if role lookup fails or user doesn't belong to tenant
    pub async fn get_user_role_for_tenant(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<TenantRole> {
        // Query tenant_users table for user's role in the tenant
        let role_str = self
            .resources
            .repos()
            .tenants
            .get_user_role(user_id, tenant_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenant role: {e}")))?
            .ok_or_else(|| {
                AppError::auth_invalid(format!(
                    "User {user_id} does not belong to tenant {tenant_id}"
                ))
            })?;

        Ok(TenantRole::from_db_string(&role_str))
    }

    /// Extract tenant context from request headers
    ///
    /// # Errors
    /// Returns an error if header parsing fails
    pub async fn extract_tenant_from_header(
        &self,
        headers: &HeaderMap,
    ) -> AppResult<Option<TenantContext>> {
        // Look for tenant ID in headers
        if let Some(tenant_id_header) = headers.get("x-tenant-id") {
            let tenant_id_str = tenant_id_header.to_str().map_err(|e| {
                warn!(error = %e, "Invalid x-tenant-id header format (non-UTF8)");
                AppError::invalid_input("Invalid tenant ID header format")
            })?;

            let tenant_id: TenantId = tenant_id_str
                .parse()
                .map_err(|e| {
                    warn!(tenant_id = %tenant_id_str, error = %e, "Invalid tenant ID format in x-tenant-id header");
                    AppError::invalid_input("Invalid tenant ID format")
                })?;

            let tenant_name = self.get_tenant_name(tenant_id).await;

            // Header-derived: there is no user and no membership lookup, so no
            // role is established. The constructor says so, rather than filling
            // the field with a placeholder role the type would then present as
            // verified.
            return Ok(Some(TenantContext::for_tenant_scoped_operation(
                tenant_id,
                tenant_name,
                Uuid::nil(), // No user context available from headers
            )));
        }

        Ok(None)
    }

    /// Extract tenant context from user (using their default tenant)
    ///
    /// # Errors
    /// Returns an error if user lookup or tenant extraction fails
    pub async fn extract_tenant_from_user(&self, user_id: Uuid) -> AppResult<TenantContext> {
        let tenant_id = self.get_user_default_tenant(user_id).await?;
        let tenant_name = self.get_tenant_name(tenant_id).await;
        let user_role = self.get_user_role_for_tenant(user_id, tenant_id).await?;

        Ok(TenantContext::from_verified_membership(
            tenant_id,
            tenant_name,
            user_id,
            user_role,
        ))
    }

    /// Extract tenant context from user with a specific tenant ID
    ///
    /// # Errors
    /// Returns an error if user doesn't belong to tenant
    pub async fn extract_tenant_from_user_with_tenant(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<TenantContext> {
        // Verify user belongs to this tenant
        self.verify_user_tenant_membership(user_id, tenant_id)
            .await?;

        let tenant_name = self.get_tenant_name(tenant_id).await;
        let user_role = self.get_user_role_for_tenant(user_id, tenant_id).await?;

        Ok(TenantContext::from_verified_membership(
            tenant_id,
            tenant_name,
            user_id,
            user_role,
        ))
    }

    /// Check if user has access to a specific resource
    ///
    /// # Errors
    /// Returns an error if role lookup fails
    pub async fn check_resource_access(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        resource_type: &str,
    ) -> AppResult<bool> {
        // Verify user belongs to the tenant
        let user_role = self.get_user_role_for_tenant(user_id, tenant_id).await?;

        // Basic access control - can be extended based on requirements
        match resource_type {
            "oauth_credentials" => Ok(matches!(user_role, TenantRole::Owner | TenantRole::Member)),
            "fitness_data" => Ok(matches!(user_role, TenantRole::Owner | TenantRole::Member)),
            "tenant_settings" => Ok(matches!(user_role, TenantRole::Owner)),
            _ => {
                warn!("Unknown resource type: {}", resource_type);
                Ok(false)
            }
        }
    }

    /// Validate that a user can perform an action on behalf of a tenant
    ///
    /// # Errors
    /// Returns an error if validation fails
    pub async fn validate_tenant_action(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        action: &str,
    ) -> AppResult<()> {
        let user_role = self.get_user_role_for_tenant(user_id, tenant_id).await?;

        match action {
            "read_oauth_credentials" | "store_oauth_credentials" => {
                if matches!(user_role, TenantRole::Owner | TenantRole::Member) {
                    Ok(())
                } else {
                    Err(AppError::auth_invalid(format!(
                        "User {user_id} does not have permission to {action} for tenant {tenant_id}"
                    )))
                }
            }
            "modify_tenant_settings" => {
                if matches!(user_role, TenantRole::Owner) {
                    Ok(())
                } else {
                    Err(AppError::auth_invalid(format!(
                        "User {user_id} does not have owner permission for tenant {tenant_id}"
                    )))
                }
            }
            _ => {
                warn!("Unknown action for validation: {}", action);
                Err(AppError::invalid_input(format!("Unknown action: {action}")))
            }
        }
    }
}

/// JWT token validation result
#[derive(Debug, Clone)]
pub struct JwtValidationResult {
    /// User ID extracted from the JWT token
    pub user_id: Uuid,
    /// Tenant context associated with the user
    pub tenant_context: TenantContext,
    /// When the JWT token expires
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Standalone function for JWT validation (used by HTTP middleware)
///
/// The active tenant is determined from the JWT claims `active_tenant_id` field.
/// If no active tenant is specified, the user's default tenant is used.
///
/// # Errors
/// Returns an error if JWT validation or user lookup fails
pub async fn validate_jwt_token_for_mcp(
    token: &str,
    auth_manager: &AuthManager,
    jwks_manager: &JwksManager,
    repos: &Arc<pierre_database::RepositoryRegistry>,
) -> AppResult<JwtValidationResult> {
    let claims = auth_manager
        .validate_token(token, jwks_manager)
        .map_err(|e| AppError::auth_invalid(format!("Failed to validate token: {e}")))?;

    // Parse user ID from claims
    let user_id = parse_uuid(&claims.sub).map_err(|e| {
        warn!(sub = %claims.sub, error = %e, "Invalid user ID in JWT token claims (MCP validation)");
        AppError::auth_invalid("Invalid user ID in token")
    })?;

    // SECURITY: Global lookup — tenant extraction from JWT, tenant not yet known
    repos
        .users
        .get_global(user_id)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user: {e}")))?
        .ok_or_else(|| AppError::not_found("User"))?;

    // Get tenant ID from JWT claims or fall back to user's default tenant
    let tenant_id: TenantId = if let Some(tenant_id_str) = claims.active_tenant_id.as_deref() {
        let tid: TenantId = tenant_id_str.parse().map_err(|e| {
            warn!(tenant_id = %tenant_id_str, error = %e, "Invalid tenant ID format in JWT claims (MCP validation)");
            AppError::invalid_input("Invalid tenant ID format in token")
        })?;

        // Verify user belongs to this tenant
        let role = repos
            .tenants
            .get_user_role(user_id, tid)
            .await
            .map_err(|e| AppError::database(format!("Failed to check tenant membership: {e}")))?;

        if role.is_none() {
            return Err(AppError::auth_invalid(format!(
                "User {user_id} does not belong to tenant {tid}"
            )));
        }

        tid
    } else {
        // Get user's default tenant
        let tenants = repos
            .tenants
            .list_for_user(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenants: {e}")))?;

        tenants
            .first()
            .map(|t| t.id)
            .ok_or_else(|| AppError::auth_invalid("User does not belong to any tenant"))?
    };

    let tenant_name = match repos.tenants.get_by_id(tenant_id).await {
        Ok(tenant) => tenant.name,
        _ => "Unknown Tenant".to_owned(),
    };

    // Get user's role in this tenant
    let user_role = repos
        .tenants
        .get_user_role(user_id, tenant_id)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user tenant role: {e}")))?
        .map_or(TenantRole::Member, |role_str| {
            TenantRole::from_db_string(&role_str)
        });

    // JWT `jti` as the Guardian turn token for the MCP/headless path.
    let tenant_context =
        TenantContext::from_verified_membership(tenant_id, tenant_name, user_id, user_role)
            .with_session_id(Some(claims.jti.clone()));

    // Expiry is the JWT's own `exp` claim (unix seconds), already verified by validate_token.
    let expires_at = chrono::DateTime::from_timestamp(claims.exp, 0)
        .ok_or_else(|| AppError::auth_invalid("Invalid expiry timestamp in token claims"))?;

    Ok(JwtValidationResult {
        user_id,
        tenant_context,
        expires_at,
    })
}

/// Extract tenant context from various sources (internal helper)
///
/// Priority order:
/// 1. Explicit `tenant_id` parameter
/// 2. `x-tenant-id` header
/// 3. User's default tenant (from `tenant_users` table)
///
/// # Errors
/// Returns an error if tenant extraction fails
pub async fn extract_tenant_context_internal(
    repos: &Arc<pierre_database::RepositoryRegistry>,
    user_id: Option<Uuid>,
    tenant_id: Option<TenantId>,
    headers: Option<&HeaderMap>,
) -> AppResult<Option<TenantContext>> {
    // Try to extract from explicit tenant ID first
    if let Some(tenant_id) = tenant_id {
        // If user_id is provided, verify membership and get role
        let resolved_role = if let Some(uid) = user_id {
            let role_str = repos
                .tenants
                .get_user_role(uid, tenant_id)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to check tenant membership: {e}"))
                })?;

            Some((
                role_str.map_or(TenantRole::Member, |r| TenantRole::from_db_string(&r)),
                uid,
            ))
        } else {
            None
        };

        let tenant_name = match repos.tenants.get_by_id(tenant_id).await {
            Ok(tenant) => tenant.name,
            _ => "Unknown Tenant".to_owned(),
        };

        // With no user there is no membership to look up, so the context is
        // tenant-scoped and carries no role — rather than a placeholder one.
        return Ok(Some(resolved_role.map_or_else(
            || {
                TenantContext::for_tenant_scoped_operation(
                    tenant_id,
                    tenant_name.clone(),
                    Uuid::nil(),
                )
            },
            |(role, uid)| {
                TenantContext::from_verified_membership(tenant_id, tenant_name.clone(), uid, role)
            },
        )));
    }

    // Try to extract from headers
    if let Some(headers) = headers {
        if let Some(tenant_id_header) = headers.get("x-tenant-id") {
            if let Ok(tenant_id_str) = tenant_id_header.to_str() {
                if let Ok(header_tenant_id) = tenant_id_str.parse::<TenantId>() {
                    // If user_id is provided, verify membership
                    let resolved_role = if let Some(uid) = user_id {
                        let role_str = repos
                            .tenants
                            .get_user_role(uid, header_tenant_id)
                            .await
                            .map_err(|e| {
                                AppError::database(format!(
                                    "Failed to check tenant membership: {e}"
                                ))
                            })?;

                        Some((
                            role_str.map_or(TenantRole::Member, |r| TenantRole::from_db_string(&r)),
                            uid,
                        ))
                    } else {
                        None
                    };

                    let tenant_name = match repos.tenants.get_by_id(header_tenant_id).await {
                        Ok(tenant) => tenant.name,
                        _ => "Unknown Tenant".to_owned(),
                    };

                    // No user means no membership lookup, so no role.
                    return Ok(Some(resolved_role.map_or_else(
                        || {
                            TenantContext::for_tenant_scoped_operation(
                                header_tenant_id,
                                tenant_name.clone(),
                                Uuid::nil(),
                            )
                        },
                        |(role, uid)| {
                            TenantContext::from_verified_membership(
                                header_tenant_id,
                                tenant_name.clone(),
                                uid,
                                role,
                            )
                        },
                    )));
                }
            }
        }
    }

    // Try to extract from user's default tenant (via tenant_users table)
    if let Some(user_id) = user_id {
        // SECURITY: Global lookup — resolving user's default tenant
        repos
            .users
            .get_global(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user: {e}")))?
            .ok_or_else(|| AppError::not_found("User"))?;

        // Get user's tenants from tenant_users table
        let tenants = repos
            .tenants
            .list_for_user(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenants: {e}")))?;

        if let Some(default_tenant) = tenants.first() {
            let user_role = repos
                .tenants
                .get_user_role(user_id, default_tenant.id)
                .await
                .map_err(|e| AppError::database(format!("Failed to get user tenant role: {e}")))?
                .map_or(TenantRole::Member, |r| TenantRole::from_db_string(&r));

            return Ok(Some(TenantContext::from_verified_membership(
                default_tenant.id,
                default_tenant.name.clone(),
                user_id,
                user_role,
            )));
        }
    }

    Ok(None)
}
