// ABOUTME: NotificationService facade — the single public entry point for all notification operations
// ABOUTME: Encapsulates persistence (SQLite/PostgreSQL), dispatch pipeline, and cron scheduling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Notification Service
//!
//! `NotificationService` is the public facade for the notification subsystem.
//! Consumers construct it via `from_sqlite()` or `from_postgres()` and interact
//! through its methods — the underlying `NotificationRepository` is never exposed.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::notifications::{
    CreateNotificationParams, CreateScheduledNotificationParams, DeviceToken, Notification,
    NotificationAnalytics, NotificationPreference, ScheduledNotification,
    UpdateScheduledNotificationParams, UpsertNotificationPreferenceParams,
};
use pierre_core::models::TenantId;
use tokio::task::AbortHandle;
use uuid::Uuid;

use crate::dispatch::{DispatchOutcome, DispatchRequest, NotificationDispatcher};
use crate::expo_push::ExpoPushService;
use crate::repository::NotificationRepository;
use crate::scheduler;

/// Public facade for the notification subsystem
///
/// Owns the database repository, dispatch pipeline, and scheduler lifecycle.
/// Constructed via `from_sqlite()` or `from_postgres()`.
pub struct NotificationService {
    /// Database-agnostic notification repository
    repo: Arc<dyn NotificationRepository>,
    /// Notification dispatcher (preference checks, persistence, push delivery)
    dispatcher: Arc<NotificationDispatcher>,
}

impl NotificationService {
    /// Create a `NotificationService` backed by a `SQLite` database pool
    ///
    /// # Errors
    /// Returns an error if the Expo push HTTP client cannot be initialized
    #[cfg(feature = "sqlite")]
    pub fn from_sqlite(pool: sqlx::SqlitePool) -> AppResult<Self> {
        use crate::repository::sqlite::SqliteNotificationRepository;

        let repo: Arc<dyn NotificationRepository> =
            Arc::new(SqliteNotificationRepository::new(pool));
        let expo_push = Arc::new(ExpoPushService::new()?);
        let dispatcher = Arc::new(NotificationDispatcher::new(Arc::clone(&repo), expo_push));
        Ok(Self { repo, dispatcher })
    }

    /// Create a `NotificationService` backed by a `PostgreSQL` database pool
    ///
    /// # Errors
    /// Returns an error if the Expo push HTTP client cannot be initialized
    #[cfg(feature = "postgresql")]
    pub fn from_postgres(pool: sqlx::PgPool) -> AppResult<Self> {
        use crate::repository::postgres::PostgresNotificationRepository;

        let repo: Arc<dyn NotificationRepository> =
            Arc::new(PostgresNotificationRepository::new(pool));
        let expo_push = Arc::new(ExpoPushService::new()?);
        let dispatcher = Arc::new(NotificationDispatcher::new(Arc::clone(&repo), expo_push));
        Ok(Self { repo, dispatcher })
    }

    /// Start the background scheduler that polls for due scheduled notifications.
    ///
    /// Returns an `AbortHandle` to stop the scheduler on shutdown.
    #[must_use]
    pub fn start_scheduler(&self) -> AbortHandle {
        scheduler::start_notification_scheduler(
            Arc::clone(&self.repo),
            Arc::clone(&self.dispatcher),
        )
    }

    // ── Dispatch ──

    /// Dispatch a notification through the full pipeline (preference checks, persist, push)
    ///
    /// # Errors
    /// Returns an error if database operations fail. Push delivery failures
    /// are logged but do not propagate.
    pub async fn dispatch(&self, request: &DispatchRequest) -> AppResult<DispatchOutcome> {
        self.dispatcher.dispatch(request).await
    }

    // ── Device Tokens ──

