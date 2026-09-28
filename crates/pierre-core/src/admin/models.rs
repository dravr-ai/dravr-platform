// ABOUTME: Data models and types for admin authentication and authorization system
// ABOUTME: Defines admin permissions, token structures, and validation types for admin operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//! Admin Token Models
//!
//! Strong Rust types for the admin authentication system

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::str::FromStr;
use uuid::Uuid;

use crate::errors::{AppError, AppResult, ErrorCode};

/// Admin token with full details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminToken {
    /// Unique token identifier (UUID)
    pub id: String,
    /// Service or system name using this token
    pub service_name: String,
    /// Optional description of the service
    pub service_description: Option<String>,
    /// SHA-256 hash of the token for verification
    pub token_hash: String,
    /// First 8 characters of token for identification
    pub token_prefix: String,
    /// SHA-256 hash of JWT secret for this token
    pub jwt_secret_hash: String,
    /// Granted admin permissions
    pub permissions: AdminPermissions,
    /// Whether this is a super admin token
    pub is_super_admin: bool,
    /// Whether the token is active
    pub is_active: bool,
    /// Tenant this token is scoped to (`None` for super-admin / global tokens)
    pub tenant_id: Option<String>,
    /// The operator this token acts as: the super-admin who approved the
    /// device login that minted it. `None` for a service token, which names
    /// no person.
    pub operator_user_id: Option<Uuid>,
    /// When the token was created
    pub created_at: DateTime<Utc>,
    /// Optional token expiration time
    pub expires_at: Option<DateTime<Utc>>,
    /// When the token was last used
    pub last_used_at: Option<DateTime<Utc>>,
    /// IP address from last use
    pub last_used_ip: Option<String>,
    /// Total number of times token has been used
    pub usage_count: u64,
}

/// Redacted admin token for API responses (excludes sensitive hash fields)
#[derive(Debug, Clone, Serialize)]
pub struct AdminTokenSummary {
    /// Unique token identifier (UUID)
    pub id: String,
    /// Service or system name using this token
    pub service_name: String,
    /// Optional description of the service
    pub service_description: Option<String>,
    /// First 8 characters of token for identification
    pub token_prefix: String,
    /// Granted admin permissions, as a flat list: what the console renders
    /// with `permissions.map()` and what `packages/shared-types` declares
    pub permissions: Vec<AdminPermission>,
    /// Whether this is a super admin token
    pub is_super_admin: bool,
    /// Whether the token is active
    pub is_active: bool,
    /// Tenant this token is scoped to (`None` for super-admin / global tokens)
    pub tenant_id: Option<String>,
    /// The operator this token acts as: the super-admin who approved the
    /// device login that minted it. `None` for a service token, which names
    /// no person.
    pub operator_user_id: Option<Uuid>,
    /// When the token was created
    pub created_at: DateTime<Utc>,
    /// Optional token expiration time
    pub expires_at: Option<DateTime<Utc>>,
    /// When the token was last used
    pub last_used_at: Option<DateTime<Utc>>,
    /// Total number of times token has been used
    pub usage_count: u64,
}

impl From<AdminToken> for AdminTokenSummary {
    fn from(token: AdminToken) -> Self {
        Self {
            id: token.id,
            service_name: token.service_name,
            service_description: token.service_description,
            token_prefix: token.token_prefix,
            permissions: token.permissions.to_vec(),
            is_super_admin: token.is_super_admin,
            is_active: token.is_active,
            tenant_id: token.tenant_id,
            operator_user_id: token.operator_user_id,
            created_at: token.created_at,
            expires_at: token.expires_at,
            last_used_at: token.last_used_at,
            usage_count: token.usage_count,
        }
    }
}

/// Admin permissions with strong typing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdminPermissions {
    /// Set of granted permissions
    permissions: HashSet<AdminPermission>,
}

impl AdminPermissions {
    /// Create new permissions set
    #[must_use]
    pub fn new(permissions: Vec<AdminPermission>) -> Self {
        Self {
            permissions: permissions.into_iter().collect(),
        }
    }

    /// Create default permissions for regular admin
    #[must_use]
    pub fn default_admin() -> Self {
        Self::new(vec![
            AdminPermission::ProvisionKeys,
            AdminPermission::ListKeys,
            AdminPermission::RevokeKeys,
            AdminPermission::UpdateKeyLimits,
        ])
    }

