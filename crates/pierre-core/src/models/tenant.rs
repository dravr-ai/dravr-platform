// ABOUTME: Multi-tenant organization models for OAuth apps and LLM credentials
// ABOUTME: TenantId newtype, Tenant, OAuthApp, OAuthAppParams, AuthorizationCode, and credential types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Multi-tenant organization models, and the tenant-identity fence.
//!
//! # The fence
//!
//! Every scoped query in this codebase carries a [`TenantId`] in its `WHERE`
//! clause. That makes the type the load-bearing part of tenant isolation, so
//! it is deliberately awkward to produce one by accident:
//!
//! - The inner UUID is **private**. A `TenantId` cannot be built by tuple
//!   construction, and `.0` is not reachable from other crates — callers go
//!   through [`TenantId::from_uuid`] and [`TenantId::as_uuid`], which read as
//!   deliberate conversions at the call site.
//! - There is **no `Default`**. A defaulted tenant id used to mean
//!   `Uuid::new_v4()`, so a `..Default::default()` or a derived `Default`
//!   silently minted a tenant that exists in no table — and every
//!   `WHERE tenant_id = $1` against it then returned *empty* rather than
//!   failing. Minting is now spelled [`TenantId::generate`], which reads as
//!   the act it is and appears only at real tenant-creation sites.
//! - There is **no `Deserialize`**. A tenant id cannot be parsed out of a
//!   request body, a cached blob, or any other document — the type simply has
//!   no serde entry point. `Serialize` is kept because writing one out cannot
//!   forge anything.
//! - There is **no `From<Uuid>` and no `AsRef<Uuid>`**, so a bare UUID never
//!   becomes a tenant via an invisible `.into()`. Conversion is spelled
//!   [`TenantId::from_uuid`].
//! - There is **no `FromStr`**, so no anonymous `"...".parse()` can mint one.
//!   Parsing is spelled [`TenantId::parse_str`], which `rg` finds in one
//!   search — including on the two paths that matter most, the JWT
//!   `active_tenant_id` claim and the `x-tenant-id` header.
//!
//! The only remaining ways in are `from_uuid`, `parse_str`, `generate`, `nil`,
//! and the sqlx `Decode` impls.
//!
//! # What this fence does NOT give you
//!
//! State this plainly rather than overclaim, because the gap is where bugs
//! live:
//!
//! - **This is a lint-and-review fence, not a compiler fence.** The
//!   constructors above are `pub` because other crates legitimately need them,
//!   so a determined caller elsewhere can still mint a `TenantId`. What the
//!   type removes is the *accidental* path — the stray `.into()`, the
//!   defaulted field, the payload that happens to carry a `tenant_id`. Review
//!   closes the deliberate one.
//! - **Holding a `TenantId` proves nothing about authorization.** It does not
//!   mean the caller belongs to that tenant. Membership is verified in
//!   `pierre_middleware::tenant` (`verify_tenant_membership`), and this type
//!   does not carry that verdict. Do not read a `TenantId` parameter as
//!   "already authorized".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Type-safe wrapper for tenant identifiers.
///
/// Provides compile-time distinction between tenant IDs and other UUIDs. See
/// the module documentation for what this fence does and does not guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct TenantId(Uuid);

impl TenantId {
    /// Mint a brand-new random `TenantId` for a tenant being created.
    ///
    /// This allocates a fresh identity — call it only where a tenant is
    /// genuinely being brought into existence, never as a fallback for a
    /// value that failed to parse or was not supplied. There is deliberately
    /// no `Default` impl for exactly that reason.
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a `TenantId` from a UUID
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the inner UUID value
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// The nil (all-zeros) `TenantId`, used as the explicit "not tenant-scoped"
    /// marker in cache keys for genuinely global resources.
    ///
    /// `CacheKey` requires a tenant, so deliberately global entries — the
    /// link-token replay burn-list and the per-user mint rate limiter, neither
    /// of which belongs to a tenant — name this sentinel to say so out loud.
    ///
    /// It is **not** a fallback for a tenant that failed to parse. Returning
    /// nil on a decode error turns a data-integrity fault into a valid-looking
    /// value that quietly matches nothing; propagate the error instead.
    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }

    /// Check if this is a nil `TenantId`
    #[must_use]
    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }
}

