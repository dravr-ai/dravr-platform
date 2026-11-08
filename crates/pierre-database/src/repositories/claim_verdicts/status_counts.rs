// ABOUTME: Counts claim verdicts per status over the whole set an admin list filter selects, in one scope
// ABOUTME: The Claim Verdicts screen's status tiles; one statement for every scope, emitted for each backend by macro

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The status tiles above the admin verdict list.
//!
//! The list returns one page; the tiles count every row its filter selects, so
//! they are read here with the same axes as [`super::LIST_VERDICTS_FILTERED_SQL`]
//! and no `LIMIT`.

use async_trait::async_trait;
use pierre_core::admin::scope::AdminScope;
use pierre_core::errors::AppResult;

use super::{VerdictListFilter, VerdictStatusBreakdown};

/// Per-status counts over a filtered verdict set.
#[async_trait]
pub trait VerdictStatusCounts: Send + Sync {
    /// How many verdicts in `scope` the filter selects, per status. The
    /// filter's `limit` is ignored: every matching row counts.
    async fn count_verdicts_by_status(
        &self,
        scope: AdminScope,
        filter: &VerdictListFilter,
    ) -> AppResult<VerdictStatusBreakdown>;
}

/// The verdicts of tenant `$1`, or of every tenant when it binds `NULL`, per
/// status, over the axes `list_verdicts_filtered` binds in the same order.
pub(crate) const COUNT_VERDICTS_BY_STATUS_SQL: &str = concat!(
    "
            SELECT status, CAST(COUNT(*) AS BIGINT) AS n
            FROM claim_verdicts
            WHERE ($1 IS NULL OR tenant_id = $1)
              AND ($2 IS NULL OR status = $2)
              AND ($3 IS NULL OR category = $3)
              AND ($4 IS NULL OR agent_id = $4)
              AND ($5 IS NULL OR layer_fired = $5)
              AND ($6 IS NULL OR ($6 = '",
    undisposed_filter!(),
    "' AND disposition IS NULL) OR disposition = $6)
              AND ($7 IS NULL OR user_id = $7)
            GROUP BY status
            "
);

/// Emit [`VerdictStatusCounts`] for one backend type. The invoking shell
/// brings into scope `AdminScope`, `AppError`, `AppResult`, `ClaimCategory`,
/// `ClaimStatus`, `DispositionFilter`, `Row`, `VerdictLayer`,
/// `VerdictListFilter`, `VerdictStatusBreakdown`, `VerdictStatusCounts`,
/// `apply_status_count` and the statement above.
macro_rules! impl_verdict_status_counts {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl VerdictStatusCounts for $ty {
            async fn count_verdicts_by_status(
                &self,
                scope: AdminScope,
                filter: &VerdictListFilter,
            ) -> AppResult<VerdictStatusBreakdown> {
                let rows = sqlx::query(COUNT_VERDICTS_BY_STATUS_SQL)
                    .bind(scope.tenant_filter().map(|t| t.to_string()))
                    .bind(filter.status.map(ClaimStatus::as_str))
                    .bind(filter.category.map(ClaimCategory::as_str))
                    .bind(filter.agent_id.as_deref())
                    .bind(filter.layer_fired.map(VerdictLayer::as_str))
                    .bind(filter.disposition.map(DispositionFilter::as_str))
                    .bind(filter.user_id.as_deref())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count verdicts by status: {e}"))
                    })?;
                let mut counts = VerdictStatusBreakdown::default();
                for row in &rows {
                    let status: String = row
                        .try_get("status")
                        .map_err(|e| AppError::database(format!("claim_verdicts status: {e}")))?;
                    let n: i64 = row
                        .try_get("n")
                        .map_err(|e| AppError::database(format!("claim_verdicts count: {e}")))?;
                    apply_status_count(&mut counts, &status, n);
                }
                Ok(counts)
            }
        }
    };
}
pub(crate) use impl_verdict_status_counts;
