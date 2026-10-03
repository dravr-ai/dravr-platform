// ABOUTME: Tenant administration commands for pierre-cli (plan, tool overrides, feature flags, OAuth apps)
// ABOUTME: Operator backdoor for tenant-scoped settings outside the Stripe/web loops
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::io::{self, BufRead};
use std::sync::Arc;

use clap::Subcommand;
use pierre_auth::tenant::oauth_manager::default_rate_limit_for_provider;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, TenantOAuthCredentials, ToolEnablementSource};
use pierre_database::RepositoryRegistry;
use pierre_services::admin_ops;
use pierre_tool_runtime::tool_selection::ToolSelectionService;
use uuid::Uuid;

use crate::commands::user::{admin_actor, parse_feature_key};

type Result<T> = AppResult<T>;

/// Set a tenant's plan (Starter / Professional / Enterprise), which gates
/// plan-restricted tools via `tool_catalog.min_plan`.
///
/// The tenant is resolved from the user's email: if the user belongs to exactly
/// one tenant it is used, otherwise `--tenant-id` must disambiguate. Delegates
/// to the shared [`admin_ops::set_tenant_plan`].
pub async fn set_plan(
    repos: &RepositoryRegistry,
    email: String,
    plan: String,
    tenant_id: Option<String>,
) -> Result<()> {
    let user = repos
        .users
        .get_by_email(&email)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User with email {email}")))?;

    let tenant_id = resolve_tenant(repos, user.id, tenant_id.as_deref(), &email).await?;

    let updated = admin_ops::set_tenant_plan(repos, tenant_id, &plan).await?;
    println!(
        "Success: tenant {} plan set to {} \
         (unlocks plan-gated tools; up to ~5 min to reach the live server cache)",
        updated.id, updated.plan
    );
    Ok(())
}

/// Resolve the target tenant for an operator command: an explicit `--tenant-id`
/// wins; otherwise use the user's sole tenant, erroring if there are zero or
/// multiple.
async fn resolve_tenant(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: Option<&str>,
    email: &str,
) -> Result<TenantId> {
    if let Some(raw) = tenant_id {
        let uuid = Uuid::parse_str(raw)
            .map_err(|e| AppError::invalid_input(format!("Invalid --tenant-id '{raw}': {e}")))?;
        return Ok(TenantId::from_uuid(uuid));
    }

    let tenants = repos.tenants.list_for_user(user_id).await?;
    match tenants.as_slice() {
        [] => Err(AppError::not_found(format!(
            "User {email} belongs to no tenant; pass --tenant-id"
        ))),
        [single] => Ok(single.id),
        many => Err(AppError::invalid_input(format!(
            "User {email} belongs to {} tenants; pass --tenant-id to choose one",
            many.len()
        ))),
    }
}

/// Resolve the tenant for a command from `--email` (+ optional `--tenant-id`).
async fn tenant_for(
    repos: &RepositoryRegistry,
    email: &str,
    tenant_id: Option<&str>,
) -> Result<TenantId> {
    let user = repos
        .users
        .get_by_email(email)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User with email {email}")))?;
    resolve_tenant(repos, user.id, tenant_id, email).await
}

/// The acting admin required by the tenant tool-override audit column.
async fn required_actor(repos: &RepositoryRegistry) -> Result<Uuid> {
    admin_actor(repos).await.ok_or_else(|| {
        AppError::invalid_input(
            "No admin user exists to attribute this override to — \
             run `pierre-cli user create` or `seed bootstrap` first",
        )
    })
}

