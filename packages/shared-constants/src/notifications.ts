// ABOUTME: Shared notification constants and utilities for web and mobile
// ABOUTME: Category metadata (colors, labels, icon names), time formatting, preference merging

import type {
  NotificationCategory,
  NotificationPreferenceItem,
  UpdateNotificationPreferenceRequest,
} from '@pierre/shared-types';

/** Category display metadata shared across all frontends */
export interface NotificationCategoryMeta {
  /** Human-readable label */
  label: string;
  /** Hex color for the category badge/dot */
  color: string;
  /** Lucide icon name (both web and mobile use lucide) */
  iconName: string;
}

/**
 * Canonical category metadata for notification rendering.
 *
 * Colors are aligned with the Boreal Editorial palette so badges read
 * correctly on the light `surface` (#F9F9F6). Training and recovery pull
 * from `PILLAR_COLORS`; coach/ai/reminders use tonal derivatives of the
 * primary forest-green family.
 */
export const NOTIFICATION_CATEGORY_META: Record<NotificationCategory, NotificationCategoryMeta> = {
  training: { label: 'Training', color: '#3c6658', iconName: 'dumbbell' },     // activity pillar
  recovery: { label: 'Recovery', color: '#5e7a82', iconName: 'heart' },        // recovery pillar
  coach: { label: 'Coach', color: '#00241a', iconName: 'message-circle' },     // primary
  achievement: { label: 'Achievements', color: '#8f6a2e', iconName: 'trophy' }, // nutrition pillar / warm bronze
  system: { label: 'System', color: '#717974', iconName: 'settings' },          // outline
  ai: { label: 'AI Insights', color: '#0d3b2e', iconName: 'brain' },           // primary_container
  reminders: { label: 'Reminders', color: '#7a4d5e', iconName: 'clock' },      // mobility pillar / aged rose
} as const;

/** All notification categories in display order */
export const NOTIFICATION_CATEGORIES: readonly NotificationCategory[] = [
  'training',
  'recovery',
  'coach',
  'achievement',
  'system',
  'ai',
  'reminders',
] as const;

/**
 * Format a timestamp as a relative time string (e.g., "5m ago", "2h ago").
 *
 * Shared between web and mobile to ensure consistent display.
 */
export function formatNotificationTime(dateStr: string): string {
  const now = Date.now();
  const date = new Date(dateStr).getTime();
  const diffMs = now - date;
  const diffMin = Math.floor(diffMs / 60_000);
  const diffHr = Math.floor(diffMs / 3_600_000);
  const diffDay = Math.floor(diffMs / 86_400_000);

  if (diffMin < 1) return 'Just now';
  if (diffMin < 60) return `${diffMin}m ago`;
  if (diffHr < 24) return `${diffHr}h ago`;
  if (diffDay < 7) return `${diffDay}d ago`;
  return new Date(dateStr).toLocaleDateString();
}

/**
 * Format a collapsed notification count for display.
 *
 * Returns null when the notification is not collapsed (count <= 1).
 */
export function formatCollapsedCount(count: number | undefined): string | null {
  if (!count || count <= 1) return null;
  return `+${count - 1} similar`;
}

/**
 * The daily-cap choices a preference surface offers, in menu order.
 *
 * `null` means no cap. The server validates `max_per_day` against `0..=1000`,
 * so every value here is inside that range and a cap of 0 is deliberately
 * absent — muting a category is what the enabled switch is for.
 */
export const NOTIFICATION_MAX_PER_DAY_CHOICES: readonly (number | null)[] = [
  null,
  1,
  3,
  5,
  10,
  20,
] as const;

/**
 * Build the request that changes one field of a category preference.
 *
 * `PUT /api/notifications/preferences` is an upsert over the whole row: any
 * field the request omits is written as NULL, so sending `{category, enabled}`
 * alone silently erases that category's quiet hours and daily cap. Every field
 * of the current item is therefore restated and only `patch` differs.
 *
 * Both preference surfaces build their request here so neither can rediscover
 * that the hard way.
 */
export function notificationPreferenceUpdate(
  current: NotificationPreferenceItem,
  patch: Partial<Omit<UpdateNotificationPreferenceRequest, 'category'>>,
): UpdateNotificationPreferenceRequest {
  const request: UpdateNotificationPreferenceRequest = {
    category: current.category,
    enabled: current.enabled,
  };
  if (current.sub_preferences !== null) request.sub_preferences = current.sub_preferences;
  if (current.quiet_hours_start !== null) request.quiet_hours_start = current.quiet_hours_start;
  if (current.quiet_hours_end !== null) request.quiet_hours_end = current.quiet_hours_end;
  if (current.timezone !== null) request.timezone = current.timezone;
  if (current.max_per_day !== null) request.max_per_day = current.max_per_day;

  for (const [key, value] of Object.entries(patch)) {
    if (value === undefined) {
      // An explicit undefined clears the field: the upsert writes NULL for
      // whatever the request leaves out, which is exactly "no quiet hours".
      delete request[key as keyof UpdateNotificationPreferenceRequest];
    } else {
      Object.assign(request, { [key]: value });
    }
  }
  return request;
}
