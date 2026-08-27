// ABOUTME: Narrow runtime-context traits decoupling pierre-server subsystems from ServerContext
// ABOUTME: Each trait represents exactly the slice one extractable crate needs to depend on
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Runtime Context Traits
//!
//! `ServerContext` (in `pierre-server`) is the composition root: it holds every
//! shared resource (database, repository registry, auth/JWKS managers, CSRF
//! manager, tool registry, providers, cache, …). Extractable subsystems —
//! middleware, route groups, config-routes, providers (server-side) — need
//! a *subset* of those resources.
//!
//! Rather than pull `ServerContext` into every leaf crate (which would cycle —
//! `pierre-server` ↔ leaf), this crate exposes one narrow trait per subsystem.
//! `ServerContext` implements them; the leaves accept `&impl MiddlewareCtx` (or
//! whichever applies) and never see the composition root.
//!
//! ## Design rules
//!
//! 1. **Interface segregation.** One trait per consumer concern. Don't add a
//!    method until a concrete extracted crate proves it needs it.
//! 2. **Return types live in base crates.** Every method returns a type from
//!    `pierre-database`, `pierre-auth`, `pierre-cache`, or `pierre-core` — never
//!    from `pierre-server`. Anything else would re-import the leak.
//! 3. **No default methods that read fields.** Keep traits free of state so
//!    `pierre-server` can implement them however it wants (field, method,
//!    derived value).

/// Data context holding database, cache, provider registry, and activity-intelligence handles.
///
/// Moved out of `pierre-server::context` so leaf crates can accept it
/// without depending on the composition root.
pub mod data;
/// Canonical per-request tenant resolution (see [`tenant`]).
pub mod tenant;

pub use data::DataContext;
pub use tenant::{resolve_tenant, TenantMode};

use std::sync::Arc;

use async_trait::async_trait;
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::{AuthManager, AuthResult};
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::billing::BillingProvider;
use pierre_core::errors::AppResult;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_llm::{ChatProvider, LlmProvider};

/// Slice of runtime state the identity route layer needs.
///
/// Today this covers the `/api/users/oauth-apps` endpoints (per-user OAuth
/// app credential storage backed by the `oauth_tokens` repository) — i.e.
/// just the repository registry. The OAuth 2.0 authorization server endpoints
/// in `pierre-routes-identity::oauth2` carry their own focused-context struct
/// (`OAuth2Context`) and do not flow through this trait.
///
/// When additional identity surfaces are extracted (credential login,
/// provider-OAuth-callback handling, Sciotte hosted-login) the trait will
/// grow new accessors; today those routes are still tangled with
/// `pierre_auth::firebase::FirebaseAuth`, `crate::services::auth::*`, and the
/// focused-context wrappers in `crate::context::*` and remain in
/// `pierre-server`.
pub trait IdentityCtx: Send + Sync + 'static {
    /// Repository registry — primary data-access surface for the
    /// `/api/users/oauth-apps` endpoints (per-user OAuth app credentials).
    fn repos(&self) -> &Arc<RepositoryRegistry>;
}

/// Slice of runtime state the dashboard route layer needs.
///
/// Covers `/api/dashboard/{overview,analytics,rate-limits,request-logs,request-stats,tool-usage}`
/// and the alternative `/dashboard/*` aliases. Every handler reads from the
/// repository registry only (`api_keys`, `usage`, `llm_usage` repos) — no
/// auth manager, no providers, no cache. The composition root in
/// `pierre-server` implements this trait on its `ServerContext`; the route
/// handlers also require [`MiddlewareCtx`] to satisfy the
/// `AuthenticatedUser` extractor.
pub trait DashboardCtx: Send + Sync + 'static {
    /// Repository registry — backs api-key, usage-stats, and LLM-usage
    /// daily-series lookups the dashboard endpoints perform.
    fn repos(&self) -> &Arc<RepositoryRegistry>;
}

