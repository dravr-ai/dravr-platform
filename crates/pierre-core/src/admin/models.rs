// ABOUTME: Data models and types for admin authentication and authorization system
// ABOUTME: Defines admin permissions, token structures, and validation types for admin operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//! Admin Token Models
//!
//! Strong Rust types for the admin authentication system

use dravr_tronc::dates::{Elapsed, Timestamp};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::str::FromStr;
use uuid::Uuid;

use crate::errors::{AppError, AppResult, ErrorCode};

/// Admin token with full details
///
/// The row holds two values no line of output is a place for: the hash a
/// presented token is verified against and the hash of the secret it was
/// signed under. `Debug` is written by hand and prints a fixed placeholder
/// for each. The type neither serializes nor deserializes, so no JSON can
/// carry a hash out: a row comes from the store, and an API answers with
/// [`AdminTokenSummary`].
#[derive(Clone)]
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
    /// The admin this token acts as: the admin who approved the
    /// device login that minted it. `None` for a service token, which names
    /// no person.
    /// Named after the `admin_tokens.operator_user_id` column, which is never
    /// renamed: a column rename makes a rollback deploy unsafe.
    pub operator_user_id: Option<Uuid>,
    /// The id of the token a rotation replaced this one with. `None` for a
    /// live token and for a revoked one, and for a token rotated before the
    /// column existed, which reads as a revoked one.
    pub rotated_to_token_id: Option<String>,
    /// When the token was created
    pub created_at: Timestamp,
    /// Optional token expiration time
    pub expires_at: Option<Timestamp>,
    /// When the token was last used
    pub last_used_at: Option<Timestamp>,
    /// IP address from last use
    pub last_used_ip: Option<String>,
    /// Total number of times token has been used
    pub usage_count: u64,
}

/// What stands where a hash would print.
const REDACTED_HASH: &str = "[REDACTED]";

/// Every field as stored, except the two hashes, which print as
/// `[REDACTED]` whatever they hold.
impl Debug for AdminToken {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("AdminToken")
            .field("id", &self.id)
            .field("service_name", &self.service_name)
            .field("service_description", &self.service_description)
            .field("token_hash", &REDACTED_HASH)
            .field("token_prefix", &self.token_prefix)
            .field("jwt_secret_hash", &REDACTED_HASH)
            .field("permissions", &self.permissions)
            .field("is_super_admin", &self.is_super_admin)
            .field("is_active", &self.is_active)
            .field("tenant_id", &self.tenant_id)
            .field("operator_user_id", &self.operator_user_id)
            .field("rotated_to_token_id", &self.rotated_to_token_id)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("last_used_at", &self.last_used_at)
            .field("last_used_ip", &self.last_used_ip)
            .field("usage_count", &self.usage_count)
            .finish()
    }
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
    /// The admin this token acts as: the admin who approved the
    /// device login that minted it. `None` for a service token, which names
    /// no person.
    /// Named after the `admin_tokens.operator_user_id` column, which is never
    /// renamed: a column rename makes a rollback deploy unsafe.
    pub operator_user_id: Option<Uuid>,
    /// The id of the token a rotation replaced this one with, when one did:
    /// what tells a rotated token from a revoked one among the inactive.
    pub rotated_to_token_id: Option<String>,
    /// When the token was created
    pub created_at: Timestamp,
    /// Optional token expiration time
    pub expires_at: Option<Timestamp>,
    /// When the token was last used
    pub last_used_at: Option<Timestamp>,
    /// Total number of times token has been used
    pub usage_count: u64,
}

/// What revoking a rotated token is answered.
///
/// One sentence for the admin API and `pierre-cli token revoke` alike: the
/// token has a live replacement, so revoking it again would end nothing. It
/// names the replacement by id, never a credential.
#[must_use]
pub fn rotated_token_refusal(token_id: &str, replacement_id: &str) -> String {
    format!(
        "Admin token {token_id} was rotated and is already retired; its replacement is \
         {replacement_id}. To end the credential, revoke that one."
    )
}

