// ABOUTME: Defines ToolExecutionContext which provides tools with access to resources and user context.
// ABOUTME: This replaces scattered parameter passing with a unified context object.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Tool Execution Context
//!
//! Provides a unified context object for tool execution, containing:
//! - User and tenant identity
//! - Access to shared server resources
//! - Request tracing information
//! - Authentication method details
//!
//! This design eliminates the need to pass multiple parameters through
//! tool execution chains and provides consistent access to resources.

use pierre_core::models::TenantId;
use pierre_intelligence::IntelligenceConfig;
use std::fmt;
use std::sync::Arc;

use dravr_tronc::mcp::tool::ToolContext as TroncToolContext;
use serde_json::Value;
use uuid::Uuid;

use crate::runtime::ToolRuntime;
use crate::tool_selection::ToolSelectionService;
use pierre_cache::Cache;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::User;
use pierre_database::backends::factory::Database;
use pierre_intelligence::ActivityIntelligence;
use pierre_providers::ProviderRegistry;

tokio::task_local! {
    /// Originating Pierre conversation id for the in-flight tool call.
    ///
    /// The third-party tronc `ToolContext` (which backs every satellite) carries
    /// only host-agnostic identity strings and cannot be extended with a
    /// conversation field. [`crate::protocol::executor::UniversalExecutor`]
    /// therefore opens this task-local scope around `McpTool::execute` for the
    /// duration of one tool call, and [`ToolExecutionContext::from_tronc`] reads
    /// it back so a tool can correlate detached work (e.g. a background activity
    /// backfill) with the channel that triggered it. The value is constant for
    /// the executor's per-turn lifetime; concurrent turns run in separate tokio
    /// tasks, so the slot never crosses turns. Absent (the scope is never
    /// entered) for MCP-direct / A2A / SSE calls that have no conversation.
    pub static CONVERSATION_ID: Option<String>;

    /// Guardian turn token for the in-flight tool call.
    ///
    /// Scoped by the executor
    /// around `McpTool::execute` so a tool that dispatches NESTED tool calls
    /// (e.g. an analytics tool that internally reads `get_activities`) builds its
    /// child [`crate::protocol::executor::UniversalExecutor`] on the SAME turn
    /// key. Without this, a nested `UniversalExecutor::new` (which does not call
    /// `with_turn_token`) would key on a throwaway per-call nonce and the child's
    /// `UNTRUSTED_OUTPUT` sub-read would fail to taint the real turn — severing
    /// the runtime taint backstop for transitively-pulled untrusted content.
    /// Constant for the executor's per-turn lifetime; absent for a top-level
    /// executor built outside any tool body (which sets its token explicitly).
    pub static GUARDIAN_TURN_TOKEN: Option<String>;
}

/// How the user authenticated for this request.
///
/// Useful for audit logging and determining available permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// JWT Bearer token (most common)
    JwtBearer,
    /// API key authentication
    ApiKey,
    /// OAuth 2.0 token from MCP OAuth flow
    OAuth2,
    /// MCP client registration token
    McpClient,
    /// Messaging channel link (`Telegram`, `WhatsApp`, `Discord`, `Slack`,
    /// `Messenger`) authenticated via [`pierre_auth::auth::AuthMethod::ChannelLink`].
    ChannelLink,
}

impl AuthMethod {
    /// Get a string representation for logging
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::JwtBearer => "jwt_bearer",
            Self::ApiKey => "api_key",
            Self::OAuth2 => "oauth2",
            Self::McpClient => "mcp_client",
            Self::ChannelLink => "channel_link",
        }
    }

    /// Parse the label produced by [`Self::as_str`] back into an `AuthMethod`,
    /// falling back to [`Self::JwtBearer`] for an unknown or absent label.
    ///
    /// Used when rebuilding a [`ToolExecutionContext`] from the host-agnostic
    /// `auth_method` string carried on a tronc `ToolContext`.
    #[must_use]
    pub fn from_label(label: &str) -> Self {
        match label {
            "api_key" => Self::ApiKey,
            "oauth2" => Self::OAuth2,
            "mcp_client" => Self::McpClient,
            "channel_link" => Self::ChannelLink,
            _ => Self::JwtBearer,
        }
    }
}