/// Slice of runtime state the middleware layer needs.
///
/// Covers what `tenant`, `extractors`, `admin_guard`, and `csrf` actually pull
/// from pierre-server's `ServerContext`: the auth manager + JWKS for JWT claim
/// validation, the repository registry for tenant + user lookups, the CSRF
/// token manager for state-changing-request validation, and a delegating
/// `authenticate_request` entry point that dispatches into the auth-middleware
/// pipeline (token type detection, rate limiting, user status enforcement).
/// Anything broader belongs on a separate trait.
#[async_trait]
pub trait MiddlewareCtx: Send + Sync + 'static {
    /// Auth manager — validates JWT claims.
    fn auth_manager(&self) -> &Arc<AuthManager>;

    /// JWKS manager — provides the signing keys consumed by `auth_manager`.
    fn jwks_manager(&self) -> &Arc<JwksManager>;

    /// Repository registry — primary data-access surface (tenant + user lookups).
    fn repos(&self) -> &Arc<RepositoryRegistry>;

    /// CSRF token manager — validates `X-CSRF-Token` headers on state-changing
    /// requests from cookie-authenticated browser sessions.
    fn csrf_manager(&self) -> &Arc<CsrfTokenManager>;

    /// Run an inbound `Authorization` header value (or cookie-derived bearer
    /// token) through the full auth-middleware pipeline. Returns the resolved
    /// `AuthResult` (user id, auth method, rate limit, active tenant) or an
    /// `AppError` if the token is missing/invalid or the user fails policy
    /// checks.
    async fn authenticate_request(&self, auth_header: Option<&str>) -> AppResult<AuthResult>;

    /// Allowlist of hostnames permitted as a user-supplied LLM `base_url` (the
    /// `local`/OpenAI-compatible provider). An empty slice denies every custom
    /// `base_url`, closing the SSRF where a tenant points the server's outbound LLM
    /// call at an internal host (T3MP3ST F2 / CWE-918). Sourced from
    /// `SecurityConfig::llm_base_url_allowlist`; returned as a primitive slice so this
    /// crate stays free of a `pierre-config` dependency.
    fn llm_base_url_allowlist(&self) -> &[String];
}

/// Slice of runtime state the MCP transport dispatch layer needs.
///
/// Covers the JWT-validation + tenant-resolution surface that `pierre-mcp-transport`'s
/// `tenant_isolation` helpers (`validate_jwt_token_for_mcp`, `extract_tenant_context_internal`,
/// and the `TenantIsolation` accessor struct) pull from `ServerContext` today.
///
/// The methods are intentionally narrow: just the auth pair (manager + JWKS) plus the
/// repository registry. Future strategy-B extraction waves (transport startup,
/// per-request dispatch, sampling peer routing) will grow this trait with new
/// accessors — never with general "give me the whole context" knobs.
pub trait McpDispatchCtx: Send + Sync + 'static {
    /// Auth manager — used by `validate_token` to verify JWT signatures and parse claims.
    fn auth_manager(&self) -> &Arc<AuthManager>;

    /// JWKS manager — supplies signing keys consumed by `auth_manager`.
    fn jwks_manager(&self) -> &Arc<JwksManager>;

    /// Repository registry — backs the tenant + user + OAuth-token lookups
    /// the dispatch layer performs after a JWT is accepted.
    fn repos(&self) -> &Arc<RepositoryRegistry>;
}