impl From<TenantId> for Uuid {
    fn from(tenant_id: TenantId) -> Self {
        tenant_id.0
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TenantId {
    /// Parse a tenant id from a string.
    ///
    /// Deliberately **not** a `FromStr` impl. `FromStr` makes every
    /// `"...".parse()` in the codebase a potential tenant-minting site,
    /// findable only by inferring the target type — including the two that
    /// matter most, the JWT `active_tenant_id` claim and the `x-tenant-id`
    /// header. A named constructor puts the act in the source text, so
    /// `rg TenantId::parse_str` finds every one of them.
    ///
    /// The name deliberately claims nothing about trust. Some callers pass a
    /// `tenant_id` column the server wrote; others pass a request header the
    /// client controls. **Parsing is not authorization** — a well-formed UUID
    /// says only that it is well-formed. Membership is verified separately in
    /// `pierre_middleware::tenant`, and callers on the client-input paths must
    /// do that before the value is used to scope anything.
    ///
    /// # Errors
    /// Returns `uuid::Error` when `s` is not a valid UUID.
    pub fn parse_str(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

// SQLite stores UUIDs as TEXT. The default sqlx transparent derive delegates to Uuid
// which encodes as BLOB for SQLite. These manual implementations ensure TenantId
// serializes as a hyphenated TEXT string, matching the database schema.
#[cfg(feature = "sqlx-sqlite")]
mod sqlite_impl {
    use sqlx::encode::IsNull;
    use sqlx::error::BoxDynError;
    use sqlx::sqlite::{Sqlite, SqliteArgumentValue, SqliteTypeInfo, SqliteValueRef};
    use sqlx::{Decode, Encode, Type};
    use uuid::Uuid;

    use super::TenantId;

    impl Type<Sqlite> for TenantId {
        fn type_info() -> SqliteTypeInfo {
            <String as Type<Sqlite>>::type_info()
        }
    }

    impl<'q> Encode<'q, Sqlite> for TenantId {
        fn encode_by_ref(
            &self,
            buf: &mut Vec<SqliteArgumentValue<'q>>,
        ) -> Result<IsNull, BoxDynError> {
            let text = self.0.to_string();
            <String as Encode<'q, Sqlite>>::encode(text, buf)
        }
    }

    impl<'r> Decode<'r, Sqlite> for TenantId {
        fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
            let text = <String as Decode<'r, Sqlite>>::decode(value)?;
            let uuid = Uuid::parse_str(&text)?;
            Ok(Self(uuid))
        }
    }
}

// PostgreSQL stores UUIDs natively. These implementations delegate to Uuid's
// built-in Postgres Type/Encode/Decode, wrapping/unwrapping the TenantId newtype.
#[cfg(feature = "sqlx-postgres")]
mod postgres_impl {
    use sqlx::encode::IsNull;
    use sqlx::error::BoxDynError;
    use sqlx::postgres::{PgArgumentBuffer, PgTypeInfo, PgValueRef, Postgres};
    use sqlx::{Decode, Encode, Type};
    use uuid::Uuid;

    use super::TenantId;

    impl Type<Postgres> for TenantId {
        fn type_info() -> PgTypeInfo {
            <Uuid as Type<Postgres>>::type_info()
        }
    }

    impl<'q> Encode<'q, Postgres> for TenantId {
        fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
            <Uuid as Encode<'q, Postgres>>::encode_by_ref(&self.0, buf)
        }
    }

    impl<'r> Decode<'r, Postgres> for TenantId {
        fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
            let uuid = <Uuid as Decode<'r, Postgres>>::decode(value)?;
            Ok(Self(uuid))
        }
    }
}

/// Tenant organization in multi-tenant setup
#[derive(Debug, Clone, Serialize)]
pub struct Tenant {
    /// Unique tenant identifier
    pub id: TenantId,
    /// Tenant organization name
    pub name: String,
    /// URL-safe slug for tenant
    pub slug: String,
    /// Custom domain for tenant (optional)
    pub domain: Option<String>,
    /// Subscription plan (basic, pro, enterprise)
    pub plan: String,
    /// User ID of the tenant owner
    pub owner_user_id: Uuid,
    /// When tenant was created
    pub created_at: DateTime<Utc>,
    /// When tenant was last updated
    pub updated_at: DateTime<Utc>,
}

impl Tenant {
    /// Creates a new tenant with the given details
    #[must_use]
    pub fn new(
        name: String,
        slug: String,
        domain: Option<String>,
        plan: String,
        owner_user_id: Uuid,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: TenantId::generate(),
            name,
            slug,
            domain,
            plan,
            owner_user_id,
            created_at: now,
            updated_at: now,
        }
    }
}

/// OAuth application registration for MCP clients
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthApp {
    /// Unique app identifier
    pub id: Uuid,
    /// OAuth client ID
    pub client_id: String,
    /// OAuth client secret
    pub client_secret: String,
    /// Application name
    pub name: String,
    /// Application description
    pub description: Option<String>,
    /// Allowed redirect URIs
    pub redirect_uris: Vec<String>,
    /// Permitted scopes
    pub scopes: Vec<String>,
    /// OAuth 2.0 client type: "public" or "confidential"
    pub app_type: String,
    /// User ID of the app owner
    pub owner_user_id: Uuid,
    /// When app was registered
    pub created_at: DateTime<Utc>,
    /// When app was last updated
    pub updated_at: DateTime<Utc>,
}

/// OAuth app creation parameters
pub struct OAuthAppParams {
    /// OAuth 2.0 client identifier
    pub client_id: String,
    /// OAuth 2.0 client secret for authentication
    pub client_secret: String,
    /// Human-readable name of the OAuth application
    pub name: String,
    /// Optional description of the application's purpose
    pub description: Option<String>,
    /// List of authorized redirect URIs for OAuth flow
    pub redirect_uris: Vec<String>,
    /// List of OAuth scopes the app can request
    pub scopes: Vec<String>,
    /// OAuth 2.0 client type: "public" or "confidential"
    pub app_type: String,
    /// UUID of the user who owns this OAuth app
    pub owner_user_id: Uuid,
}

