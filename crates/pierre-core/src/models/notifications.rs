// ABOUTME: Push notification models for device tokens, preferences, and notification records
// ABOUTME: Supports multi-tenant notification delivery with per-user category controls and quiet hours
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::TenantId;

/// Supported notification categories
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationCategory {
    /// Training activity notifications (synced, load alerts)
    Training,
    /// Recovery score and overtraining alerts
    Recovery,
    /// Social interactions (friend requests, kudos)
    Social,
    /// Coach messages and plan updates
    Coach,
    /// Personal records and milestones
    Achievement,
    /// System notifications (OAuth expiry, sync failure)
    System,
    /// AI-generated insights and recommendations
    Ai,
    /// Scheduled reminders
    Reminders,
}

impl NotificationCategory {
    /// Return all valid categories for initialization
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Training,
            Self::Recovery,
            Self::Social,
            Self::Coach,
            Self::Achievement,
            Self::System,
            Self::Ai,
            Self::Reminders,
        ]
    }

    /// String representation for database storage
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Training => "training",
            Self::Recovery => "recovery",
            Self::Social => "social",
            Self::Coach => "coach",
            Self::Achievement => "achievement",
            Self::System => "system",
            Self::Ai => "ai",
            Self::Reminders => "reminders",
        }
    }

    /// Parse from string, returning None for unrecognized values
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "training" => Some(Self::Training),
            "recovery" => Some(Self::Recovery),
            "social" => Some(Self::Social),
            "coach" => Some(Self::Coach),
            "achievement" => Some(Self::Achievement),
            "system" => Some(Self::System),
            "ai" => Some(Self::Ai),
            "reminders" => Some(Self::Reminders),
            _ => None,
        }
    }
}

impl fmt::Display for NotificationCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Mobile device platform
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DevicePlatform {
    /// Apple iOS
    Ios,
    /// Google Android
    Android,
}

impl DevicePlatform {
    /// String representation for database storage
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }

    /// Parse from string
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "ios" => Some(Self::Ios),
            "android" => Some(Self::Android),
            _ => None,
        }
    }
}

impl fmt::Display for DevicePlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Registered device for push notifications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceToken {
    /// Unique device token identifier
    pub id: Uuid,
    /// User who owns this device
    pub user_id: Uuid,
    /// Tenant scope for multi-tenant isolation
    pub tenant_id: TenantId,
    /// Expo push token string (e.g. `ExponentPushToken[...]`)
    pub expo_push_token: String,
    /// Device platform (ios or android)
    pub platform: DevicePlatform,
    /// Human-readable device name
    pub device_name: Option<String>,
    /// Whether this token is active for receiving notifications
    pub active: bool,
    /// When the token was first registered
    pub created_at: DateTime<Utc>,
    /// When the token was last updated
    pub updated_at: DateTime<Utc>,
}

/// Request to register or update a device token
#[derive(Debug, Clone, Deserialize)]
pub struct RegisterDeviceTokenRequest {
    /// Expo push token string
    pub expo_push_token: String,
    /// Device platform
    pub platform: DevicePlatform,
    /// Human-readable device name
    pub device_name: Option<String>,
}

/// Per-category notification preference for a user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationPreference {
    /// Unique preference identifier
    pub id: Uuid,
    /// User who owns this preference
    pub user_id: Uuid,
    /// Tenant scope for multi-tenant isolation
    pub tenant_id: TenantId,
    /// Notification category this preference controls
    pub category: NotificationCategory,
    /// Whether notifications in this category are enabled
    pub enabled: bool,
    /// Granular per-type toggles within the category
    pub sub_preferences: Option<serde_json::Value>,
    /// Quiet hours start time (no notifications sent during quiet hours)
    pub quiet_hours_start: Option<NaiveTime>,
    /// Quiet hours end time
    pub quiet_hours_end: Option<NaiveTime>,
    /// Timezone for quiet hours calculation
    pub timezone: Option<String>,
    /// Maximum notifications per day for this category
    pub max_per_day: Option<i32>,
    /// When the preference was created
    pub created_at: DateTime<Utc>,
    /// When the preference was last updated
    pub updated_at: DateTime<Utc>,
}