    /// Permissions of a plain `Admin` signed in to the web console.
    ///
    /// [`Self::default_admin`] plus `ManageUsers` and `ManageAdminTokens`: the
    /// console's user and token tabs are served by the same handlers the
    /// admin-token API mounts, and those check these permissions. The token
    /// handlers keep super-admin tokens to super-admin callers whatever the
    /// permission set, so a plain Admin mints, rotates and revokes only
    /// non-super tokens, and grants only permissions it holds itself. A console
    /// session and an admin token of the same role therefore grant different
    /// authority by design. Configuration, audit logs and tiers stay out of
    /// this set.
    #[must_use]
    pub fn console_admin() -> Self {
        let mut permissions = Self::default_admin();
        permissions.add_permission(AdminPermission::ManageUsers);
        permissions.add_permission(AdminPermission::ManageAdminTokens);
        permissions
    }

    /// Create super admin permissions (every [`AdminPermission`] variant).
    #[must_use]
    pub fn super_admin() -> Self {
        Self::new(vec![
            AdminPermission::ProvisionKeys,
            AdminPermission::ListKeys,
            AdminPermission::RevokeKeys,
            AdminPermission::UpdateKeyLimits,
            AdminPermission::ManageAdminTokens,
            AdminPermission::ViewAuditLogs,
            AdminPermission::ManageUsers,
            AdminPermission::ViewConfiguration,
            AdminPermission::ManageConfiguration,
        ])
    }

    /// Check if permission is granted
    #[must_use]
    pub fn has_permission(&self, permission: &AdminPermission) -> bool {
        self.permissions.contains(permission)
    }

    /// Add permission
    pub fn add_permission(&mut self, permission: AdminPermission) -> bool {
        self.permissions.insert(permission)
    }

    /// Remove permission
    pub fn remove_permission(&mut self, permission: &AdminPermission) -> bool {
        self.permissions.remove(permission)
    }

    /// Get all permissions as vector
    #[must_use]
    pub fn to_vec(&self) -> Vec<AdminPermission> {
        self.permissions.iter().cloned().collect()
    }

    /// Convert to JSON string for database storage
    ///
    /// # Errors
    ///
    /// Returns `serde_json::Error` if serialization fails
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let permission_strings: Vec<String> =
            self.permissions.iter().map(ToString::to_string).collect();
        serde_json::to_string(&permission_strings)
    }

    /// Create from JSON string from database
    ///
    /// # Errors
    ///
    /// Returns `serde_json::Error` if deserialization fails
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let permission_strings: Vec<String> = serde_json::from_str(json)?;
        let permissions = permission_strings
            .into_iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        Ok(Self::new(permissions))
    }
}

/// Individual admin permissions
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AdminPermission {
    /// Provision new API keys for users
    ProvisionKeys,
    /// List existing API keys
    ListKeys,
    /// Revoke/deactivate API keys
    RevokeKeys,
    /// Update API key rate limits
    UpdateKeyLimits,
    /// Manage admin tokens (super admin only)
    ManageAdminTokens,
    /// View audit logs (super admin only)
    ViewAuditLogs,
    /// Manage user accounts (super admin only)
    ManageUsers,
    /// View configuration settings (tool catalog, tenant overrides)
    ViewConfiguration,
    /// Manage configuration settings (tool overrides, tenant settings)
    ManageConfiguration,
}

impl Display for AdminPermission {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::ProvisionKeys => write!(f, "provision_keys"),
            Self::ListKeys => write!(f, "list_keys"),
            Self::RevokeKeys => write!(f, "revoke_keys"),
            Self::UpdateKeyLimits => write!(f, "update_key_limits"),
            Self::ManageAdminTokens => write!(f, "manage_admin_tokens"),
            Self::ViewAuditLogs => write!(f, "view_audit_logs"),
            Self::ManageUsers => write!(f, "manage_users"),
            Self::ViewConfiguration => write!(f, "view_configuration"),
            Self::ManageConfiguration => write!(f, "manage_configuration"),
        }
    }
}

