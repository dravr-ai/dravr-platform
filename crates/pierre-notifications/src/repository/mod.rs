// ABOUTME: Internal notification repository trait for database-agnostic notification persistence
// ABOUTME: Implemented by SQLite and PostgreSQL backends, not exported to crate consumers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Internal notification repository trait
//!
//! This trait defines the persistence interface for notification data access.
//! It is `pub` — only `NotificationService` calls it directly.

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "postgresql")]
pub mod postgres;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::notifications::{
    CreateNotificationParams, CreateScheduledNotificationParams, DeviceToken, Notification,
    NotificationAnalytics, NotificationPreference, ScheduledNotification,
    UpdateScheduledNotificationParams, UpsertNotificationPreferenceParams,
};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// Internal persistence trait for push notification data
///
/// All queries are tenant-scoped for multi-tenant isolation.
/// This trait is intentionally `pub` — consumers interact
/// through `NotificationService` instead.
#[async_trait]
pub trait NotificationRepository: Send + Sync {
    // ── Device Tokens ──

    /// Register or update a device token for push notifications
    async fn upsert_device_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        expo_push_token: &str,
        platform: &str,
        device_name: Option<&str>,
    ) -> AppResult<DeviceToken>;

    /// Get all active device tokens for a user within a tenant
    async fn get_device_tokens(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<DeviceToken>>;

    /// Deactivate a device token (soft-delete)
    async fn deactivate_device_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        token_id: Uuid,
    ) -> AppResult<bool>;

    // ── Notification Preferences ──

    /// Get all notification preferences for a user within a tenant
    async fn get_notification_preferences(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<NotificationPreference>>;

    /// Upsert a notification preference for a specific category
    async fn upsert_notification_preference(
        &self,
        params: &UpsertNotificationPreferenceParams,
    ) -> AppResult<NotificationPreference>;

    // ── Notifications ──

    /// Create a new notification record
    async fn create_notification(
        &self,
        params: &CreateNotificationParams,
    ) -> AppResult<Notification>;

    /// List notifications for a user with optional filters.
    /// Returns (notifications, `total_count`, `unread_count`).
    async fn list_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        limit: u32,
        offset: u32,
        category: Option<&str>,
        unread_only: bool,
    ) -> AppResult<(Vec<Notification>, i64, i64)>;

    /// Mark a notification as read
    async fn mark_notification_read(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool>;

    /// Mark all notifications as read for a user within a tenant
    async fn mark_all_notifications_read(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<u64>;

    /// Delete a notification
    async fn delete_notification(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool>;

    /// Get unread notification count for a user within a tenant
    async fn get_unread_count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<i64>;

    /// Count notifications of a specific category created since a given timestamp
    async fn count_notifications_since(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        category: &str,
        since: DateTime<Utc>,
    ) -> AppResult<i64>;

    // ── Notification Analytics ──

    /// Record when a user opened a notification
    async fn mark_notification_opened(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool>;

    /// Record when a user dismissed a notification
    async fn mark_notification_dismissed(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool>;

    /// Get notification analytics for a specific user within a tenant
    async fn get_notification_analytics(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        category: Option<&str>,
    ) -> AppResult<NotificationAnalytics>;

    // ── Scheduled Notifications ──

    /// Get a single scheduled notification by ID (tenant-isolated)
    async fn get_scheduled_notification_by_id(
        &self,
        id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<ScheduledNotification>>;

    /// Count scheduled notifications for a user within a tenant
    async fn count_scheduled_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<i64>;

    /// Create a scheduled notification
    async fn create_scheduled_notification(
        &self,
        params: &CreateScheduledNotificationParams,
    ) -> AppResult<ScheduledNotification>;

    /// List scheduled notifications for a user within a tenant
    async fn list_scheduled_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ScheduledNotification>>;

    /// Delete a scheduled notification (tenant-isolated)
    async fn delete_scheduled_notification(
        &self,
        id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// Update a scheduled notification (enable/disable, change cron/timezone)
    async fn update_scheduled_notification(
        &self,
        params: &UpdateScheduledNotificationParams,
    ) -> AppResult<bool>;

    /// Get all due scheduled notifications (where `next_fire_at` <= now and enabled)
    async fn get_due_scheduled_notifications(
        &self,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<ScheduledNotification>>;

    /// Update a scheduled notification after it fires
    async fn update_scheduled_notification_fired(
        &self,
        id: Uuid,
        last_fired_at: DateTime<Utc>,
        next_fire_at: DateTime<Utc>,
    ) -> AppResult<bool>;
}