/// Request to update notification preferences for a category
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateNotificationPreferenceRequest {
    /// Category to update
    pub category: NotificationCategory,
    /// Whether to enable or disable notifications
    pub enabled: Option<bool>,
    /// Granular per-type toggles
    pub sub_preferences: Option<serde_json::Value>,
    /// Quiet hours start (HH:MM format)
    pub quiet_hours_start: Option<String>,
    /// Quiet hours end (HH:MM format)
    pub quiet_hours_end: Option<String>,
    /// Timezone for quiet hours
    pub timezone: Option<String>,
    /// Maximum notifications per day
    pub max_per_day: Option<i32>,
}

/// Parameters for upserting a notification preference in the database
#[derive(Debug, Clone)]
pub struct UpsertNotificationPreferenceParams {
    /// User who owns this preference
    pub user_id: Uuid,
    /// Tenant scope for multi-tenant isolation
    pub tenant_id: TenantId,
    /// Category to update
    pub category: String,
    /// Whether notifications are enabled
    pub enabled: bool,
    /// Granular per-type toggles
    pub sub_preferences: Option<serde_json::Value>,
    /// Quiet hours start (HH:MM format)
    pub quiet_hours_start: Option<String>,
    /// Quiet hours end (HH:MM format)
    pub quiet_hours_end: Option<String>,
    /// Timezone for quiet hours
    pub timezone: Option<String>,
    /// Maximum notifications per day
    pub max_per_day: Option<i32>,
}

/// API response for notification preferences
#[derive(Debug, Clone, Serialize)]
pub struct NotificationPreferencesResponse {
    /// User who owns these preferences
    pub user_id: Uuid,
    /// Tenant scope
    pub tenant_id: TenantId,
    /// List of per-category preference settings
    pub preferences: Vec<NotificationPreferenceItem>,
}

/// Single preference item in the API response
#[derive(Debug, Clone, Serialize)]
pub struct NotificationPreferenceItem {
    /// Notification category
    pub category: NotificationCategory,
    /// Whether enabled
    pub enabled: bool,
    /// Granular per-type toggles
    pub sub_preferences: Option<serde_json::Value>,
    /// Quiet hours start (HH:MM format)
    pub quiet_hours_start: Option<String>,
    /// Quiet hours end (HH:MM format)
    pub quiet_hours_end: Option<String>,
    /// Timezone for quiet hours
    pub timezone: Option<String>,
    /// Maximum notifications per day
    pub max_per_day: Option<i32>,
}

impl From<NotificationPreference> for NotificationPreferenceItem {
    fn from(pref: NotificationPreference) -> Self {
        Self {
            category: pref.category,
            enabled: pref.enabled,
            sub_preferences: pref.sub_preferences,
            quiet_hours_start: pref
                .quiet_hours_start
                .map(|t| t.format("%H:%M").to_string()),
            quiet_hours_end: pref.quiet_hours_end.map(|t| t.format("%H:%M").to_string()),
            timezone: pref.timezone,
            max_per_day: pref.max_per_day,
        }
    }
}

/// Type of action a notification button can trigger
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationActionType {
    /// Navigate to a specific screen
    OpenScreen,
    /// Show accept/decline buttons (e.g., friend requests)
    AcceptDecline,
    /// Show a quick reply input (e.g., coach messages)
    QuickReply,
}

/// An action button attached to a notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationAction {
    /// Unique identifier for this action within the notification
    pub id: String,
    /// Button label displayed to the user
    pub title: String,
    /// Type of action triggered when the button is tapped
    pub action_type: NotificationActionType,
}

