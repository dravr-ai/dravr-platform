// ABOUTME: PostgreSQL implementation of the notification repository trait
// ABOUTME: Uses native PG types (UUID, TIMESTAMPTZ, BOOLEAN, JSONB) for optimal performance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, NaiveTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::notifications::{
    CategoryAnalytics, CreateNotificationParams, CreateScheduledNotificationParams, DevicePlatform,
    DeviceToken, Notification, NotificationAction, NotificationAnalytics, NotificationCategory,
    NotificationPreference, ScheduledNotification, UpdateScheduledNotificationParams,
    UpsertNotificationPreferenceParams,
};
use pierre_core::models::TenantId;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use tracing::warn;
use uuid::Uuid;

use super::NotificationRepository;

// ════════════════════════════════════════════════════════════════
// Helper functions for row parsing (native PG types)
// ════════════════════════════════════════════════════════════════

/// Parse a device token row from `PostgreSQL` (native `UUID`, `TIMESTAMPTZ`, `BOOLEAN`)
fn parse_device_token_row(row: &PgRow) -> AppResult<DeviceToken> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("Missing id: {e}")))?;
    let user_id: Uuid = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("Missing user_id: {e}")))?;
    let tenant_id: Uuid = row
        .try_get("tenant_id")
        .map_err(|e| AppError::database(format!("Missing tenant_id: {e}")))?;
    let expo_push_token: String = row
        .try_get("expo_push_token")
        .map_err(|e| AppError::database(format!("Missing expo_push_token: {e}")))?;
    let platform_str: String = row
        .try_get("platform")
        .map_err(|e| AppError::database(format!("Missing platform: {e}")))?;
    let device_name: Option<String> = row.try_get("device_name").unwrap_or(None);
    let active: bool = row.try_get("active").unwrap_or(true);
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| AppError::database(format!("Missing created_at: {e}")))?;
    let updated_at: DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|e| AppError::database(format!("Missing updated_at: {e}")))?;

    Ok(DeviceToken {
        id,
        user_id,
        tenant_id: TenantId(tenant_id),
        expo_push_token,
        platform: DevicePlatform::from_str_opt(&platform_str)
            .ok_or_else(|| AppError::database(format!("Invalid platform: {platform_str}")))?,
        device_name,
        active,
        created_at,
        updated_at,
    })
}

/// Parse a notification preference row from `PostgreSQL`
fn parse_preference_row(row: &PgRow) -> AppResult<NotificationPreference> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("Missing id: {e}")))?;
    let user_id: Uuid = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("Missing user_id: {e}")))?;
    let tenant_id: Uuid = row
        .try_get("tenant_id")
        .map_err(|e| AppError::database(format!("Missing tenant_id: {e}")))?;
    let category_str: String = row
        .try_get("category")
        .map_err(|e| AppError::database(format!("Missing category: {e}")))?;
    let enabled: bool = row.try_get("enabled").unwrap_or(true);
    let sub_prefs_json: Option<serde_json::Value> = row.try_get("sub_preferences").unwrap_or(None);
    let quiet_start: Option<NaiveTime> = row.try_get("quiet_hours_start").unwrap_or(None);
    let quiet_end: Option<NaiveTime> = row.try_get("quiet_hours_end").unwrap_or(None);
    let timezone: Option<String> = row.try_get("timezone").unwrap_or(None);
    let max_per_day: Option<i32> = row.try_get("max_per_day").unwrap_or(None);
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| AppError::database(format!("Missing created_at: {e}")))?;
    let updated_at: DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|e| AppError::database(format!("Missing updated_at: {e}")))?;

    Ok(NotificationPreference {
        id,
        user_id,
        tenant_id: TenantId(tenant_id),
        category: NotificationCategory::from_str_opt(&category_str)
            .ok_or_else(|| AppError::database(format!("Invalid category: {category_str}")))?,
        enabled,
        sub_preferences: sub_prefs_json,
        quiet_hours_start: quiet_start,
        quiet_hours_end: quiet_end,
        timezone,
        max_per_day,
        created_at,
        updated_at,
    })
}