/// Force-enable or force-disable a tool for a whole tenant via the same
/// `ToolSelectionService` write path the web and token surfaces use.
/// Effective on the live server after its tenant cache expires (~5 min).
pub async fn set_tool(
    repos: &Arc<RepositoryRegistry>,
    email: String,
    tool: String,
    is_enabled: bool,
    tenant_id: Option<String>,
    reason: Option<String>,
) -> Result<()> {
    let tenant = tenant_for(repos, &email, tenant_id.as_deref()).await?;
    let actor = required_actor(repos).await?;
    let svc = ToolSelectionService::new(repos);
    let reason = reason.or_else(|| Some("set via pierre-cli".to_owned()));
    svc.set_tool_override(tenant, &tool, is_enabled, actor, reason)
        .await?;
    let verb = if is_enabled { "enabled" } else { "disabled" };
    println!(
        "Success: tool '{tool}' {verb} for tenant {tenant} \
         (tenant-wide; up to ~5 min to reach the live server cache)"
    );
    Ok(())
}

/// Remove a tenant tool override so the tool reverts to plan/catalog default.
pub async fn reset_tool(
    repos: &Arc<RepositoryRegistry>,
    email: String,
    tool: String,
    tenant_id: Option<String>,
) -> Result<()> {
    let tenant = tenant_for(repos, &email, tenant_id.as_deref()).await?;
    let svc = ToolSelectionService::new(repos);
    let removed = svc.remove_tool_override(tenant, &tool).await?;
    if removed {
        println!("Success: cleared tenant override for '{tool}' on tenant {tenant}");
    } else {
        println!("No tenant override existed for '{tool}' on tenant {tenant} (nothing to clear)");
    }
    Ok(())
}

/// List the tenant's effective tools with the source of each decision.
pub async fn list_tools(
    repos: &Arc<RepositoryRegistry>,
    email: String,
    tenant_id: Option<String>,
) -> Result<()> {
    let tenant = tenant_for(repos, &email, tenant_id.as_deref()).await?;
    let svc = ToolSelectionService::new(repos);
    let tools = svc.get_effective_tools(tenant).await?;

    println!(
        "Effective tools for tenant {tenant} ({} total):",
        tools.len()
    );
    println!("{:<34}  {:<9}  SOURCE", "TOOL", "STATE");
    for t in tools {
        let state = if t.is_enabled { "enabled" } else { "disabled" };
        let source = match t.source {
            ToolEnablementSource::Default => "default",
            ToolEnablementSource::TenantOverride => "tenant_override",
            ToolEnablementSource::UserOverride => "user_override",
            ToolEnablementSource::PlanRestriction => "plan_restriction",
            ToolEnablementSource::GlobalDisabled => "global_disabled",
        };
        println!("{:<34}  {:<9}  {source}", t.tool_name, state);
    }
    Ok(())
}

/// Set a tenant-default feature flag (per-user overrides still win).
pub async fn set_feature(
    repos: &RepositoryRegistry,
    email: String,
    key: String,
    enabled: bool,
    tenant_id: Option<String>,
) -> Result<()> {
    let tenant = tenant_for(repos, &email, tenant_id.as_deref()).await?;
    let feature_key = parse_feature_key(&key)?;
    let actor = admin_actor(repos).await;
    repos
        .feature_flags
        .set_tenant_default(tenant.as_uuid(), feature_key, enabled, actor)
        .await?;
    let verb = if enabled { "enabled" } else { "disabled" };
    println!(
        "Success: feature '{}' {verb} as tenant default for {tenant}",
        feature_key.as_str()
    );
    Ok(())
}

/// Clear a tenant-default feature flag; the built-in default applies again.
pub async fn clear_feature(
    repos: &RepositoryRegistry,
    email: String,
    key: String,
    tenant_id: Option<String>,
) -> Result<()> {
    let tenant = tenant_for(repos, &email, tenant_id.as_deref()).await?;
    let feature_key = parse_feature_key(&key)?;
    let removed = repos
        .feature_flags
        .clear_tenant_default(tenant.as_uuid(), feature_key)
        .await?;
    if removed {
        println!(
            "Success: cleared tenant default for '{}' on {tenant}; built-in default applies",
            feature_key.as_str()
        );
    } else {
        println!(
            "No tenant default existed for '{}' on {tenant} (nothing to clear)",
            feature_key.as_str()
        );
    }
    Ok(())
}