/// A notification record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// Unique notification identifier
    pub id: Uuid,
    /// Recipient user
    pub user_id: Uuid,
    /// Tenant scope for multi-tenant isolation
    pub tenant_id: TenantId,
    /// Notification category (training, recovery, social, etc.)
    pub category: NotificationCategory,
    /// Specific notification type within the category (e.g. `activity_synced`)
    pub notification_type: String,
    /// Notification title
    pub title: String,
    /// Notification body text
    pub body: String,
    /// JSON payload for deep-link routing and action data
    pub data: Option<serde_json::Value>,
    /// Optional image URL for rich notifications
    pub image_url: Option<String>,
    /// When the notification was read (None if unread)
    pub read_at: Option<DateTime<Utc>>,
    /// When the push notification was delivered to the device
    pub delivered_at: Option<DateTime<Utc>>,
    /// When the user opened the notification
    pub opened_at: Option<DateTime<Utc>>,
    /// When the user dismissed the notification
    pub dismissed_at: Option<DateTime<Utc>>,
    /// Action buttons attached to this notification
    pub actions: Option<Vec<NotificationAction>>,
    /// When the notification was created
    pub created_at: DateTime<Utc>,
}

/// Parameters for creating a new notification
#[derive(Debug, Clone)]
pub struct CreateNotificationParams {
    /// Recipient user
    pub user_id: Uuid,
    /// Tenant scope for multi-tenant isolation
    pub tenant_id: TenantId,
    /// Notification category
    pub category: NotificationCategory,
    /// Specific notification type within the category
    pub notification_type: String,
    /// Notification title
    pub title: String,
    /// Notification body text
    pub body: String,
    /// JSON payload for deep-link routing
    pub data: Option<serde_json::Value>,
    /// Optional image URL
    pub image_url: Option<String>,
    /// Action buttons attached to this notification
    pub actions: Option<Vec<NotificationAction>>,
}

/// Query parameters for listing notifications
#[derive(Debug, Clone, Deserialize)]
pub struct ListNotificationsQuery {
    /// Maximum number of results (default 20, max 100)
    pub limit: Option<u32>,
    /// Offset for pagination
    pub offset: Option<u32>,
    /// Filter by category
    pub category: Option<String>,
    /// Only return unread notifications
    pub unread_only: Option<bool>,
}

/// Response for notification feed endpoint
#[derive(Debug, Clone, Serialize)]
pub struct NotificationFeedResponse {
    /// List of notifications
    pub data: Vec<NotificationItem>,
    /// Total count of matching notifications
    pub total: i64,
    /// Count of unread notifications (across all categories)
    pub unread_count: i64,
}

/// Single notification in the feed response
#[derive(Debug, Clone, Serialize)]
pub struct NotificationItem {
    /// Unique notification identifier
    pub id: Uuid,
    /// Notification category
    pub category: NotificationCategory,
    /// Specific notification type
    pub notification_type: String,
    /// Notification title
    pub title: String,
    /// Notification body text
    pub body: String,
    /// JSON payload for deep-link routing
    pub data: Option<serde_json::Value>,
    /// Optional image URL
    pub image_url: Option<String>,
    /// When the notification was read (RFC3339 or null)
    pub read_at: Option<String>,
    /// When the push was delivered (RFC3339 or null)
    pub delivered_at: Option<String>,
    /// When the user opened the notification (RFC3339 or null)
    pub opened_at: Option<String>,
    /// When the notification was created (RFC3339)
    pub created_at: String,
    /// Action buttons attached to this notification
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<NotificationAction>>,
    /// Number of collapsed notifications (1 = not collapsed, >1 = group representative)
    #[serde(skip_serializing_if = "is_one")]
    pub collapsed_count: u32,
    /// IDs of notifications collapsed into this group (empty if not collapsed)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub collapsed_ids: Vec<Uuid>,
}

/// Helper for `skip_serializing_if` on `collapsed_count`
///
/// Takes a reference because serde's `skip_serializing_if` requires `&T -> bool`.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_one(val: &u32) -> bool {
    *val == 1
}