/// Parse a notification row from `PostgreSQL`
fn parse_notification_row(row: &PgRow) -> AppResult<Notification> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("Missing id: {e}")))?;
    let user_id: Uuid = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("Missing user_id: {e}")))?;
    let tenant_id: Uuid = row
        .try_get("tenant_id")
        .map_err(|e| AppError::database(format!("Missing tenant_id: {e}")))?;
    let category_str: String = row
        .try_get("category")
        .map_err(|e| AppError::database(format!("Missing category: {e}")))?;
    let notification_type: String = row
        .try_get("notification_type")
        .map_err(|e| AppError::database(format!("Missing notification_type: {e}")))?;
    let title: String = row
        .try_get("title")
        .map_err(|e| AppError::database(format!("Missing title: {e}")))?;
    let body: String = row
        .try_get("body")
        .map_err(|e| AppError::database(format!("Missing body: {e}")))?;
    let data: Option<serde_json::Value> = row.try_get("data").unwrap_or(None);
    let image_url: Option<String> = row.try_get("image_url").unwrap_or(None);
    let actions_json: Option<serde_json::Value> = row.try_get("actions").unwrap_or(None);
    let read_at: Option<DateTime<Utc>> = row.try_get("read_at").unwrap_or(None);
    let delivered_at: Option<DateTime<Utc>> = row.try_get("delivered_at").unwrap_or(None);
    let opened_at: Option<DateTime<Utc>> = row.try_get("opened_at").unwrap_or(None);
    let dismissed_at: Option<DateTime<Utc>> = row.try_get("dismissed_at").unwrap_or(None);
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| AppError::database(format!("Missing created_at: {e}")))?;

    let actions = actions_json.and_then(|v| {
        serde_json::from_value::<Vec<NotificationAction>>(v)
            .map_err(|e| {
                warn!(error = %e, "Malformed actions JSON in notification row, treating as None");
            })
            .ok()
    });

    Ok(Notification {
        id,
        user_id,
        tenant_id: TenantId(tenant_id),
        category: NotificationCategory::from_str_opt(&category_str)
            .ok_or_else(|| AppError::database(format!("Invalid category: {category_str}")))?,
        notification_type,
        title,
        body,
        data,
        image_url,
        read_at,
        delivered_at,
        opened_at,
        dismissed_at,
        actions,
        created_at,
    })
}

/// Parse a scheduled notification row from `PostgreSQL`
fn parse_scheduled_notification_row(row: &PgRow) -> AppResult<ScheduledNotification> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("Missing id: {e}")))?;
    let user_id: Uuid = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("Missing user_id: {e}")))?;
    let tenant_id: Uuid = row
        .try_get("tenant_id")
        .map_err(|e| AppError::database(format!("Missing tenant_id: {e}")))?;
    let notification_type: String = row
        .try_get("notification_type")
        .map_err(|e| AppError::database(format!("Missing notification_type: {e}")))?;
    let schedule_cron: String = row
        .try_get("schedule_cron")
        .map_err(|e| AppError::database(format!("Missing schedule_cron: {e}")))?;
    let timezone: String = row.try_get("timezone").unwrap_or_else(|_| "UTC".to_owned());
    let enabled: bool = row.try_get("enabled").unwrap_or(true);
    let next_fire_at: Option<DateTime<Utc>> = row.try_get("next_fire_at").unwrap_or(None);
    let last_fired_at: Option<DateTime<Utc>> = row.try_get("last_fired_at").unwrap_or(None);
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| AppError::database(format!("Missing created_at: {e}")))?;

    Ok(ScheduledNotification {
        id,
        user_id,
        tenant_id: TenantId(tenant_id),
        notification_type,
        schedule_cron,
        timezone,
        next_fire_at,
        enabled,
        last_fired_at,
        created_at,
    })
}

// ════════════════════════════════════════════════════════════════
// PostgreSQL Repository Implementation
// ════════════════════════════════════════════════════════════════

/// `PostgreSQL`-backed notification repository using native PG types
pub struct PostgresNotificationRepository {
    pool: PgPool,
}

impl PostgresNotificationRepository {
    /// Create a new `PostgreSQL` notification repository from a connection pool
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl NotificationRepository for PostgresNotificationRepository {
    async fn upsert_device_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        expo_push_token: &str,
        platform: &str,
        device_name: Option<&str>,
    ) -> AppResult<DeviceToken> {
        let id = Uuid::new_v4();
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO device_tokens (id, user_id, tenant_id, expo_push_token, platform, device_name, active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, true, $7, $8)
            ON CONFLICT(user_id, tenant_id, expo_push_token) DO UPDATE SET
                platform = EXCLUDED.platform,
                device_name = EXCLUDED.device_name,
                active = true,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(expo_push_token)
        .bind(platform)
        .bind(device_name)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert device token: {e}")))?;

        let row = sqlx::query(
            r"
            SELECT * FROM device_tokens
            WHERE user_id = $1 AND tenant_id = $2 AND expo_push_token = $3
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(expo_push_token)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to fetch device token after upsert: {e}"))
        })?;

