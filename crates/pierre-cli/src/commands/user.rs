// ABOUTME: User management commands for pierre-cli
// ABOUTME: Handles creation and management of admin users for frontend login
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str::FromStr;

use bcrypt::{hash, DEFAULT_COST};
use dravr_tronc::dates::civil_clock::UTC_MINUTE;
use dravr_tronc::dates::Timestamp;
use pierre_core::constants::tiers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::{
    default_locale, CoachingPersona, Tenant, TenantId, User, UserStatus, UserTier,
};
use pierre_core::permissions::UserRole;
use pierre_database::database::CreateUserMcpTokenRequest;
use pierre_database::RepositoryRegistry;
use pierre_middleware::mask_email;
use pierre_services::admin_ops::{
    self, AccountCaller, AdminStandingLost, WithdrawnAdminCredentials,
};

type Result<T> = AppResult<T>;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::helpers::display::{display_admin_user_success, display_plain_user_success};

/// Non-identity attributes of the account `create` establishes.
pub(crate) struct AdminUserOptions {
    /// Update the account instead of refusing when the email already exists.
    ///
    /// The host-level break-glass path (carnet#1589): the account is written
    /// active with the password and role asked for, whatever its standing,
    /// with no approval first. The approve-first rule binds the admin
    /// surfaces (`user promote`, the console, the admin API); this command
    /// writes the database from the host shell, whose holder holds every
    /// power, and is how an admin is restored when no admin can sign in.
    pub force: bool,
    /// BCP-47 short locale for the account. `None` keeps the platform default
    /// on a new account and leaves an existing account's own locale untouched.
    pub locale: Option<String>,
    /// The role the account is created with: an admin unless a plain user is
    /// asked for.
    pub role: UserRole,
}

/// Create or update an account for frontend login: an admin, or with
/// `--role user` a plain active user.
pub(crate) async fn create(
    repos: &RepositoryRegistry,
    email: String,
    password: String,
    name: Option<String>,
    options: AdminUserOptions,
) -> Result<()> {
    let AdminUserOptions {
        force,
        locale,
        role,
    } = options;
    // Derive display name from email prefix if not provided
    let display_name =
        name.unwrap_or_else(|| email.split('@').next().unwrap_or("Admin").to_owned());

    info!(
        "User Creating {} user: {}",
        role.as_str(),
        mask_email(&email)
    );

    // Check if user already exists and handle accordingly
    if let Ok(Some(existing_user)) = repos.users.get_by_email(&email).await {
        update_existing_user(
            repos,
            existing_user,
            &email,
            &password,
            &display_name,
            UpdateOptions {
                force,
                locale,
                role,
            },
        )
        .await?;
    } else {
        create_new_user(repos, &email, &password, &display_name, locale, role).await?;
    }

    if role.is_admin() {
        display_admin_user_success(&email, &display_name);
        initialize_admin_jwt_secret(repos).await?;
        println!("\nSuccess Admin user is ready to use!");
    } else {
        display_plain_user_success(&email, &display_name);
    }

    Ok(())
}

/// The subset of [`AdminUserOptions`] the update path needs, kept as a struct so the
/// function stays under the argument threshold.
struct UpdateOptions {
    force: bool,
    locale: Option<String>,
    role: UserRole,
}

/// The tier an account `create` establishes holds: every tier's access for
/// an admin, the tier a sign-up starts on for a plain user.
const fn created_tier(role: UserRole) -> UserTier {
    match role {
        UserRole::Admin => UserTier::Enterprise,
        UserRole::User => UserTier::Starter,
    }
}