/// Context provided to every tool execution.
///
/// This struct provides tools with everything they need to execute:
/// - User and tenant identity for authorization
/// - Access to shared resources (database, providers, etc.)
/// - Request tracing information
///
/// # Arc Cloning Note
///
/// The `resources` field is `Arc<dyn ToolRuntime>` which is cloned when
/// creating new contexts. This is necessary because:
/// - `ToolRuntime` is implemented by the canonical `ServerContext`, which
///   holds expensive resources (DB pools, managers)
/// - Multiple tools may execute concurrently
/// - Arc cloning is cheap (atomic increment)
///
/// See [`crate::runtime`] for the trait definition; the production
/// impl lives on `ServerContext` in the pierre-server crate.
#[derive(Clone)]
pub struct ToolExecutionContext {
    /// Authenticated user ID (always present after auth)
    pub user_id: Uuid,
    /// Tenant ID (present for tenant-scoped operations)
    pub tenant_id: Option<Uuid>,
    /// Request ID for tracing/logging
    pub request_id: Option<Value>,
    /// Narrow runtime façade for accessing shared services.
    pub resources: Arc<dyn ToolRuntime>,
    /// Authentication method used (for audit logging)
    pub auth_method: AuthMethod,
    /// Originating Pierre conversation id when this call was surfaced from a
    /// chat turn, `None` for MCP-direct / A2A / SSE calls. Lets a tool route
    /// detached follow-up work (e.g. a background backfill push) back to the
    /// channel that triggered it.
    pub conversation_id: Option<String>,
    /// Whether the user has admin privileges (cached to avoid repeated DB queries)
    is_admin: Option<bool>,
}

impl ToolExecutionContext {
    /// Create a new context with required fields
    ///
    /// # Arguments
    ///
    /// * `user_id` - The authenticated user's ID
    /// * `tenant_id` - The tenant context for multi-tenant isolation
    /// * `resources` - Arc-wrapped tool runtime façade
    /// * `auth_method` - How the user authenticated
    #[must_use]
    pub fn new(
        user_id: Uuid,
        tenant_id: Option<TenantId>,
        resources: Arc<dyn ToolRuntime>,
        auth_method: AuthMethod,
    ) -> Self {
        Self {
            user_id,
            tenant_id: tenant_id.map(|t| t.as_uuid()),
            request_id: None,
            resources,
            auth_method,
            conversation_id: None,
            is_admin: None,
        }
    }

