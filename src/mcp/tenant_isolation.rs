// ABOUTME: Tenant isolation and multi-tenancy management for MCP server
// ABOUTME: Handles user validation, tenant context extraction, and access control

use super::resources::ServerResources;
use crate::auth::{AuthManager, AuthResult};
use crate::database_plugins::{factory::Database, DatabaseProvider};
use crate::tenant::{TenantContext, TenantRole};
use crate::utils::json_responses::api_error;
use anyhow::Result;
use std::sync::Arc;
use tracing::{error, warn};
use uuid::Uuid;

/// Manages tenant isolation and multi-tenancy for the MCP server
pub struct TenantIsolation {
    resources: Arc<ServerResources>,
}

impl TenantIsolation {
    /// Create a new tenant isolation manager
    pub fn new(resources: Arc<ServerResources>) -> Self {
        Self { resources }
    }

    /// Validate JWT token and extract tenant context
    pub async fn validate_tenant_access(&self, jwt_token: &str) -> Result<TenantContext> {
        let auth_result = self.resources.auth_manager.validate_jwt(jwt_token)?;

        match auth_result {
            AuthResult::Valid { user_id } => {
                let user = self.get_user_with_tenant(user_id).await?;
                let tenant_id = self.extract_tenant_id(&user)?;
                let tenant_name = self.get_tenant_name(tenant_id).await;
                let user_role = self.get_user_role_for_tenant(user_id, tenant_id).await?;

                Ok(TenantContext {
                    tenant_id,
                    user_id,
                    tenant_name,
                    user_role,
                })
            }
            AuthResult::Invalid => Err(anyhow::anyhow!("Invalid JWT token")),
            AuthResult::Expired => Err(anyhow::anyhow!("JWT token expired")),
        }
    }

    /// Get user with tenant information
    pub async fn get_user_with_tenant(&self, user_id: Uuid) -> Result<crate::models::User> {
        self.resources
            .database
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("User not found"))
    }

    /// Extract tenant ID from user
    pub fn extract_tenant_id(&self, user: &crate::models::User) -> Result<Uuid> {
        user.tenant_id
            .ok_or_else(|| anyhow::anyhow!("User does not belong to any tenant"))
    }

    /// Get tenant name by ID
    pub async fn get_tenant_name(&self, tenant_id: Uuid) -> String {
        match self.resources.database.get_tenant_by_id(tenant_id).await {
            Ok(Some(tenant)) => tenant.name,
            Ok(None) => {
                warn!("Tenant {} not found, using default name", tenant_id);
                "Unknown Tenant".to_string()
            }
            Err(e) => {
                warn!(
                    "Failed to get tenant {}: {}, using default name",
                    tenant_id, e
                );
                "Unknown Tenant".to_string()
            }
        }
    }

    /// Get user's role in a tenant
    pub async fn get_user_role_for_tenant(
        &self,
        user_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<TenantRole> {
        // Check if user belongs to the tenant
        let user = self.get_user_with_tenant(user_id).await?;

        if user.tenant_id != Some(tenant_id) {
            return Err(anyhow::anyhow!(
                "User {} does not belong to tenant {}",
                user_id,
                tenant_id
            ));
        }

        // For now, return Member role for all users
        // TODO: Implement proper role management
        Ok(TenantRole::Member)
    }

    /// Extract tenant context from request headers
    pub async fn extract_tenant_from_header(
        &self,
        headers: &warp::http::HeaderMap,
    ) -> Result<Option<TenantContext>> {
        // Look for tenant ID in headers
        if let Some(tenant_id_header) = headers.get("x-tenant-id") {
            let tenant_id_str = tenant_id_header.to_str().map_err(|_| {
                anyhow::anyhow!("Invalid tenant ID header format")
            })?;

            let tenant_id = Uuid::parse_str(tenant_id_str).map_err(|_| {
                anyhow::anyhow!("Invalid tenant ID format")
            })?;

            let tenant_name = self.get_tenant_name(tenant_id).await;

            // For header-based tenant context, we don't have user info
            return Ok(Some(TenantContext {
                tenant_id,
                user_id: Uuid::nil(), // Placeholder
                tenant_name,
                user_role: TenantRole::Member,
            }));
        }

        Ok(None)
    }

    /// Extract tenant context from user
    pub async fn extract_tenant_from_user(&self, user_id: Uuid) -> Result<TenantContext> {
        let user = self.get_user_with_tenant(user_id).await?;
        let tenant_id = self.extract_tenant_id(&user)?;
        let tenant_name = self.get_tenant_name(tenant_id).await;
        let user_role = self.get_user_role_for_tenant(user_id, tenant_id).await?;

        Ok(TenantContext {
            tenant_id,
            user_id,
            tenant_name,
            user_role,
        })
    }

    /// Check if user has access to a specific resource
    pub async fn check_resource_access(
        &self,
        user_id: Uuid,
        tenant_id: Uuid,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<bool> {
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

    /// Isolate database operations to tenant scope
    pub async fn isolate_resources(&self, tenant_id: Uuid) -> Result<TenantResources> {
        // Create tenant-scoped resource accessor
        Ok(TenantResources {
            tenant_id,
            database: self.resources.database.clone(),
        })
    }

    /// Validate that a user can perform an action on behalf of a tenant
    pub async fn validate_tenant_action(
        &self,
        user_id: Uuid,
        tenant_id: Uuid,
        action: &str,
    ) -> Result<()> {
        let user_role = self.get_user_role_for_tenant(user_id, tenant_id).await?;

        match action {
            "read_oauth_credentials" | "store_oauth_credentials" => {
                if matches!(user_role, TenantRole::Owner | TenantRole::Member) {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "User {} does not have permission to {} for tenant {}",
                        user_id,
                        action,
                        tenant_id
                    ))
                }
            }
            "modify_tenant_settings" => {
                if matches!(user_role, TenantRole::Owner) {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "User {} does not have owner permission for tenant {}",
                        user_id,
                        tenant_id
                    ))
                }
            }
            _ => {
                warn!("Unknown action for validation: {}", action);
                Err(anyhow::anyhow!("Unknown action: {}", action))
            }
        }
    }
}

