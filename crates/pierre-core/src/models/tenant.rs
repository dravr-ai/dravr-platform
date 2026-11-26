// ABOUTME: Multi-tenant organization models for OAuth apps and LLM credentials
// ABOUTME: TenantId newtype, Tenant, client grants and credential types
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
//!   `pierre_runtime_context::tenant::resolve_tenant`, and this type does not
//!   carry that verdict. Do not read a `TenantId` parameter as "already
//!   authorized".

use dravr_tronc::dates::Timestamp;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

use super::{ChildGrant, StepUpTerms};

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
    /// says only that it is well-formed. Membership is verified separately by
    /// `pierre_runtime_context::tenant::resolve_tenant`, and callers on the
    /// client-input paths must do that before the value is used to scope
    /// anything.
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
    pub created_at: Timestamp,
    /// When tenant was last updated
    pub updated_at: Timestamp,
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
        let now = Timestamp::now();
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

    /// The tenant a user's own lifecycle events are attributed to: the one
    /// they own (their personal tenant), else the first they belong to.
    ///
    /// Sign-up keys its operational events on the personal tenant, so an
    /// approval that later joins the user to an admin's tenant must still
    /// pick the personal one, or the two events land on different identities.
    #[must_use]
    pub fn attribution_tenant(tenants: &[Self], user_id: Uuid) -> Option<TenantId> {
        tenants
            .iter()
            .find(|tenant| tenant.owner_user_id == user_id)
            .or_else(|| tenants.first())
            .map(|tenant| tenant.id)
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
    pub granted_at: Timestamp,
    /// When the grant was revoked (soft delete); `None` while active
    pub revoked_at: Option<Timestamp>,
    /// The version of the consent screen's text the user approved: a digest
    /// of every line the screen can show, in [`Self::consent_locale`]. A
    /// later authorization asks again when the text in that language has
    /// changed since. `None` for a grant recorded before versions were, which
    /// is asked again too.
    pub consent_version: Option<String>,
    /// The language the consent screen was shown in when the user approved.
    pub consent_locale: Option<String>,
    /// The RFC 9396 authorization details the grant was resolved to, as one
    /// JSON document. `None` is a plain grant, which reaches what every grant
    /// reached before details existed.
    pub authorization_details: Option<String>,
    /// When the grant stops admitting a token; `None` never expires.
    pub expires_at: Option<Timestamp>,
}

impl OAuthClientGrant {
    /// Whether the grant still admits a token at `now`: not revoked, and not
    /// past its expiry.
    #[must_use]
    pub fn is_live_at(&self, now: Timestamp) -> bool {
        self.revoked_at.is_none() && self.expires_at.is_none_or(|expires_at| now < expires_at)
    }
}

/// Where a user's consent to one OAuth client stands in one tenant.
///
/// What a resource server asks of an access token it holds: the token is
/// self-contained, so only this says whether the athlete has since taken the
/// access back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientGrantStanding {
    /// Whether a live grant exists for the client: un-revoked and unexpired
    pub active: bool,
    /// When a grant for the client was last revoked; `None` when none ever was
    pub last_revoked_at: Option<Timestamp>,
    /// The client's live grants: un-revoked and unexpired when read
    pub grants: Vec<StandingGrant>,
}

/// One live grant of a [`ClientGrantStanding`]: what a token naming it by
/// its `client_grant_id` claim is admitted under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingGrant {
    /// The grant's id, the `oauth_client_grants` row
    pub id: String,
    /// The exact space-separated scope string the grant was consented to
    pub scope: String,
    /// The grant's resolved authorization details; `None` for a plain grant
    pub authorization_details: Option<String>,
    /// When the grant stops admitting a token; `None` never expires
    pub expires_at: Option<Timestamp>,
    /// The grant's live step-up child, if any (carnet#1544 slice 4b): what a
    /// token naming this grant also reads, never a grant a token names
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child: Option<StandingChild>,
}

