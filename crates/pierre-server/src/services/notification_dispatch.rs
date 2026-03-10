// ABOUTME: Central notification dispatch service that checks preferences, enforces quiet hours
// ABOUTME: and frequency caps, then persists notifications and sends push via Expo
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Notification Dispatch Service
//!
//! Central pipeline for delivering notifications to users. Accepts a notification event,
//! checks user preferences (category enabled, quiet hours, frequency cap), persists the
//! notification record, looks up device tokens, and sends push via Expo.

use chrono::{Duration, NaiveTime, Timelike, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::notifications::{
    CreateNotificationParams, DeviceToken, NotificationCategory, NotificationPreference,
};
use pierre_core::models::TenantId;
use pierre_database::plugins::factory::Database;
use pierre_database::repositories::PushNotificationRepository;
use serde_json::Value;
use std::env;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use super::expo_push::{ExpoPushMessage, ExpoPushService};
use crate::constants::notifications as notif_constants;

/// Reason a notification was suppressed before delivery
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuppressionReason {
    /// User disabled this notification category
    CategoryDisabled,
    /// Current time falls within the user's quiet hours window
    QuietHours,
    /// User has exceeded their daily notification cap for this category
    FrequencyCapExceeded,
}

/// Outcome of a dispatch attempt
#[derive(Debug)]
pub enum DispatchOutcome {
    /// Notification was persisted and push was sent to N devices
    Delivered {
        /// ID of the persisted notification record
        notification_id: Uuid,
        /// Number of devices the push was sent to
        devices: usize,
    },
    /// Notification was persisted but user has no active device tokens
    PersistedNoDevices {
        /// ID of the persisted notification record
        notification_id: Uuid,
    },
    /// Notification was suppressed by user preferences
    Suppressed(SuppressionReason),
}

/// Request to dispatch a notification through the pipeline
#[derive(Debug, Clone)]
pub struct DispatchRequest {
    /// Target user
    pub user_id: Uuid,
    /// Tenant scope for multi-tenant isolation
    pub tenant_id: TenantId,
    /// Notification category (training, recovery, achievement, etc.)
    pub category: NotificationCategory,
    /// Specific notification type within the category
    pub notification_type: String,
    /// Notification title
    pub title: String,
    /// Notification body text
    pub body: String,
    /// JSON payload for deep-link routing
    pub data: Option<Value>,
    /// Optional image URL for rich notifications
    pub image_url: Option<String>,
    /// When true, skip the daily frequency cap check (used for coach notifications)
    /// Quiet hours and category-disabled checks still apply.
    pub bypass_frequency_cap: bool,
}

/// Central notification dispatch service
pub struct NotificationDispatcher {
    /// Database for notification persistence and preference lookup
    database: Arc<Database>,
    /// Expo push service for device delivery
    expo_push: Arc<ExpoPushService>,
}

impl NotificationDispatcher {
    /// Create a new dispatcher with shared database and push service
    #[must_use]
    pub fn new(database: Arc<Database>, expo_push: Arc<ExpoPushService>) -> Self {
        Self {
            database,
            expo_push,
        }
    }

    /// Dispatch a notification through the full pipeline:
    /// 1. Check user preferences (category enabled, quiet hours, frequency cap)
    /// 2. Persist the notification record
    /// 3. Look up active device tokens
    /// 4. Send push via Expo
    ///
    /// # Errors
    /// Returns an error if database operations fail. Push delivery failures
    /// are logged but do not propagate errors (fire-and-forget semantics).
    pub async fn dispatch(&self, request: &DispatchRequest) -> AppResult<DispatchOutcome> {
        // Step 1: Check user preferences
        if let Some(reason) = self
            .check_suppression(
                request.user_id,
                request.tenant_id,
                request.category,
                request.bypass_frequency_cap,
            )
            .await?
        {
            info!(
                user_id = %request.user_id,
                category = %request.category,
                reason = ?reason,
                "Notification suppressed by user preferences"
            );
            return Ok(DispatchOutcome::Suppressed(reason));
        }

        // Step 2: Persist notification
        let params = CreateNotificationParams {
            user_id: request.user_id,
            tenant_id: request.tenant_id,
            category: request.category,
            notification_type: request.notification_type.clone(),
            title: request.title.clone(),
            body: request.body.clone(),
            data: request.data.clone(),
            image_url: request.image_url.clone(),
        };
        let notification = self.database.create_notification(&params).await?;

        // Step 3: Look up device tokens
        let tokens = self
            .database
            .get_device_tokens(request.user_id, request.tenant_id)
            .await?;

        if tokens.is_empty() {
            info!(
                user_id = %request.user_id,
                notification_id = %notification.id,
                "Notification persisted but user has no active device tokens"
            );
            return Ok(DispatchOutcome::PersistedNoDevices {
                notification_id: notification.id,
            });
        }

        // Step 4: Send push to all active devices
        let device_count = tokens.len();
        self.deliver_push(request, &tokens, notification.id).await;

        Ok(DispatchOutcome::Delivered {
            notification_id: notification.id,
            devices: device_count,
        })
    }

    /// Build push messages from device tokens and send them via Expo.
    /// Failures are logged but never propagated (fire-and-forget delivery).
    async fn deliver_push(
        &self,
        request: &DispatchRequest,
        tokens: &[DeviceToken],
        notification_id: Uuid,
    ) {
        let messages: Vec<ExpoPushMessage> = tokens
            .iter()
            .map(|token| ExpoPushMessage {
                to: token.expo_push_token.clone(),
                title: request.title.clone(),
                body: request.body.clone(),
                data: request.data.clone(),
                sound: Some("default".to_owned()),
                badge: None,
                channel_id: Some(request.category.as_str().to_owned()),
                category_id: Some(request.category.as_str().to_owned()),
                priority: Some("high".to_owned()),
            })
            .collect();

        let device_count = messages.len();
        match self.expo_push.send_batch(&messages).await {
            Ok(tickets) => {
                let failure_count = tickets.iter().filter(|t| t.status != "ok").count();
                if failure_count > 0 {
                    warn!(
                        notification_id = %notification_id,
                        total = device_count,
                        failed = failure_count,
                        "Some push deliveries failed"
                    );
                }
                info!(
                    notification_id = %notification_id,
                    devices = device_count,
                    "Push notification delivered"
                );
            }
            Err(e) => {
                warn!(
                    notification_id = %notification_id,
                    error = %e,
                    "Push delivery failed — notification persisted but not pushed"
                );
            }
        }
    }

    /// Check whether a notification should be suppressed based on user preferences.
    /// Returns `Some(reason)` if suppressed, `None` if delivery should proceed.
    /// When `bypass_frequency_cap` is true, the daily frequency cap check is skipped
    /// (used for coach notifications that must always reach the athlete).
    async fn check_suppression(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        category: NotificationCategory,
        bypass_frequency_cap: bool,
    ) -> AppResult<Option<SuppressionReason>> {
        let preferences = self
            .database
            .get_notification_preferences(user_id, tenant_id)
            .await?;

        // Find the preference for this specific category
        let pref = preferences.iter().find(|p| p.category == category);

        // If no preference is set, default to enabled (deliver the notification)
        let Some(pref) = pref else {
            return Ok(None);
        };

        // Check if category is disabled
        if !pref.enabled {
            return Ok(Some(SuppressionReason::CategoryDisabled));
        }

        // Check quiet hours
        if Self::is_quiet_hours(pref) {
            return Ok(Some(SuppressionReason::QuietHours));
        }

        // Check frequency cap (skipped when bypass is requested, e.g., for coach notifications)
        if !bypass_frequency_cap
            && self
                .is_frequency_cap_exceeded(user_id, tenant_id, category, pref)
                .await?
        {
            return Ok(Some(SuppressionReason::FrequencyCapExceeded));
        }

        Ok(None)
    }

    /// Check if the current time falls within the user's quiet hours window
    fn is_quiet_hours(pref: &NotificationPreference) -> bool {
        let (Some(start), Some(end)) = (pref.quiet_hours_start, pref.quiet_hours_end) else {
            return false;
        };

        // Resolve current time in user's timezone, falling back to UTC
        let now_time = pref
            .timezone
            .as_ref()
            .and_then(|tz_str| tz_str.parse::<chrono_tz::Tz>().ok())
            .map_or_else(
                || Utc::now().time(),
                |tz| {
                    let local = Utc::now().with_timezone(&tz);
                    NaiveTime::from_hms_opt(local.hour(), local.minute(), local.second())
                        .unwrap_or_else(|| Utc::now().time())
                },
            );

        // Handle overnight quiet windows (e.g., 22:00 → 07:00)
        if start <= end {
            now_time >= start && now_time < end
        } else {
            now_time >= start || now_time < end
        }
    }

    /// Check if the daily frequency cap has been exceeded for this category
    async fn is_frequency_cap_exceeded(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        category: NotificationCategory,
        pref: &NotificationPreference,
    ) -> AppResult<bool> {
        let default_cap = env::var("NOTIFICATION_DEFAULT_MAX_PER_DAY")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(notif_constants::DEFAULT_MAX_PER_DAY as i32);
        let max_per_day = i64::from(pref.max_per_day.unwrap_or(default_cap));
        let since = Utc::now() - Duration::hours(24);
        let count = self
            .database
            .count_notifications_since(user_id, tenant_id, category.as_str(), since)
            .await?;

        Ok(count >= max_per_day)
    }
}
