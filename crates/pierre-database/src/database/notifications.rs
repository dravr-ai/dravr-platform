// ABOUTME: SQLite implementation of push notification repository operations
// ABOUTME: Device token management, notification preferences, and notification CRUD with tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, NaiveTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::notifications::{
    CategoryAnalytics, CreateNotificationParams, CreateScheduledNotificationParams, DevicePlatform,
    DeviceToken, Notification, NotificationAnalytics, NotificationCategory, NotificationPreference,
    ScheduledNotification, UpdateScheduledNotificationParams, UpsertNotificationPreferenceParams,
};
use pierre_core::models::TenantId;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use super::Database;

// ════════════════════════════════════════════════════════════════
// Helper functions for row parsing
// ════════════════════════════════════════════════════════════════

/// Parse a device token row from `SQLite`
fn parse_device_token_row(row: &SqliteRow) -> AppResult<DeviceToken> {
    let id_str: String = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("Missing id: {e}")))?;
    let user_id_str: String = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("Missing user_id: {e}")))?;
    let tenant_id_str: String = row
        .try_get("tenant_id")
        .map_err(|e| AppError::database(format!("Missing tenant_id: {e}")))?;
    let expo_push_token: String = row
        .try_get("expo_push_token")
        .map_err(|e| AppError::database(format!("Missing expo_push_token: {e}")))?;
    let platform_str: String = row
        .try_get("platform")
        .map_err(|e| AppError::database(format!("Missing platform: {e}")))?;
    let device_name: Option<String> = row.try_get("device_name").unwrap_or(None);
    let active: i32 = row.try_get("active").unwrap_or(1);
    let created_at_str: String = row
        .try_get("created_at")
        .map_err(|e| AppError::database(format!("Missing created_at: {e}")))?;
    let updated_at_str: String = row
        .try_get("updated_at")
        .map_err(|e| AppError::database(format!("Missing updated_at: {e}")))?;

    Ok(DeviceToken {
        id: id_str.parse()?,
        user_id: user_id_str.parse()?,
        tenant_id: TenantId(tenant_id_str.parse()?),
        expo_push_token,
        platform: DevicePlatform::from_str_opt(&platform_str)
            .ok_or_else(|| AppError::database(format!("Invalid platform: {platform_str}")))?,
        device_name,
        active: active != 0,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
    })
}

/// Parse a notification preference row from `SQLite`
fn parse_preference_row(row: &SqliteRow) -> AppResult<NotificationPreference> {
    let id_str: String = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("Missing id: {e}")))?;
    let user_id_str: String = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("Missing user_id: {e}")))?;
    let tenant_id_str: String = row
        .try_get("tenant_id")
        .map_err(|e| AppError::database(format!("Missing tenant_id: {e}")))?;
    let category_str: String = row
        .try_get("category")
        .map_err(|e| AppError::database(format!("Missing category: {e}")))?;
    let enabled: i32 = row.try_get("enabled").unwrap_or(1);
    let sub_prefs_str: Option<String> = row.try_get("sub_preferences").unwrap_or(None);
    let quiet_start: Option<String> = row.try_get("quiet_hours_start").unwrap_or(None);
    let quiet_end: Option<String> = row.try_get("quiet_hours_end").unwrap_or(None);
    let timezone: Option<String> = row.try_get("timezone").unwrap_or(None);
    let max_per_day: Option<i32> = row.try_get("max_per_day").unwrap_or(None);
    let created_at_str: String = row
        .try_get("created_at")
        .map_err(|e| AppError::database(format!("Missing created_at: {e}")))?;
    let updated_at_str: String = row
        .try_get("updated_at")
        .map_err(|e| AppError::database(format!("Missing updated_at: {e}")))?;

    let sub_preferences = sub_prefs_str.and_then(|s| serde_json::from_str(&s).ok());

    let quiet_hours_start = quiet_start.and_then(|s| NaiveTime::parse_from_str(&s, "%H:%M").ok());
    let quiet_hours_end = quiet_end.and_then(|s| NaiveTime::parse_from_str(&s, "%H:%M").ok());

    Ok(NotificationPreference {
        id: id_str.parse()?,
        user_id: user_id_str.parse()?,
        tenant_id: TenantId(tenant_id_str.parse()?),
        category: NotificationCategory::from_str_opt(&category_str)
            .ok_or_else(|| AppError::database(format!("Invalid category: {category_str}")))?,
        enabled: enabled != 0,
        sub_preferences,
        quiet_hours_start,
        quiet_hours_end,
        timezone,
        max_per_day,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
    })
}

