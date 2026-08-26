// ABOUTME: Store review decisions on SQLite: approving a coach into the catalogue and rejecting one
// ABOUTME: Approval assigns the catalogue handle inside the same transaction as the status change
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::PublishStatus;
use pierre_core::models::TenantId;
use uuid::Uuid;

use super::{CoachWithListing, StoreListingsManager};
use crate::database::coach_handle::ensure_catalogue_handle;

impl StoreListingsManager {
    /// Approve a coach and publish to the Store
    ///
    /// # Errors
    ///
    /// Returns an error if listing not found or not pending review
    pub async fn approve_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: impl Into<Option<Uuid>>,
    ) -> AppResult<CoachWithListing> {
        let admin_user_id = admin_user_id.into();
        let now = Utc::now();

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::database(format!("Failed to begin approval: {e}")))?;
        let result = sqlx::query(
            r"
            UPDATE store_listings SET
                publish_status = $1,
                published_at = $2,
                review_decision_at = $2,
                review_decision_by = $3,
                rejection_reason = NULL,
                updated_at = $2
            WHERE coach_id = $4 AND tenant_id = $5 AND publish_status = 'pending_review'
            ",
        )
        .bind(PublishStatus::Published.as_str())
        .bind(now.to_rfc3339())
        .bind(admin_user_id.map(|id| id.to_string()))
        .bind(coach_id)
        .bind(tenant_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::database(format!("Failed to approve coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::invalid_input(
                "Coach not found or not pending review",
            ));
        }

        ensure_catalogue_handle(&mut tx, coach_id, tenant_id).await?;
        tx.commit()
            .await
            .map_err(|e| AppError::database(format!("Failed to commit approval: {e}")))?;

        self.get_coach_with_listing(coach_id, &tenant_id).await
    }

    /// Reject a coach with a reason
    ///
    /// # Errors
    ///
    /// Returns an error if listing not found or not pending review
    pub async fn reject_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        admin_user_id: impl Into<Option<Uuid>>,
        reason: &str,
    ) -> AppResult<CoachWithListing> {
        let admin_user_id = admin_user_id.into();
        let now = Utc::now();

        let result = sqlx::query(
            r"
            UPDATE store_listings SET
                publish_status = $1,
                review_decision_at = $2,
                review_decision_by = $3,
                rejection_reason = $4,
                updated_at = $2
            WHERE coach_id = $5 AND tenant_id = $6 AND publish_status = 'pending_review'
            ",
        )
        .bind(PublishStatus::Rejected.as_str())
        .bind(now.to_rfc3339())
        .bind(admin_user_id.map(|id| id.to_string()))
        .bind(reason)
        .bind(coach_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to reject coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::invalid_input(
                "Coach not found or not pending review",
            ));
        }

        self.get_coach_with_listing(coach_id, &tenant_id).await
    }
}
