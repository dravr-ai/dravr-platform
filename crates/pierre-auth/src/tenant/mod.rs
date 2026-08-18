// ABOUTME: Multi-tenant architecture support for enterprise SaaS deployment
// ABOUTME: Provides tenant management, OAuth credential isolation, and per-tenant rate limiting
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Multi-Tenant Architecture
//!
//! This module implements true multi-tenancy for Pierre MCP Server, enabling:
//! - Per-tenant OAuth credential management
//! - Per-tenant LLM API key management
//! - Tenant-isolated rate limiting
//! - Enterprise-ready `SaaS` deployment
//! - Secure tenant data isolation

/// LLM credential management for tenants and users
pub mod llm_manager;
/// Tenant-aware OAuth client implementation
pub mod oauth_client;
/// OAuth credential management for tenants
pub mod oauth_manager;
/// Tenant database schema and models
pub mod schema;

pub use llm_manager::{
    CredentialSource, LlmCredentials, LlmProvider, StoreLlmCredentialsRequest, TenantLlmManager,
};
pub use oauth_client::{StoreCredentialsRequest, TenantOAuthClient};
pub use oauth_manager::{CredentialConfig, TenantOAuthManager};
pub use pierre_core::models::{LlmCredentialRecord, LlmCredentialSummary, TenantOAuthCredentials};
pub use schema::{Tenant, TenantProviderUsage, TenantRole, TenantUser};

use pierre_core::models::TenantId;
use uuid::Uuid;

/// Tenant context for all operations.
///
/// # The role fence
///
/// This type is used two different ways, and only one of them establishes a
/// role:
///
/// - **Authorization contexts** come from a membership lookup against the
///   `tenant_users` junction table — the source of truth — and carry the role
///   it returned. [`TenantContext::from_verified_membership`].
/// - **Tenant-scoped contexts** are built by callers that already hold a
///   tenant and user and just need to name them for a downstream call (minting
///   an OAuth authorize URL, for instance). No membership lookup happened, so
///   there is no role to carry. [`TenantContext::for_tenant_scoped_operation`].
///
/// `user_role` is therefore `Option<TenantRole>` and **private**. Because one
/// field is private, this struct cannot be built by struct-literal syntax
/// outside this module — every construction has to name which of the two
/// constructors above applies, and say out loud whether a role was ever
/// established. [`TenantContext::is_admin`] is false for a context that never
/// resolved one, by construction rather than by convention.
///
/// This closes a real trapdoor: three call sites used to write
/// `user_role: TenantRole::Member` as a filler value simply to satisfy the
/// struct literal. Nothing read it, so nothing broke — but the type was
/// claiming a verified role it did not have, and `is_admin()` reads that field.
///
/// # What this does NOT give you
///
/// The remaining fields are still `pub`, so a context you were handed can be
/// read freely — the fence is on *minting* a context, not on reading one. And
/// holding a `TenantContext` proves a tenant was resolved, not that the caller
/// was authorized for any particular operation; role checks are still the
/// caller's job.
#[derive(Debug, Clone)]
pub struct TenantContext {
    /// Tenant ID
    pub tenant_id: TenantId,
    /// Tenant name for display
    pub tenant_name: String,
    /// User ID within tenant context
    pub user_id: Uuid,
    /// The user's role in this tenant, or `None` when no membership lookup
    /// established one. Private so it cannot be filled in with a placeholder;
    /// read it through [`TenantContext::role`].
    user_role: Option<TenantRole>,
    /// The Guardian per-turn token for the MCP/headless path — the JWT `jti`, but
    /// ONLY when the token is minted per turn (the ACP bridge mints one `jti` per
    /// chat turn, so every native tool call in that turn shares it). `None` for a
    /// reused session token (a stateless MCP client) so the Guardian keys each of
    /// its calls independently rather than accumulating budget/taint across the
    /// whole session (#2). Not an identity/audit field — its sole consumer is the
    /// Guardian turn key.
    pub session_id: Option<String>,
    /// The chat conversation this MCP call belongs to, on a per-turn (ACP)
    /// token only. `None` for a reused session token, which belongs to no
    /// single turn.
    ///
    /// Read from the signed `turn_conversation_id` claim, never from a header:
    /// its consumer is the router for detached follow-up work (the background
    /// activity backfill's completion push), and a caller-settable route would
    /// let one hand a notice to a conversation it does not own.
    pub conversation_id: Option<String>,
}

impl TenantContext {
    /// Build a context from a completed membership lookup.
    ///
    /// Call this only where `user_role` came from the `tenant_users` table for
    /// this exact (user, tenant) pair. The role travels with the context and
    /// feeds [`Self::is_admin`], so passing a guessed or default role here
    /// would launder a guess into an authorization input.
    #[must_use]
    pub const fn from_verified_membership(
        tenant_id: TenantId,
        tenant_name: String,
        user_id: Uuid,
        user_role: TenantRole,
    ) -> Self {
        Self {
            tenant_id,
            tenant_name,
            user_id,
            user_role: Some(user_role),
            session_id: None,
            conversation_id: None,
        }
    }

    /// Build a context that names a tenant and user for a scoped operation,
    /// without asserting anything about membership.
    ///
    /// For callers that already hold both ids and need to hand them to a
    /// downstream API — minting an OAuth authorize URL, resolving per-tenant
    /// provider credentials. No role is established, so [`Self::is_admin`] is
    /// false. If you need an authorization decision, do the membership lookup
    /// and use [`Self::from_verified_membership`] instead.
    #[must_use]
    pub const fn for_tenant_scoped_operation(
        tenant_id: TenantId,
        tenant_name: String,
        user_id: Uuid,
    ) -> Self {
        Self {
            tenant_id,
            tenant_name,
            user_id,
            user_role: None,
            session_id: None,
            conversation_id: None,
        }
    }

    /// Attach the originating session/token id (the JWT `jti`) used by the
    /// Guardian as the turn token for taint accumulation on the MCP path.
    #[must_use]
    pub fn with_session_id(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }

    /// Attach the originating chat conversation, from the signed per-turn
    /// claim, so a natively-called tool can route detached work back to it.
    #[must_use]
    pub fn with_conversation_id(mut self, conversation_id: Option<String>) -> Self {
        self.conversation_id = conversation_id;
        self
    }

    /// The user's role in this tenant, or `None` if no membership lookup
    /// established one.
    #[must_use]
    pub const fn role(&self) -> Option<TenantRole> {
        self.user_role
    }

    /// Check if user has admin privileges in this tenant.
    ///
    /// False when no membership lookup established a role — an unresolved
    /// context is never an admin.
    #[must_use]
    pub const fn is_admin(&self) -> bool {
        matches!(self.user_role, Some(TenantRole::Admin | TenantRole::Owner))
    }
}