impl From<Notification> for NotificationItem {
    fn from(n: Notification) -> Self {
        Self {
            id: n.id,
            category: n.category,
            notification_type: n.notification_type,
            title: n.title,
            body: n.body,
            data: n.data,
            image_url: n.image_url,
            read_at: n.read_at.map(|t| t.to_rfc3339()),
            delivered_at: n.delivered_at.map(|t| t.to_rfc3339()),
            opened_at: n.opened_at.map(|t| t.to_rfc3339()),
            created_at: n.created_at.to_rfc3339(),
            actions: n.actions,
            collapsed_count: 1,
            collapsed_ids: Vec::new(),
        }
    }
}

/// Notification types eligible for collapsing in the feed
const COLLAPSIBLE_TYPES: &[&str] = &["kudos_received", "sync_failure", "friend_request"];

/// Collapse consecutive notifications of the same type into grouped items
///
/// Groups notifications by `notification_type` when they share the same type
/// and appear within the same feed page. The most recent notification in each
/// group becomes the representative, with an updated title showing the count.
#[must_use]
pub fn collapse_notifications(items: Vec<NotificationItem>) -> Vec<NotificationItem> {
    if items.is_empty() {
        return items;
    }

    let mut result: Vec<NotificationItem> = Vec::with_capacity(items.len());

    for item in items {
        if !COLLAPSIBLE_TYPES.contains(&item.notification_type.as_str()) {
            result.push(item);
            continue;
        }

        // Try to merge with the last item in result if same type
        let should_merge = result.last().is_some_and(|last| {
            last.notification_type == item.notification_type && last.category == item.category
        });

        if should_merge {
            if let Some(last) = result.last_mut() {
                last.collapsed_ids.push(item.id);
                last.collapsed_count += 1;
                update_collapsed_title(last);
            }
        } else {
            result.push(item);
        }
    }

    result
}

/// Update the title of a collapsed notification group to reflect the count
fn update_collapsed_title(item: &mut NotificationItem) {
    let count = item.collapsed_count;
    match item.notification_type.as_str() {
        "kudos_received" => {
            item.title = format!("{count} kudos received");
            item.body = format!("You received {count} kudos recently");
        }
        "sync_failure" => {
            item.title = format!("{count} sync failures");
            item.body = format!("{count} activity syncs failed recently");
        }
        "friend_request" => {
            item.title = format!("{count} friend requests");
            item.body = format!("You have {count} pending friend requests");
        }
        _ => {}
    }
}

/// A scheduled notification record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledNotification {
    /// Unique scheduled notification identifier
    pub id: Uuid,
    /// Target user
    pub user_id: Uuid,
    /// Tenant scope for multi-tenant isolation
    pub tenant_id: TenantId,
    /// Notification type to send on each trigger
    pub notification_type: String,
    /// Cron expression defining the schedule
    pub schedule_cron: String,
    /// Timezone for cron evaluation
    pub timezone: String,
    /// Pre-computed next fire timestamp
    pub next_fire_at: Option<DateTime<Utc>>,
    /// Whether this schedule is active
    pub enabled: bool,
    /// When this schedule last triggered
    pub last_fired_at: Option<DateTime<Utc>>,
    /// When the schedule was created
    pub created_at: DateTime<Utc>,
}

/// Parameters for creating a new scheduled notification
#[derive(Debug, Clone)]
pub struct CreateScheduledNotificationParams {
    /// Target user
    pub user_id: Uuid,
    /// Tenant scope for multi-tenant isolation
    pub tenant_id: TenantId,
    /// Notification type to send on each trigger (e.g. `weekly_training_summary`)
    pub notification_type: String,
    /// Cron expression (validated before storage)
    pub schedule_cron: String,
    /// Timezone for cron evaluation (IANA timezone name)
    pub timezone: String,
    /// Pre-computed next fire timestamp (computed from cron + timezone)
    pub next_fire_at: DateTime<Utc>,
}

