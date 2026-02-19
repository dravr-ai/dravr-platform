// ABOUTME: Database operations for Store listings (publishing workflow for coaches)
// ABOUTME: Manages coach publish status, reviews, installs, and Store browsing queries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::coaches::{
    Coach, CoachCategory, CoachVisibility, PublishStatus, StoreAdminStats,
};
use crate::errors::{AppError, AppResult};
use crate::pagination::{Cursor, CursorPage, StoreCursor, StoreSortOrder};
use chrono::{DateTime, Utc};
use pierre_core::models::TenantId;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, Row, SqlitePool};
use std::collections::HashMap;
use uuid::Uuid;

use super::coaches::row_to_coach;

/// A store listing tracks the publishing state of a coach in the Store
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreListing {
    /// Unique identifier for the listing
    pub id: Uuid,
    /// Coach this listing belongs to
    pub coach_id: Uuid,
    /// Tenant for multi-tenancy isolation
    pub tenant_id: String,
    /// Publishing status for Store workflow
    #[serde(default)]
    pub publish_status: PublishStatus,
    /// When the coach was published to the store
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    /// When the coach was submitted for review
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_submitted_at: Option<DateTime<Utc>>,
    /// When admin made the review decision
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_decision_at: Option<DateTime<Utc>>,
    /// Admin user who made the review decision
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_decision_by: Option<String>,
    /// Reason for rejection (if rejected)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
    /// Number of Store installs (denormalized for performance)
    #[serde(default)]
    pub install_count: u32,
    /// URL to coach icon/avatar for Store display
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// Author profile ID (for published coaches)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

/// A coach combined with its store listing data (for store-related queries)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachWithListing {
    /// The coach data
    #[serde(flatten)]
    pub coach: Coach,
    /// The store listing data
    pub listing: StoreListing,
}

/// Store listings database operations manager
pub struct StoreListingsManager {
    pool: SqlitePool,
}

impl StoreListingsManager {
    /// Create a new store listings manager
    #[must_use]
    pub const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Submit a coach for Store review (creates listing if needed)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Coach not found or user doesn't own it
    /// - Coach already has a non-draft listing
    /// - Database operation fails
    pub async fn submit_for_review(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<StoreListing> {
        let now = Utc::now();

        // Verify the coach exists and belongs to the user
        let coach_row = sqlx::query(
            "SELECT id, tenant_id FROM coaches WHERE id = $1 AND user_id = $2 AND tenant_id = $3",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to check coach ownership: {e}")))?;

        if coach_row.is_none() {
            return Err(AppError::invalid_input(
                "Coach not found, not owned by you, or not in your tenant",
            ));
        }

        // Check if listing already exists
        let existing =
            sqlx::query("SELECT id, publish_status FROM store_listings WHERE coach_id = $1")
                .bind(coach_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to check existing listing: {e}"))
                })?;

        if let Some(row) = existing {
            let status: String = row.get("publish_status");
            if status != "draft" {
                return Err(AppError::invalid_input(
                    "Coach is not in draft status — cannot submit for review",
                ));
            }
            let listing_id: String = row.get("id");

            // Update existing draft listing to pending_review
            sqlx::query(
                r"
                UPDATE store_listings SET
                    publish_status = $1,
                    review_submitted_at = $2,
                    updated_at = $2
                WHERE id = $3
                ",
            )
            .bind(PublishStatus::PendingReview.as_str())
            .bind(now.to_rfc3339())
            .bind(&listing_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to submit for review: {e}")))?;

            // Also update coaches.updated_at to reflect the change
            sqlx::query("UPDATE coaches SET updated_at = $1 WHERE id = $2")
                .bind(now.to_rfc3339())
                .bind(coach_id)
                .execute(&self.pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to update coach timestamp: {e}"))
                })?;

            return self
                .get_listing(coach_id)
                .await?
                .ok_or_else(|| AppError::internal("Failed to fetch updated listing"));
        }

        // Create new listing with pending_review status
        let listing_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT INTO store_listings (
                id, coach_id, tenant_id, publish_status, review_submitted_at,
                install_count, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, 0, $5, $5)
            ",
        )
        .bind(listing_id.to_string())
        .bind(coach_id)
        .bind(tenant_id)
        .bind(PublishStatus::PendingReview.as_str())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create store listing: {e}")))?;

        // Also update coaches.updated_at
        sqlx::query("UPDATE coaches SET updated_at = $1 WHERE id = $2")
            .bind(now.to_rfc3339())
            .bind(coach_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to update coach timestamp: {e}")))?;

        self.get_listing(coach_id)
            .await?
            .ok_or_else(|| AppError::internal("Failed to fetch created listing"))
    }

    /// Get a store listing by coach ID
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_listing(&self, coach_id: &str) -> AppResult<Option<StoreListing>> {
        let row = sqlx::query(
            r"
            SELECT id, coach_id, tenant_id, publish_status, published_at,
                   review_submitted_at, review_decision_at, review_decision_by,
                   rejection_reason, install_count, icon_url, author_id,
                   created_at, updated_at
            FROM store_listings
            WHERE coach_id = $1
            ",
        )
        .bind(coach_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get store listing: {e}")))?;

        row.map(|r| row_to_store_listing(&r)).transpose()
    }

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
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to approve coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::invalid_input(
                "Coach not found or not pending review",
            ));
        }

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