/// List a tenant's explicit feature-flag defaults.
pub async fn list_features(
    repos: &RepositoryRegistry,
    email: String,
    tenant_id: Option<String>,
) -> Result<()> {
    let tenant = tenant_for(repos, &email, tenant_id.as_deref()).await?;
    let rows = repos
        .feature_flags
        .list_tenant_defaults(tenant.as_uuid())
        .await?;
    if rows.is_empty() {
        println!("No tenant-default feature flags for {tenant}");
        return Ok(());
    }
    println!("Tenant-default feature flags for {tenant}:");
    for row in rows {
        let state = if row.enabled { "enabled" } else { "disabled" };
        println!("  {:<16}  {state}", row.feature_key.as_str());
    }
    Ok(())
}

/// The tenant's own OAuth app for one provider, as an operator configures it.
pub struct OAuthAppSettings {
    /// Provider the app belongs to (`strava`, `whoop`, ...).
    pub provider: String,
    /// The app's public client id.
    pub client_id: String,
    /// The callback the provider redirects to after consent.
    pub redirect_uri: String,
    /// Scopes the app requests.
    pub scopes: Vec<String>,
    /// Daily request budget; the provider's default when absent.
    pub rate_limit_per_day: Option<u32>,
}

/// Store a tenant's OAuth app credentials for one provider.
///
/// The client secret is read from the first line of stdin, never from an
/// argument, so it stays out of the shell history and the process list, and
/// it is never printed. The repository encrypts it at rest, bound to the
/// tenant and provider, exactly as the connect flow's read path decrypts it.
pub async fn set_oauth_app(
    repos: &RepositoryRegistry,
    email: String,
    tenant_id: Option<String>,
    settings: OAuthAppSettings,
) -> Result<()> {
    let tenant = tenant_for(repos, &email, tenant_id.as_deref()).await?;
    let client_secret = read_secret_line(&mut io::stdin().lock())?;
    let provider = settings.provider.trim().to_lowercase();
    if provider.is_empty() || settings.client_id.trim().is_empty() {
        return Err(AppError::invalid_input(
            "--provider and --client-id must not be empty",
        ));
    }
    let rate_limit_per_day = settings
        .rate_limit_per_day
        .unwrap_or_else(|| default_rate_limit_for_provider(&provider));
    repos
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id: tenant,
            provider: provider.clone(),
            client_id: settings.client_id.trim().to_owned(),
            client_secret,
            redirect_uri: settings.redirect_uri,
            scopes: settings.scopes,
            rate_limit_per_day,
        })
        .await?;
    println!(
        "Success: {provider} OAuth app {} stored for tenant {tenant} \
         (secret encrypted at rest; {rate_limit_per_day} requests/day)",
        settings.client_id.trim()
    );
    Ok(())
}

/// The client secret: the first line of `input`, trimmed. An empty line is
/// refused rather than stored as an empty secret.
fn read_secret_line(input: &mut impl BufRead) -> Result<String> {
    let mut line = String::new();
    input.read_line(&mut line).map_err(|e| {
        AppError::invalid_input(format!("failed to read the client secret from stdin: {e}"))
    })?;
    let secret = line.trim();
    if secret.is_empty() {
        return Err(AppError::invalid_input(
            "no client secret on stdin — pipe it in, e.g. `printf %s \"$SECRET\" | pierre-cli tenant set-oauth-app ...`",
        ));
    }
    Ok(secret.to_owned())
}