/// Parse a notification row from `SQLite`
fn parse_notification_row(row: &SqliteRow) -> AppResult<Notification> {
    let id_str: String = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("Missing id: {e}")))?;
    let user_id_str: String = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("Missing user_id: {e}")))?;
    let tenant_id_str: String = row
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
    let data_str: Option<String> = row.try_get("data").unwrap_or(None);
    let image_url: Option<String> = row.try_get("image_url").unwrap_or(None);
    let read_at_str: Option<String> = row.try_get("read_at").unwrap_or(None);
    let delivered_at_str: Option<String> = row.try_get("delivered_at").unwrap_or(None);
    let opened_at_str: Option<String> = row.try_get("opened_at").unwrap_or(None);
    let dismissed_at_str: Option<String> = row.try_get("dismissed_at").unwrap_or(None);
    let created_at_str: String = row
        .try_get("created_at")
        .map_err(|e| AppError::database(format!("Missing created_at: {e}")))?;

    let parse_dt = |s: &str| -> Option<chrono::DateTime<Utc>> {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .ok()
    };

    Ok(Notification {
        id: id_str.parse()?,
        user_id: user_id_str.parse()?,
        tenant_id: TenantId(tenant_id_str.parse()?),
        category: NotificationCategory::from_str_opt(&category_str)
            .ok_or_else(|| AppError::database(format!("Invalid category: {category_str}")))?,
        notification_type,
        title,
        body,
        data: data_str.and_then(|s| serde_json::from_str(&s).ok()),
        image_url,
        read_at: read_at_str.as_deref().and_then(parse_dt),
        delivered_at: delivered_at_str.as_deref().and_then(parse_dt),
        opened_at: opened_at_str.as_deref().and_then(parse_dt),
        dismissed_at: dismissed_at_str.as_deref().and_then(parse_dt),
        created_at: parse_dt(&created_at_str).unwrap_or_else(Utc::now),
    })
}

// ════════════════════════════════════════════════════════════════
// Implementation methods (called by trait dispatch in factory)
// ════════════════════════════════════════════════════════════════

