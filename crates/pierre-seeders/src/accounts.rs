// ABOUTME: Looks up a seeded account by email with the tenant it acts in, for seeders that write on its behalf
// ABOUTME: One lookup shared by the seeders that need an account another seeder or `user create` made first, and the one way a seeder creates one
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use bcrypt::{hash, DEFAULT_COST};
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::permissions::UserRole;
use pierre_database::seed_models::{SeedDemoUser, SeedTenant, SEED_LOCALE};
use pierre_database::RepositoryRegistry;
use pierre_middleware::mask_email;
use uuid::Uuid;

/// One seeded account, its email and the tenant it acts in.
pub(crate) struct Account {
    pub id: Uuid,
    pub email: String,
    pub tenant: TenantId,
}

/// The account behind `email`, with its tenant.
///
/// # Errors
///
/// Returns a configuration error when no user has that email, when the user
/// has no tenant, or when the stored tenant id is not a UUID. Each names the
/// email masked, as every seeder message does.
pub(crate) async fn account(repos: &RepositoryRegistry, email: &str) -> AppResult<Account> {
    let masked = || mask_email(email);
    let user = repos
        .seeder
        .seed_find_user_by_email(email)
        .await?
        .ok_or_else(|| AppError::config(format!("User {} not found; seed it first", masked())))?;
    let tenant = repos
        .seeder
        .seed_get_user_tenant(user.id)
        .await?
        .ok_or_else(|| AppError::config(format!("User {} has no tenant_id", masked())))?;
    let tenant = Uuid::parse_str(&tenant)
        .map_err(|e| AppError::config(format!("Invalid tenant_id UUID for {}: {e}", masked())))?;
    Ok(Account {
        id: user.id,
        email: user.email,
        tenant: TenantId::from_uuid(tenant),
    })
}

/// A seeded account to create: who, the hash of the password they sign in
/// with ([`hash_password`]), and their plan.
pub(crate) struct NewAccount<'spec> {
    pub email: &'spec str,
    pub display_name: &'spec str,
    pub password_hash: &'spec str,
    pub tier: &'spec str,
    pub status: &'spec str,
    /// The personal tenant's slug; `None` for the one a sign-up gives,
    /// `user-<id>`. A seeder that must later recognise an account it
    /// created names the slug. The slug alone proves nothing, since anyone
    /// signed in may create a workspace with a slug of their choosing: the
    /// proof is the account's own workspace (`users.tenant_id`), which no
    /// sign-up names with a chosen slug.
    pub tenant_slug: Option<&'spec str>,
}

/// The stored hash of `password`. Hashing is deliberately slow, so a seeder
/// creating many accounts with one password hashes it once.
///
/// # Errors
///
/// Returns a configuration error when the password cannot be hashed.
pub(crate) fn hash_password(password: &str) -> AppResult<String> {
    hash(password, DEFAULT_COST).map_err(|e| AppError::config(format!("bcrypt error: {e}")))
}

/// Create the account with its own personal tenant, as a sign-up does, and
/// return its id. The account is written in one transaction
/// (`seed_create_account`): a run that fails part way leaves no account
/// behind, never one without its tenant that no rerun could complete.
///
/// `None` when an account already exists at the email, which is left
/// exactly as it was: a run that raced another to create it never changes
/// the password the other set.
///
/// # Errors
///
/// Returns the repository error from the transaction.
pub(crate) async fn create_account(
    repos: &RepositoryRegistry,
    new: &NewAccount<'_>,
) -> AppResult<Option<Uuid>> {
    let user_id = Uuid::new_v4();
    let now = Timestamp::now();
    let created = repos
        .seeder
        .seed_create_account(
            &SeedDemoUser {
                id: user_id,
                email: new.email.to_owned(),
                display_name: new.display_name.to_owned(),
                password_hash: new.password_hash.to_owned(),
                tier: new.tier.to_owned(),
                status: new.status.to_owned(),
                role: UserRole::User,
                created_at: now,
                locale: SEED_LOCALE.to_owned(),
            },
            // The personal tenant, its plan the user's tier.
            &SeedTenant {
                id: Uuid::new_v4(),
                name: format!("{}'s Workspace", new.display_name),
                slug: new.tenant_slug.map_or_else(
                    || format!("user-{}", user_id.as_simple()),
                    ToOwned::to_owned,
                ),
                plan: new.tier.to_owned(),
                owner_user_id: user_id,
                created_at: now,
                updated_at: now,
            },
        )
        .await?;
    Ok(created.then_some(user_id))
}
