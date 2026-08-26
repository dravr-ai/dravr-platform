// ABOUTME: User management commands for pierre-cli
// ABOUTME: Handles creation and management of admin users for frontend login
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str::FromStr;

use bcrypt::{hash, DEFAULT_COST};
use chrono::Utc;
use pierre_core::constants::tiers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::{CoachingPersona, Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::database::CreateUserMcpTokenRequest;
use pierre_database::RepositoryRegistry;
use pierre_services::admin_ops;
use pierre_services::auth::AuthService;

type Result<T> = AppResult<T>;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::helpers::display::display_admin_user_success;

/// Create or update admin user for frontend login
pub async fn create(
    repos: &RepositoryRegistry,
    email: String,
    password: String,
    name: Option<String>,
    force: bool,
    super_admin: bool,
) -> Result<()> {
    // Derive display name from email prefix if not provided
    let display_name =
        name.unwrap_or_else(|| email.split('@').next().unwrap_or("Admin").to_owned());

    let role_str = if super_admin { "super admin" } else { "admin" };
    info!("User Creating {} user: {}", role_str, email);

    // Check if user already exists and handle accordingly
    if let Ok(Some(existing_user)) = repos.users.get_by_email(&email).await {
        update_existing_admin_user(
            repos,
            existing_user,
            &email,
            &password,
            &display_name,
            force,
            super_admin,
        )
        .await?;
    } else {
        create_new_admin_user(repos, &email, &password, &display_name, super_admin).await?;
    }

    display_admin_user_success(&email, &display_name, super_admin);
    initialize_admin_jwt_secret(repos).await?;

    println!("\nSuccess Admin user is ready to use!");

    Ok(())
}

async fn update_existing_admin_user(
    repos: &RepositoryRegistry,
    existing_user: User,
    email: &str,
    password: &str,
    name: &str,
    force: bool,
    super_admin: bool,
) -> Result<()> {
    if !force {
        display_existing_user_error(&existing_user);
        return Err(AppError::invalid_input(
            "User already exists (use --force to update)",
        ));
    }

    let role_str = if super_admin { "super admin" } else { "admin" };
    info!("Updating existing {} user...", role_str);

    let role = if super_admin {
        UserRole::SuperAdmin
    } else {
        UserRole::Admin
    };

    // NOTE: tenant_id is managed via tenant_users junction table, not on User struct
    let updated_user = User {
        id: existing_user.id,
        email: email.to_owned(),
        display_name: Some(name.to_owned()),
        password_hash: hash(password, DEFAULT_COST)
            .map_err(|e| AppError::internal(format!("bcrypt error: {e}")))?,
        tier: UserTier::Enterprise,
        strava_token: existing_user.strava_token,
        fitbit_token: existing_user.fitbit_token,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: true,
        role,
        approved_by: existing_user.approved_by,
        approved_at: existing_user.approved_at,
        created_at: existing_user.created_at,
        last_active: Utc::now(),
        firebase_uid: existing_user.firebase_uid,
        auth_provider: existing_user.auth_provider,
        analytics_consent: existing_user.analytics_consent,
        analytics_consent_at: existing_user.analytics_consent_at,
        locale: existing_user.locale,
        coaching_persona: existing_user.coaching_persona,
        manages_roster: existing_user.manages_roster,
        timezone: existing_user.timezone.clone(),
        theme: existing_user.theme,
    };

    repos.users.create(&updated_user).await?;
    Ok(())
}

fn display_existing_user_error(existing_user: &User) {
    let details = format!(
        "Email: {}\nName: {:?}\nCreated: {}",
        existing_user.email,
        existing_user.display_name,
        existing_user.created_at.format("%Y-%m-%d %H:%M UTC")
    );

    error!("Error User '{}' already exists!", existing_user.email);
    info!("Use --force flag to update existing user");
    info!(
        "   Current user details:\n   - {}",
        details.replace('\n', "\n   - ")
    );
}

/// Auto-create a default MCP token for a newly activated user.
/// This is a non-fatal operation - failure is logged but does not propagate.
async fn create_default_mcp_token_for_user(repos: &RepositoryRegistry, user_id: Uuid) {
    let token_request = CreateUserMcpTokenRequest {
        name: "Default Token".to_owned(),
        expires_in_days: None, // Never expires
    };

    match repos
        .user_mcp_tokens
        .create_token(user_id, &token_request)
        .await
    {
        Ok(token_result) => {
            info!(
                user_id = %user_id,
                token_id = %token_result.token.id,
                "Auto-created default MCP token for admin user"
            );
        }
        Err(e) => {
            // Log error but don't fail - user can create token manually
            warn!(
                user_id = %user_id,
                error = %e,
                "Failed to auto-create MCP token for admin user (non-fatal)"
            );
        }
    }
}

/// Create a personal tenant for a user and link them to it
async fn create_and_link_personal_tenant(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    name: &str,
    slug_prefix: &str,
) -> Result<()> {
    let tenant_id = TenantId::generate();
    let tenant_slug = format!("{slug_prefix}-{}", user_id.as_simple());
    let tenant = Tenant {
        id: tenant_id,
        name: format!("{name}'s Workspace"),
        slug: tenant_slug,
        domain: None,
        plan: tiers::ENTERPRISE.to_owned(),
        owner_user_id: user_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    repos.tenants.create(&tenant).await?;
    info!("Created personal tenant: {} ({})", tenant.name, tenant_id);

    repos.users.update_tenant_id(user_id, tenant_id).await?;

    Ok(())
}

/// Build admin user model with the given parameters
fn build_admin_user(
    user_id: Uuid,
    email: &str,
    password_hash: String,
    name: &str,
    role: UserRole,
) -> User {
    // NOTE: tenant_id is managed via tenant_users junction table, not on User struct
    User {
        id: user_id,
        email: email.to_owned(),
        display_name: Some(name.to_owned()),
        password_hash,
        tier: UserTier::Enterprise,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: true,
        role,
        approved_by: None,
        approved_at: Some(Utc::now()),
        created_at: Utc::now(),
        last_active: Utc::now(),
        firebase_uid: None,
        auth_provider: "email".to_owned(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::default(),
        manages_roster: false,
        timezone: None,
        theme: None,
    }
}

async fn create_new_admin_user(
    repos: &RepositoryRegistry,
    email: &str,
    password: &str,
    name: &str,
    super_admin: bool,
) -> Result<()> {
    let role_str = if super_admin { "super admin" } else { "admin" };
    info!("Creating new {} user...", role_str);

    let role = if super_admin {
        UserRole::SuperAdmin
    } else {
        UserRole::Admin
    };

    let user_id = Uuid::new_v4();
    let password_hash = hash(password, DEFAULT_COST)
        .map_err(|e| AppError::internal(format!("bcrypt error: {e}")))?;
    let new_user = build_admin_user(user_id, email, password_hash, name, role);

    repos.users.create(&new_user).await?;
    info!("Created {} user: {}", role_str, email);

    create_and_link_personal_tenant(repos, user_id, name, "admin").await?;

    create_default_mcp_token_for_user(repos, new_user.id).await;

    Ok(())
}

async fn initialize_admin_jwt_secret(repos: &RepositoryRegistry) -> Result<()> {
    info!("Ensuring admin JWT secret exists...");
    repos
        .security
        .get_or_create_system_secret("admin_jwt_secret")
        .await?;
    info!("Admin JWT signing key initialized successfully");
    Ok(())
}

/// Promote an existing user to admin
///
/// Operates directly on the database — no HTTP layer involved. The `set_admin_status`
/// repo method handles the privilege transition. To promote to super-admin, use
/// `pierre-cli user create --email X --password Y --force --super-admin` instead.
pub async fn promote(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let user = repos
        .users
        .get_by_email(&email)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User with email {email}")))?;

    if user.is_admin {
        warn!("User {} is already an admin", email);
        println!("User {email} is already an admin (no change)");
        return Ok(());
    }

    let updated = repos.users.set_admin_status(user.id, true).await?;
    println!(
        "User {} promoted to admin (id={}, role={})",
        updated.email,
        updated.id,
        updated.role.as_str()
    );
    Ok(())
}

/// Demote an admin user back to a regular user
///
/// Super-admins cannot be demoted via this command to prevent accidental privilege loss;
/// super-admin demotion requires direct DB intervention.
pub async fn demote(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let user = repos
        .users
        .get_by_email(&email)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User with email {email}")))?;

    if !user.is_admin {
        warn!("User {} is not an admin", email);
        println!("User {email} is not an admin (no change)");
        return Ok(());
    }

    let updated = repos.users.set_admin_status(user.id, false).await?;
    println!(
        "User {} demoted from admin (id={}, role={})",
        updated.email,
        updated.id,
        updated.role.as_str()
    );
    Ok(())
}

/// Set a user's billing/quota tier (Starter / Professional / Enterprise).
///
/// Writes `users.tier` (the effective quota tier the server reads per request)
/// and records the admin-override marker so a later Stripe webhook cannot
/// clobber it. Delegates to the shared [`admin_ops::set_user_tier`] so the CLI,
/// the web admin route, and the super-admin token route share one implementation.
pub async fn set_tier(
    repos: &RepositoryRegistry,
    email: String,
    tier: String,
    note: Option<String>,
) -> Result<()> {
    let user = repos
        .users
        .get_by_email(&email)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User with email {email}")))?;

    let parsed = match tier.to_ascii_lowercase().as_str() {
        "starter" => UserTier::Starter,
        "professional" => UserTier::Professional,
        "enterprise" => UserTier::Enterprise,
        other => {
            return Err(AppError::invalid_input(format!(
                "Unknown tier '{other}' — expected starter, professional, or enterprise"
            )));
        }
    };

    let note = note.unwrap_or_else(|| "set via pierre-cli".to_owned());
    let updated =
        admin_ops::set_user_tier(repos, user.id, parsed.clone(), Some(note), None).await?;
    println!(
        "Success: {} tier set to {} (quota/rate-limit tier; recorded as admin override)",
        updated.email, parsed
    );
    Ok(())
}

/// Clear a user's admin tier override so the billing webhook drives the tier
/// again. Leaves `users.tier` as-is.
pub async fn clear_tier(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let user = repos
        .users
        .get_by_email(&email)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User with email {email}")))?;

    let removed = admin_ops::clear_user_tier_override(repos, user.id).await?;
    if removed {
        println!("Success: cleared tier override for {email}; Stripe will re-drive the tier");
    } else {
        println!("No tier override existed for {email} (nothing to clear)");
    }
    Ok(())
}

/// Resolve the CLI's acting-admin identity for audit columns: the first
/// admin user's UUID, or `None` on a pre-bootstrap database (audit fields
/// are nullable for exactly this service-actor case). A repository error is
/// logged instead of silently collapsing to `None` — a silent `.ok()` here
/// masked the broken `get_first_admin_user` SELECT for a full day.
pub async fn admin_actor(repos: &RepositoryRegistry) -> Option<Uuid> {
    match repos.users.get_first_admin_user().await {
        Ok(user) => user.map(|u| u.id),
        Err(e) => {
            warn!(error = %e, "admin_actor lookup failed; audit set_by will be NULL");
            None
        }
    }
}

/// Look up a user by email or fail with a uniform not-found error.
async fn lookup(repos: &RepositoryRegistry, email: &str) -> Result<User> {
    repos
        .users
        .get_by_email(email)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User with email {email}")))
}

/// Approve a pending user (status → active) and provision their default
/// MCP token. Mirrors the super-admin web approve (global scope, no tenant
/// attach — tenant assignment is the tenant-scoped web flow's concern).
pub async fn approve(
    repos: &RepositoryRegistry,
    email: String,
    reason: Option<String>,
) -> Result<()> {
    let user = lookup(repos, &email).await?;
    let actor = admin_actor(repos).await;
    let updated =
        admin_ops::transition_user_status(repos, user.id, UserStatus::Active, actor).await?;
    admin_ops::create_default_mcp_token_for_user(repos.user_mcp_tokens.as_ref(), user.id).await;
    info!(
        target_user_id = %user.id,
        reason = reason.as_deref().unwrap_or("No reason provided"),
        "User approved via pierre-cli"
    );
    println!(
        "Success: {} approved (status: {})",
        updated.email, updated.user_status
    );
    Ok(())
}

/// Normalize an operator-supplied email for allow-list operations: trimmed,
/// lowercased, and shape-checked before it becomes a lookup key.
fn normalize_email(email: &str) -> Result<String> {
    let normalized = email.trim().to_lowercase();
    if !AuthService::is_valid_email(&normalized) {
        return Err(AppError::invalid_input(format!(
            "'{normalized}' is not a valid email address"
        )));
    }
    Ok(normalized)
}

/// Pre-approve an email address so it gets in without the approval queue.
///
/// Three cases: no account yet -> record a standing allow that registration
/// consumes (the account lands active, `approved_by` set to the acting
/// operator); a pending account -> approve it now, exactly like `user
/// approve`, default MCP token included; an active account -> no-op. A
/// suspended account is left untouched: suspension is a deliberate operator
/// act, reversed explicitly with `user approve`, never as a side effect of a
/// bulk allow.
pub async fn allow(repos: &RepositoryRegistry, email: String, note: Option<String>) -> Result<()> {
    let email = normalize_email(&email)?;

    if let Some(user) = repos.users.get_by_email(&email).await? {
        match user.user_status {
            UserStatus::Active => {
                println!("{email} already has an active account (no change)");
            }
            UserStatus::Suspended => {
                println!(
                    "{email} has a suspended account — left unchanged. \
                     Reinstate explicitly with: pierre-cli user approve --email {email}"
                );
            }
            UserStatus::Pending => {
                let actor = admin_actor(repos).await;
                let updated =
                    admin_ops::transition_user_status(repos, user.id, UserStatus::Active, actor)
                        .await?;
                admin_ops::create_default_mcp_token_for_user(
                    repos.user_mcp_tokens.as_ref(),
                    user.id,
                )
                .await;
                info!(
                    target_user_id = %user.id,
                    "Pending user approved via pierre-cli user allow"
                );
                println!(
                    "Success: {} had a pending account — approved now (status: {})",
                    updated.email, updated.user_status
                );
            }
        }
        return Ok(());
    }

    let actor = admin_actor(repos).await;
    let added = repos
        .pre_approved_emails
        .allow(&email, actor, note.as_deref())
        .await?;
    if added {
        println!(
            "Success: {email} pre-approved — their registration will land active (no approval queue)"
        );
    } else {
        println!("{email} is already pre-approved (no change)");
    }
    Ok(())
}

/// Remove a standing pre-approval. Never touches an existing account's
/// status — an already-registered user keeps whatever status they hold.
pub async fn disallow(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let email = normalize_email(&email)?;
    let removed = repos.pre_approved_emails.remove(&email).await?;
    if removed {
        println!("Success: {email} removed from the pre-approved list");
    } else {
        println!("{email} was not on the pre-approved list (nothing to remove)");
    }
    if let Ok(Some(user)) = repos.users.get_by_email(&email).await {
        println!(
            "Note: an account already exists for {email} (status: {}) — disallow does not change it",
            user.user_status
        );
    }
    Ok(())
}

/// List standing pre-approvals with each address's registration state.
pub async fn list_allowed(repos: &RepositoryRegistry) -> Result<()> {
    let entries = repos.pre_approved_emails.list().await?;
    if entries.is_empty() {
        println!("No pre-approved emails");
        return Ok(());
    }
    println!("Pre-approved emails ({} total):", entries.len());
    println!(
        "{:<36}  {:<10}  {:<17}  NOTE",
        "EMAIL", "ACCOUNT", "ALLOWED AT"
    );
    for entry in entries {
        let account = match repos.users.get_by_email(&entry.email).await? {
            Some(user) => user.user_status.to_string(),
            None => "not yet".to_owned(),
        };
        println!(
            "{:<36}  {:<10}  {:<17}  {}",
            entry.email,
            account,
            entry.created_at.format("%Y-%m-%d %H:%M"),
            entry.note.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

/// Suspend a user (status → suspended); they can no longer log in.
pub async fn suspend(
    repos: &RepositoryRegistry,
    email: String,
    reason: Option<String>,
) -> Result<()> {
    let user = lookup(repos, &email).await?;
    let actor = admin_actor(repos).await;
    let updated =
        admin_ops::transition_user_status(repos, user.id, UserStatus::Suspended, actor).await?;
    info!(
        target_user_id = %user.id,
        reason = reason.as_deref().unwrap_or("No reason provided"),
        "User suspended via pierre-cli"
    );
    println!(
        "Success: {} suspended (status: {})",
        updated.email, updated.user_status
    );
    Ok(())
}

/// Issue a one-time password reset token for a user and print it for
/// out-of-band delivery. Only the token's hash is stored server-side.
pub async fn reset_password(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let user = lookup(repos, &email).await?;
    let token = admin_ops::issue_password_reset_token(repos, user.id, "pierre-cli").await?;
    let minutes = admin_ops::PASSWORD_RESET_TTL_SECONDS / 60;
    println!("Password reset token for {email} (deliver out-of-band, shown once):");
    println!("  {token}");
    println!("Expires in {minutes} minutes.");
    Ok(())
}

/// Set a per-user rate-limit override. An omitted dimension means
/// "unlimited at that dimension" — passing neither cap grants a fully
/// unlimited override.
pub async fn set_rate_limit(
    repos: &RepositoryRegistry,
    email: String,
    daily: Option<u32>,
    monthly: Option<u32>,
    note: Option<String>,
) -> Result<()> {
    let user = lookup(repos, &email).await?;
    let actor = admin_actor(repos).await;
    let note = note.or_else(|| Some("set via pierre-cli".to_owned()));
    admin_ops::set_user_rate_limit_override(repos, user.id, daily, monthly, note, actor).await?;
    let fmt = |v: Option<u32>| v.map_or_else(|| "unlimited".to_owned(), |n| n.to_string());
    println!(
        "Success: rate-limit override for {email} — daily: {}, monthly: {}",
        fmt(daily),
        fmt(monthly)
    );
    Ok(())
}

/// Clear a per-user rate-limit override; the user reverts to tier defaults.
pub async fn clear_rate_limit(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let user = lookup(repos, &email).await?;
    let removed = admin_ops::clear_user_rate_limit_override(repos, user.id).await?;
    if removed {
        println!("Success: cleared rate-limit override for {email}; tier defaults apply");
    } else {
        println!("No rate-limit override existed for {email} (nothing to clear)");
    }
    Ok(())
}

/// Parse a feature-flag key or fail listing the valid ones.
pub fn parse_feature_key(key: &str) -> Result<FeatureKey> {
    FeatureKey::from_str(key).map_err(|_| {
        let valid: Vec<&str> = FeatureKey::ALL.iter().map(|k| k.as_str()).collect();
        AppError::invalid_input(format!(
            "Unknown feature key '{key}' — expected one of: {}",
            valid.join(", ")
        ))
    })
}

/// Set a per-user feature-flag override (wins over the tenant default).
pub async fn set_feature(
    repos: &RepositoryRegistry,
    email: String,
    key: String,
    enabled: bool,
) -> Result<()> {
    let user = lookup(repos, &email).await?;
    let feature_key = parse_feature_key(&key)?;
    let actor = admin_actor(repos).await;
    repos
        .feature_flags
        .set_user_override(user.id, feature_key, enabled, actor)
        .await?;
    let verb = if enabled { "enabled" } else { "disabled" };
    println!(
        "Success: feature '{}' {verb} for {email} (per-user override)",
        feature_key.as_str()
    );
    Ok(())
}

/// Clear a per-user feature-flag override; the tenant default applies again.
pub async fn clear_feature(repos: &RepositoryRegistry, email: String, key: String) -> Result<()> {
    let user = lookup(repos, &email).await?;
    let feature_key = parse_feature_key(&key)?;
    let removed = repos
        .feature_flags
        .clear_user_override(user.id, feature_key)
        .await?;
    if removed {
        println!(
            "Success: cleared '{}' override for {email}; tenant default applies",
            feature_key.as_str()
        );
    } else {
        println!(
            "No '{}' override existed for {email} (nothing to clear)",
            feature_key.as_str()
        );
    }
    Ok(())
}

/// List a user's explicit per-user feature-flag overrides.
pub async fn list_features(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let user = lookup(repos, &email).await?;
    let overrides = repos.feature_flags.list_user_overrides(user.id).await?;
    if overrides.is_empty() {
        println!("No per-user feature overrides for {email}");
        return Ok(());
    }
    println!("Per-user feature overrides for {email}:");
    for row in overrides {
        let state = if row.enabled { "enabled" } else { "disabled" };
        println!("  {:<16}  {state}", row.feature_key.as_str());
    }
    Ok(())
}

/// List all admin users, printing email and role
pub async fn list_admins(repos: &RepositoryRegistry) -> Result<()> {
    let admins = repos.users.list_admins().await?;

    if admins.is_empty() {
        println!("No admin users found");
        return Ok(());
    }

    println!("Admin users ({} total):", admins.len());
    println!("{:<12}  {:<30}  ID", "ROLE", "EMAIL");
    for u in admins {
        println!("{:<12}  {:<30}  {}", u.role.as_str(), u.email, u.id);
    }
    Ok(())
}