/// API response for a scheduled notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledNotificationItem {
    /// Unique scheduled notification identifier
    pub id: Uuid,
    /// Notification type
    pub notification_type: String,
    /// Cron expression
    pub schedule_cron: String,
    /// Timezone for evaluation
    pub timezone: String,
    /// Next fire timestamp (RFC3339 or null)
    pub next_fire_at: Option<String>,
    /// Whether this schedule is active
    pub enabled: bool,
    /// When this schedule last triggered (RFC3339 or null)
    pub last_fired_at: Option<String>,
    /// When the schedule was created (RFC3339)
    pub created_at: String,
}

impl From<ScheduledNotification> for ScheduledNotificationItem {
    fn from(s: ScheduledNotification) -> Self {
        Self {
            id: s.id,
            notification_type: s.notification_type,
            schedule_cron: s.schedule_cron,
            timezone: s.timezone,
            next_fire_at: s.next_fire_at.map(|t| t.to_rfc3339()),
            enabled: s.enabled,
            last_fired_at: s.last_fired_at.map(|t| t.to_rfc3339()),
            created_at: s.created_at.to_rfc3339(),
        }
    }
}

/// Request to create a scheduled notification
#[derive(Debug, Clone, Deserialize)]
pub struct CreateScheduledNotificationRequest {
    /// Notification type (e.g. `weekly_training_summary`, `workout_reminder`, `weekly_digest`)
    pub notification_type: String,
    /// Cron expression (5-field: minute hour day-of-month month day-of-week)
    pub schedule_cron: String,
    /// IANA timezone for cron evaluation (e.g. `America/New_York`)
    pub timezone: Option<String>,
}

/// Request to update a scheduled notification (enable/disable or change schedule)
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateScheduledNotificationRequest {
    /// Whether the schedule should be enabled
    pub enabled: Option<bool>,
    /// Updated cron expression
    pub schedule_cron: Option<String>,
    /// Updated timezone
    pub timezone: Option<String>,
}

/// Parameters for updating a scheduled notification in the database
#[derive(Debug, Clone)]
pub struct UpdateScheduledNotificationParams {
    /// Schedule ID to update
    pub id: Uuid,
    /// Owner user ID (for authorization)
    pub user_id: Uuid,
    /// Tenant scope for multi-tenant isolation
    pub tenant_id: TenantId,
    /// Whether the schedule should be enabled
    pub enabled: Option<bool>,
    /// Updated cron expression
    pub schedule_cron: Option<String>,
    /// Updated timezone
    pub timezone: Option<String>,
    /// Recomputed next fire timestamp
    pub next_fire_at: Option<DateTime<Utc>>,
}

/// Aggregated notification analytics for a tenant
#[derive(Debug, Clone, Serialize)]
pub struct NotificationAnalytics {
    /// Total notifications sent in the period
    pub total_sent: i64,
    /// Fraction of sent that were delivered (0.0–1.0)
    pub delivery_rate: f64,
    /// Fraction of delivered that were opened (0.0–1.0)
    pub open_rate: f64,
    /// Fraction of delivered that were dismissed (0.0–1.0)
    pub dismiss_rate: f64,
    /// Breakdown by notification category
    pub category_breakdown: Vec<CategoryAnalytics>,
    /// Average seconds between delivery and open
    pub avg_time_to_open_seconds: Option<f64>,
}

/// Per-category analytics breakdown
#[derive(Debug, Clone, Serialize)]
pub struct CategoryAnalytics {
    /// Category name
    pub category: String,
    /// Number sent in this category
    pub sent: i64,
    /// Number opened
    pub opened: i64,
    /// Number dismissed
    pub dismissed: i64,
}

/// Query parameters for analytics endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct NotificationAnalyticsQuery {
    /// Start date filter (RFC3339)
    pub since: Option<String>,
    /// End date filter (RFC3339)
    pub until: Option<String>,
    /// Filter by category
    pub category: Option<String>,
}
