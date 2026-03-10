// ABOUTME: Notification repository dispatch for the database factory
// ABOUTME: Delegates PushNotificationRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::repositories::PushNotificationRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
#[cfg(feature = "postgresql")]
use pierre_core::errors::AppError;
use pierre_core::errors::AppResult;
use pierre_core::models::notifications::{
    CreateNotificationParams, CreateScheduledNotificationParams, DeviceToken, Notification,
    NotificationAnalytics, NotificationPreference, ScheduledNotification,
    UpdateScheduledNotificationParams, UpsertNotificationPreferenceParams,
};
use pierre_core::models::TenantId;
use uuid::Uuid;

#[async_trait]
impl PushNotificationRepository for Database {
    async fn upsert_device_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        expo_push_token: &str,
        platform: &str,
        device_name: Option<&str>,
    ) -> AppResult<DeviceToken> {
        match self {
            Self::SQLite(db) => {
                db.upsert_device_token_impl(
                    user_id,
                    tenant_id,
                    expo_push_token,
                    platform,
                    device_name,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn get_device_tokens(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<DeviceToken>> {
        match self {
            Self::SQLite(db) => db.get_device_tokens_impl(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn deactivate_device_token(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        token_id: Uuid,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.deactivate_device_token_impl(user_id, tenant_id, token_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn get_notification_preferences(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<NotificationPreference>> {
        match self {
            Self::SQLite(db) => {
                db.get_notification_preferences_impl(user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn upsert_notification_preference(
        &self,
        params: &UpsertNotificationPreferenceParams,
    ) -> AppResult<NotificationPreference> {
        match self {
            Self::SQLite(db) => db.upsert_notification_preference_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn create_notification(
        &self,
        params: &CreateNotificationParams,
    ) -> AppResult<Notification> {
        match self {
            Self::SQLite(db) => db.create_notification_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn list_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        limit: u32,
        offset: u32,
        category: Option<&str>,
        unread_only: bool,
    ) -> AppResult<(Vec<Notification>, i64, i64)> {
        match self {
            Self::SQLite(db) => {
                db.list_notifications_impl(user_id, tenant_id, limit, offset, category, unread_only)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn mark_notification_read(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.mark_notification_read_impl(user_id, tenant_id, notification_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn mark_all_notifications_read(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => {
                db.mark_all_notifications_read_impl(user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn delete_notification(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.delete_notification_impl(user_id, tenant_id, notification_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn get_unread_count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => db.get_unread_count_impl(user_id, tenant_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn count_notifications_since(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        category: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => {
                db.count_notifications_since_impl(user_id, tenant_id, category, since)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn mark_notification_opened(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.mark_notification_opened_impl(user_id, tenant_id, notification_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn mark_notification_dismissed(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        notification_id: Uuid,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.mark_notification_dismissed_impl(user_id, tenant_id, notification_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn get_notification_analytics(
        &self,
        tenant_id: TenantId,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        category: Option<&str>,
    ) -> AppResult<NotificationAnalytics> {
        match self {
            Self::SQLite(db) => {
                db.get_notification_analytics_impl(tenant_id, since, until, category)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn get_scheduled_notification_by_id(
        &self,
        id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<ScheduledNotification>> {
        match self {
            Self::SQLite(db) => {
                db.get_scheduled_notification_by_id_impl(id, user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn count_scheduled_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        match self {
            Self::SQLite(db) => {
                db.count_scheduled_notifications_impl(user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn create_scheduled_notification(
        &self,
        params: &CreateScheduledNotificationParams,
    ) -> AppResult<ScheduledNotification> {
        match self {
            Self::SQLite(db) => db.create_scheduled_notification_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn list_scheduled_notifications(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ScheduledNotification>> {
        match self {
            Self::SQLite(db) => {
                db.list_scheduled_notifications_impl(user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn delete_scheduled_notification(
        &self,
        id: Uuid,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.delete_scheduled_notification_impl(id, user_id, tenant_id)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn update_scheduled_notification(
        &self,
        params: &UpdateScheduledNotificationParams,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => db.update_scheduled_notification_impl(params).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn get_due_scheduled_notifications(
        &self,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<ScheduledNotification>> {
        match self {
            Self::SQLite(db) => db.get_due_scheduled_notifications_impl(now).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }

    async fn update_scheduled_notification_fired(
        &self,
        id: Uuid,
        last_fired_at: DateTime<Utc>,
        next_fire_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        match self {
            Self::SQLite(db) => {
                db.update_scheduled_notification_fired_impl(id, last_fired_at, next_fire_at)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(_db) => Err(AppError::internal(
                "PostgreSQL push notification repository requires migration",
            )),
        }
    }
}