    /// Register or update a device token for push notifications
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn upsert_device_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        expo_push_token: &str,
        platform: &str,
        device_name: Option<&str>,
    ) -> AppResult<DeviceToken> {
        self.repo
            .upsert_device_token(user_id, tenant_id, expo_push_token, platform, device_name)
            .await
    }

    /// Get all active device tokens for a user within a tenant
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn get_device_tokens(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<DeviceToken>> {
        self.repo.get_device_tokens(user_id, tenant_id).await
    }

    /// Deactivate a device token (soft-delete)
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn deactivate_device_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        token_id: Uuid,
    ) -> AppResult<bool> {
        self.repo
            .deactivate_device_token(user_id, tenant_id, token_id)
            .await
    }

    // ── Notification Preferences ──

    /// Get all notification preferences for a user within a tenant
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn get_notification_preferences(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<NotificationPreference>> {
        self.repo
            .get_notification_preferences(user_id, tenant_id)
            .await
    }

    /// Upsert a notification preference for a specific category
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn upsert_notification_preference(
        &self,
        params: &UpsertNotificationPreferenceParams,
    ) -> AppResult<NotificationPreference> {
        self.repo.upsert_notification_preference(params).await
    }

    // ── Notifications ──

    /// Create a new notification record
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn create_notification(
        &self,
        params: &CreateNotificationParams,
    ) -> AppResult<Notification> {
        self.repo.create_notification(params).await
    }

    /// List notifications for a user with optional filters.
    /// Returns (notifications, `total_count`, `unread_count`).
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn list_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        limit: u32,
        offset: u32,
        category: Option<&str>,
        unread_only: bool,
    ) -> AppResult<(Vec<Notification>, i64, i64)> {
        self.repo
            .list_notifications(user_id, tenant_id, limit, offset, category, unread_only)
            .await
    }

    /// Mark a notification as read
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn mark_notification_read(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        self.repo
            .mark_notification_read(user_id, tenant_id, notification_id)
            .await
    }

    /// Mark all notifications as read for a user within a tenant
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn mark_all_notifications_read(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<u64> {
        self.repo
            .mark_all_notifications_read(user_id, tenant_id)
            .await
    }

    /// Delete a notification
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn delete_notification(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        self.repo
            .delete_notification(user_id, tenant_id, notification_id)
            .await
    }

    /// Get unread notification count for a user within a tenant
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn get_unread_count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<i64> {
        self.repo.get_unread_count(user_id, tenant_id).await
    }

    /// Count notifications of a specific category created since a given timestamp
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn count_notifications_since(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        category: &str,
        since: DateTime<Utc>,
    ) -> AppResult<i64> {
        self.repo
            .count_notifications_since(user_id, tenant_id, category, since)
            .await
    }

    // ── Notification Analytics ──

    /// Record when a user opened a notification
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn mark_notification_opened(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        self.repo
            .mark_notification_opened(user_id, tenant_id, notification_id)
            .await
    }

    /// Record when a user dismissed a notification
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn mark_notification_dismissed(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        self.repo
            .mark_notification_dismissed(user_id, tenant_id, notification_id)
            .await
    }

    /// Get notification analytics for a specific user within a tenant
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn get_notification_analytics(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        category: Option<&str>,
    ) -> AppResult<NotificationAnalytics> {
        self.repo
            .get_notification_analytics(user_id, tenant_id, since, until, category)
            .await
    }

    // ── Scheduled Notifications ──

    /// Get a single scheduled notification by ID (tenant-isolated)
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn get_scheduled_notification_by_id(
        &self,
        id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<ScheduledNotification>> {
        self.repo
            .get_scheduled_notification_by_id(id, user_id, tenant_id)
            .await
    }

    /// Count scheduled notifications for a user within a tenant
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn count_scheduled_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        self.repo
            .count_scheduled_notifications(user_id, tenant_id)
            .await
    }

    /// Create a scheduled notification
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn create_scheduled_notification(
        &self,
        params: &CreateScheduledNotificationParams,
    ) -> AppResult<ScheduledNotification> {
        self.repo.create_scheduled_notification(params).await
    }

    /// List scheduled notifications for a user within a tenant
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn list_scheduled_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ScheduledNotification>> {
        self.repo
            .list_scheduled_notifications(user_id, tenant_id)
            .await
    }

    /// Delete a scheduled notification (tenant-isolated)
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn delete_scheduled_notification(
        &self,
        id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        self.repo
            .delete_scheduled_notification(id, user_id, tenant_id)
            .await
    }

    /// Update a scheduled notification (enable/disable, change cron/timezone)
    ///
    /// # Errors
    /// Returns an error if the database operation fails
    pub async fn update_scheduled_notification(
        &self,
        params: &UpdateScheduledNotificationParams,
    ) -> AppResult<bool> {
        self.repo.update_scheduled_notification(params).await
    }
}