    /// Rebuild a context from the resource façade and a tronc per-call
    /// [`TroncToolContext`] (the identity the MCP dispatch layer resolved).
    ///
    /// The tronc context carries identity as host-agnostic strings; this parses
    /// them back into the platform's typed fields (`Uuid` ids, [`AuthMethod`]).
    /// An unparsable/absent user id becomes the nil UUID, and the admin flag is
    /// taken as already-resolved so tools don't re-query the database.
    ///
    /// The originating Pierre `conversation_id` is read from the
    /// [`CONVERSATION_ID`] task-local that the executor scopes around the tool
    /// call — the tronc context has no field for it, so this side-channel keeps
    /// the conversation available without extending the third-party type. It is
    /// `None` outside a scoped chat dispatch (MCP-direct / A2A / SSE).
    #[must_use]
    pub fn from_tronc(resources: &Arc<dyn ToolRuntime>, ctx: &TroncToolContext) -> Self {
        let user_id = ctx
            .user_id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_default();
        let tenant_id = ctx
            .tenant_id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok());
        let auth_method = ctx
            .auth_method
            .as_deref()
            .map_or(AuthMethod::JwtBearer, AuthMethod::from_label);
        Self {
            user_id,
            tenant_id,
            request_id: ctx.request_id.clone(),
            resources: resources.clone(),
            auth_method,
            conversation_id: CONVERSATION_ID.try_with(Clone::clone).ok().flatten(),
            is_admin: Some(ctx.is_admin),
        }
    }

    /// Set tenant ID
    #[must_use]
    pub fn with_tenant(mut self, tenant_id: TenantId) -> Self {
        self.tenant_id = Some(tenant_id.as_uuid());
        self
    }

    /// Set request ID for tracing
    #[must_use]
    pub fn with_request_id(mut self, request_id: Value) -> Self {
        self.request_id = Some(request_id);
        self
    }

    /// Set the originating Pierre conversation id (chat-surfaced calls).
    #[must_use]
    pub fn with_conversation_id(mut self, conversation_id: String) -> Self {
        self.conversation_id = Some(conversation_id);
        self
    }

    /// Set admin status (cached to avoid repeated DB queries)
    #[must_use]
    pub const fn with_admin_status(mut self, is_admin: bool) -> Self {
        self.is_admin = Some(is_admin);
        self
    }

    /// Get tenant ID or return error for tools requiring tenant context
    ///
    /// # Errors
    ///
    /// Returns `AppError` with `PermissionDenied` if no tenant context is available
    pub fn require_tenant(&self) -> AppResult<Uuid> {
        self.tenant_id.ok_or_else(|| {
            AppError::new(
                ErrorCode::PermissionDenied,
                "Tenant context required for this operation",
            )
        })
    }

    /// Check if the user has admin privileges
    ///
    /// Uses cached value if available, otherwise queries the database.
    ///
    /// # Errors
    ///
    /// Returns `AppError` if database query fails or user not found
    pub async fn is_admin(&self) -> AppResult<bool> {
        // Return cached value if available
        if let Some(is_admin) = self.is_admin {
            return Ok(is_admin);
        }

        // Query database for user to check admin status
        // SECURITY: Global lookup — tool context checks admin status before tenant is known
        let user: User = self
            .resources
            .repos()
            .users
            .get_global(self.user_id)
            .await?
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::ResourceNotFound,
                    format!("User {} not found", self.user_id),
                )
            })?;

        Ok(user.is_admin)
    }

    /// Require admin privileges for the current operation
    ///
    /// # Errors
    ///
    /// Returns `AppError` with `PermissionDenied` if user is not an admin
    pub async fn require_admin(&self) -> AppResult<()> {
        if self.is_admin().await? {
            Ok(())
        } else {
            Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Admin privileges required for this operation",
            ))
        }
    }

    /// Get a reference to the database
    #[must_use]
    pub fn database(&self) -> &Database {
        self.resources.database()
    }

    /// Get a reference to the provider registry
    #[must_use]
    pub fn provider_registry(&self) -> &ProviderRegistry {
        self.resources.provider_registry()
    }

    /// Get a reference to the activity intelligence engine
    #[must_use]
    pub fn activity_intelligence(&self) -> &ActivityIntelligence {
        self.resources.activity_intelligence()
    }

    /// Return a cheap clone of the current cageux intelligence config
    /// snapshot.
    ///
    /// The returned `Arc` owns a consistent view of the configuration for
    /// the duration of the request — callers should clone it once at the
    /// top of their handler and use the local clone for any sub-config
    /// reads (`.sleep_recovery`, `.nutrition`, `.algorithms`, etc.) rather
    /// than calling this method repeatedly, because each call takes a
    /// read lock on the registry.
    #[must_use]
    pub fn cageux_config(&self) -> Arc<IntelligenceConfig<true>> {
        self.resources.cageux_config()
    }

    /// Get a reference to the cache
    #[must_use]
    pub fn cache(&self) -> &Cache {
        self.resources.cache()
    }

    /// Get a reference to the tool selection service
    #[must_use]
    pub fn tool_selection(&self) -> &ToolSelectionService {
        self.resources.tool_selection()
    }

    /// Create a child context with the same resources but different user
    ///
    /// Useful for service-to-service calls or impersonation scenarios.
    #[must_use]
    pub fn with_user(&self, user_id: Uuid) -> Self {
        Self {
            user_id,
            tenant_id: self.tenant_id,
            request_id: self.request_id.clone(),
            resources: self.resources.clone(),
            auth_method: self.auth_method,
            conversation_id: self.conversation_id.clone(),
            is_admin: None, // Reset admin cache for new user
        }
    }

    /// Get tracing span attributes for this context
    #[must_use]
    pub fn span_attributes(&self) -> Vec<(&'static str, String)> {
        let mut attrs = vec![
            ("user_id", self.user_id.to_string()),
            ("auth_method", self.auth_method.as_str().to_owned()),
        ];

        if let Some(tenant_id) = self.tenant_id {
            attrs.push(("tenant_id", tenant_id.to_string()));
        }

        if let Some(request_id) = &self.request_id {
            attrs.push(("request_id", request_id.to_string()));
        }

        attrs
    }
}

impl fmt::Debug for ToolExecutionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolExecutionContext")
            .field("user_id", &self.user_id)
            .field("tenant_id", &self.tenant_id)
            .field("request_id", &self.request_id)
            .field("auth_method", &self.auth_method)
            .field("conversation_id", &self.conversation_id)
            .field("is_admin", &self.is_admin)
            .field("resources", &"<dyn ToolRuntime>")
            .finish()
    }
}