impl FromStr for AdminPermission {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "provision_keys" => Ok(Self::ProvisionKeys),
            "list_keys" => Ok(Self::ListKeys),
            "revoke_keys" => Ok(Self::RevokeKeys),
            "update_key_limits" => Ok(Self::UpdateKeyLimits),
            "manage_admin_tokens" => Ok(Self::ManageAdminTokens),
            "view_audit_logs" => Ok(Self::ViewAuditLogs),
            "manage_users" => Ok(Self::ManageUsers),
            "view_configuration" => Ok(Self::ViewConfiguration),
            "manage_configuration" => Ok(Self::ManageConfiguration),
            _ => Err(format!("Unknown permission: {s}")),
        }
    }
}

/// Admin token creation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAdminTokenRequest {
    /// Name of the service requesting the token
    pub service_name: String,
    /// Optional description of the service
    pub service_description: Option<String>,
    /// List of permissions to grant (None = default permissions)
    pub permissions: Option<Vec<AdminPermission>>,
    /// Number of days until token expires (None = never expires)
    pub expires_in_days: Option<u64>,
    /// Whether this is a super admin token with all permissions
    pub is_super_admin: bool,
    /// Tenant to scope this token to (`None` for global / super-admin)
    pub tenant_id: Option<String>,
    /// The operator the token acts as, set only by the device grant from the
    /// approving super-admin's account. Never read from a request body: an
    /// operator identity is earned by approving a login, not named.
    #[serde(skip)]
    pub operator_user_id: Option<Uuid>,
}

impl CreateAdminTokenRequest {
    /// Create request for regular admin token
    #[must_use]
    pub const fn new(service_name: String) -> Self {
        Self {
            service_name,
            service_description: None,
            permissions: None,          // Will use default
            expires_in_days: Some(365), // 1 year default
            is_super_admin: false,
            tenant_id: None,
            operator_user_id: None,
        }
    }

    /// Create request for super admin token
    #[must_use]
    pub fn super_admin(service_name: String) -> Self {
        Self {
            service_name,
            service_description: Some("Super Admin Token".into()),
            permissions: None,     // Will use super admin permissions
            expires_in_days: None, // Never expires
            is_super_admin: true,
            tenant_id: None,
            operator_user_id: None,
        }
    }

    /// The request that replaces `existing` on rotation: same name, scope and
    /// super-admin flag, a fresh expiry. The operator it acts as carries over
    /// only when `rotated_by` is that operator, so a rotation never hands one
    /// operator's identity to whoever rotated the token.
    #[must_use]
    pub fn rotation_of(
        existing: AdminToken,
        expires_in_days: u64,
        rotated_by: Option<Uuid>,
    ) -> Self {
        Self {
            service_name: existing.service_name,
            service_description: existing.service_description,
            permissions: None,
            expires_in_days: Some(expires_in_days),
            is_super_admin: existing.is_super_admin,
            tenant_id: existing.tenant_id,
            operator_user_id: existing
                .operator_user_id
                .filter(|operator| rotated_by == Some(*operator)),
        }
    }

    /// The request as an admin-token creator may submit it.
    ///
    /// Names starting with [`DEVICE_CLI_SERVICE_PREFIX`] belong to the device
    /// grant alone, so a token named like one was minted by an approved login.
    /// The operator a token acts as is its `operator_user_id`, never its name,
    /// so a look-alike could not borrow an identity either way — this keeps
    /// the token list from showing one.
    ///
    /// # Errors
    /// Returns an invalid-input error for a reserved name.
    pub fn for_creator(self) -> AppResult<Self> {
        if self.service_name.starts_with(DEVICE_CLI_SERVICE_PREFIX) {
            return Err(AppError::invalid_input(format!(
                "Service names starting with '{DEVICE_CLI_SERVICE_PREFIX}' are reserved for device logins"
            )));
        }
        Ok(self)
    }

    /// The same request, for a token that acts as `operator_user_id`.
    #[must_use]
    pub const fn operated_by(mut self, operator_user_id: Uuid) -> Self {
        self.operator_user_id = Some(operator_user_id);
        self
    }
}

/// Service-name prefix of the super-admin token a device login mints.
///
/// The rest of the name is the id of the super-admin who approved the login —
/// a label for the token list, never an identity and never an email (service
/// names are logged). The identity is the token's `operator_user_id`, stored
/// on its row at the device grant; creators may not choose a name with this
/// prefix ([`CreateAdminTokenRequest::for_creator`]).
pub const DEVICE_CLI_SERVICE_PREFIX: &str = "device-cli:";