/// What rotating a rotated token is answered.
///
/// One sentence for the admin API (409) and `pierre-cli token rotate`
/// alike: the token already has a live replacement, so the answer names it
/// by id, never a credential, and asks for no new token beside it.
#[must_use]
pub fn rerotated_token_refusal(token_id: &str, replacement_id: &str) -> String {
    format!(
        "Admin token {token_id} was already rotated and cannot be rotated again; its \
         replacement is {replacement_id}. To get a new credential, rotate that one."
    )
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
            rotated_to_token_id: token.rotated_to_token_id,
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

    /// Every [`AdminPermission`] variant: what a super-admin token carries, and
    /// what every admin's console session carries (carnet#1567).
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
            AdminPermission::ManageWebsite,
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
    /// Serve the dravr.ai website: send docs sign-in links and redeem them
    ManageWebsite,
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
            Self::ManageWebsite => write!(f, "manage_website"),
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
            "manage_website" => Ok(Self::ManageWebsite),
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
    /// The admin the token acts as, set only by the device grant from the
    /// approving admin's account. Never read from a request body: an
    /// admin identity is earned by approving a login, not named.
    /// Named after the `admin_tokens.operator_user_id` column, which is never
    /// renamed: a column rename makes a rollback deploy unsafe.
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

    /// The permissions the token this request mints carries: the ones it
    /// names, or, for a request that names none, every permission when it
    /// asks for the super-admin flag and the default set
    /// ([`AdminPermissions::default_admin`]) when it does not.
    ///
    /// The one definition of what an unnamed list grants: the store mints
    /// from it, and the admin API reads it to hold a caller to what it may
    /// hand out before anything is minted.
    #[must_use]
    pub fn granted_permissions(&self) -> AdminPermissions {
        self.permissions.as_ref().map_or_else(
            || {
                if self.is_super_admin {
                    AdminPermissions::super_admin()
                } else {
                    AdminPermissions::default_admin()
                }
            },
            |named| AdminPermissions::new(named.clone()),
        )
    }

    /// When a token minted at `now` from this request expires: `None` for a
    /// request that names no lifetime or names `0` days, which never
    /// expires.
    ///
    /// The one definition of the lifetime rule: the store mints from it, and
    /// the admin API asks it first, so a lifetime the calendar cannot hold
    /// is refused before anything is minted or claimed.
    ///
    /// # Errors
    /// Returns an invalid-input error for a lifetime that ends past the last
    /// instant a timestamp can name.
    pub fn expires_at(&self, now: Timestamp) -> AppResult<Option<Timestamp>> {
        match self.expires_in_days {
            None | Some(0) => Ok(None),
            Some(days) => i64::try_from(days)
                .ok()
                .and_then(Elapsed::try_days)
                .and_then(|lifetime| now.checked_add(lifetime))
                .map(Some)
                .ok_or_else(|| {
                    AppError::invalid_input(format!(
                        "expires_in_days is too large: {days} days from now is past any date"
                    ))
                }),
        }
    }

    /// The request that replaces `existing` on rotation, the one definition
    /// of what a rotation carries over, for the admin API and `pierre-cli`
    /// alike: the name, the description, the tenant scope, the super-admin
    /// flag, and the permissions as stored unless the flag grants them all.
    /// The lifetime is the caller's: `expires_in_days` from now, `0` for a
    /// replacement that never expires.
    ///
    /// The admin it acts as carries over only when `rotated_by` is that
    /// admin, so a rotation never hands one admin's identity to whoever
    /// rotated the token; a caller that is no admin's session (a service
    /// token, `pierre-cli` on the host) passes `None`. One admin carries
    /// over whoever rotates: [`DELETED_ACCOUNT_ADMIN`]. It is no one's
    /// identity to hand over, and a replacement without it would be a
    /// service credential minted from a deleted account's device login.
    ///
    /// Only a live token rotates. A revoked one stays revoked: its
    /// replacement would be a live credential with the scope someone took
    /// away, and for a token withdrawn with its admin's standing, one that
    /// acts as no admin at all. A rotated one already has its replacement,
    /// and the refusal names it ([`rerotated_token_refusal`]).
    ///
    /// # Errors
    /// Returns an invalid-input error for a token that is not active.
    pub fn rotation_of(
        existing: AdminToken,
        expires_in_days: u64,
        rotated_by: Option<Uuid>,
    ) -> AppResult<Self> {
        if !existing.is_active {
            return Err(AppError::invalid_input(
                existing.rotated_to_token_id.as_deref().map_or_else(
                    || "A revoked admin token cannot be rotated; mint a new one".to_owned(),
                    |replacement| rerotated_token_refusal(&existing.id, replacement),
                ),
            ));
        }
        Ok(Self {
            service_name: existing.service_name,
            service_description: existing.service_description,
            // A super-admin token holds every permission by its flag, so its
            // replacement takes the full set as it stands today, not the
            // list stored when the old token was minted: a permission added
            // since would otherwise be missing from it.
            permissions: (!existing.is_super_admin).then(|| existing.permissions.to_vec()),
            expires_in_days: Some(expires_in_days),
            is_super_admin: existing.is_super_admin,
            tenant_id: existing.tenant_id,
            operator_user_id: existing
                .operator_user_id
                .filter(|admin| *admin == DELETED_ACCOUNT_ADMIN || rotated_by == Some(*admin)),
        })
    }

    /// The request as an admin-token creator may submit it.
    ///
    /// Names starting with [`DEVICE_CLI_SERVICE_PREFIX`] belong to the device
    /// grant alone, so a token named like one was minted by an approved login.
    /// The admin a token acts as is its `operator_user_id`, never its name,
    /// so a look-alike could not borrow an identity either way — this keeps
    /// the token list from showing one.
    ///
    /// A lifetime no date can hold is refused here too ([`Self::expires_at`]),
    /// so the creator is answered before anything is minted.
    ///
    /// # Errors
    /// Returns an invalid-input error for a reserved name, or for a lifetime
    /// past any date.
    pub fn for_creator(self) -> AppResult<Self> {
        self.expires_at(Timestamp::now())?;
        if self.service_name.starts_with(DEVICE_CLI_SERVICE_PREFIX) {
            return Err(AppError::invalid_input(format!(
                "Service names starting with '{DEVICE_CLI_SERVICE_PREFIX}' are reserved for device logins"
            )));
        }
        Ok(self)
    }

    /// The same request, for a token that acts as `admin`.
    #[must_use]
    pub const fn operated_by(mut self, admin: Uuid) -> Self {
        self.operator_user_id = Some(admin);
        self
    }
}