/// Tenant-scoped resource accessor
pub struct TenantResources {
    pub tenant_id: Uuid,
    pub database: Arc<Database>,
}

impl TenantResources {
    /// Get OAuth credentials for this tenant
    pub async fn get_oauth_credentials(
        &self,
        provider: &str,
    ) -> Result<Option<crate::models::TenantOAuthCredential>> {
        self.database
            .get_tenant_oauth_credential(self.tenant_id, provider)
            .await
    }

    /// Store OAuth credentials for this tenant
    pub async fn store_oauth_credentials(
        &self,
        credential: &crate::models::TenantOAuthCredential,
    ) -> Result<()> {
        // Ensure the credential belongs to this tenant
        if credential.tenant_id != self.tenant_id {
            return Err(anyhow::anyhow!(
                "Credential tenant ID mismatch: expected {}, got {}",
                self.tenant_id,
                credential.tenant_id
            ));
        }

        self.database.store_tenant_oauth_credential(credential).await
    }

    /// Get user OAuth tokens for this tenant
    pub async fn get_user_oauth_tokens(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<crate::models::UserOAuthToken>> {
        self.database
            .get_user_oauth_token(user_id, provider)
            .await
    }

    /// Store user OAuth token for this tenant
    pub async fn store_user_oauth_token(
        &self,
        token: &crate::models::UserOAuthToken,
    ) -> Result<()> {
        // Additional validation could be added here to ensure
        // the user belongs to this tenant
        self.database.store_user_oauth_token(token).await
    }
}

/// JWT token validation result
#[derive(Debug, Clone)]
pub struct JwtValidationResult {
    pub user_id: Uuid,
    pub tenant_context: TenantContext,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Standalone function for JWT validation (used by HTTP middleware)
pub async fn validate_jwt_token_for_mcp(
    token: &str,
    auth_manager: &AuthManager,
    database: &Arc<Database>,
) -> Result<JwtValidationResult> {
    let auth_result = auth_manager.validate_jwt(token)?;

    match auth_result {
        AuthResult::Valid { user_id } => {
            // Get user and tenant information
            let user = database
                .get_user_by_id(user_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("User not found"))?;

            let tenant_id = user
                .tenant_id
                .ok_or_else(|| anyhow::anyhow!("User does not belong to any tenant"))?;

            let tenant_name = match database.get_tenant_by_id(tenant_id).await {
                Ok(Some(tenant)) => tenant.name,
                _ => "Unknown Tenant".to_string(),
            };

            let tenant_context = TenantContext {
                tenant_id,
                user_id,
                tenant_name,
                user_role: TenantRole::Member, // Default role
            };

            // For now, set a default expiration
            let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);

            Ok(JwtValidationResult {
                user_id,
                tenant_context,
                expires_at,
            })
        }
        AuthResult::Invalid => Err(anyhow::anyhow!("Invalid JWT token")),
        AuthResult::Expired => Err(anyhow::anyhow!("JWT token expired")),
    }
}

/// Extract tenant context from various sources (internal helper)
pub async fn extract_tenant_context_internal(
    database: &Arc<Database>,
    user_id: Option<Uuid>,
    tenant_id: Option<Uuid>,
    headers: Option<&warp::http::HeaderMap>,
) -> Result<Option<TenantContext>> {
    // Try to extract from user ID first
    if let Some(user_id) = user_id {
        let user = database
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("User not found"))?;

        if let Some(tenant_id) = user.tenant_id {
            let tenant_name = match database.get_tenant_by_id(tenant_id).await {
                Ok(Some(tenant)) => tenant.name,
                _ => "Unknown Tenant".to_string(),
            };

            return Ok(Some(TenantContext {
                tenant_id,
                user_id,
                tenant_name,
                user_role: TenantRole::Member,
            }));
        }
    }

    // Try to extract from explicit tenant ID
    if let Some(tenant_id) = tenant_id {
        let tenant_name = match database.get_tenant_by_id(tenant_id).await {
            Ok(Some(tenant)) => tenant.name,
            _ => "Unknown Tenant".to_string(),
        };

        return Ok(Some(TenantContext {
            tenant_id,
            user_id: user_id.unwrap_or_else(Uuid::nil),
            tenant_name,
            user_role: TenantRole::Member,
        }));
    }

    // Try to extract from headers
    if let Some(headers) = headers {
        if let Some(tenant_id_header) = headers.get("x-tenant-id") {
            if let Ok(tenant_id_str) = tenant_id_header.to_str() {
                if let Ok(tenant_id) = Uuid::parse_str(tenant_id_str) {
                    let tenant_name = match database.get_tenant_by_id(tenant_id).await {
                        Ok(Some(tenant)) => tenant.name,
                        _ => "Unknown Tenant".to_string(),
                    };

                    return Ok(Some(TenantContext {
                        tenant_id,
                        user_id: user_id.unwrap_or_else(Uuid::nil),
                        tenant_name,
                        user_role: TenantRole::Member,
                    }));
                }
            }
        }
    }

    Ok(None)
}