/// Generated admin token response
#[derive(Debug, Clone, Serialize)]
pub struct GeneratedAdminToken {
    /// Unique identifier for the token
    pub token_id: String,
    /// Name of the service
    pub service_name: String,
    /// The actual JWT token (shown only once!)
    pub jwt_token: String,
    /// Visible prefix for identification
    pub token_prefix: String,
    /// Permissions granted to this token
    pub permissions: AdminPermissions,
    /// Whether this is a super admin token
    pub is_super_admin: bool,
    /// When the token expires (if set)
    pub expires_at: Option<DateTime<Utc>>,
    /// When the token was created
    pub created_at: DateTime<Utc>,
}

/// Admin token usage audit entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminTokenUsage {
    /// Unique identifier for this usage record
    pub id: Option<i64>,
    /// ID of the admin token that was used
    pub admin_token_id: String,
    /// When the action was performed
    pub timestamp: DateTime<Utc>,
    /// Type of admin action performed
    pub action: AdminAction,
    /// Resource that was targeted (if applicable)
    pub target_resource: Option<String>,
    /// Client IP address
    pub ip_address: Option<String>,
    /// Client user agent
    pub user_agent: Option<String>,
    /// Size of the request in bytes
    pub request_size_bytes: Option<u32>,
    /// Whether the action succeeded
    pub success: bool,
    /// Error message if the action failed
    pub error_message: Option<String>,
    /// Response time in milliseconds
    pub response_time_ms: Option<u32>,
}

/// Admin actions for audit logging
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AdminAction {
    /// Provision a new API key
    ProvisionKey,
    /// Revoke an existing API key
    RevokeKey,
    /// List all API keys
    ListKeys,
    /// Update rate limits for an API key
    UpdateKeyLimits,
    /// List all admin tokens
    ListAdminTokens,
    /// Revoke an admin token
    RevokeAdminToken,
    /// View audit logs
    ViewAuditLogs,
    /// Manage user accounts
    ManageUser,
    /// Delete every row one provider contributed, across every tenant
    PurgeProviderData,
}

impl Display for AdminAction {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::ProvisionKey => write!(f, "provision_key"),
            Self::RevokeKey => write!(f, "revoke_key"),
            Self::ListKeys => write!(f, "list_keys"),
            Self::UpdateKeyLimits => write!(f, "update_key_limits"),
            Self::ListAdminTokens => write!(f, "list_admin_tokens"),
            Self::RevokeAdminToken => write!(f, "revoke_admin_token"),
            Self::ViewAuditLogs => write!(f, "view_audit_logs"),
            Self::ManageUser => write!(f, "manage_user"),
            Self::PurgeProviderData => write!(f, "purge_provider_data"),
        }
    }
}

impl FromStr for AdminAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "provision_key" => Ok(Self::ProvisionKey),
            "revoke_key" => Ok(Self::RevokeKey),
            "list_keys" => Ok(Self::ListKeys),
            "update_key_limits" => Ok(Self::UpdateKeyLimits),
            "list_admin_tokens" => Ok(Self::ListAdminTokens),
            "revoke_admin_token" => Ok(Self::RevokeAdminToken),
            "view_audit_logs" => Ok(Self::ViewAuditLogs),
            "manage_user" => Ok(Self::ManageUser),
            "purge_provider_data" => Ok(Self::PurgeProviderData),
            _ => Err(format!("Unknown admin action: {s}")),
        }
    }
}

/// API key provisioning request from admin service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyProvisionRequest {
    /// Email of the user to provision the key for
    pub user_email: String,
    /// Optional user ID (will be looked up if not provided)
    pub user_id: Option<Uuid>,
    /// Tier level ("starter", "professional", "enterprise")
    pub tier: String,
    /// Maximum requests allowed
    pub rate_limit_requests: u32,
    /// Rate limit period
    pub rate_limit_period: RateLimitPeriod,
    /// Number of days until the key expires
    pub expires_in_days: Option<u64>,
    /// Additional metadata (company name, use case, etc.)
    pub metadata: Option<serde_json::Value>,
}