/// Slice of runtime state the A2A (Agent-to-Agent) protocol layer needs.
///
/// Covers what `pierre_a2a::protocol::A2AServer` and `pierre_a2a::auth::A2AAuthenticator`
/// pull from `ServerContext` today: the JWT-validation auth pair (manager + JWKS) used
/// by the A2A protocol's bearer-token authentication, the repository registry for
/// A2A-client / task / push-config lookups, and the configured server base URL
/// used to construct the agent card's `supportedInterfaces` URLs.
///
/// Deliberately omitted from this trait (to keep `pierre-runtime-context` free of
/// `pierre-tool-runtime` / `pierre-middleware` / `pierre-a2a` cycles):
/// - `Arc<dyn ToolRuntime>` — `A2AServer` accepts a separate `Arc<dyn ToolRuntime>`
///   for `SendMessage` tool-intent dispatch.
/// - `Arc<McpAuthMiddleware>` — `A2AAuthenticator` accepts a separate handle for
///   API-key authentication.
/// - `Arc<A2AClientManager>` — `A2AAuthenticator` accepts a separate handle for
///   per-client rate-limit lookups (its type lives in `pierre-a2a`).
pub trait A2ACtx: Send + Sync + 'static {
    /// Auth manager — validates JWT bearer tokens for A2A protocol requests.
    fn auth_manager(&self) -> &Arc<AuthManager>;

    /// JWKS manager — supplies signing keys consumed by `auth_manager`.
    fn jwks_manager(&self) -> &Arc<JwksManager>;

    /// Repository registry — primary data-access surface for A2A operations
    /// (a2a clients, sessions, tasks, push-notification configs, usage;
    /// `tenants` for default-tenant resolution during `OAuth2`
    /// authentication).
    fn repos(&self) -> &Arc<RepositoryRegistry>;

    /// Configured server base URL — used to construct the agent card's
    /// `supportedInterfaces` URLs.
    fn base_url(&self) -> &str;
}

/// Slice of runtime state the billing route layer needs.
///
/// Covers `/api/billing/{checkout,portal,subscription,invoices}` and the
/// `/webhooks/{provider}` receiver: the active `BillingProvider` trait
/// object (for hosted-session creation, invoice listing, webhook
/// signature verification + payload normalization) and the repository
/// registry (for subscription row reads/writes and webhook-event
/// idempotency keyed on `(provider, event_id)`). Anything broader
/// belongs on a separate trait.
pub trait BillingCtx: Send + Sync + 'static {
    /// Billing provider — handles checkout/portal sessions, invoice
    /// listing, and webhook envelope parsing for the configured vendor
    /// (Stripe, `RevenueCat`, the in-tree `DummyProvider`, …).
    fn billing_provider(&self) -> &Arc<dyn BillingProvider>;

    /// Repository registry — primary data-access surface for
    /// subscription row mutations and webhook idempotency tracking.
    fn repos(&self) -> &Arc<RepositoryRegistry>;
}

/// Slice of runtime state the slash-command handler layer needs.
///
/// Covers what `pierre-commands` handlers actually pull from
/// pierre-server's `ServerContext`: the repository registry (for coach /
/// group / user / chat / provider-connection lookups), the
/// [`pierre_groups::GroupService`] (for `/group invite` code minting and
/// the group settings `/coach add` changes in a group conversation), the
/// [`pierre_contremaitre::MessagingStringsRegistry`] (for the localized
/// reply rendering every handler performs), and the coach-generation
/// slice `/coach create` needs — the chat provider, the generation prompt
/// and the admin-config quota lookup.
///
/// Deliberately omitted from this trait (to keep `pierre-runtime-context`
/// free of the `pierre-commands` → `pierre-runtime-context` cycle):
/// - `Arc<CommandRegistry>` / `Arc<CommandHandlerRegistry>` — the
///   dispatcher entry point (`pierre_commands::dispatch::try_dispatch`)
///   takes both as explicit arguments. Callers pull them from
///   `ServerContext.command_registry` / `ServerContext.command_handler_registry`
///   and hand them in directly; no trait round-trip is needed because
///   `try_dispatch` is the only consumer.
pub trait CommandCtx: Send + Sync + 'static {
    /// Repository registry — backs every coach / group / user / chat /
    /// provider-connection lookup the handlers perform.
    fn repos(&self) -> &Arc<RepositoryRegistry>;

    /// Group coaching service — used by `/group invite` to mint a fresh
    /// invite code and by the group-settings writes `/coach add` and
    /// `/group coach` perform in a group conversation.
    fn group_service(&self) -> &Arc<pierre_groups::GroupService>;

    /// Messaging strings registry — every handler renders user-facing
    /// reply text through this registry, keyed by `(template_key, locale)`.
    fn messaging_strings_registry(&self) -> &Arc<MessagingStringsRegistry>;

    /// Admin config lookup — `/coach create confirm` enforces
    /// `usage_quotas.max_coaches_per_user` through it and `/group create`
    /// reads the tenant's `group_creation_policy`, the same reads
    /// `POST /api/coaches` and the REST group-create route perform.
    ///
    /// `None` when admin config is not wired into the running server; both
    /// then degrade to their documented defaults rather than being skipped.
    /// Returns an owned `Arc` because the concrete service in `pierre-server`
    /// is not stored as a trait object; the upcast happens here.
    fn admin_config(&self) -> Option<Arc<dyn AdminConfigLookup>>;

    /// Primary chat-completion provider — `/coach create` drafts the
    /// persona through it, on the shared singleton every chat turn uses.
    fn chat_provider(&self) -> Option<&Arc<ChatProvider>>;

    /// Lower-level LLM provider — the fallback `/coach create` wraps when no
    /// dedicated [`ChatProvider`] is configured.
    fn llm_provider(&self) -> Option<&Arc<dyn LlmProvider>>;

    /// Coach generation system prompt. Resolves from the contremaitre
    /// hot-reload registry when enabled, else the compiled-in fallback in
    /// `pierre-llm`.
    fn coach_generation_prompt(&self) -> String;
}

