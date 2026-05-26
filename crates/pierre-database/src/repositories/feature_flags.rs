// ABOUTME: Feature-flags repository trait — tenant defaults + per-user overrides
// ABOUTME: Resolution precedence: per-user override > tenant default > compile-time default
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::feature_flags::FeatureKey;
use std::collections::HashMap;
use uuid::Uuid;

/// A single stored feature-flag row.
///
/// Returned by [`FeatureFlagsRepository::list_tenant_defaults`] for tenant
/// scope and by [`FeatureFlagsRepository::list_user_overrides`] for per-user
/// scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureFlagRow {
    /// The feature being flagged.
    pub feature_key: FeatureKey,
    /// Stored value at this scope (tenant or user).
    pub enabled: bool,
    /// Last write timestamp.
    pub updated_at: DateTime<Utc>,
    /// Admin user who last wrote the row (audit trail). `None` when the
    /// referenced admin has been deleted (FK `ON DELETE SET NULL`).
    pub updated_by: Option<Uuid>,
}

/// CRUD for `tenant_feature_defaults` and `user_feature_overrides`.
///
/// Resolution semantics: `resolve_for_user` returns the effective value for
/// every known [`FeatureKey`] using the precedence
/// **per-user override > tenant default > compile-time default**.
///
/// Admin endpoints layer tenant-membership checks on top; this trait does
/// raw row CRUD only.
#[async_trait]
pub trait FeatureFlagsRepository: Send + Sync {
    /// Every stored tenant default for `tenant_id`, in `feature_key` order.
    /// Empty vec when nothing is configured.
    async fn list_tenant_defaults(&self, tenant_id: Uuid) -> AppResult<Vec<FeatureFlagRow>>;

    /// Insert-or-update one tenant default. `updated_at` is set to now;
    /// `updated_by` is the admin acting (audit), or `None` for system calls.
    async fn set_tenant_default(
        &self,
        tenant_id: Uuid,
        feature_key: FeatureKey,
        enabled: bool,
        updated_by: Option<Uuid>,
    ) -> AppResult<()>;

    /// Remove one tenant-default row. Returns `true` when a row was deleted,
    /// `false` when none matched (caller can treat both as success).
    async fn clear_tenant_default(
        &self,
        tenant_id: Uuid,
        feature_key: FeatureKey,
    ) -> AppResult<bool>;

    /// Every stored per-user override for `user_id`, in `feature_key` order.
    async fn list_user_overrides(&self, user_id: Uuid) -> AppResult<Vec<FeatureFlagRow>>;

    /// Insert-or-update one per-user override.
    async fn set_user_override(
        &self,
        user_id: Uuid,
        feature_key: FeatureKey,
        enabled: bool,
        updated_by: Option<Uuid>,
    ) -> AppResult<()>;

    /// Remove one per-user override row.
    async fn clear_user_override(&self, user_id: Uuid, feature_key: FeatureKey) -> AppResult<bool>;

    /// Effective flag map for a user: user-override > tenant-default >
    /// `FeatureKey::default_enabled()`. The returned map always contains
    /// every variant in `FeatureKey::ALL`.
    ///
    /// Default impl performs two reads (`list_user_overrides`,
    /// `list_tenant_defaults`) then merges; backends inherit unless they
    /// have a SQL-side optimisation.
    async fn resolve_for_user(
        &self,
        tenant_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<HashMap<FeatureKey, bool>> {
        let tenant = self.list_tenant_defaults(tenant_id).await?;
        let user = self.list_user_overrides(user_id).await?;

        let mut effective: HashMap<FeatureKey, bool> = FeatureKey::ALL
            .iter()
            .map(|k| (*k, k.default_enabled()))
            .collect();

        for row in tenant {
            effective.insert(row.feature_key, row.enabled);
        }
        for row in user {
            effective.insert(row.feature_key, row.enabled);
        }

        Ok(effective)
    }
}