/// The live step-up child of a [`StandingGrant`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingChild {
    /// The child's id, its `oauth_client_grants` row
    pub id: String,
    /// The child's resolved authorization details
    pub authorization_details: String,
    /// When the child stops reading
    pub expires_at: Option<Timestamp>,
    /// What the step-up recorded
    pub terms: StepUpTerms,
}

impl ClientGrantStanding {
    /// The standing of a client whose every grant, live or revoked, is
    /// `grants`, read at `now`.
    #[must_use]
    pub fn of_grants(grants: &[OAuthClientGrant], now: Timestamp) -> Self {
        let live: Vec<StandingGrant> = grants
            .iter()
            .filter(|grant| grant.is_live_at(now))
            .map(|grant| StandingGrant {
                id: grant.id.clone(),
                scope: grant.scope.clone(),
                authorization_details: grant.authorization_details.clone(),
                expires_at: grant.expires_at,
                child: None,
            })
            .collect();
        Self {
            active: !live.is_empty(),
            last_revoked_at: grants.iter().filter_map(|grant| grant.revoked_at).max(),
            grants: live,
        }
    }

    /// This standing with each live, unexpired child of `children` attached
    /// to its live parent; a child whose parent is not among the live grants
    /// is dropped, as is one carrying no details.
    #[must_use]
    pub fn with_children(mut self, children: &[ChildGrant], now: Timestamp) -> Self {
        for child in children.iter().filter(|child| child.grant.is_live_at(now)) {
            let Some(document) = child.grant.authorization_details.clone() else {
                continue;
            };
            if let Some(parent) = self
                .grants
                .iter_mut()
                .find(|grant| grant.id == child.terms.parent_grant_id)
            {
                parent.child = Some(StandingChild {
                    id: child.grant.id.clone(),
                    authorization_details: document,
                    expires_at: child.grant.expires_at,
                    terms: child.terms.clone(),
                });
            }
        }
        self
    }

    /// Whether an access token issued at `issued_at` (seconds since the epoch,
    /// the `iat` claim) and naming `client_grant_id` still speaks for the
    /// athlete at `now`.
    ///
    /// A token that names its grant is admitted only while that grant is
    /// among the live ones and unexpired at `now`: a standing read before the
    /// expiry may still be cached after it. A token naming none, minted
    /// before grants were named, keeps [`Self::admits_token_issued_at`].
    #[must_use]
    pub fn admits_token(
        &self,
        issued_at: i64,
        client_grant_id: Option<&str>,
        now: Timestamp,
    ) -> bool {
        client_grant_id.map_or_else(
            || self.admits_token_issued_at(issued_at),
            |id| {
                self.grants.iter().any(|grant| {
                    grant.id == id && grant.expires_at.is_none_or(|expires_at| now < expires_at)
                })
            },
        )
    }

    /// The live grant whose authorization details a token is held to, if any.
    ///
    /// A token naming its grant (`client_grant_id`) is held to that grant:
    /// `None` when that grant is plain. A token naming none, minted before
    /// grants were named, is held to the client's live grant for its `scope`
    /// when that one carries details, and otherwise to any live grant that
    /// does: a token without the claim never reads wider than a detailed
    /// consent the client holds. `None` only when no live grant carries
    /// details.
    #[must_use]
    pub fn detailed_grant_for(
        &self,
        client_grant_id: Option<&str>,
        scope: &str,
    ) -> Option<&StandingGrant> {
        let detailed = |grant: &&StandingGrant| grant.authorization_details.is_some();
        client_grant_id.map_or_else(
            || {
                self.grants
                    .iter()
                    .find(|grant| grant.scope == scope)
                    .filter(detailed)
                    .or_else(|| self.grants.iter().find(detailed))
            },
            |id| {
                self.grants
                    .iter()
                    .find(|grant| grant.id == id)
                    .filter(detailed)
            },
        )
    }