impl Database {
    /// Upsert a device token for push notifications
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn upsert_device_token_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        expo_push_token: &str,
        platform: &str,
        device_name: Option<&str>,
    ) -> AppResult<DeviceToken> {
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        let user_id_str = user_id.to_string();
        let tenant_id_str = tenant_id.0.to_string();

        sqlx::query(
            r"
            INSERT INTO device_tokens (id, user_id, tenant_id, expo_push_token, platform, device_name, active, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?)
            ON CONFLICT(user_id, tenant_id, expo_push_token) DO UPDATE SET
                platform = excluded.platform,
                device_name = excluded.device_name,
                active = 1,
                updated_at = excluded.updated_at
            ",
        )
        .bind(id.to_string())
        .bind(&user_id_str)
        .bind(&tenant_id_str)
        .bind(expo_push_token)
        .bind(platform)
        .bind(device_name)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert device token: {e}")))?;

        // Fetch the upserted row (could be new or updated)
        let row = sqlx::query(
            r"
            SELECT * FROM device_tokens
            WHERE user_id = ? AND tenant_id = ? AND expo_push_token = ?
            ",
        )
        .bind(&user_id_str)
        .bind(&tenant_id_str)
        .bind(expo_push_token)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to fetch device token after upsert: {e}"))
        })?;

        parse_device_token_row(&row)
    }

    /// Get all active device tokens for a user
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn get_device_tokens_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<DeviceToken>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM device_tokens
            WHERE user_id = ? AND tenant_id = ? AND active = 1
            ORDER BY updated_at DESC
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get device tokens: {e}")))?;

        rows.iter().map(parse_device_token_row).collect()
    }

    /// Deactivate a device token
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn deactivate_device_token_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        token_id: Uuid,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            r"
            UPDATE device_tokens
            SET active = 0, updated_at = ?
            WHERE id = ? AND user_id = ? AND tenant_id = ?
            ",
        )
        .bind(&now)
        .bind(token_id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to deactivate device token: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Get notification preferences for a user
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn get_notification_preferences_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<NotificationPreference>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM notification_preferences
            WHERE user_id = ? AND tenant_id = ?
            ORDER BY category
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get notification preferences: {e}")))?;

        rows.iter().map(parse_preference_row).collect()
    }

    /// Upsert a notification preference
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn upsert_notification_preference_impl(
        &self,
        params: &UpsertNotificationPreferenceParams,
    ) -> AppResult<NotificationPreference> {
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        let user_id_str = params.user_id.to_string();
        let tenant_id_str = params.tenant_id.0.to_string();
        let enabled_int: i32 = i32::from(params.enabled);
        let sub_prefs_str = params.sub_preferences.as_ref().map(ToString::to_string);

        sqlx::query(
            r"
            INSERT INTO notification_preferences
                (id, user_id, tenant_id, category, enabled, sub_preferences,
                 quiet_hours_start, quiet_hours_end, timezone, max_per_day, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(user_id, tenant_id, category) DO UPDATE SET
                enabled = excluded.enabled,
                sub_preferences = excluded.sub_preferences,
                quiet_hours_start = excluded.quiet_hours_start,
                quiet_hours_end = excluded.quiet_hours_end,
                timezone = excluded.timezone,
                max_per_day = excluded.max_per_day,
                updated_at = excluded.updated_at
            ",
        )
        .bind(id.to_string())
        .bind(&user_id_str)
        .bind(&tenant_id_str)
        .bind(&params.category)
        .bind(enabled_int)
        .bind(&sub_prefs_str)
        .bind(params.quiet_hours_start.as_deref())
        .bind(params.quiet_hours_end.as_deref())
        .bind(params.timezone.as_deref())
        .bind(params.max_per_day)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to upsert notification preference: {e}"))
        })?;

        let row = sqlx::query(
            r"
            SELECT * FROM notification_preferences
            WHERE user_id = ? AND tenant_id = ? AND category = ?
            ",
        )
        .bind(&user_id_str)
        .bind(&tenant_id_str)
        .bind(&params.category)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch preference after upsert: {e}")))?;

        parse_preference_row(&row)
    }

    /// Create a notification record
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn create_notification_impl(
        &self,
        params: &CreateNotificationParams,
    ) -> AppResult<Notification> {
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        let data_str = params.data.as_ref().map(ToString::to_string);

        sqlx::query(
            r"
            INSERT INTO notifications
                (id, user_id, tenant_id, category, notification_type, title, body, data, image_url, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(id.to_string())
        .bind(params.user_id.to_string())
        .bind(params.tenant_id.0.to_string())
        .bind(params.category.as_str())
        .bind(&params.notification_type)
        .bind(&params.title)
        .bind(&params.body)
        .bind(&data_str)
        .bind(&params.image_url)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create notification: {e}")))?;

        let row = sqlx::query("SELECT * FROM notifications WHERE id = ?")
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to fetch notification after create: {e}"))
            })?;

        parse_notification_row(&row)
    }

    /// List notifications with filtering and pagination
    ///
    /// # Errors
    /// Returns an error if the database query fails
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    pub async fn list_notifications_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        limit: u32,
        offset: u32,
        category: Option<&str>,
        unread_only: bool,
    ) -> AppResult<(Vec<Notification>, i64, i64)> {
        let user_id_str = user_id.to_string();
        let tenant_id_str = tenant_id.0.to_string();
        let clamped_limit = limit.clamp(1, 100) as i32;
        let clamped_offset = offset as i32;

        // Build dynamic query parts
        let mut conditions = String::from("user_id = ? AND tenant_id = ?");
        if category.is_some() {
            conditions.push_str(" AND category = ?");
        }
        if unread_only {
            conditions.push_str(" AND read_at IS NULL");
        }

        // Get total count
        let count_query = format!("SELECT COUNT(*) as cnt FROM notifications WHERE {conditions}");
        let mut count_q = sqlx::query(&count_query)
            .bind(&user_id_str)
            .bind(&tenant_id_str);
        if let Some(cat) = category {
            count_q = count_q.bind(cat);
        }
        let count_row = count_q
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to count notifications: {e}")))?;
        let total: i64 = count_row.try_get::<i64, _>("cnt").unwrap_or(0);

        // Get unread count (always for full user+tenant scope)
        let unread_row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM notifications WHERE user_id = ? AND tenant_id = ? AND read_at IS NULL",
        )
        .bind(&user_id_str)
        .bind(&tenant_id_str)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count unread notifications: {e}")))?;
        let unread_count: i64 = unread_row.try_get::<i64, _>("cnt").unwrap_or(0);

        // Get paginated results
        let data_query = format!(
            "SELECT * FROM notifications WHERE {conditions} ORDER BY created_at DESC LIMIT ? OFFSET ?"
        );
        let mut data_q = sqlx::query(&data_query)
            .bind(&user_id_str)
            .bind(&tenant_id_str);
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

    /// Mark a notification as read
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn mark_notification_read_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            r"
            UPDATE notifications
            SET read_at = ?
            WHERE id = ? AND user_id = ? AND tenant_id = ? AND read_at IS NULL
            ",
        )
        .bind(&now)
        .bind(notification_id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark notification read: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Mark all notifications as read
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn mark_all_notifications_read_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<u64> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            r"
            UPDATE notifications
            SET read_at = ?
            WHERE user_id = ? AND tenant_id = ? AND read_at IS NULL
            ",
        )
        .bind(&now)
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark all notifications read: {e}")))?;

        Ok(result.rows_affected())
    }

    /// Delete a notification
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn delete_notification_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM notifications
            WHERE id = ? AND user_id = ? AND tenant_id = ?
            ",
        )
        .bind(notification_id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete notification: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Get unread notification count
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn get_unread_count_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM notifications WHERE user_id = ? AND tenant_id = ? AND read_at IS NULL",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count unread notifications: {e}")))?;

        Ok(row.try_get::<i64, _>("cnt").unwrap_or(0))
    }

    /// Count notifications of a specific category created since a timestamp
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn count_notifications_since_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        category: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<i64> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as cnt FROM notifications
            WHERE user_id = ? AND tenant_id = ? AND category = ? AND created_at >= ?
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .bind(category)
        .bind(since.to_rfc3339())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to count notifications since timestamp: {e}"
            ))
        })?;

        Ok(row.try_get::<i64, _>("cnt").unwrap_or(0))
    }

    // ── Notification Analytics ──

    /// Mark a notification as opened (user tapped on it)
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn mark_notification_opened_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            r"
            UPDATE notifications
            SET opened_at = ?
            WHERE id = ? AND user_id = ? AND tenant_id = ? AND opened_at IS NULL
            ",
        )
        .bind(&now)
        .bind(notification_id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark notification opened: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Mark a notification as dismissed
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn mark_notification_dismissed_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            r"
            UPDATE notifications
            SET dismissed_at = ?
            WHERE id = ? AND user_id = ? AND tenant_id = ? AND dismissed_at IS NULL
            ",
        )
        .bind(&now)
        .bind(notification_id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to mark notification dismissed: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Get aggregated notification analytics for a tenant
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn get_notification_analytics_impl(
        &self,
        tenant_id: TenantId,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        category: Option<&str>,
    ) -> AppResult<NotificationAnalytics> {
        let since_str = since
            .unwrap_or_else(|| Utc::now() - chrono::Duration::days(30))
            .to_rfc3339();
        let until_str = until.unwrap_or_else(Utc::now).to_rfc3339();

        // Build category filter clause
        let category_clause = if category.is_some() {
            "AND category = ?"
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
            WHERE tenant_id = ? AND created_at >= ? AND created_at <= ?
            {category_clause}
            "
        );

        let mut query = sqlx::query(&totals_sql)
            .bind(tenant_id.0.to_string())
            .bind(&since_str)
            .bind(&until_str);
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

        // Average time to open (seconds between created_at and opened_at)
        let avg_sql = format!(
            r"
            SELECT AVG(
                CAST((julianday(opened_at) - julianday(created_at)) * 86400 AS REAL)
            ) as avg_seconds
            FROM notifications
            WHERE tenant_id = ? AND created_at >= ? AND created_at <= ?
            AND opened_at IS NOT NULL
            {category_clause}
            "
        );

        let mut avg_query = sqlx::query(&avg_sql)
            .bind(tenant_id.0.to_string())
            .bind(&since_str)
            .bind(&until_str);
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
            WHERE tenant_id = ? AND created_at >= ? AND created_at <= ?
            {category_clause}
            GROUP BY category
            ORDER BY sent DESC
            "
        );

        let mut breakdown_query = sqlx::query(&breakdown_sql)
            .bind(tenant_id.0.to_string())
            .bind(&since_str)
            .bind(&until_str);
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

    // ── Scheduled Notifications ──

    /// Parse a scheduled notification from a `SqliteRow`
    fn parse_scheduled_notification_row(row: &SqliteRow) -> AppResult<ScheduledNotification> {
        let id_str: String = row
            .try_get("id")
            .map_err(|e| AppError::database(format!("Missing id: {e}")))?;
        let user_id_str: String = row
            .try_get("user_id")
            .map_err(|e| AppError::database(format!("Missing user_id: {e}")))?;
        let tenant_id_str: String = row
            .try_get("tenant_id")
            .map_err(|e| AppError::database(format!("Missing tenant_id: {e}")))?;

        let id = Uuid::parse_str(&id_str)
            .map_err(|e| AppError::database(format!("Invalid id UUID: {e}")))?;
        let user_id = Uuid::parse_str(&user_id_str)
            .map_err(|e| AppError::database(format!("Invalid user_id UUID: {e}")))?;
        let tenant_uuid = Uuid::parse_str(&tenant_id_str)
            .map_err(|e| AppError::database(format!("Invalid tenant_id UUID: {e}")))?;

        let notification_type: String = row
            .try_get("notification_type")
            .map_err(|e| AppError::database(format!("Missing notification_type: {e}")))?;
        let schedule_cron: String = row
            .try_get("schedule_cron")
            .map_err(|e| AppError::database(format!("Missing schedule_cron: {e}")))?;
        let timezone: String = row.try_get("timezone").unwrap_or_else(|_| "UTC".to_owned());
        let enabled: bool = row.try_get::<bool, _>("enabled").unwrap_or(true);

        let next_fire_at = row
            .try_get::<String, _>("next_fire_at")
            .ok()
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let last_fired_at = row
            .try_get::<String, _>("last_fired_at")
            .ok()
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let created_at = row
            .try_get::<String, _>("created_at")
            .ok()
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map_or_else(Utc::now, |dt| dt.with_timezone(&Utc));

        Ok(ScheduledNotification {
            id,
            user_id,
            tenant_id: TenantId(tenant_uuid),
            notification_type,
            schedule_cron,
            timezone,
            next_fire_at,
            enabled,
            last_fired_at,
            created_at,
        })
    }

    /// Get a single scheduled notification by ID, scoped to user and tenant
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn get_scheduled_notification_by_id_impl(
        &self,
        id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<ScheduledNotification>> {
        let row = sqlx::query(
            r"
            SELECT * FROM scheduled_notifications
            WHERE id = ? AND user_id = ? AND tenant_id = ?
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to get scheduled notification by id: {e}"))
        })?;

        row.as_ref()
            .map(Self::parse_scheduled_notification_row)
            .transpose()
    }

    /// Count scheduled notifications for a user within a tenant
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn count_scheduled_notifications_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM scheduled_notifications WHERE user_id = ? AND tenant_id = ?",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to count scheduled notifications: {e}"))
        })?;

        Ok(row.try_get::<i64, _>("cnt").unwrap_or(0))
    }

    /// Create a new scheduled notification
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn create_scheduled_notification_impl(
        &self,
        params: &CreateScheduledNotificationParams,
    ) -> AppResult<ScheduledNotification> {
        let id = Uuid::new_v4();
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO scheduled_notifications (id, user_id, tenant_id, notification_type,
                schedule_cron, timezone, next_fire_at, enabled, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)
            ",
        )
        .bind(id.to_string())
        .bind(params.user_id.to_string())
        .bind(params.tenant_id.0.to_string())
        .bind(&params.notification_type)
        .bind(&params.schedule_cron)
        .bind(&params.timezone)
        .bind(params.next_fire_at.to_rfc3339())
        .bind(now.to_rfc3339())
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

    /// List scheduled notifications for a user within a tenant
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn list_scheduled_notifications_impl(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ScheduledNotification>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM scheduled_notifications
            WHERE user_id = ? AND tenant_id = ?
            ORDER BY created_at DESC
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list scheduled notifications: {e}")))?;

        rows.iter()
            .map(Self::parse_scheduled_notification_row)
            .collect()
    }

    /// Delete a scheduled notification (tenant-isolated)
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn delete_scheduled_notification_impl(
        &self,
        id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM scheduled_notifications
            WHERE id = ? AND user_id = ? AND tenant_id = ?
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id.0.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete scheduled notification: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Update a scheduled notification (enable/disable, change schedule)
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn update_scheduled_notification_impl(
        &self,
        params: &UpdateScheduledNotificationParams,
    ) -> AppResult<bool> {
        let mut set_clauses = Vec::new();
        let mut binds: Vec<String> = Vec::new();

        if let Some(e) = params.enabled {
            set_clauses.push("enabled = ?");
            binds.push(if e { "1".to_owned() } else { "0".to_owned() });
        }
        if let Some(ref cron) = params.schedule_cron {
            set_clauses.push("schedule_cron = ?");
            binds.push(cron.clone());
        }
        if let Some(ref tz) = params.timezone {
            set_clauses.push("timezone = ?");
            binds.push(tz.clone());
        }
        if let Some(nf) = params.next_fire_at {
            set_clauses.push("next_fire_at = ?");
            binds.push(nf.to_rfc3339());
        }

        if set_clauses.is_empty() {
            return Ok(false);
        }

        let sql = format!(
            "UPDATE scheduled_notifications SET {} WHERE id = ? AND user_id = ? AND tenant_id = ?",
            set_clauses.join(", ")
        );

        let mut query = sqlx::query(&sql);
        for b in &binds {
            query = query.bind(b);
        }
        query = query
            .bind(params.id.to_string())
            .bind(params.user_id.to_string())
            .bind(params.tenant_id.0.to_string());

        let result = query.execute(&self.pool).await.map_err(|e| {
            AppError::database(format!("Failed to update scheduled notification: {e}"))
        })?;

        Ok(result.rows_affected() > 0)
    }

    /// Get all due scheduled notifications
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn get_due_scheduled_notifications_impl(
        &self,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<ScheduledNotification>> {
        let rows = sqlx::query(
            r"
            SELECT * FROM scheduled_notifications
            WHERE next_fire_at <= ? AND enabled = 1
            ORDER BY next_fire_at ASC
            ",
        )
        .bind(now.to_rfc3339())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to get due scheduled notifications: {e}"))
        })?;

        rows.iter()
            .map(Self::parse_scheduled_notification_row)
            .collect()
    }

    /// Update a scheduled notification after firing
    ///
    /// # Errors
    /// Returns an error if the database query fails
    pub async fn update_scheduled_notification_fired_impl(
        &self,
        id: Uuid,
        last_fired_at: DateTime<Utc>,
        next_fire_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            UPDATE scheduled_notifications
            SET last_fired_at = ?, next_fire_at = ?
            WHERE id = ?
            ",
        )
        .bind(last_fired_at.to_rfc3339())
        .bind(next_fire_at.to_rfc3339())
        .bind(id.to_string())
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