/// Service-name prefix of the super-admin token a device login mints.
///
/// The rest of the name is the id of the admin who approved the login —
/// a label for the token list, never an identity and never an email (service
/// names are logged). The identity is the token's `operator_user_id`, stored
/// on its row at the device grant; creators may not choose a name with this
/// prefix ([`CreateAdminTokenRequest::for_creator`]).
pub const DEVICE_CLI_SERVICE_PREFIX: &str = "device-cli:";

/// What stands in a token's name where a deleted account's id stood.
///
/// A service name is free text, so an account's id can be anywhere in one:
/// after [`DEVICE_CLI_SERVICE_PREFIX`] in a device login's, or wherever
/// whoever minted a service token wrote it. The account deletion rewrites
/// the id to this wherever a name carries it, and leaves the rest of the
/// name as it was.
pub const DELETED_ACCOUNT_LABEL: &str = "deleted-account";

/// The name a device login keeps once the account its name carried is
/// deleted: [`DEVICE_CLI_SERVICE_PREFIX`] then [`DELETED_ACCOUNT_LABEL`].
///
/// A rotation by someone else hands a device login's replacement to whoever
/// rotated it under the same name, so the name can outlive the approver as
/// a label on a token that acts as no one. The account deletion rewrites
/// that label to this one, and gives the same name to the device logins the
/// account approved, which it keeps as rows that act as
/// [`DELETED_ACCOUNT_ADMIN`]: nothing in `admin_tokens` names a deleted
/// account.
pub const DEVICE_CLI_DELETED_ACCOUNT_SERVICE: &str = "device-cli:deleted-account";

/// The admin a deleted account's device logins are left with: the nil
/// UUID, which is no account's id (ids are random version-4 UUIDs, and the
/// two statements that insert a `users` row refuse this one).
///
/// The account deletion keeps those rows, so the request log and the record
/// of the keys they provisioned stay, and rewrites who they act as to this.
/// It is never `NULL`: a token with no admin is a service credential,
/// which no admin's standing guards. A token that acts as this one is
/// refused by the admin gate for as long as it exists, whatever its row says
/// about being active, because the admin it names is no admin; and a
/// rotation carries it over ([`CreateAdminTokenRequest::rotation_of`]), so
/// neither is its replacement.
pub const DELETED_ACCOUNT_ADMIN: Uuid = Uuid::nil();

/// Generated admin token response
#[derive(Debug, Clone, Serialize)]
pub struct GeneratedAdminToken {
    /// Unique identifier for the token
    pub token_id: String,
    /// Name of the service
    pub service_name: String,
    /// The actual JWT token (shown only once!)
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub jwt_token: SecretString,
    /// Visible prefix for identification
    pub token_prefix: String,
    /// Permissions granted to this token
    pub permissions: AdminPermissions,
    /// Whether this is a super admin token
    pub is_super_admin: bool,
    /// When the token expires (if set)
    pub expires_at: Option<Timestamp>,
    /// When the token was created
    pub created_at: Timestamp,
}

/// Admin token usage audit entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminTokenUsage {
    /// Unique identifier for this usage record
    pub id: Option<i64>,
    /// ID of the admin token that was used
    pub admin_token_id: String,
    /// When the action was performed
    pub timestamp: Timestamp,
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
    /// Clear a deleted account's import tombstone, so its identity is
    /// accepted again
    ClearImportTombstone,
    /// Copy one account's signed-in-session provider connection onto another
    CloneProviderConnection,
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
            Self::ClearImportTombstone => write!(f, "clear_import_tombstone"),
            Self::CloneProviderConnection => write!(f, "clone_provider_connection"),
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
            "clear_import_tombstone" => Ok(Self::ClearImportTombstone),
            "clone_provider_connection" => Ok(Self::CloneProviderConnection),
            _ => Err(format!("Unknown admin action: {s}")),
        }
    }
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
    /// The admin the token acts as: the approving admin for a
    /// device-login token (from its stored row), the signed-in admin for a
    /// console session; `None` for a service token, which names no person
    /// Named after the `admin_tokens.operator_user_id` column, which is never
    /// renamed: a column rename makes a rollback deploy unsafe.
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

#[cfg(test)]
mod tests {
    use super::AdminAction;

    #[test]
    fn the_clone_action_reads_back_as_stored() {
        let stored = AdminAction::CloneProviderConnection.to_string();
        // A persisted value: `admin_token_usage.action` rows already carry it.
        assert_eq!(
            stored, "clone_provider_connection",
            "the stored action name changed under existing audit rows"
        );
        assert_eq!(
            stored.parse::<AdminAction>(),
            Ok(AdminAction::CloneProviderConnection)
        );
    }
}