    /// Whether an access token issued at `issued_at` (seconds since the epoch,
    /// the `iat` claim) still speaks for the athlete.
    ///
    /// A client with no grant row was never withdrawn: consent predates the
    /// table, or was recorded before a write could refuse it, and its token
    /// runs to its expiry. Revoking ends every token the client held, so one
    /// issued before the last revocation stays refused after the athlete
    /// consents again: `iat` has one-second resolution, and a token issued in
    /// the revocation's own second counts as before it.
    #[must_use]
    pub fn admits_token_issued_at(&self, issued_at: i64) -> bool {
        self.last_revoked_at
            .is_none_or(|revoked_at| self.active && issued_at > revoked_at.epoch_seconds())
    }
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
    pub client_secret: SecretString,
    /// OAuth redirect URI
    pub redirect_uri: String,
    /// OAuth scopes
    pub scopes: Vec<String>,
    /// Daily rate limit for this tenant
    pub rate_limit_per_day: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use dravr_tronc::dates::Elapsed;

    fn owned_by(owner: Uuid) -> Tenant {
        Tenant::new(
            "t".to_owned(),
            "t".to_owned(),
            None,
            "starter".to_owned(),
            owner,
        )
    }

    #[test]
    fn attribution_prefers_the_tenant_the_user_owns() {
        let user = Uuid::new_v4();
        let admins = owned_by(Uuid::new_v4());
        let personal = owned_by(user);
        let tenants = [admins, personal.clone()];
        assert_eq!(
            Tenant::attribution_tenant(&tenants, user),
            Some(personal.id)
        );
    }

    #[test]
    fn attribution_falls_back_to_the_first_tenant() {
        let user = Uuid::new_v4();
        let first = owned_by(Uuid::new_v4());
        let tenants = [first.clone(), owned_by(Uuid::new_v4())];
        assert_eq!(Tenant::attribution_tenant(&tenants, user), Some(first.id));
    }

    #[test]
    fn attribution_is_none_without_tenants() {
        assert_eq!(Tenant::attribution_tenant(&[], Uuid::new_v4()), None);
    }

    fn grant(
        id: &str,
        revoked_at: Option<Timestamp>,
        expires_at: Option<Timestamp>,
    ) -> OAuthClientGrant {
        OAuthClientGrant {
            id: id.to_owned(),
            user_id: "u".to_owned(),
            tenant_id: "t".to_owned(),
            client_id: "c".to_owned(),
            scope: "fitness:read".to_owned(),
            granted_at: Timestamp::now() - Elapsed::days(1),
            revoked_at,
            consent_version: None,
            consent_locale: None,
            authorization_details: None,
            expires_at,
        }
    }

    #[test]
    fn a_standing_lists_only_live_unexpired_grants() {
        let now = Timestamp::now();
        let hour = Elapsed::hours(1);
        let standing = ClientGrantStanding::of_grants(
            &[
                grant("live", None, None),
                grant("dated", None, Some(now + hour)),
                grant("expired", None, Some(now - hour)),
                grant("revoked", Some(now - hour), None),
            ],
            now,
        );
        let ids: Vec<&str> = standing.grants.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(
            ids,
            ["live", "dated"],
            "bug: an expired or revoked grant is listed as live"
        );
        assert!(
            standing.active,
            "bug: a client with live grants reads as inactive"
        );
        assert_eq!(
            standing.last_revoked_at,
            Some(now - hour),
            "bug: the last revocation was not kept"
        );
    }

    #[test]
    fn an_expired_grant_does_not_count_as_active() {
        let now = Timestamp::now();
        let standing = ClientGrantStanding::of_grants(
            &[grant("expired", None, Some(now - Elapsed::seconds(1)))],
            now,
        );
        assert!(!standing.active, "bug: an expired grant counts as active");
        assert_eq!(
            standing.grants,
            Vec::new(),
            "bug: an expired grant is listed as live"
        );
    }

