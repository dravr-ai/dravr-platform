// ABOUTME: AdminApiContext + AdminApiContextInit — focused-context bundle for admin routes
// ABOUTME: Constructed by the composition root, passed by Arc to every admin handler
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin API context — the focused state bundle the admin route group consumes.
//!
//! Lifted from `crate::routes::admin::mod` in `pierre-server`; the `pierre-server`
//! composition root constructs an [`AdminApiContext`] from its `ServerContext`
//! fields and passes `Arc<AdminApiContext>` to the [`crate::routes::AdminRoutes`]
//! constructor. No `Arc<ServerContext>` ever leaks into a handler.

use std::sync::Arc;

use pierre_auth::auth::AuthManager;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::{
    cageux_config::CageuxConfigRegistry, persona_contracts::PersonaContractRegistry,
    ContremaitreConfig, EvidenceRegistry, MessagingStringsRegistry, PromptRegistry,
    ToolDescriptionRegistry,
};
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_email::ResendEmailService;
use pierre_services::user_approval::UserApprovalNotifier;
use tracing::info;

use crate::auth::service::AdminAuthService;
use crate::auth::JwksManager;

/// Admin API context shared across all endpoints
///
/// Carries the full `RepositoryRegistry` because admin handlers span
/// every domain (auth/users, coaching, fitness, social, usage, content
/// — admin is the cross-cutting role by definition). Each handler
/// narrows to the view(s) it needs at the call site via the registry's
/// `auth_repos()` / `usage_repos()` / etc. accessors.
#[derive(Clone)]
pub struct AdminApiContext {
    /// Database connection for persistence operations (lifecycle, system settings, pool access)
    pub database: Arc<Database>,
    /// Repository registry for data access via trait objects
    pub repos: Arc<RepositoryRegistry>,
    /// Admin authentication service
    pub auth_service: AdminAuthService,
    /// Authentication manager for token operations
    pub auth_manager: Arc<AuthManager>,
    /// JWT secret for admin token validation
    pub admin_jwt_secret: String,
    /// JWKS manager for key rotation and validation
    pub jwks_manager: Arc<JwksManager>,
    /// Default monthly request limit for admin-provisioned API keys
    pub admin_api_key_monthly_limit: u32,
    /// Transactional email service for user lifecycle notifications (None when Resend is unconfigured)
    pub email_service: Option<Arc<ResendEmailService>>,
    /// Public frontend URL used to build sign-in links in outbound emails
    pub frontend_url: Option<String>,
    /// Notifier that emails and messages a just-approved user across their
    /// linked channels (injected by the composition root; `None` until wired).
    pub approval_notifier: Option<Arc<dyn UserApprovalNotifier>>,
    /// Shared coaching harness config registry, mutated by the
    /// `PUT /admin/settings/harness` handler so subsequent chat turns
    /// pick up the new compaction / Tier 6 guardrail values without a
    /// server restart.
    pub harness_config_registry: Arc<HarnessConfigRegistry>,
    /// Hot-reloadable system + coach prompt registry consumed by
    /// `/api/admin/contremaitre/prompts*` and the manual sync endpoint.
    pub prompt_registry: Arc<PromptRegistry>,
    /// Tool description overlay registry (MCP tool schema rewrites).
    pub tool_description_registry: Arc<ToolDescriptionRegistry>,
    /// Claim verification evidence corpus registry.
    pub evidence_registry: Arc<EvidenceRegistry>,
    /// User-facing canned reply / messaging-strings registry.
    pub messaging_strings_registry: Arc<MessagingStringsRegistry>,
    /// Cageux sports-science config registry (runtime calibration values).
    pub cageux_config_registry: Arc<CageuxConfigRegistry>,
    /// Persona contract registry (voice/style guardrails per persona).
    pub persona_contract_registry: Arc<PersonaContractRegistry>,
    /// Contremaitre GitHub sync configuration; `None` when contremaitre is
    /// disabled in the running binary.
    pub contremaitre_config: Option<ContremaitreConfig>,
}

/// Initial wiring required to construct an [`AdminApiContext`].
///
/// Lives as a dedicated struct so [`AdminApiContext::new`] doesn't need a
/// sprawling positional argument list — all callers go through named fields.
pub struct AdminApiContextInit {
    /// Backing database handle
    pub database: Arc<Database>,
    /// Repository registry for admin queries
    pub repos: Arc<RepositoryRegistry>,
    /// Admin JWT signing secret
    pub jwt_secret: String,
    /// Shared auth manager
    pub auth_manager: Arc<AuthManager>,
    /// Shared JWKS manager
    pub jwks_manager: Arc<JwksManager>,
    /// Per-tenant monthly API-key limit for admin-provisioned keys
    pub admin_api_key_monthly_limit: u32,
    /// TTL for admin-token validation cache, in seconds
    pub admin_token_cache_ttl_secs: u64,
    /// Harness config registry surfaced to admin eval flows
    pub harness_config_registry: Arc<HarnessConfigRegistry>,
    /// Hot-reloadable system + coach prompt registry
    pub prompt_registry: Arc<PromptRegistry>,
    /// Tool description overlay registry
    pub tool_description_registry: Arc<ToolDescriptionRegistry>,
    /// Claim verification evidence corpus registry
    pub evidence_registry: Arc<EvidenceRegistry>,
    /// User-facing canned reply registry
    pub messaging_strings_registry: Arc<MessagingStringsRegistry>,
    /// Cageux sports-science config registry
    pub cageux_config_registry: Arc<CageuxConfigRegistry>,
    /// Persona contract registry
    pub persona_contract_registry: Arc<PersonaContractRegistry>,
    /// Contremaitre GitHub sync configuration (None when disabled)
    pub contremaitre_config: Option<ContremaitreConfig>,
}

impl AdminApiContext {
    /// Creates a new admin API context from the init bundle.
    #[must_use]
    pub fn new(init: AdminApiContextInit) -> Self {
        info!("AdminApiContext initialized with JWT signing key");
        let auth_service = AdminAuthService::new(
            Arc::clone(&init.repos.admin),
            init.jwks_manager.clone(),
            init.admin_token_cache_ttl_secs,
        );
        Self {
            database: init.database,
            repos: init.repos,
            auth_service,
            auth_manager: init.auth_manager,
            admin_jwt_secret: init.jwt_secret,
            jwks_manager: init.jwks_manager,
            admin_api_key_monthly_limit: init.admin_api_key_monthly_limit,
            email_service: None,
            frontend_url: None,
            approval_notifier: None,
            harness_config_registry: init.harness_config_registry,
            prompt_registry: init.prompt_registry,
            tool_description_registry: init.tool_description_registry,
            evidence_registry: init.evidence_registry,
            messaging_strings_registry: init.messaging_strings_registry,
            cageux_config_registry: init.cageux_config_registry,
            persona_contract_registry: init.persona_contract_registry,
            contremaitre_config: init.contremaitre_config,
        }
    }
}