/// `pierre-cli tenant` subcommands: plan, tool overrides, feature flags, OAuth apps.
#[non_exhaustive]
#[derive(Subcommand)]
pub enum TenantCommand {
    /// Set a tenant's plan (unlocks plan-gated tools via `tool_catalog.min_plan`)
    SetPlan {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// Plan: starter | professional | enterprise
        #[arg(long)]
        plan: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// Force-enable an MCP tool for the whole tenant (overrides plan gating)
    EnableTool {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// MCP tool name (must exist in the tool catalog)
        #[arg(long)]
        tool: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,

        /// Operator note recorded on the override
        #[arg(long)]
        reason: Option<String>,
    },

    /// Force-disable an MCP tool for the whole tenant
    DisableTool {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// MCP tool name (must exist in the tool catalog)
        #[arg(long)]
        tool: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,

        /// Operator note recorded on the override
        #[arg(long)]
        reason: Option<String>,
    },

    /// Remove a tenant tool override (revert to plan/catalog default)
    ResetTool {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// MCP tool name
        #[arg(long)]
        tool: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// List the tenant's effective tools with each decision's source
    ListTools {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// Set a tenant-default feature flag (per-user overrides still win)
    SetFeature {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// Feature key (e.g. `api_tokens`, `billing_header`)
        #[arg(long)]
        key: String,

        /// true to enable, false to disable (explicit value required — a bare
        /// presence flag would make the omitted case silently mean "disable")
        #[arg(long, action = clap::ArgAction::Set)]
        enabled: bool,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// Clear a tenant-default feature flag (built-in default applies again)
    ClearFeature {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// Feature key
        #[arg(long)]
        key: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// List a tenant's explicit feature-flag defaults
    ListFeatures {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,
    },

    /// Store the tenant's own OAuth app for a provider; the client secret is
    /// read from stdin and encrypted at rest
    SetOauthApp {
        /// Email of a user in the target tenant
        #[arg(long)]
        email: String,

        /// Tenant id (required only if the user belongs to multiple tenants)
        #[arg(long)]
        tenant_id: Option<String>,

        /// Provider the app belongs to (e.g. strava, whoop)
        #[arg(long)]
        provider: String,

        /// The app's public client id
        #[arg(long)]
        client_id: String,

        /// The callback URL registered with the provider
        #[arg(long)]
        redirect_uri: String,

        /// Scopes the app requests, comma-separated
        #[arg(long, value_delimiter = ',', required = true)]
        scopes: Vec<String>,

        /// Daily request budget (defaults to the provider's)
        #[arg(long)]
        rate_limit_per_day: Option<u32>,
    },
}

/// Run one `pierre-cli tenant` subcommand against the local database.
///
/// # Errors
/// Returns the subcommand's error.
pub async fn dispatch(repos: &Arc<RepositoryRegistry>, action: TenantCommand) -> Result<()> {
    match action {
        TenantCommand::SetPlan {
            email,
            plan,
            tenant_id,
        } => {
            set_plan(repos, email, plan, tenant_id).await?;
        }
        TenantCommand::EnableTool {
            email,
            tool,
            tenant_id,
            reason,
        } => {
            set_tool(repos, email, tool, true, tenant_id, reason).await?;
        }
        TenantCommand::DisableTool {
            email,
            tool,
            tenant_id,
            reason,
        } => {
            set_tool(repos, email, tool, false, tenant_id, reason).await?;
        }
        TenantCommand::ResetTool {
            email,
            tool,
            tenant_id,
        } => {
            reset_tool(repos, email, tool, tenant_id).await?;
        }
        TenantCommand::ListTools { email, tenant_id } => {
            list_tools(repos, email, tenant_id).await?;
        }
        TenantCommand::SetFeature {
            email,
            key,
            enabled,
            tenant_id,
        } => {
            set_feature(repos, email, key, enabled, tenant_id).await?;
        }
        TenantCommand::ClearFeature {
            email,
            key,
            tenant_id,
        } => {
            clear_feature(repos, email, key, tenant_id).await?;
        }
        TenantCommand::ListFeatures { email, tenant_id } => {
            list_features(repos, email, tenant_id).await?;
        }
        TenantCommand::SetOauthApp {
            email,
            tenant_id,
            provider,
            client_id,
            redirect_uri,
            scopes,
            rate_limit_per_day,
        } => {
            let settings = OAuthAppSettings {
                provider,
                client_id,
                redirect_uri,
                scopes,
                rate_limit_per_day,
            };
            set_oauth_app(repos, email, tenant_id, settings).await?;
        }
    }
    Ok(())
}
