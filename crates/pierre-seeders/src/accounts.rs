// ABOUTME: Looks up a seeded account by email with the tenant it acts in, for seeders that write on its behalf
// ABOUTME: One lookup shared by the seeders that need an account another seeder or `user create` made first
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// One seeded account, its email and the tenant it acts in.
pub struct Account {
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
pub async fn account(repos: &RepositoryRegistry, email: &str) -> AppResult<Account> {
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