/// The identity a config read resolves against.
///
/// Overrides are stored per scope and resolved most-specific-first: a
/// per-user row wins over the user's tenant row, which wins over the
/// system-wide row. A field left `None` simply drops that rung from the
/// walk, so [`Self::global`] resolves system-wide values only.
///
/// Both dimensions travel in one struct rather than as two positional
/// `Option<&str>` arguments: a caller that swapped them would compile
/// and silently read the wrong scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConfigLookupScope<'a> {
    /// User the lookup is for. Highest-priority rung when set.
    pub user_id: Option<&'a str>,
    /// Tenant the lookup is for. Consulted when no per-user row exists.
    pub tenant_id: Option<&'a str>,
}

impl<'a> ConfigLookupScope<'a> {
    /// System-wide scope: no per-user or per-tenant rung.
    #[must_use]
    pub const fn global() -> Self {
        Self {
            user_id: None,
            tenant_id: None,
        }
    }

    /// Resolve against a tenant, then system-wide.
    #[must_use]
    pub const fn tenant(tenant_id: &'a str) -> Self {
        Self {
            user_id: None,
            tenant_id: Some(tenant_id),
        }
    }

    /// Resolve against a user, then their tenant, then system-wide.
    #[must_use]
    pub const fn user(user_id: &'a str, tenant_id: &'a str) -> Self {
        Self {
            user_id: Some(user_id),
            tenant_id: Some(tenant_id),
        }
    }
}

/// Read-only accessor for scope-aware admin config values.
///
/// Wraps the slice of `AdminConfigService` the route + service layers
/// actually consume (quota lookups, feature thresholds). The concrete
/// service lives in `pierre-server`; leaf crates depend only on this
/// trait so they don't pull in the composition root.
#[async_trait]
pub trait AdminConfigLookup: Send + Sync + 'static {
    /// Look up an effective config value for the given key. Returns
    /// `Some(value)` for both overrides and parameter defaults; `None`
    /// only when the key is not registered.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying repository read fails.
    async fn get_value(
        &self,
        key: &str,
        scope: ConfigLookupScope<'_>,
    ) -> AppResult<Option<serde_json::Value>>;

    /// Look up an explicit admin override for `key`, ignoring the
    /// parameter-definition default. Returns `None` when no override is
    /// set so callers can fall back to a tier-keyed or hard-coded
    /// default (e.g. `TierQuotaConfig` caps) without being shadowed by
    /// the flat catalog default.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying repository read fails.
    async fn get_override_value(
        &self,
        key: &str,
        scope: ConfigLookupScope<'_>,
    ) -> AppResult<Option<serde_json::Value>>;
}

/// No-override [`AdminConfigLookup`] used when the database-backed admin
/// config service failed to initialise.
///
/// Both lookups return `None`, so consumers fall back to their
/// compile-time defaults — for [`crate::AdminConfigLookup`]-driven quota
/// checks that means the per-tier caps in
/// `pierre_core::models::TierQuotaConfig`. Routing enforcement through
/// this instead of skipping it keeps quotas active (degraded to tier
/// defaults) rather than silently disabling them when admin config is
/// unavailable.
pub struct DefaultAdminConfig;