    #[test]
    fn a_token_naming_its_grant_is_admitted_only_while_that_grant_lives() {
        let now = Timestamp::now();
        let hour = Elapsed::hours(1);
        let standing = ClientGrantStanding::of_grants(
            &[
                grant("live", None, None),
                grant("revoked", Some(now - hour), None),
                grant("soon", None, Some(now + hour)),
            ],
            now,
        );
        let iat = now.epoch_seconds();
        assert!(
            standing.admits_token(iat, Some("live"), now),
            "bug: a token naming a live grant was refused"
        );
        assert!(
            !standing.admits_token(iat, Some("revoked"), now),
            "bug: a token naming a revoked grant was admitted"
        );
        assert!(
            !standing.admits_token(iat, Some("unknown"), now),
            "bug: a token naming an unknown grant was admitted"
        );
        // A standing cached before the expiry still refuses after it.
        assert!(
            standing.admits_token(iat, Some("soon"), now),
            "bug: a token naming a grant before its expiry was refused"
        );
        assert!(
            !standing.admits_token(iat, Some("soon"), now + hour),
            "bug: a cached standing admitted a token past its grant's expiry"
        );
    }

    #[test]
    fn a_token_naming_no_grant_keeps_the_issued_at_rule() {
        let now = Timestamp::now();
        let revoked_at = now - Elapsed::hours(1);
        let standing = ClientGrantStanding::of_grants(
            &[
                grant("old", Some(revoked_at), None),
                grant("new", None, None),
            ],
            now,
        );
        assert!(
            standing.admits_token(now.epoch_seconds(), None, now),
            "bug: a claimless token issued after the last revocation was refused"
        );
        assert!(
            !standing.admits_token(revoked_at.epoch_seconds(), None, now),
            "bug: a claimless token issued before the last revocation was admitted"
        );
        let never = ClientGrantStanding::of_grants(&[], now);
        assert!(
            never.admits_token(now.epoch_seconds(), None, now),
            "bug: a claimless token was refused though no grant was ever revoked"
        );
        assert!(
            !never.admits_token(now.epoch_seconds(), Some("any"), now),
            "bug: a token naming a grant was admitted with no grant live"
        );
    }

    fn detailed(id: &str, scope: &str) -> OAuthClientGrant {
        OAuthClientGrant {
            scope: scope.to_owned(),
            authorization_details: Some("[]".to_owned()),
            ..grant(id, None, None)
        }
    }

    #[test]
    fn a_token_is_held_to_the_detailed_grant_it_names_or_its_scope_falls_under() {
        let now = Timestamp::now();
        let standing = ClientGrantStanding::of_grants(
            &[
                grant("plain", None, None),
                detailed("preset", "fitness:read profile:read"),
            ],
            now,
        );
        let held = |claim, scope| {
            standing
                .detailed_grant_for(claim, scope)
                .map(|grant| grant.id.clone())
        };
        assert_eq!(
            held(Some("preset"), "fitness:read"),
            Some("preset".to_owned()),
            "bug: a token naming a preset grant is not held to it"
        );
        assert_eq!(
            held(Some("plain"), "fitness:read"),
            None,
            "a plain grant has no ceiling"
        );
        assert_eq!(
            held(Some("gone"), "fitness:read"),
            None,
            "bug: a token naming an unknown grant is held to one"
        );
        assert_eq!(
            held(None, "fitness:read profile:read"),
            Some("preset".to_owned()),
            "a claimless token is held to its scope's grant"
        );
        assert_eq!(
            held(None, "fitness:read"),
            Some("preset".to_owned()),
            "bug: a claimless token whose scope names a plain grant read wider than the \
             client's detailed one"
        );
        let plain_only = ClientGrantStanding::of_grants(&[grant("plain", None, None)], now);
        assert!(
            plain_only
                .detailed_grant_for(None, "fitness:read")
                .is_none(),
            "bug: a client with only plain grants held a token to details"
        );
    }
}