    /// Unpublish a coach (revert from published to draft)
    ///
    /// # Errors
    ///
    /// Returns an error if listing not found or not published
    pub async fn unpublish_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<CoachWithListing> {
        let now = Utc::now();

        let result = sqlx::query(
            r"
            UPDATE store_listings SET
                publish_status = $1,
                published_at = NULL,
                updated_at = $2
            WHERE coach_id = $3 AND tenant_id = $4 AND publish_status = 'published'
            ",
        )
        .bind(PublishStatus::Draft.as_str())
        .bind(now.to_rfc3339())
        .bind(coach_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to unpublish coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::invalid_input("Coach not found or not published"));
        }

        self.get_coach_with_listing(coach_id, &tenant_id).await
    }

    /// Get coaches pending admin review
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_pending_review_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let limit_val = i64::from(limit.unwrap_or(50).min(100));
        let offset_val = i64::from(offset.unwrap_or(0));

        let rows = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE sl.tenant_id = $1 AND sl.publish_status = 'pending_review'
            ORDER BY sl.review_submitted_at ASC
            LIMIT $2 OFFSET $3
            "
        ))
        .bind(tenant_id)
        .bind(limit_val)
        .bind(offset_val)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get pending review coaches: {e}")))?;

        rows.iter().map(row_to_coach_with_listing).collect()
    }

    /// Get coaches that have been rejected
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_rejected_coaches(
        &self,
        tenant_id: TenantId,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let limit_val = i64::from(limit.unwrap_or(50).min(100));
        let offset_val = i64::from(offset.unwrap_or(0));

        let rows = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE sl.tenant_id = $1 AND sl.publish_status = 'rejected'
            ORDER BY sl.review_decision_at DESC
            LIMIT $2 OFFSET $3
            "
        ))
        .bind(tenant_id)
        .bind(limit_val)
        .bind(offset_val)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get rejected coaches: {e}")))?;

        rows.iter().map(row_to_coach_with_listing).collect()
    }

    /// Get store admin statistics
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_store_admin_stats(&self, tenant_id: TenantId) -> AppResult<StoreAdminStats> {
        let row = sqlx::query(
            r"
            SELECT
                COUNT(CASE WHEN publish_status = 'pending_review' THEN 1 END) as pending_count,
                COUNT(CASE WHEN publish_status = 'published' THEN 1 END) as published_count,
                COUNT(CASE WHEN publish_status = 'rejected' THEN 1 END) as rejected_count,
                COALESCE(SUM(CASE WHEN publish_status = 'published' THEN install_count ELSE 0 END), 0) as total_installs
            FROM store_listings
            WHERE tenant_id = $1
            ",
        )
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get store stats: {e}")))?;

        let pending_count: i64 = row.get("pending_count");
        let published_count: i64 = row.get("published_count");
        let rejected_count: i64 = row.get("rejected_count");
        let total_installs: i64 = row.get("total_installs");

        // Calculate rejection rate
        let total_decided = published_count + rejected_count;
        #[allow(clippy::cast_precision_loss)]
        let rejection_rate = if total_decided > 0 {
            (rejected_count as f64 / total_decided as f64) * 100.0
        } else {
            0.0
        };

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(StoreAdminStats {
            pending_count: pending_count as u32,
            published_count: published_count as u32,
            rejected_count: rejected_count as u32,
            total_installs: total_installs as u32,
            rejection_rate,
        })
    }

    /// Get author email for a coach by looking up the user
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_author_email(&self, user_id: Uuid) -> AppResult<Option<String>> {
        let row = sqlx::query("SELECT email FROM users WHERE id = $1")
            .bind(user_id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get author email: {e}")))?;

        Ok(row.map(|r| r.get("email")))
    }

    /// Get published coaches for the Store (cross-tenant)
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_published_coaches(
        &self,
        category: Option<CoachCategory>,
        sort_by: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let limit_val = i64::from(limit.unwrap_or(50).min(100));
        let offset_val = i64::from(offset.unwrap_or(0));

        let order_clause = match sort_by {
            Some("popular") => "sl.install_count DESC, sl.published_at DESC",
            Some("title") => "c.title ASC",
            _ => "sl.published_at DESC",
        };

        let category_filter = category.map_or_else(String::new, |cat| {
            format!("AND c.category = '{}'", cat.as_str())
        });

        let query = format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE sl.publish_status = 'published' {category_filter}
            ORDER BY {order_clause}
            LIMIT $1 OFFSET $2
            "
        );

        let rows = sqlx::query(&query)
            .bind(limit_val)
            .bind(offset_val)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get published coaches: {e}")))?;

        rows.iter().map(row_to_coach_with_listing).collect()
    }

    /// Get published coaches with cursor-based pagination
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails or cursor is invalid
    pub async fn get_published_coaches_cursor(
        &self,
        category: Option<CoachCategory>,
        sort_by: StoreSortOrder,
        limit: u32,
        cursor: Option<&str>,
    ) -> AppResult<CursorPage<CoachWithListing>> {
        let limit_val = limit.min(100);
        let fetch_limit = i64::from(limit_val) + 1;

        let decoded_cursor = if let Some(cursor_str) = cursor {
            let cursor_obj = Cursor::from_string(cursor_str.to_owned());
            let decoded = StoreCursor::decode(&cursor_obj, sort_by)
                .ok_or_else(|| AppError::invalid_input("Invalid cursor for current sort order"))?;
            Some(decoded)
        } else {
            None
        };

        let category_filter = category.map_or_else(String::new, |cat| {
            format!("AND c.category = '{}'", cat.as_str())
        });

        let rows = match sort_by {
            StoreSortOrder::Newest => {
                self.query_newest_sort(&category_filter, decoded_cursor.as_ref(), fetch_limit)
                    .await?
            }
            StoreSortOrder::Popular => {
                self.query_popular_sort(&category_filter, decoded_cursor.as_ref(), fetch_limit)
                    .await?
            }
            StoreSortOrder::Title => {
                self.query_title_sort(&category_filter, decoded_cursor.as_ref(), fetch_limit)
                    .await?
            }
        };

        let mut all_items: Vec<CoachWithListing> = Vec::new();
        for row in rows {
            all_items.push(row_to_coach_with_listing(&row)?);
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let has_more = all_items.len() > limit_val as usize;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let items: Vec<CoachWithListing> = all_items.into_iter().take(limit_val as usize).collect();

        let next_cursor = if has_more {
            items.last().map(|cwl| {
                let store_cursor = match sort_by {
                    StoreSortOrder::Newest => {
                        StoreCursor::newest(cwl.coach.id.to_string(), cwl.listing.published_at)
                    }
                    StoreSortOrder::Popular => StoreCursor::popular(
                        cwl.coach.id.to_string(),
                        cwl.listing.install_count,
                        cwl.listing.published_at,
                    ),
                    StoreSortOrder::Title => {
                        StoreCursor::title(cwl.coach.id.to_string(), cwl.coach.title.clone())
                    }
                };
                store_cursor.encode()
            })
        } else {
            None
        };

        Ok(CursorPage::new(items, next_cursor, None, has_more))
    }

    /// Query for newest sort order (`published_at` DESC, id DESC)
    async fn query_newest_sort(
        &self,
        category_filter: &str,
        cursor: Option<&StoreCursor>,
        fetch_limit: i64,
    ) -> AppResult<Vec<SqliteRow>> {
        if let Some(c) = cursor {
            let ts = c.published_at.map_or(0, |dt| dt.timestamp_millis());
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                  AND (
                    (CAST(strftime('%s', sl.published_at) AS INTEGER) * 1000 +
                     CAST(strftime('%f', sl.published_at) * 1000 AS INTEGER) % 1000) < $1
                    OR (
                      (CAST(strftime('%s', sl.published_at) AS INTEGER) * 1000 +
                       CAST(strftime('%f', sl.published_at) * 1000 AS INTEGER) % 1000) = $1
                      AND c.id < $2
                    )
                  )
                ORDER BY sl.published_at DESC, c.id DESC
                LIMIT $3
                "
            );
            sqlx::query(&query)
                .bind(ts)
                .bind(&c.id)
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to query coaches (newest): {e}")))
        } else {
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                ORDER BY sl.published_at DESC, c.id DESC
                LIMIT $1
                "
            );
            sqlx::query(&query)
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to query coaches (newest first): {e}"))
                })
        }
    }

    /// Query for popular sort order (`install_count` DESC, `published_at` DESC, id DESC)
    async fn query_popular_sort(
        &self,
        category_filter: &str,
        cursor: Option<&StoreCursor>,
        fetch_limit: i64,
    ) -> AppResult<Vec<SqliteRow>> {
        if let Some(c) = cursor {
            let count = c.install_count.unwrap_or(0);
            let ts = c.published_at.map_or(0, |dt| dt.timestamp_millis());
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                  AND (
                    sl.install_count < $1
                    OR (
                      sl.install_count = $1
                      AND (CAST(strftime('%s', sl.published_at) AS INTEGER) * 1000 +
                           CAST(strftime('%f', sl.published_at) * 1000 AS INTEGER) % 1000) < $2
                    )
                    OR (
                      sl.install_count = $1
                      AND (CAST(strftime('%s', sl.published_at) AS INTEGER) * 1000 +
                           CAST(strftime('%f', sl.published_at) * 1000 AS INTEGER) % 1000) = $2
                      AND c.id < $3
                    )
                  )
                ORDER BY sl.install_count DESC, sl.published_at DESC, c.id DESC
                LIMIT $4
                "
            );
            sqlx::query(&query)
                .bind(count)
                .bind(ts)
                .bind(&c.id)
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to query coaches (popular): {e}")))
        } else {
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                ORDER BY sl.install_count DESC, sl.published_at DESC, c.id DESC
                LIMIT $1
                "
            );
            sqlx::query(&query)
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to query coaches (popular first): {e}"))
                })
        }
    }

    /// Query for title sort order (title ASC, id ASC)
    async fn query_title_sort(
        &self,
        category_filter: &str,
        cursor: Option<&StoreCursor>,
        fetch_limit: i64,
    ) -> AppResult<Vec<SqliteRow>> {
        if let Some(c) = cursor {
            let title = c.title.as_deref().unwrap_or("");
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                  AND (
                    c.title > $1
                    OR (c.title = $1 AND c.id > $2)
                  )
                ORDER BY c.title ASC, c.id ASC
                LIMIT $3
                "
            );
            sqlx::query(&query)
                .bind(title)
                .bind(&c.id)
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to query coaches (title): {e}")))
        } else {
            let query = format!(
                r"
                SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
                FROM coaches c
                JOIN store_listings sl ON c.id = sl.coach_id
                WHERE sl.publish_status = 'published' {category_filter}
                ORDER BY c.title ASC, c.id ASC
                LIMIT $1
                "
            );
            sqlx::query(&query)
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to query coaches (title first): {e}"))
                })
        }
    }

    /// Search published coaches
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn search_published_coaches(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> AppResult<Vec<CoachWithListing>> {
        let limit_val = i64::from(limit.unwrap_or(20).min(100));
        let search_pattern = format!("%{query}%");

        let rows = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE sl.publish_status = 'published'
              AND (c.title LIKE $1 OR c.description LIKE $1 OR c.tags LIKE $1)
            ORDER BY sl.install_count DESC, sl.published_at DESC
            LIMIT $2
            "
        ))
        .bind(&search_pattern)
        .bind(limit_val)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to search published coaches: {e}")))?;

        rows.iter().map(row_to_coach_with_listing).collect()
    }

    /// Get a single published coach by ID (cross-tenant)
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_published_coach(&self, coach_id: &str) -> AppResult<Option<CoachWithListing>> {
        let row = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE c.id = $1 AND sl.publish_status = 'published'
            "
        ))
        .bind(coach_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get published coach: {e}")))?;

        row.map(|r| row_to_coach_with_listing(&r)).transpose()
    }

    /// Get category counts for published coaches
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_category_counts(&self) -> AppResult<HashMap<CoachCategory, i64>> {
        let rows = sqlx::query(
            r"
            SELECT c.category, COUNT(*) as count
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE sl.publish_status = 'published'
            GROUP BY c.category
            ",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get category counts: {e}")))?;

        let mut counts = HashMap::new();
        for row in &rows {
            let cat_str: String = row.get("category");
            let count: i64 = row.get("count");
            counts.insert(CoachCategory::parse(&cat_str), count);
        }
        Ok(counts)
    }

    /// Increment install count for a coach's store listing
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn increment_install_count(&self, coach_id: &str) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE store_listings
            SET install_count = install_count + 1, updated_at = $1
            WHERE coach_id = $2 AND publish_status = 'published'
            ",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(coach_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to increment install count: {e}")))?;

        Ok(())
    }

    /// Decrement install count for a coach's store listing
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn decrement_install_count(&self, coach_id: &str) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE store_listings
            SET install_count = MAX(install_count - 1, 0), updated_at = $1
            WHERE coach_id = $2
            ",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(coach_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to decrement install count: {e}")))?;

        Ok(())
    }

    /// Install a coach from the Store (creates user's copy)
    ///
    /// # Errors
    ///
    /// Returns an error if source coach not found/published, already installed, or DB fails
    pub async fn install_from_store(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        // Get the source coach (must be published, cross-tenant lookup)
        let source = self
            .get_published_coach(source_coach_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Published coach {source_coach_id}")))?;

        // Check if user already has this coach installed
        let existing = sqlx::query(
            "SELECT id FROM coaches WHERE user_id = $1 AND tenant_id = $2 AND forked_from = $3",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(source_coach_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to check existing installation: {e}")))?;

        if existing.is_some() {
            return Err(AppError::invalid_input(format!(
                "Coach {} is already installed",
                source.coach.title
            )));
        }

        // Create the user's copy (without store fields — it's a personal coach)
        let now = Utc::now();
        let id = Uuid::new_v4();
        let tags_json = serde_json::to_string(&source.coach.tags)?;
        let sample_prompts_json = serde_json::to_string(&source.coach.sample_prompts)?;
        let prerequisites_json = serde_json::to_string(&source.coach.prerequisites)?;

        sqlx::query(
            r"
            INSERT INTO coaches (
                id, user_id, tenant_id, title, description, system_prompt, category, tags,
                sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites, forked_from
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, 0, $12, $13, $14)
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(&source.coach.title)
        .bind(&source.coach.description)
        .bind(&source.coach.system_prompt)
        .bind(source.coach.category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(i64::from(source.coach.token_count))
        .bind(now.to_rfc3339())
        .bind(CoachVisibility::Private.as_str())
        .bind(&prerequisites_json)
        .bind(source_coach_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to install coach: {e}")))?;

        // Create self-assignment row for the installed coach
        let assignment_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT OR IGNORE INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, is_active, use_count, last_used_at)
            VALUES ($1, $2, $3, $3, $4, 0, 0, 0, NULL)
            ",
        )
        .bind(assignment_id.to_string())
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create coach assignment: {e}")))?;

        // Increment install count on the source coach's listing
        self.increment_install_count(source_coach_id).await?;

        // Fetch and return the created coach
        let row = sqlx::query(&format!(
            "SELECT {COACH_COLUMNS} FROM coaches WHERE id = $1 AND user_id = $2 AND tenant_id = $3"
        ))
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch installed coach: {e}")))?;

        row_to_coach(&row)
    }

    /// Uninstall a coach (delete user's installed copy)
    ///
    /// # Errors
    ///
    /// Returns an error if coach not found, not a store install, or DB fails
    pub async fn uninstall_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<String> {
        // Get the coach to verify ownership and get forked_from
        let row = sqlx::query(
            "SELECT id, forked_from FROM coaches WHERE id = $1 AND user_id = $2 AND tenant_id = $3",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get coach: {e}")))?
        .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;

        let source_id: Option<String> = row.get("forked_from");
        let source_id = source_id.ok_or_else(|| {
            AppError::invalid_input("This coach was not installed from the Store")
        })?;

        // Delete the user's copy
        sqlx::query("DELETE FROM coaches WHERE id = $1 AND user_id = $2 AND tenant_id = $3")
            .bind(coach_id)
            .bind(user_id.to_string())
            .bind(tenant_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to uninstall coach: {e}")))?;

        // Decrement install count on the source coach's listing
        self.decrement_install_count(&source_id).await?;

        Ok(source_id)
    }

    /// Get user's installed coaches from the Store
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_installed_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>> {
        let rows = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS}
            FROM coaches
            WHERE user_id = $1 AND tenant_id = $2 AND forked_from IS NOT NULL
            ORDER BY created_at DESC
            "
        ))
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get installed coaches: {e}")))?;

        rows.iter().map(row_to_coach).collect()
    }

    /// Create or ensure a store listing exists for a coach (used during `icon_url` updates)
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn ensure_listing(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<StoreListing> {
        if let Some(listing) = self.get_listing(coach_id).await? {
            return Ok(listing);
        }

        let now = Utc::now();
        let listing_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT INTO store_listings (
                id, coach_id, tenant_id, publish_status, install_count, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, 0, $5, $5)
            ",
        )
        .bind(listing_id.to_string())
        .bind(coach_id)
        .bind(tenant_id)
        .bind(PublishStatus::Draft.as_str())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create store listing: {e}")))?;

        self.get_listing(coach_id)
            .await?
            .ok_or_else(|| AppError::internal("Failed to fetch created listing"))
    }

    /// Get a coach with its listing by `coach_id` and `tenant_id`
    async fn get_coach_with_listing(
        &self,
        coach_id: &str,
        tenant_id: &TenantId,
    ) -> AppResult<CoachWithListing> {
        let row = sqlx::query(&format!(
            r"
            SELECT {COACH_COLUMNS_ALIASED}, {LISTING_COLUMNS_ALIASED}
            FROM coaches c
            JOIN store_listings sl ON c.id = sl.coach_id
            WHERE c.id = $1 AND sl.tenant_id = $2
            "
        ))
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get coach with listing: {e}")))?;

        row_to_coach_with_listing(&row)
    }
}

/// Column list for coach queries (no table alias)
const COACH_COLUMNS: &str = r"id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations";

/// Column list for coach queries with table alias
const COACH_COLUMNS_ALIASED: &str = r"c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
                   c.category, c.tags, c.sample_prompts, c.token_count,
                   c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
                   c.forked_from, c.max_tool_iterations";

/// Column list for store listing queries with table alias
const LISTING_COLUMNS_ALIASED: &str = r"sl.id as sl_id, sl.publish_status, sl.published_at,
                   sl.review_submitted_at, sl.review_decision_at, sl.review_decision_by,
                   sl.rejection_reason, sl.install_count, sl.icon_url, sl.author_id,
                   sl.created_at as sl_created_at, sl.updated_at as sl_updated_at";

/// Convert a database row to a `StoreListing` struct
fn row_to_store_listing(row: &SqliteRow) -> AppResult<StoreListing> {
    let id_str: String = row
        .try_get("id")
        .or_else(|_| row.try_get("sl_id"))
        .map_err(|e| AppError::internal(format!("Missing listing id: {e}")))?;
    let coach_id_str: String = row
        .try_get("coach_id")
        .or_else(|_| {
            // In JOINed queries, coach_id might come from coaches.id
            row.try_get("id")
        })
        .map_err(|e| AppError::internal(format!("Missing coach_id: {e}")))?;

    let publish_status_str: String = row
        .try_get("publish_status")
        .unwrap_or_else(|_| "draft".to_owned());
    let published_at_str: Option<String> = row.try_get("published_at").ok().flatten();
    let review_submitted_at_str: Option<String> = row.try_get("review_submitted_at").ok().flatten();
    let review_decision_at_str: Option<String> = row.try_get("review_decision_at").ok().flatten();
    let review_decision_by: Option<String> = row.try_get("review_decision_by").ok().flatten();
    let rejection_reason: Option<String> = row.try_get("rejection_reason").ok().flatten();
    let install_count: i64 = row.try_get("install_count").unwrap_or(0);
    let icon_url: Option<String> = row.try_get("icon_url").ok().flatten();
    let author_id: Option<String> = row.try_get("author_id").ok().flatten();

    let created_at_str: String = row
        .try_get("sl_created_at")
        .or_else(|_| row.try_get("created_at"))
        .unwrap_or_else(|_| Utc::now().to_rfc3339());
    let updated_at_str: String = row
        .try_get("sl_updated_at")
        .or_else(|_| row.try_get("updated_at"))
        .unwrap_or_else(|_| Utc::now().to_rfc3339());

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(StoreListing {
        id: Uuid::parse_str(&id_str)
            .map_err(|e| AppError::internal(format!("Invalid listing UUID: {e}")))?,
        coach_id: Uuid::parse_str(&coach_id_str)
            .map_err(|e| AppError::internal(format!("Invalid coach UUID: {e}")))?,
        tenant_id: row.try_get("tenant_id").unwrap_or_else(|_| String::new()),
        publish_status: PublishStatus::parse(&publish_status_str),
        published_at: published_at_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        review_submitted_at: review_submitted_at_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        review_decision_at: review_decision_at_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        review_decision_by,
        rejection_reason,
        install_count: install_count as u32,
        icon_url,
        author_id,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
    })
}

/// Convert a `JOIN`ed database row to a `CoachWithListing`
fn row_to_coach_with_listing(row: &SqliteRow) -> AppResult<CoachWithListing> {
    let coach = row_to_coach(row)?;

    // For the listing, we need to read sl_id as the listing id
    let sl_id_str: String = row
        .try_get("sl_id")
        .map_err(|e| AppError::internal(format!("Missing sl_id in JOIN: {e}")))?;

    let publish_status_str: String = row
        .try_get("publish_status")
        .unwrap_or_else(|_| "draft".to_owned());
    let published_at_str: Option<String> = row.try_get("published_at").ok().flatten();
    let review_submitted_at_str: Option<String> = row.try_get("review_submitted_at").ok().flatten();
    let review_decision_at_str: Option<String> = row.try_get("review_decision_at").ok().flatten();
    let review_decision_by: Option<String> = row.try_get("review_decision_by").ok().flatten();
    let rejection_reason: Option<String> = row.try_get("rejection_reason").ok().flatten();
    let install_count: i64 = row.try_get("install_count").unwrap_or(0);
    let icon_url: Option<String> = row.try_get("icon_url").ok().flatten();
    let author_id: Option<String> = row.try_get("author_id").ok().flatten();

    let sl_created_at_str: String = row
        .try_get("sl_created_at")
        .unwrap_or_else(|_| Utc::now().to_rfc3339());
    let sl_updated_at_str: String = row
        .try_get("sl_updated_at")
        .unwrap_or_else(|_| Utc::now().to_rfc3339());

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let listing = StoreListing {
        id: Uuid::parse_str(&sl_id_str)
            .map_err(|e| AppError::internal(format!("Invalid listing UUID: {e}")))?,
        coach_id: coach.id,
        tenant_id: coach.tenant_id.clone(),
        publish_status: PublishStatus::parse(&publish_status_str),
        published_at: published_at_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        review_submitted_at: review_submitted_at_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        review_decision_at: review_decision_at_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        review_decision_by,
        rejection_reason,
        install_count: install_count as u32,
        icon_url,
        author_id,
        created_at: DateTime::parse_from_rfc3339(&sl_created_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
        updated_at: DateTime::parse_from_rfc3339(&sl_updated_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
    };

    Ok(CoachWithListing { coach, listing })
}