#[async_trait]
impl AdminConfigLookup for DefaultAdminConfig {
    async fn get_value(
        &self,
        _: &str,
        _: ConfigLookupScope<'_>,
    ) -> AppResult<Option<serde_json::Value>> {
        Ok(None)
    }

    async fn get_override_value(
        &self,
        _: &str,
        _: ConfigLookupScope<'_>,
    ) -> AppResult<Option<serde_json::Value>> {
        Ok(None)
    }
}

static DEFAULT_ADMIN_CONFIG: DefaultAdminConfig = DefaultAdminConfig;

/// Shared `'static` [`DefaultAdminConfig`] fallback.
///
/// Enforcement paths use this when the database-backed admin config
/// service is absent, so quota checks and counter writes degrade to tier
/// defaults instead of being skipped.
#[must_use]
pub fn default_admin_config() -> &'static dyn AdminConfigLookup {
    &DEFAULT_ADMIN_CONFIG
}

/// Slice of runtime state the coaches/roster/store route layer needs.
///
/// Covers `/api/coaches/*`, `/api/admin/coaches/*`, `/api/admin/store/*`,
/// `/api/roster/*`, and `/api/store/*`. Pulls the repository registry
/// (coaches + store-listings + roster + tenants + users repos), the
/// platform's `Database` handle (for the version-history author lookup),
/// and provider handles for the LLM re-rank of coach proposals.
///
/// `admin_config()` is optional because the service is wired only when
/// SQLite-backed quota config is enabled; routes degrade to defaults
/// when it is absent.
///
/// `notification_service()` is gated by the `client-notifications`
/// feature so leaf builds without push notifications don't pay the
/// `dravr-commere` dependency cost.
pub trait CoachesCtx: MiddlewareCtx {
    /// Database handle — the version-history routes reach the users
    /// repository through it to name a version's author.
    fn database(&self) -> &Arc<Database>;

    /// Admin config lookup — used by `handle_create` to enforce
    /// `usage_quotas.max_coaches_per_user` per tenant. `None` when
    /// admin config is not wired into the running server. Returns an
    /// owned `Arc` because the concrete service in `pierre-server` is
    /// not stored as a trait object; the upcast happens here.
    fn admin_config(&self) -> Option<Arc<dyn AdminConfigLookup>>;

    /// Notification dispatch service — used by admin coach update and
    /// assign handlers to fire `plan_updated` push notifications.
    /// `None` when notifications are not wired.
    #[cfg(feature = "client-notifications")]
    fn notification_service(&self) -> Option<&Arc<pierre_notifications::NotificationService>>;

    /// Primary chat-completion provider — the LLM re-rank of coach
    /// proposals runs on it. Falls back to wrapping `llm_provider()` when
    /// this is `None`.
    fn chat_provider(&self) -> Option<&Arc<ChatProvider>>;

    /// Lower-level LLM provider — the fallback the proposal re-rank wraps
    /// when no dedicated `ChatProvider` is configured.
    fn llm_provider(&self) -> Option<&Arc<dyn LlmProvider>>;
}

/// Slice of runtime state the groups and notifications route layers need.
///
/// Covers `/api/groups/*` (group CRUD, membership, invites, analytics) and
/// `/api/notifications/*` (device tokens, preferences, feed, scheduling).
/// The trait exposes the repository registry, the optional notification
/// service facade, the [`pierre_groups::GroupService`] business-logic
/// engine, an admin-config value reader (for the tenant
/// `group_creation_policy` check), and a fitness-snapshot builder used by
/// the group analytics endpoints. The snapshot builder is implemented in
/// `pierre-server` (it needs OAuth provider construction + per-member
/// activity fetches that live in the composition root); exposing it via
/// the trait keeps this crate decoupled from those internals.
#[async_trait]
pub trait GroupsCtx: Send + Sync + 'static {
    /// Optional push-notification service (Expo Push + dravr-commere).
    /// `None` when notifications are disabled by feature flag or
    /// initialization failed at startup.
    ///
    /// Repository access for the group and notification routes is provided
    /// by the [`MiddlewareCtx::repos`] accessor; the route handlers require
    /// both `GroupsCtx` and `MiddlewareCtx` bounds.
    #[cfg(feature = "client-notifications")]
    fn notification_service(&self) -> Option<&Arc<pierre_notifications::NotificationService>>;

    /// Group coaching service — strategies, aggregate stats, weekly
    /// reports, and health-flag computation over fitness snapshots.
    fn group_service(&self) -> &Arc<pierre_groups::GroupService>;

    /// Read a single admin-config value (per-tenant or platform-wide).
    ///
    /// Returns `None` when the admin-config service is not initialized,
    /// the key is unknown, the cached value can't be decoded, or any
    /// repository read fails — the route layer is expected to fall back
    /// to a documented default in that case.
    async fn admin_config_get(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> Option<serde_json::Value>;
}