async fn update_existing_user(
    repos: &RepositoryRegistry,
    existing_user: User,
    email: &str,
    password: &str,
    name: &str,
    options: UpdateOptions,
) -> Result<()> {
    let UpdateOptions {
        force,
        locale,
        role,
    } = options;
    if !force {
        display_existing_user_error(&existing_user);
        return Err(AppError::invalid_input(
            "User already exists (use --force to update)",
        ));
    }
    // `--role user` creates and re-asserts plain accounts. Taking the role
    // from an admin is `user demote`'s, which says what the demotion
    // withdrew and puts it on the trail; a second way to demote would not.
    if existing_user.role.is_admin() && !role.is_admin() {
        return Err(AppError::invalid_input(format!(
            "{email} is an admin, and `user create --role user` does not demote. Nothing was \
             changed; run `pierre-cli user demote --email {email}` first."
        )));
    }

    // `user create --force` re-asserts the account as an admin, whatever role
    // it held: there is one admin role, so there is nothing to preserve or
    // lose. With `--role user` it re-asserts a plain account, and leaves the
    // tier that account holds alone.
    info!("Updating existing {} user...", role.as_str());
    let tier = if role.is_admin() {
        created_tier(role)
    } else {
        existing_user.tier
    };

    // NOTE: tenant_id is managed via tenant_users junction table, not on User struct
    let updated_user = User {
        id: existing_user.id,
        email: email.to_owned(),
        display_name: Some(name.to_owned()),
        password_hash: hash(password, DEFAULT_COST)
            .map_err(|e| AppError::internal(format!("bcrypt error: {e}")))?,
        tier,
        is_active: true,
        user_status: UserStatus::Active,
        role,
        approved_by: existing_user.approved_by,
        approved_at: existing_user.approved_at,
        created_at: existing_user.created_at,
        last_active: Timestamp::now(),
        firebase_uid: existing_user.firebase_uid,
        auth_provider: existing_user.auth_provider,
        analytics_consent: existing_user.analytics_consent,
        analytics_consent_at: existing_user.analytics_consent_at,
        locale: locale.unwrap_or(existing_user.locale),
        coaching_persona: existing_user.coaching_persona,
        manages_roster: existing_user.manages_roster,
        timezone: existing_user.timezone.clone(),
        theme: existing_user.theme,
    };

    // `update`, not `create`: the row exists, and `create` is insert-only on both
    // engines now — a bare INSERT would hit the unique email index. This is what
    // made `user create --force` impossible on PostgreSQL (carnet#124).
    repos.users.update(&updated_user).await?;
    Ok(())
}