        parse_device_token_row(&row)
    }

    async fn get_device_tokens(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<DeviceToken>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM device_tokens
            WHERE user_id = $1 AND tenant_id = $2 AND active = true
            ORDER BY updated_at DESC
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get device tokens: {e}")))?;

        rows.iter().map(parse_device_token_row).collect()
    }

    async fn deactivate_device_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        token_id: Uuid,
    ) -> AppResult<bool> {
        let now = Utc::now();
        let result = sqlx::query(
            r"
            UPDATE device_tokens
            SET active = false, updated_at = $1
            WHERE id = $2 AND user_id = $3 AND tenant_id = $4
            ",
        )
        .bind(now)
        .bind(token_id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to deactivate device token: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_notification_preferences(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<NotificationPreference>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM notification_preferences
            WHERE user_id = $1 AND tenant_id = $2
            ORDER BY category
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get notification preferences: {e}")))?;

        rows.iter().map(parse_preference_row).collect()
    }

    async fn upsert_notification_preference(
        &self,
        params: &UpsertNotificationPreferenceParams,
    ) -> AppResult<NotificationPreference> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let sub_prefs_json = params.sub_preferences.clone();

        let quiet_start = params
            .quiet_hours_start
            .as_deref()
            .and_then(|s| NaiveTime::parse_from_str(s, "%H:%M").ok());
        let quiet_end = params
            .quiet_hours_end
            .as_deref()
            .and_then(|s| NaiveTime::parse_from_str(s, "%H:%M").ok());

        sqlx::query(
            r"
            INSERT INTO notification_preferences
                (id, user_id, tenant_id, category, enabled, sub_preferences,
                 quiet_hours_start, quiet_hours_end, timezone, max_per_day, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT(user_id, tenant_id, category) DO UPDATE SET
                enabled = EXCLUDED.enabled,
                sub_preferences = EXCLUDED.sub_preferences,
                quiet_hours_start = EXCLUDED.quiet_hours_start,
                quiet_hours_end = EXCLUDED.quiet_hours_end,
                timezone = EXCLUDED.timezone,
                max_per_day = EXCLUDED.max_per_day,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(id)
        .bind(params.user_id.to_string())
        .bind(params.tenant_id.to_string())
        .bind(&params.category)
        .bind(params.enabled)
        .bind(&sub_prefs_json)
        .bind(quiet_start)
        .bind(quiet_end)
        .bind(params.timezone.as_deref())
        .bind(params.max_per_day)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to upsert notification preference: {e}"))
        })?;

        let row = sqlx::query(
            r"
            SELECT * FROM notification_preferences
            WHERE user_id = $1 AND tenant_id = $2 AND category = $3
            ",
        )
        .bind(params.user_id.to_string())
        .bind(params.tenant_id.to_string())
        .bind(&params.category)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch preference after upsert: {e}")))?;

        parse_preference_row(&row)
    }

    async fn create_notification(
        &self,
        params: &CreateNotificationParams,
    ) -> AppResult<Notification> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let actions_json = params
            .actions
            .as_ref()
            .and_then(|a| serde_json::to_value(a).ok());

        sqlx::query(
            r"
            INSERT INTO notifications
                (id, user_id, tenant_id, category, notification_type, title, body, data, image_url, actions, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ",
        )
        .bind(id)
        .bind(params.user_id.to_string())
        .bind(params.tenant_id.to_string())
        .bind(params.category.as_str())
        .bind(&params.notification_type)
        .bind(&params.title)
        .bind(&params.body)
        .bind(&params.data)
        .bind(&params.image_url)
        .bind(&actions_json)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create notification: {e}")))?;

        let row = sqlx::query("SELECT * FROM notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to fetch notification after create: {e}"))
            })?;

        parse_notification_row(&row)
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    async fn list_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        limit: u32,
        offset: u32,
        category: Option<&str>,
        unread_only: bool,
    ) -> AppResult<(Vec<Notification>, i64, i64)> {
        let clamped_limit = i64::from(limit.clamp(1, 100));
        let clamped_offset = i64::from(offset);

        let mut conditions = String::from("user_id = $1 AND tenant_id = $2");
        if category.is_some() {
            conditions.push_str(" AND category = $3");
        }
        if unread_only {
            conditions.push_str(" AND read_at IS NULL");
        }

        // Get total count
        let count_query = format!("SELECT COUNT(*) as cnt FROM notifications WHERE {conditions}");
        let mut count_q = sqlx::query(&count_query)
            .bind(user_id.to_string())
            .bind(tenant_id.to_string());
        if let Some(cat) = category {
            count_q = count_q.bind(cat);
        }
        let count_row = count_q
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to count notifications: {e}")))?;
        let total: i64 = count_row.try_get::<i64, _>("cnt").unwrap_or(0);

        // Get unread count
        let unread_row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM notifications WHERE user_id = $1 AND tenant_id = $2 AND read_at IS NULL",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count unread notifications: {e}")))?;
        let unread_count: i64 = unread_row.try_get::<i64, _>("cnt").unwrap_or(0);

        // Get paginated results — use numbered bind params based on whether category is present
        let (data_query, param_offset) = if category.is_some() {
            (
                format!(
                    "SELECT * FROM notifications WHERE {conditions} ORDER BY created_at DESC LIMIT $4 OFFSET $5"
                ),
                4,
            )
        } else {
            (
                format!(
                    "SELECT * FROM notifications WHERE {conditions} ORDER BY created_at DESC LIMIT $3 OFFSET $4"
                ),
                3,
            )
        };
        let _ = param_offset; // used in format string via numbered placeholders

        let mut data_q = sqlx::query(&data_query)
            .bind(user_id.to_string())
            .bind(tenant_id.to_string());
        if let Some(cat) = category {
            data_q = data_q.bind(cat);
        }
        data_q = data_q.bind(clamped_limit).bind(clamped_offset);

        let rows = data_q
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to list notifications: {e}")))?;

        let notifications: Vec<Notification> = rows
            .iter()
            .map(parse_notification_row)
            .collect::<AppResult<Vec<_>>>()?;

        Ok((notifications, total, unread_count))
    }

    async fn mark_notification_read(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        let now = Utc::now();
        let result = sqlx::query(
            r"
            UPDATE notifications
            SET read_at = $1
            WHERE id = $2 AND user_id = $3 AND tenant_id = $4 AND read_at IS NULL
            ",
        )
        .bind(now)
        .bind(notification_id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark notification read: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn mark_all_notifications_read(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<u64> {
        let now = Utc::now();
        let result = sqlx::query(
            r"
            UPDATE notifications
            SET read_at = $1
            WHERE user_id = $2 AND tenant_id = $3 AND read_at IS NULL
            ",
        )
        .bind(now)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark all notifications read: {e}")))?;

        Ok(result.rows_affected())
    }

    async fn delete_notification(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM notifications
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(notification_id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete notification: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_unread_count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM notifications WHERE user_id = $1 AND tenant_id = $2 AND read_at IS NULL",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count unread notifications: {e}")))?;

        Ok(row.try_get::<i64, _>("cnt").unwrap_or(0))
    }

    async fn count_notifications_since(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        category: &str,
        since: DateTime<Utc>,
    ) -> AppResult<i64> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as cnt FROM notifications
            WHERE user_id = $1 AND tenant_id = $2 AND category = $3 AND created_at >= $4
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(category)
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to count notifications since timestamp: {e}"
            ))
        })?;

        Ok(row.try_get::<i64, _>("cnt").unwrap_or(0))
    }

    async fn mark_notification_opened(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        let now = Utc::now();
        let result = sqlx::query(
            r"
            UPDATE notifications
            SET opened_at = $1
            WHERE id = $2 AND user_id = $3 AND tenant_id = $4 AND opened_at IS NULL
            ",
        )
        .bind(now)
        .bind(notification_id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark notification opened: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn mark_notification_dismissed(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        let now = Utc::now();
        let result = sqlx::query(
            r"
            UPDATE notifications
            SET dismissed_at = $1
            WHERE id = $2 AND user_id = $3 AND tenant_id = $4 AND dismissed_at IS NULL
            ",
        )
        .bind(now)
        .bind(notification_id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark notification dismissed: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_notification_analytics(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        category: Option<&str>,
    ) -> AppResult<NotificationAnalytics> {
        let since_val = since.unwrap_or_else(|| Utc::now() - chrono::Duration::days(30));
        let until_val = until.unwrap_or_else(Utc::now);

        let category_clause = if category.is_some() {
            "AND category = $5"
        } else {
            ""
        };

        // Aggregate totals
        let totals_sql = format!(
            r"
            SELECT
                COUNT(*) as total_sent,
                COUNT(delivered_at) as total_delivered,
                COUNT(opened_at) as total_opened,
                COUNT(dismissed_at) as total_dismissed
            FROM notifications
            WHERE user_id = $1 AND tenant_id = $2 AND created_at >= $3 AND created_at <= $4
            {category_clause}
            "
        );

        let mut query = sqlx::query(&totals_sql)
            .bind(user_id.to_string())
            .bind(tenant_id.to_string())
            .bind(since_val)
            .bind(until_val);
        if let Some(cat) = category {
            query = query.bind(cat);
        }

        let totals = query.fetch_one(&self.pool).await.map_err(|e| {
            AppError::database(format!("Failed to get notification analytics: {e}"))
        })?;

        let total_sent = totals.try_get::<i64, _>("total_sent").unwrap_or(0);
        let total_delivered = totals.try_get::<i64, _>("total_delivered").unwrap_or(0);
        let total_opened = totals.try_get::<i64, _>("total_opened").unwrap_or(0);
        let total_dismissed = totals.try_get::<i64, _>("total_dismissed").unwrap_or(0);

        #[allow(clippy::cast_precision_loss)]
        let delivery_rate = if total_sent > 0 {
            total_delivered as f64 / total_sent as f64
        } else {
            0.0
        };
        #[allow(clippy::cast_precision_loss)]
        let open_rate = if total_delivered > 0 {
            total_opened as f64 / total_delivered as f64
        } else {
            0.0
        };
        #[allow(clippy::cast_precision_loss)]
        let dismiss_rate = if total_delivered > 0 {
            total_dismissed as f64 / total_delivered as f64
        } else {
            0.0
        };

        // Average time to open using EXTRACT(EPOCH FROM ...)
        let avg_sql = format!(
            r"
            SELECT AVG(EXTRACT(EPOCH FROM (opened_at - created_at))) as avg_seconds
            FROM notifications
            WHERE user_id = $1 AND tenant_id = $2 AND created_at >= $3 AND created_at <= $4
            AND opened_at IS NOT NULL
            {category_clause}
            "
        );

        let mut avg_query = sqlx::query(&avg_sql)
            .bind(user_id.to_string())
            .bind(tenant_id.to_string())
            .bind(since_val)
            .bind(until_val);
        if let Some(cat) = category {
            avg_query = avg_query.bind(cat);
        }

        let avg_row = avg_query
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get avg time to open: {e}")))?;

        let avg_time_to_open_seconds: Option<f64> = avg_row.try_get::<f64, _>("avg_seconds").ok();

        // Category breakdown
        let breakdown_sql = format!(
            r"
            SELECT
                category,
                COUNT(*) as sent,
                COUNT(opened_at) as opened,
                COUNT(dismissed_at) as dismissed
            FROM notifications
            WHERE user_id = $1 AND tenant_id = $2 AND created_at >= $3 AND created_at <= $4
            {category_clause}
            GROUP BY category
            ORDER BY sent DESC
            "
        );

        let mut breakdown_query = sqlx::query(&breakdown_sql)
            .bind(user_id.to_string())
            .bind(tenant_id.to_string())
            .bind(since_val)
            .bind(until_val);
        if let Some(cat) = category {
            breakdown_query = breakdown_query.bind(cat);
        }

        let breakdown_rows = breakdown_query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get category breakdown: {e}")))?;

        let category_breakdown = breakdown_rows
            .iter()
            .map(|row| CategoryAnalytics {
                category: row.try_get::<String, _>("category").unwrap_or_default(),
                sent: row.try_get::<i64, _>("sent").unwrap_or(0),
                opened: row.try_get::<i64, _>("opened").unwrap_or(0),
                dismissed: row.try_get::<i64, _>("dismissed").unwrap_or(0),
            })
            .collect();

        Ok(NotificationAnalytics {
            total_sent,
            delivery_rate,
            open_rate,
            dismiss_rate,
            category_breakdown,
            avg_time_to_open_seconds,
        })
    }

    async fn get_scheduled_notification_by_id(
        &self,
        id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<ScheduledNotification>> {
        let row = sqlx::query(
            r"
            SELECT * FROM scheduled_notifications
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to get scheduled notification by id: {e}"))
        })?;

        row.as_ref()
            .map(parse_scheduled_notification_row)
            .transpose()
    }

    async fn count_scheduled_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM scheduled_notifications WHERE user_id = $1 AND tenant_id = $2",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to count scheduled notifications: {e}"))
        })?;

        Ok(row.try_get::<i64, _>("cnt").unwrap_or(0))
    }

    async fn create_scheduled_notification(
        &self,
        params: &CreateScheduledNotificationParams,
    ) -> AppResult<ScheduledNotification> {
        let id = Uuid::new_v4();
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO scheduled_notifications (id, user_id, tenant_id, notification_type,
                schedule_cron, timezone, next_fire_at, enabled, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, true, $8)
            ",
        )
        .bind(id)
        .bind(params.user_id.to_string())
        .bind(params.tenant_id.to_string())
        .bind(&params.notification_type)
        .bind(&params.schedule_cron)
        .bind(&params.timezone)
        .bind(params.next_fire_at)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create scheduled notification: {e}")))?;

        Ok(ScheduledNotification {
            id,
            user_id: params.user_id,
            tenant_id: params.tenant_id,
            notification_type: params.notification_type.clone(),
            schedule_cron: params.schedule_cron.clone(),
            timezone: params.timezone.clone(),
            next_fire_at: Some(params.next_fire_at),
            enabled: true,
            last_fired_at: None,
            created_at: now,
        })
    }

    async fn list_scheduled_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ScheduledNotification>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM scheduled_notifications
            WHERE user_id = $1 AND tenant_id = $2
            ORDER BY created_at DESC
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list scheduled notifications: {e}")))?;

        rows.iter().map(parse_scheduled_notification_row).collect()
    }

    async fn delete_scheduled_notification(
        &self,
        id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM scheduled_notifications
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(id)
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete scheduled notification: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn update_scheduled_notification(
        &self,
        params: &UpdateScheduledNotificationParams,
    ) -> AppResult<bool> {
        let mut set_parts: Vec<String> = Vec::new();
        // PostgreSQL numbered params start at $1 for the dynamic SET clauses
        // The WHERE clause params come after
        let mut param_idx: usize = 1;

        if params.enabled.is_some() {
            set_parts.push(format!("enabled = ${param_idx}"));
            param_idx += 1;
        }
        if params.schedule_cron.is_some() {
            set_parts.push(format!("schedule_cron = ${param_idx}"));
            param_idx += 1;
        }
        if params.timezone.is_some() {
            set_parts.push(format!("timezone = ${param_idx}"));
            param_idx += 1;
        }
        if params.next_fire_at.is_some() {
            set_parts.push(format!("next_fire_at = ${param_idx}"));
            param_idx += 1;
        }

        if set_parts.is_empty() {
            return Ok(false);
        }

        let id_idx = param_idx;
        let user_idx = param_idx + 1;
        let tenant_idx = param_idx + 2;

        let sql = format!(
            "UPDATE scheduled_notifications SET {} WHERE id = ${id_idx} AND user_id = ${user_idx} AND tenant_id = ${tenant_idx}",
            set_parts.join(", ")
        );

        let mut query = sqlx::query(&sql);
        if let Some(e) = params.enabled {
            query = query.bind(e);
        }
        if let Some(ref cron) = params.schedule_cron {
            query = query.bind(cron);
        }
        if let Some(ref tz) = params.timezone {
            query = query.bind(tz);
        }
        if let Some(nf) = params.next_fire_at {
            query = query.bind(nf);
        }
        query = query
            .bind(params.id)
            .bind(params.user_id.to_string())
            .bind(params.tenant_id.to_string());

        let result = query.execute(&self.pool).await.map_err(|e| {
            AppError::database(format!("Failed to update scheduled notification: {e}"))
        })?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_due_scheduled_notifications(
        &self,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<ScheduledNotification>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM scheduled_notifications
            WHERE next_fire_at <= $1 AND enabled = true
            ORDER BY next_fire_at ASC
            ",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to get due scheduled notifications: {e}"))
        })?;

        rows.iter().map(parse_scheduled_notification_row).collect()
    }

    async fn update_scheduled_notification_fired(
        &self,
        id: Uuid,
        last_fired_at: DateTime<Utc>,
        next_fire_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            UPDATE scheduled_notifications
            SET last_fired_at = $1, next_fire_at = $2
            WHERE id = $3
            ",
        )
        .bind(last_fired_at)
        .bind(next_fire_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to update scheduled notification fired: {e}"
            ))
        })?;

        Ok(result.rows_affected() > 0)
    }
}