/// Buffer-overflow strategy for SSE broadcast channels.
///
/// Mirrors `pierre_config::environment::SseBufferStrategy` as a primitive
/// the SSE layer can read through `SseCtx` without forcing this crate to
/// depend on `pierre-config` (which would create a cycle:
/// `pierre-config` → `pierre-middleware` → `pierre-runtime-context`).
///
/// The composition root in `pierre-server` maps its `SseBufferStrategy`
/// enum onto this primitive in its `SseCtx::sse_buffer_overflow_strategy`
/// implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseBufferOverflowStrategy {
    /// Drop the oldest queued messages when the broadcast channel lags
    /// (matches the default `tokio::sync::broadcast` behaviour).
    DropOldest,
    /// Configured as "drop newest"; broadcast channels inherently drop
    /// oldest, so the SSE layer logs a warning when this strategy is
    /// observed at runtime.
    DropNew,
    /// Close the SSE connection when the broadcast channel lags.
    CloseConnection,
}

/// Slice of runtime state the Server-Sent Events (SSE) layer needs.
///
/// Covers what `pierre-sse::manager` and `pierre-sse::routes` pull from
/// pierre-server's `ServerContext`: the JWT-validation auth pair
/// (`auth_manager` + `jwks_manager`) used by
/// [`pierre_mcp_transport::tenant_isolation::validate_jwt_token_for_mcp`]
/// during MCP protocol stream registration, the repository registry (for
/// the same validation plus the A2A task lookup the SSE A2A route performs),
/// the configured SSE buffer-overflow strategy (consulted by the
/// notification route when the broadcast channel lags), and the delegating
/// `authenticate_request` entry point that runs an inbound `Authorization`
/// header through the full auth-middleware pipeline.
///
/// The trait is intentionally narrow: only the four accessors plus the
/// async authentication helper. The SSE crate never touches the broader
/// composition root.
#[async_trait]
pub trait SseCtx: Send + Sync + 'static {
    /// Auth manager — validates JWT bearer tokens used by both the
    /// notification SSE stream and the MCP protocol SSE stream.
    fn auth_manager(&self) -> &Arc<AuthManager>;

    /// Configured SSE buffer-overflow strategy, mapped from
    /// `pierre_config::environment::SseConfig::buffer_overflow_strategy`.
    /// Read by the notification SSE route when the broadcast channel lags.
    fn sse_buffer_overflow_strategy(&self) -> SseBufferOverflowStrategy;

    /// JWKS manager — supplies signing keys consumed by `auth_manager`.
    fn jwks_manager(&self) -> &Arc<JwksManager>;

    /// Repository registry — backs A2A task lookups and the user/tenant
    /// lookups inside `validate_jwt_token_for_mcp`.
    fn repos(&self) -> &Arc<RepositoryRegistry>;

    /// Run an inbound `Authorization` header value through the full
    /// auth-middleware pipeline (token type detection, rate limiting, user
    /// status enforcement). The SSE routes use this to gate notification +
    /// MCP + A2A task streams.
    ///
    /// # Errors
    ///
    /// Returns an error when the header is missing/invalid or the user
    /// fails policy checks.
    async fn authenticate_request(&self, auth_header: Option<&str>) -> AppResult<AuthResult>;
}
