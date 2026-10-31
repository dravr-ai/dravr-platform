// ABOUTME: Looks up a seeded account by email with the tenant it acts in, for seeders that write on its behalf
// ABOUTME: One lookup shared by the seeders that need an account another seeder or `user create` made first, and the one way a seeder creates one
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use bcrypt::{hash, DEFAULT_COST};
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::seed_models::{SeedDemoUser, SeedTenant, SEED_LOCALE};
use pierre_database::RepositoryRegistry;
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
/// has no tenant, or when the stored tenant id is not a UUID.
pub(crate) async fn account(repos: &RepositoryRegistry, email: &str) -> AppResult<Account> {
    let user = repos
        .seeder
        .seed_find_user_by_email(email)
        .await?
        .ok_or_else(|| AppError::config(format!("User {email} not found; seed it first")))?;
    let tenant = repos
        .seeder
        .seed_get_user_tenant(user.id)
        .await?
        .ok_or_else(|| AppError::config(format!("User {email} has no tenant_id")))?;
    let tenant = Uuid::parse_str(&tenant)
        .map_err(|e| AppError::config(format!("Invalid tenant_id UUID for {email}: {e}")))?;
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
/// return its id.
///
/// # Errors
///
/// Returns the repository error from any write.
pub(crate) async fn create_account(
    repos: &RepositoryRegistry,
    new: &NewAccount<'_>,
) -> AppResult<Uuid> {
    let user_id = Uuid::new_v4();
    let now = Utc::now();

    repos
        .seeder
        .seed_insert_demo_user(&SeedDemoUser {
            id: user_id,
            email: new.email.to_owned(),
            display_name: new.display_name.to_owned(),
            password_hash: new.password_hash.to_owned(),
            tier: new.tier.to_owned(),
            status: new.status.to_owned(),
            is_admin: false,
            created_at: now,
            locale: SEED_LOCALE.to_owned(),
        })
        .await?;

    // The personal tenant, its plan the user's tier.
    let tenant_id = Uuid::new_v4();
    repos
        .seeder
        .seed_insert_tenant(&SeedTenant {
            id: tenant_id,
            name: format!("{}'s Workspace", new.display_name),
            slug: format!("user-{}", user_id.as_simple()),
            plan: new.tier.to_owned(),
            owner_user_id: user_id,
            created_at: now,
            updated_at: now,
        })
        .await?;
    // The user owns it, in the junction table and on the user row.
    repos
        .seeder
        .seed_insert_tenant_user(Uuid::new_v4(), tenant_id, user_id, now)
        .await?;
    repos
        .seeder
        .seed_update_user_tenant(user_id, tenant_id)
        .await?;
    Ok(user_id)
}