impl OAuthApp {
    /// Create new OAuth app from parameters
    #[must_use]
    pub fn new(params: OAuthAppParams) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            client_id: params.client_id,
            client_secret: params.client_secret,
            name: params.name,
            description: params.description,
            redirect_uris: params.redirect_uris,
            scopes: params.scopes,
            app_type: params.app_type,
            owner_user_id: params.owner_user_id,
            created_at: now,
            updated_at: now,
        }
    }
}

/// OAuth authorization code for token exchange
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationCode {
    /// The authorization code
    pub code: String,
    /// Client ID that requested the code
    pub client_id: String,
    /// Redirect URI used in the request
    pub redirect_uri: String,
    /// Requested scopes
    pub scope: String,
    /// User ID that authorized the request
    pub user_id: Option<Uuid>,
    /// When the code expires
    pub expires_at: DateTime<Utc>,
    /// When the code was created
    pub created_at: DateTime<Utc>,
    /// Whether the code has been used
    pub is_used: bool,
}

impl AuthorizationCode {
    /// Creates a new authorization code with 10-minute expiration
    #[must_use]
    pub fn new(
        code: String,
        client_id: String,
        redirect_uri: String,
        scope: String,
        user_id: Option<Uuid>,
    ) -> Self {
        let now = Utc::now();
        Self {
            code,
            client_id,
            redirect_uri,
            scope,
            user_id,
            expires_at: now + chrono::Duration::minutes(10), // 10 minute expiration
            created_at: now,
            is_used: false,
        }
    }

    /// Check if the authorization code is expired
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Check if the authorization code is valid for use
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.is_used && !self.is_expired()
    }

    /// Mark the authorization code as used
    pub const fn mark_used(&mut self) {
        self.is_used = true;
    }
}

/// Persisted record of a user's consent to an MCP OAuth client.
///
/// Written when a user approves a client on the OAuth consent screen. A later
/// `/oauth2/authorize` for the same `(user_id, tenant_id, client_id, scope)`
/// finds an un-revoked grant and mints the authorization code without
/// re-prompting. Revoking a connected app sets `revoked_at` (soft delete,
/// preserving the audit trail) so the next authorization shows the consent
/// screen again. `scope` is the exact space-separated scope string that was
/// consented to — a request for a different scope set does not match and
/// re-prompts.
#[derive(Debug, Clone)]
pub struct OAuthClientGrant {
    /// Unique grant identifier (uuid string supplied by the caller)
    pub id: String,
    /// User who granted consent
    pub user_id: String,
    /// Tenant the grant belongs to
    pub tenant_id: String,
    /// OAuth client the consent was granted to
    pub client_id: String,
    /// Exact space-separated scope string that was consented to
    pub scope: String,
    /// When the grant was recorded
    pub granted_at: DateTime<Utc>,
    /// When the grant was revoked (soft delete); `None` while active
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Database record for LLM credentials
#[derive(Debug, Clone)]
pub struct LlmCredentialRecord {
    /// Record ID
    pub id: Uuid,
    /// Tenant ID
    pub tenant_id: TenantId,
    /// User ID (None = tenant default)
    pub user_id: Option<Uuid>,
    /// Provider name
    pub provider: String,
    /// Encrypted API key
    pub api_key_encrypted: String,
    /// Base URL (for local providers)
    pub base_url: Option<String>,
    /// Default model
    pub default_model: Option<String>,
    /// Is this credential active
    pub is_active: bool,
    /// Created timestamp
    pub created_at: String,
    /// Updated timestamp
    pub updated_at: String,
    /// Created by user ID
    pub created_by: Uuid,
}

/// Summary of LLM credentials (for listing, without decrypted key)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCredentialSummary {
    /// Record ID
    pub id: Uuid,
    /// User ID (None = tenant default)
    pub user_id: Option<Uuid>,
    /// Provider name
    pub provider: String,
    /// Whether this is a user-specific or tenant-level credential
    pub scope: String,
    /// Base URL (for local providers)
    pub base_url: Option<String>,
    /// Default model
    pub default_model: Option<String>,
    /// Is active
    pub is_active: bool,
    /// Created timestamp
    pub created_at: String,
    /// Updated timestamp
    pub updated_at: String,
}

/// Per-tenant OAuth credentials with decrypted secret
#[derive(Debug, Clone)]
pub struct TenantOAuthCredentials {
    /// Tenant ID that owns these credentials
    pub tenant_id: TenantId,
    /// OAuth provider name
    pub provider: String,
    /// OAuth client ID (public)
    pub client_id: String,
    /// OAuth client secret (decrypted)
    pub client_secret: String,
    /// OAuth redirect URI
    pub redirect_uri: String,
    /// OAuth scopes
    pub scopes: Vec<String>,
    /// Daily rate limit for this tenant
    pub rate_limit_per_day: u32,
}
