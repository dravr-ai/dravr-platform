// ABOUTME: Tenant administration commands for pierre-cli (set plan)
// ABOUTME: Operator backdoor to set a tenant's plan outside the Stripe billing loop
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_services::admin_ops;
use uuid::Uuid;

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
        .ok_or_else(|| AppError::not_found(format!("User with email {email} not found")))?;

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
        return Ok(TenantId::from(uuid));
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
