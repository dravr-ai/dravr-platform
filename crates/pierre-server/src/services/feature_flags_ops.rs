// ABOUTME: Feature flags service layer — tenant defaults + per-user overrides + self-resolution
// ABOUTME: Layers tenant-isolation checks on top of pierre-database FeatureFlagsRepository
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use crate::context::DataContext;
use crate::errors::{AppError, ErrorCode};
use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::TenantId;
use pierre_database::repositories::FeatureFlagRow;
use tracing::info;
use uuid::Uuid;

/// Effective flag map keyed by the storage string form of [`FeatureKey`].
///
/// Using [`BTreeMap`] (over [`std::collections::HashMap`]) keeps JSON output
/// deterministic for snapshot tests and admin UI rendering.
pub type EffectiveFlags = BTreeMap<String, bool>;

/// Resolve the effective feature-flag map for a caller (no admin gate).
///
/// Backs `GET /api/me/features`. When the user has no tenant rows the map
/// still contains every known key with its compile-time default.
pub(crate) async fn resolve_self_flags(
    data: &DataContext,
    user_id: Uuid,
) -> Result<EffectiveFlags, AppError> {
    let tenant_uuid = primary_tenant_for_user(data, user_id).await?;

    let resolved = match tenant_uuid {
        Some(tid) => data
            .repos()
            .feature_flags
            .resolve_for_user(tid, user_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to resolve feature flags: {e}")))?,
        None => FeatureKey::ALL
            .iter()
            .map(|k| (*k, k.default_enabled()))
            .collect(),
    };

    Ok(resolved
        .into_iter()
        .map(|(k, v)| (k.as_str().to_string(), v))
        .collect())
}

/// Lookup the user's primary tenant for flag resolution.
///
/// Multi-tenant memberships exist in this codebase but a user has one
/// primary working tenant from the admin's perspective. We pick the first
/// row from [`TenantRepository::list_for_user`] (stable order); when the
/// user is not yet tied to any tenant we return `None` and the caller
/// falls back to compile-time defaults.
async fn primary_tenant_for_user(
    data: &DataContext,
    user_id: Uuid,
) -> Result<Option<Uuid>, AppError> {
    let tenants = data
        .repos()
        .tenants
        .list_for_user(user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to look up user tenants: {e}")))?;
    Ok(tenants.first().map(|t| t.id.0))
}

/// Admin: list stored tenant defaults. Empty vec when nothing is configured.
///
/// Authorization: super-admins see any tenant; tenant admins only their own.
pub(crate) async fn list_tenant_defaults(
    data: &DataContext,
    admin_user_id: Uuid,
    tenant_id: Uuid,
) -> Result<Vec<FeatureFlagRow>, AppError> {
    authorize_admin_for_tenant(data, admin_user_id, tenant_id).await?;
    data.repos()
        .feature_flags
        .list_tenant_defaults(tenant_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to list tenant defaults: {e}")))
}

/// Admin: insert-or-update a tenant default.
pub(crate) async fn set_tenant_default(
    data: &DataContext,
    admin_user_id: Uuid,
    tenant_id: Uuid,
    feature_key: FeatureKey,
    enabled: bool,
) -> Result<(), AppError> {
    authorize_admin_for_tenant(data, admin_user_id, tenant_id).await?;
    data.repos()
        .feature_flags
        .set_tenant_default(tenant_id, feature_key, enabled, Some(admin_user_id))
        .await
        .map_err(|e| AppError::internal(format!("Failed to set tenant default: {e}")))?;
    info!(
        admin_id = %admin_user_id,
        tenant_id = %tenant_id,
        feature_key = feature_key.as_str(),
        enabled,
        "Tenant feature default set"
    );
    Ok(())
}

/// Admin: clear a tenant default. Returns `true` when a row was removed.
pub(crate) async fn clear_tenant_default(
    data: &DataContext,
    admin_user_id: Uuid,
    tenant_id: Uuid,
    feature_key: FeatureKey,
) -> Result<bool, AppError> {
    authorize_admin_for_tenant(data, admin_user_id, tenant_id).await?;
    let removed = data
        .repos()
        .feature_flags
        .clear_tenant_default(tenant_id, feature_key)
        .await
        .map_err(|e| AppError::internal(format!("Failed to clear tenant default: {e}")))?;
    info!(
        admin_id = %admin_user_id,
        tenant_id = %tenant_id,
        feature_key = feature_key.as_str(),
        removed,
        "Tenant feature default cleared"
    );
    Ok(removed)
}

/// Admin: list stored per-user overrides for `target_user_id`.
pub(crate) async fn list_user_overrides(
    data: &DataContext,
    admin_user_id: Uuid,
    target_user_id: Uuid,
) -> Result<Vec<FeatureFlagRow>, AppError> {
    authorize_admin_for_user(data, admin_user_id, target_user_id).await?;
    data.repos()
        .feature_flags
        .list_user_overrides(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to list user overrides: {e}")))
}

/// Admin: insert-or-update a per-user override.
pub(crate) async fn set_user_override(
    data: &DataContext,
    admin_user_id: Uuid,
    target_user_id: Uuid,
    feature_key: FeatureKey,
    enabled: bool,
) -> Result<(), AppError> {
    authorize_admin_for_user(data, admin_user_id, target_user_id).await?;
    data.repos()
        .feature_flags
        .set_user_override(target_user_id, feature_key, enabled, Some(admin_user_id))
        .await
        .map_err(|e| AppError::internal(format!("Failed to set user override: {e}")))?;
    info!(
        admin_id = %admin_user_id,
        target_user_id = %target_user_id,
        feature_key = feature_key.as_str(),
        enabled,
        "Per-user feature override set"
    );
    Ok(())
}

/// Admin: clear a per-user override so the user inherits the tenant default.
pub(crate) async fn clear_user_override(
    data: &DataContext,
    admin_user_id: Uuid,
    target_user_id: Uuid,
    feature_key: FeatureKey,
) -> Result<bool, AppError> {
    authorize_admin_for_user(data, admin_user_id, target_user_id).await?;
    let removed = data
        .repos()
        .feature_flags
        .clear_user_override(target_user_id, feature_key)
        .await
        .map_err(|e| AppError::internal(format!("Failed to clear user override: {e}")))?;
    info!(
        admin_id = %admin_user_id,
        target_user_id = %target_user_id,
        feature_key = feature_key.as_str(),
        removed,
        "Per-user feature override cleared"
    );
    Ok(removed)
}

/// `Ok(())` when the admin can act on `tenant_id`: super-admins always,
/// regular admins only on tenants they belong to.
async fn authorize_admin_for_tenant(
    data: &DataContext,
    admin_user_id: Uuid,
    tenant_id: Uuid,
) -> Result<(), AppError> {
    let admin = load_admin(data, admin_user_id).await?;
    if admin.role.is_super_admin() {
        return Ok(());
    }

    let role = data
        .repos()
        .tenants
        .get_user_role(admin_user_id, TenantId(tenant_id))
        .await
        .map_err(|e| AppError::internal(format!("Failed to check tenant membership: {e}")))?;
    if role.is_some() {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Admin not a member of the target tenant",
        ))
    }
}

/// `Ok(())` when the admin can act on `target_user_id`: super-admins always,
/// regular admins only when the target belongs to one of the admin's tenants.
async fn authorize_admin_for_user(
    data: &DataContext,
    admin_user_id: Uuid,
    target_user_id: Uuid,
) -> Result<(), AppError> {
    let admin = load_admin(data, admin_user_id).await?;
    if admin.role.is_super_admin() {
        return Ok(());
    }

    let admin_tenants: Vec<Uuid> = data
        .repos()
        .tenants
        .list_for_user(admin_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to list admin tenants: {e}")))?
        .into_iter()
        .map(|t| t.id.0)
        .collect();

    let target_tenants: Vec<Uuid> = data
        .repos()
        .tenants
        .list_for_user(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to list target tenants: {e}")))?
        .into_iter()
        .map(|t| t.id.0)
        .collect();

    if admin_tenants.iter().any(|t| target_tenants.contains(t)) {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Target user is not in any tenant the admin manages",
        ))
    }
}

async fn load_admin(
    data: &DataContext,
    admin_user_id: Uuid,
) -> Result<pierre_core::models::User, AppError> {
    data.repos()
        .users
        .get_global(admin_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to load admin: {e}")))?
        .ok_or_else(|| AppError::not_found("Admin user not found"))
}