fn display_existing_user_error(existing_user: &User) {
    let details = format!(
        "Email: {}\nName: {:?}\nCreated: {}",
        mask_email(&existing_user.email),
        existing_user.display_name,
        existing_user.created_at.format_with(&UTC_MINUTE)
    );

    error!(
        "Error User '{}' already exists!",
        mask_email(&existing_user.email)
    );
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

/// Create a personal tenant for a user and link them to it: on the
/// enterprise plan for an admin, on the starter plan for a plain user.
async fn create_and_link_personal_tenant(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    name: &str,
    role: UserRole,
) -> Result<()> {
    let (slug_prefix, plan) = match role {
        UserRole::Admin => ("admin", tiers::ENTERPRISE),
        UserRole::User => ("user", tiers::STARTER),
    };
    let tenant_id = TenantId::generate();
    let tenant_slug = format!("{slug_prefix}-{}", user_id.as_simple());
    let tenant = Tenant {
        id: tenant_id,
        name: format!("{name}'s Workspace"),
        slug: tenant_slug,
        domain: None,
        plan: plan.to_owned(),
        owner_user_id: user_id,
        created_at: Timestamp::now(),
        updated_at: Timestamp::now(),
    };

    repos.tenants.create(&tenant).await?;
    info!("Created personal tenant: {} ({})", tenant.name, tenant_id);

    repos.users.update_tenant_id(user_id, tenant_id).await?;

    Ok(())
}

/// Build the model of a new active account with `role`
fn build_user(
    user_id: Uuid,
    email: &str,
    password_hash: String,
    name: &str,
    locale: Option<String>,
    role: UserRole,
) -> User {
    // NOTE: tenant_id is managed via tenant_users junction table, not on User struct
    User {
        id: user_id,
        email: email.to_owned(),
        display_name: Some(name.to_owned()),
        password_hash,
        tier: created_tier(role),
        is_active: true,
        user_status: UserStatus::Active,
        role,
        approved_by: None,
        approved_at: Some(Timestamp::now()),
        created_at: Timestamp::now(),
        last_active: Timestamp::now(),
        firebase_uid: None,
        auth_provider: "email".to_owned(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: locale.unwrap_or_else(default_locale),
        coaching_persona: CoachingPersona::default(),
        manages_roster: false,
        timezone: None,
        theme: None,
    }
}

async fn create_new_user(
    repos: &RepositoryRegistry,
    email: &str,
    password: &str,
    name: &str,
    locale: Option<String>,
    role: UserRole,
) -> Result<()> {
    info!("Creating new {} user...", role.as_str());

    let user_id = Uuid::new_v4();
    let password_hash = hash(password, DEFAULT_COST)
        .map_err(|e| AppError::internal(format!("bcrypt error: {e}")))?;
    let new_user = build_user(user_id, email, password_hash, name, locale, role);

    repos.users.create(&new_user).await?;
    info!("Created {} user: {}", role.as_str(), mask_email(email));

    create_and_link_personal_tenant(repos, user_id, name, role).await?;

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
/// Operates directly on the database — no HTTP layer involved. The write is
/// the one the admin API's promotion makes (`admin_ops::grant_admin_role`),
/// so an account that is not active is refused here too: approve it first.
pub(crate) async fn promote(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let user = repos
        .users
        .get_by_email(&email)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User with email {email}")))?;

    if user.role.is_admin() {
        warn!("User {} is already an admin", mask_email(&email));
        println!("User {email} is already an admin (no change)");
        return Ok(());
    }

    let updated = admin_ops::grant_admin_role(repos, &user).await?;
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
/// The last active admin cannot be demoted: `set_admin_status` refuses it, so the
/// platform always keeps one admin.
pub(crate) async fn demote(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let user = repos
        .users
        .get_by_email(&email)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User with email {email}")))?;

    if !user.role.is_admin() {
        warn!("User {} is not an admin", mask_email(&email));
        println!("User {email} is not an admin (no change)");
        return Ok(());
    }

    let write = repos.users.set_admin_status(user.id, false).await?;
    let (updated, withdrawn) = (write.user, write.withdrawn);
    let host = AccountCaller::host_cli(None);
    record_withdrawn(
        repos,
        &host,
        &withdrawn,
        AdminStandingLost::Demoted,
        user.id,
    )
    .await;
    println!(
        "User {} demoted from admin (id={}, role={})",
        updated.email,
        updated.id,
        updated.role.as_str()
    );
    Ok(())
}

/// Say what a demotion or suspension took away, and put it on the trail: the
/// device logins the account approved and the impersonations it had open
/// stop with its admin standing. This command wrote the database itself, so
/// the row `host` writes names no signed-in admin.
async fn record_withdrawn(
    repos: &RepositoryRegistry,
    host: &AccountCaller,
    withdrawn: &WithdrawnAdminCredentials,
    cause: AdminStandingLost,
    account: Uuid,
) {
    if withdrawn.is_empty() {
        return;
    }
    host.record_withdrawn(repos, withdrawn, cause, Some(account))
        .await;
    println!(
        "Revoked {} device login(s) and ended {} impersonation(s) the account held as an admin",
        withdrawn.device_tokens.len(),
        withdrawn.impersonations
    );
}

/// Set a user's billing/quota tier (Starter / Professional / Enterprise).
///
/// Writes `users.tier` (the effective quota tier the server reads per request)
/// and records the admin-override marker so a later Stripe webhook cannot
/// clobber it. Delegates to the shared [`admin_ops::set_user_tier`] so the CLI,
/// the web admin route, and the super-admin token route share one implementation.
pub(crate) async fn set_tier(
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
pub(crate) async fn clear_tier(repos: &RepositoryRegistry, email: String) -> Result<()> {
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
pub(crate) async fn admin_actor(repos: &RepositoryRegistry) -> Option<Uuid> {
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

/// The host shell as the account writes know it: every power, no admin's
/// session, and its changes written against the admin [`admin_actor`] finds.
async fn host_caller(repos: &RepositoryRegistry) -> AccountCaller {
    AccountCaller::host_cli(admin_actor(repos).await)
}

/// Approve a pending user (status → active) and provision their default
/// MCP token: the one approval every surface takes (`admin_ops::approve_user`),
/// with no approver's tenant, so an account that belongs to none is given
/// one of its own.
pub(crate) async fn approve(
    repos: &RepositoryRegistry,
    email: String,
    reason: Option<String>,
) -> Result<()> {
    let user = lookup(repos, &email).await?;
    let host = host_caller(repos).await;
    let acting = host.acting_on(user)?;
    let updated = admin_ops::approve_user(repos, &acting, None)
        .await?
        .into_approved()?
        .user;
    info!(
        target_user_id = %acting.id(),
        reason = reason.as_deref().unwrap_or("No reason provided"),
        "User approved via pierre-cli"
    );
    println!(
        "Success: {} approved (status: {})",
        updated.email, updated.user_status
    );
    Ok(())
}

/// Suspend a user (status → suspended); they can no longer log in.
pub(crate) async fn suspend(
    repos: &RepositoryRegistry,
    email: String,
    reason: Option<String>,
) -> Result<()> {
    let user = lookup(repos, &email).await?;
    // The host shell is no admin's session: the admin it names is an audit
    // identity, so there is no self for the transition to refuse.
    let host = host_caller(repos).await;
    let acting = host.acting_on(user)?;
    let transition =
        admin_ops::transition_user_status(repos, &acting, UserStatus::Suspended).await?;
    record_withdrawn(
        repos,
        &host,
        &transition.withdrawn,
        AdminStandingLost::Deactivated,
        acting.id(),
    )
    .await;
    let updated = transition.user;
    info!(
        target_user_id = %acting.id(),
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
pub(crate) async fn reset_password(repos: &RepositoryRegistry, email: String) -> Result<()> {
    let user = lookup(repos, &email).await?;
    let host = AccountCaller::host_cli(None);
    let token = admin_ops::issue_password_reset_token(repos, &host.acting_on(user)?).await?;
    let minutes = admin_ops::PASSWORD_RESET_TTL_SECONDS / 60;
    println!("Password reset token for {email} (deliver out-of-band, shown once):");
    println!("  {token}");
    println!("Expires in {minutes} minutes.");
    Ok(())
}

/// Set a per-user monthly rate-limit override. An omitted cap means no
/// monthly ceiling for the user, whatever its tier.
pub(crate) async fn set_rate_limit(
    repos: &RepositoryRegistry,
    email: String,
    monthly: Option<u32>,
    note: Option<String>,
) -> Result<()> {
    let user = lookup(repos, &email).await?;
    let actor = admin_actor(repos).await;
    let note = note.or_else(|| Some("set via pierre-cli".to_owned()));
    admin_ops::set_user_rate_limit_override(repos, user.id, monthly, note, actor).await?;
    let monthly = monthly.map_or_else(|| "unlimited".to_owned(), |n| n.to_string());
    println!("Success: rate-limit override for {email} — monthly: {monthly}");
    Ok(())
}

/// Clear a per-user rate-limit override; the user reverts to tier defaults.
pub(crate) async fn clear_rate_limit(repos: &RepositoryRegistry, email: String) -> Result<()> {
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
pub(crate) fn parse_feature_key(key: &str) -> Result<FeatureKey> {
    FeatureKey::from_str(key).map_err(|_| {
        let valid: Vec<&str> = FeatureKey::ALL.iter().map(|k| k.as_str()).collect();
        AppError::invalid_input(format!(
            "Unknown feature key '{key}' — expected one of: {}",
            valid.join(", ")
        ))
    })
}

/// Set a per-user feature-flag override (wins over the tenant default).
pub(crate) async fn set_feature(
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
pub(crate) async fn clear_feature(
    repos: &RepositoryRegistry,
    email: String,
    key: String,
) -> Result<()> {
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
pub(crate) async fn list_features(repos: &RepositoryRegistry, email: String) -> Result<()> {
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
pub(crate) async fn list_admins(repos: &RepositoryRegistry) -> Result<()> {
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
