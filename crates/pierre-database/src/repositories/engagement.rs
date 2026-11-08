// ABOUTME: The Engagement screen's reads, each in one admin scope: members' last activity, agent uses, published listings
// ABOUTME: One scope for every figure, so a tenant's screen never mixes in the platform's; emitted per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What the admin Engagement screen counts, read in the scope the operator
//! reads it in.
//!
//! A tenant's members are its `tenant_users` rows, the membership
//! [`super::agents`]' use totals already sum over; the every-tenant scope binds
//! `NULL` and reads every row. Store listings carry their own `tenant_id`.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::admin::scope::AdminScope;
use pierre_core::errors::AppResult;

/// The Engagement screen's reads.
#[async_trait]
pub trait EngagementRepository: Send + Sync {
    /// The last activity of every user in scope: the tenant's members, or
    /// every user. `None` for a user never seen active.
    async fn last_active_in_scope(
        &self,
        scope: AdminScope,
    ) -> AppResult<Vec<Option<DateTime<Utc>>>>;

    /// Uses per agent id, summed over the scope's users, all time.
    async fn agent_uses_in_scope(&self, scope: AdminScope) -> AppResult<HashMap<String, u32>>;

    /// Store listings published in scope.
    async fn published_listings_in_scope(&self, scope: AdminScope) -> AppResult<u32>;
}

/// The last activity of tenant `$1`'s members, or of every user when it binds
/// `NULL`.
pub(crate) const USERS_LAST_ACTIVE_SQL: &str = "SELECT u.last_active AS last_active \
     FROM users u \
     WHERE $1 IS NULL \
        OR EXISTS (SELECT 1 FROM tenant_users tu WHERE tu.user_id = u.id AND tu.tenant_id = $1)";

/// The store listings published in tenant `$1`, or in every tenant when it
/// binds `NULL`.
pub(crate) const PUBLISHED_LISTINGS_SQL: &str = "SELECT CAST(COUNT(*) AS BIGINT) AS n \
     FROM store_listings WHERE publish_status = 'published' AND ($1 IS NULL OR tenant_id = $1)";

/// Emit [`EngagementRepository`] for one backend type. The invoking shell
/// brings into scope `AdminScope`, `AppError`, `AppResult`, `DateTime`,
/// `HashMap`, `Row`, `Utc`, [`EngagementRepository`], the statements above and
/// the agents' `AGENT_USE_TOTALS_SQL`.
macro_rules! impl_engagement_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl EngagementRepository for $ty {
            async fn last_active_in_scope(
                &self,
                scope: AdminScope,
            ) -> AppResult<Vec<Option<DateTime<Utc>>>> {
                sqlx::query(USERS_LAST_ACTIVE_SQL)
                    .bind(scope.tenant_filter())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read last activity: {e}")))?
                    .iter()
                    .map(|row| {
                        row.try_get::<Option<DateTime<Utc>>, _>("last_active")
                            .map_err(|e| AppError::database(format!("users last_active: {e}")))
                    })
                    .collect()
            }

            async fn agent_uses_in_scope(
                &self,
                scope: AdminScope,
            ) -> AppResult<HashMap<String, u32>> {
                sqlx::query(AGENT_USE_TOTALS_SQL)
                    .bind(scope.tenant_filter())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to total agent uses: {e}")))?
                    .iter()
                    .map(|row| {
                        let agent_id: String = row.try_get("agent_id").map_err(|e| {
                            AppError::database(format!("agent_assignments agent_id: {e}"))
                        })?;
                        let uses: i64 = row.try_get("uses").map_err(|e| {
                            AppError::database(format!("agent_assignments uses: {e}"))
                        })?;
                        Ok((agent_id, u32::try_from(uses).unwrap_or(u32::MAX)))
                    })
                    .collect()
            }

            async fn published_listings_in_scope(&self, scope: AdminScope) -> AppResult<u32> {
                // `store_listings.tenant_id` is TEXT on both engines.
                let row = sqlx::query(PUBLISHED_LISTINGS_SQL)
                    .bind(scope.tenant_filter().map(|t| t.to_string()))
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count published listings: {e}"))
                    })?;
                let n: i64 = row
                    .try_get("n")
                    .map_err(|e| AppError::database(format!("store_listings count: {e}")))?;
                Ok(u32::try_from(n).unwrap_or(u32::MAX))
            }
        }
    };
}
pub(crate) use impl_engagement_repository;
