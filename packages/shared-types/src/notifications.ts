// ABOUTME: Shared TypeScript types for push notification features
// ABOUTME: Device tokens, preferences, notification feed, and category types

// ========== ENUMS ==========

/** Notification categories matching backend NotificationCategory enum */
export type NotificationCategory =
  | 'training'
  | 'recovery'
  | 'social'
  | 'coach'
  | 'achievement'
  | 'system'
  | 'ai'
  | 'reminders';

/** Mobile device platform */
export type DevicePlatform = 'ios' | 'android';

// ========== DEVICE TOKEN TYPES ==========

/** A registered device for push notifications */
export interface DeviceToken {
  /** Unique device token identifier */
  id: string;
  /** User who owns this device */
  user_id: string;
  /** Tenant scope for multi-tenant isolation */
  tenant_id: string;
  /** Expo push token string (e.g. ExponentPushToken[...]) */
  expo_push_token: string;
  /** Device platform (ios or android) */
  platform: DevicePlatform;
  /** Human-readable device name */
  device_name: string | null;
  /** Whether this token is active for receiving notifications */
  active: boolean;
  /** When the token was first registered (ISO 8601) */
  created_at: string;
}

/** Request to register or update a device token */
export interface RegisterDeviceTokenRequest {
  /** Expo push token string */
  expo_push_token: string;
  /** Device platform */
  platform: DevicePlatform;
  /** Human-readable device name */
  device_name?: string;
}

// ========== PREFERENCE TYPES ==========

/** Per-category notification preference */
export interface NotificationPreferenceItem {
  /** Notification category */
  category: NotificationCategory;
  /** Whether notifications in this category are enabled */
  enabled: boolean;
  /** Granular per-type toggles within the category */
  sub_preferences: Record<string, unknown> | null;
  /** Quiet hours start (HH:MM format) */
  quiet_hours_start: string | null;
  /** Quiet hours end (HH:MM format) */
  quiet_hours_end: string | null;
  /** Timezone for quiet hours calculation */
  timezone: string | null;
  /** Maximum notifications per day for this category */
  max_per_day: number | null;
}

/** Response for GET /api/notifications/preferences */
export interface NotificationPreferencesResponse {
  /** User who owns these preferences */
  user_id: string;
  /** Tenant scope */
  tenant_id: string;
  /** List of per-category preference settings */
  preferences: NotificationPreferenceItem[];
}

/** Request to update notification preferences for a category */
export interface UpdateNotificationPreferenceRequest {
  /** Category to update */
  category: NotificationCategory;
  /** Whether to enable or disable notifications */
  enabled?: boolean;
  /** Granular per-type toggles */
  sub_preferences?: Record<string, unknown>;
  /** Quiet hours start (HH:MM format) */
  quiet_hours_start?: string;
  /** Quiet hours end (HH:MM format) */
  quiet_hours_end?: string;
  /** Timezone for quiet hours */
  timezone?: string;
  /** Maximum notifications per day */
  max_per_day?: number;
}

// ========== ACTION TYPES ==========

/** Type of action a notification button can trigger */
export type NotificationActionType = 'open_screen' | 'accept_decline' | 'quick_reply';

/** An action button attached to a notification */
export interface NotificationAction {
  /** Unique identifier for this action within the notification */
  id: string;
  /** Button label displayed to the user */
  title: string;
  /** Type of action triggered when the button is tapped */
  action_type: NotificationActionType;
}

// ========== NOTIFICATION TYPES ==========

/** A single notification in the feed */
export interface NotificationItem {
  /** Unique notification identifier */
  id: string;
  /** Notification category */
  category: NotificationCategory;
  /** Specific notification type within the category */
  notification_type: string;
  /** Notification title */
  title: string;
  /** Notification body text */
  body: string;
  /** JSON payload for deep-link routing and action data */
  data: Record<string, unknown> | null;
  /** Optional image URL for rich notifications */
  image_url: string | null;
  /** When the notification was read (ISO 8601 or null) */
  read_at: string | null;
  /** When the push was delivered (ISO 8601 or null) */
  delivered_at: string | null;
  /** When the user opened the notification (ISO 8601 or null) */
  opened_at: string | null;
  /** When the notification was created (ISO 8601) */
  created_at: string;
  /** Action buttons attached to this notification */
  actions?: NotificationAction[];
  /** Number of notifications collapsed into this one (1 = standalone, >1 = group) */
  collapsed_count?: number;
  /** IDs of notifications that were collapsed into this group */
  collapsed_ids?: string[];
}

/** Response for POST /api/notifications/badge-sync */
export interface BadgeSyncResponse {
  /** Current unread notification count */
  count: number;
}

/** Response for GET /api/notifications (feed) */
export interface NotificationFeedResponse {
  /** List of notifications */
  data: NotificationItem[];
  /** Total count of matching notifications */
  total: number;
  /** Count of unread notifications */
  unread_count: number;
}

/** Response for GET /api/notifications/unread-count */
export interface UnreadCountResponse {
  /** Number of unread notifications */
  unread_count: number;
}

/** Response for POST /api/notifications/read-all */
export interface MarkAllReadResponse {
  /** Number of notifications marked as read */
  updated_count: number;
}

/** Query parameters for listing notifications */
export interface ListNotificationsParams {
  /** Maximum number of results (default 20, max 100) */
  limit?: number;
  /** Offset for pagination */
  offset?: number;
  /** Filter by category */
  category?: NotificationCategory;
  /** Only return unread notifications */
  unread_only?: boolean;
}