/// Rate limit periods for API keys
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RateLimitPeriod {
    /// Rate limit per hour
    Hour,
    /// Rate limit per day
    Day,
    /// Rate limit per month
    Month,
}

impl Display for RateLimitPeriod {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Hour => write!(f, "hour"),
            Self::Day => write!(f, "day"),
            Self::Month => write!(f, "month"),
        }
    }
}

impl RateLimitPeriod {
    /// Get the window duration in seconds
    #[must_use]
    pub const fn window_seconds(&self) -> u64 {
        match self {
            Self::Hour => 3_600,
            Self::Day => 86_400,
            Self::Month => 2_592_000, // 30 days
        }
    }
}

/// API key provisioning response
#[derive(Debug, Clone, Serialize)]
pub struct ProvisionedApiKey {
    /// Unique identifier for the API key
    pub api_key_id: String,
    /// The actual API key (shown only once!)
    pub api_key: String,
    /// ID of the user who owns the key
    pub user_id: Uuid,
    /// Email of the user who owns the key
    pub user_email: String,
    /// Tier level of the key
    pub tier: String,
    /// Maximum requests allowed
    pub rate_limit_requests: u32,
    /// Rate limit period
    pub rate_limit_period: RateLimitPeriod,
    /// When the key expires (if set)
    pub expires_at: Option<DateTime<Utc>>,
    /// When the key was created
    pub created_at: DateTime<Utc>,
}

/// Admin token validation result
#[derive(Debug, Clone)]
pub struct ValidatedAdminToken {
    /// Unique identifier for the token
    pub token_id: String,
    /// Name of the service
    pub service_name: String,
    /// Permissions granted to this token
    pub permissions: AdminPermissions,
    /// Whether this is a super admin token
    pub is_super_admin: bool,
    /// Tenant this token is scoped to (`None` for super-admin / global tokens)
    pub tenant_id: Option<String>,
    /// The operator the token acts as: the approving super-admin for a
    /// device-login token (from its stored row), the signed-in admin for a
    /// console session; `None` for a service token, which names no person
    pub operator_user_id: Option<Uuid>,
    /// Additional user info from JWT claims
    pub user_info: Option<serde_json::Value>,
}

impl ValidatedAdminToken {
    /// Check if the token has the required permission.
    ///
    /// Super admin tokens bypass permission checks.
    ///
    /// # Errors
    /// Returns `AppError` with `PermissionDenied` code if the permission is not granted.
    pub fn require_permission(&self, permission: &AdminPermission) -> AppResult<()> {
        (self.is_super_admin || self.permissions.has_permission(permission)).ok_or_else(|| {
            AppError::new(
                ErrorCode::PermissionDenied,
                format!("Permission required: {permission}"),
            )
        })
    }

    /// Verify the token is allowed to access the given tenant.
    ///
    /// - Super-admin tokens (`is_super_admin = true`) can access any tenant.
    /// - Scoped tokens must match the `requested_tenant` exactly.
    /// - Tokens with no tenant scope and no super-admin flag are rejected.
    ///
    /// # Errors
    ///
    /// Returns `AppError` with `PermissionDenied` if the token does not have
    /// access to the requested tenant.
    pub fn require_tenant_access(&self, requested_tenant: &str) -> AppResult<()> {
        if self.is_super_admin {
            return Ok(());
        }
        match &self.tenant_id {
            Some(bound) if bound == requested_tenant => Ok(()),
            Some(bound) => Err(AppError::new(
                ErrorCode::PermissionDenied,
                format!(
                    "Token is scoped to tenant {bound}, cannot access tenant {requested_tenant}"
                ),
            )),
            None => Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Token has no tenant scope — super-admin required for cross-tenant access",
            )),
        }
    }
}

/// One row of `admin_config_overrides` returned by the list-by-category repository method.
///
/// Plain DTO; consumers (the pricing-registry loader, audit tooling)
/// parse `config_value` into whatever JSON shape the category dictates.
#[derive(Debug, Clone)]
pub struct AdminConfigOverrideRow {
    /// Identifier of the override row, e.g. `gemini.gemini-2.0-flash`.
    pub config_key: String,
    /// Tenant scope. `None` means the override is global (system-wide).
    pub tenant_id: Option<String>,
    /// JSON-encoded payload whose schema is category-specific.
    pub config_value: String,
}
