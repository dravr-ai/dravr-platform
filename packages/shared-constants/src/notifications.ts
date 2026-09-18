// ABOUTME: Shared notification constants and utilities for web and mobile
// ABOUTME: Category metadata (scheme-aware hues, labels, icon names), time formatting, preference merging

import type { ColorScheme } from './design-system';
import type {
  NotificationCategory,
  NotificationPreferenceItem,
  UpdateNotificationPreferenceRequest,
} from '@pierre/shared-types';

/** A client's `t`. This module has no locale of its own. */
type Translate = (key: string, params?: Record<string, string | number>) => string;

/** Category display metadata shared across all frontends */
export interface NotificationCategoryMeta {
  /** Corpus key for the category name; the client resolves it. */
  labelKey: string;
  /** Lucide icon name (both web and mobile use lucide) */
  iconName: string;
}

/** One hex per category, for one scheme. */
export type NotificationCategoryColors = Record<NotificationCategory, string>;

/**
 * Canonical category metadata for notification rendering.
 *
 * Notification categories (training/recovery/coach/achievement/system/ai/
 * reminders) are their own taxonomy, not the four fitness pillars
 * (activity/nutrition/recovery/mobility) — `training` and `recovery` share a
 * name with a pillar but not a source: three of the seven categories here
 * (coach, achievement, ai, reminders) have no pillar equivalent at all. These
 * are this taxonomy's own swatches — not pulled from `PILLAR_COLORS`, and not
 * meant to be — but they follow its scheme-paired shape, for the reason that
 * shape exists.
 */
// `labelKey` rather than `label`: this metadata is shared with mobile and
// rendered on an athlete screen, so the words come from the corpus. They
// shipped as English strings here, which is why no scan of the frontend
// components ever saw them.
export const NOTIFICATION_CATEGORY_META: Record<NotificationCategory, NotificationCategoryMeta> = {
  training: { labelKey: 'notifPrefs.catTraining', iconName: 'dumbbell' },
  recovery: { labelKey: 'notifPrefs.catRecovery', iconName: 'heart' },
  coach: { labelKey: 'notifPrefs.catAgent', iconName: 'message-circle' },
  achievement: { labelKey: 'notifPrefs.catAchievement', iconName: 'trophy' },
  system: { labelKey: 'notifPrefs.catSystem', iconName: 'settings' },
  ai: { labelKey: 'notifPrefs.catAi', iconName: 'brain' },
  reminders: { labelKey: 'notifPrefs.catReminders', iconName: 'clock' },
} as const;

/**
 * The category hues, per scheme.
 *
 * These began as one flat hex per category, described as "chosen to read on
 * the app's `surface` in either scheme". That held while the only consumer was
 * the web panel's 8px dot. Mobile then painted the category *word* in the same
 * hex, and on the dark canvas (`#11130f`) the near-black forest greens landed
 * at 1.13:1 (coach) and 1.50:1 (ai) — the label was the canvas. Every value
 * here now clears 4.5:1 against its own scheme's surface; `recovery` and
 * `system` moved on the light side too, where they sat at 4.34 and 4.25.
 *
 * Paired like `PILLARS` and read the same way: index by the athlete's scheme
 * rather than reaching for one half, because a consumer that takes one and
 * hard-codes the other is how a palette drifts.
 */
export const NOTIFICATION_CATEGORY_COLORS: Record<ColorScheme, NotificationCategoryColors> = {
  light: {
    training: '#3c6658',    // deep sage
    recovery: '#4e6c74',    // muted slate
    coach: '#00241a',       // near-black forest green
    achievement: '#896429', // warm bronze
    system: '#5f6762',      // neutral grey
    ai: '#0d3b2e',          // dark forest green
    reminders: '#7a4d5e',   // aged rose
  },
  dark: {
    training: '#7fae9b',    // deep sage, lifted for dark surfaces
    recovery: '#8fb3bc',    // muted slate, lifted
    coach: '#5cc9a3',       // forest green, lifted
    achievement: '#d4a95e', // warm bronze, lifted
    system: '#a6aea8',      // neutral grey, lifted
    ai: '#48b391',          // dark forest green, lifted
    reminders: '#c896a6',   // aged rose, lifted
  },
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
 * What a category means before the athlete has ever touched it.
 *
 * The dispatcher decides this, not the screen: `check_suppression` looks the
 * category up among the stored rows and, finding none, delivers the
 * notification. Absent therefore means enabled, with no quiet hours, no zone
 * and no per-day cap of the athlete's own.
 */
export function defaultNotificationPreference(
  category: NotificationCategory,
): NotificationPreferenceItem {
  return {
    category,
    enabled: true,
    sub_preferences: null,
    quiet_hours_start: null,
    quiet_hours_end: null,
    timezone: null,
    max_per_day: null,
  };
}

/**
 * Every category's current setting, defaults filled in.
 *
 * `GET /api/notifications/preferences` returns OVERRIDES: the table holds a row
 * only once something has been changed, so an account that has never opened the
 * screen gets `{"preferences": []}`. Both surfaces looked each known category
 * up in that response and kept only what came back, which for that account is
 * nothing — no rows, no quiet hours, no daily cap, on a screen whose entire
 * content is that list.
 *
 * One function, because both clients ask the same question and a second copy is
 * a second answer. Anything the server returns that this build does not know is
 * appended rather than dropped, so a category added server-side still reaches
 * the athlete.
 */
export function mergeNotificationPreferences(
  stored: readonly NotificationPreferenceItem[],
): NotificationPreferenceItem[] {
  const byCategory = new Map<string, NotificationPreferenceItem>(
    stored.map((p) => [p.category, p]),
  );
  const known = NOTIFICATION_CATEGORIES.map(
    (category) => byCategory.get(category) ?? defaultNotificationPreference(category),
  );
  const extra = stored.filter((p) => !NOTIFICATION_CATEGORIES.includes(p.category));
  return [...known, ...extra];
}

/**
 * Format a timestamp as a relative time string, in the athlete's language.
 *
 * Shared between web and mobile. It built `Just now` and `5m ago` by hand,
 * which both notification centres rendered verbatim under French chrome, and
 * the first fix reached for `Intl.RelativeTimeFormat` — which the phone's
 * JavaScript engine does not ship, so the notification list crashed with
 * "Cannot read property 'prototype' of undefined" (carnet#227). The catalogue
 * carries the wording instead: it exists on every runtime, and the four
 * phrasings are short enough that no locale needs a plural rule.
 */
export function formatNotificationTime(dateStr: string, t: Translate): string {
  const elapsedMs = Date.now() - new Date(dateStr).getTime();
  const minutes = Math.floor(elapsedMs / 60_000);
  const hours = Math.floor(elapsedMs / 3_600_000);
  const days = Math.floor(elapsedMs / 86_400_000);

  // Beyond a week a date reads better than "8 days ago", which is the call the
  // hand-rolled version made too.
  if (days >= 7) return new Date(dateStr).toLocaleDateString();
  if (minutes < 1) return t('notifications.justNow');
  if (hours < 1) return t('notifications.minutesAgo', { count: minutes });
  if (days < 1) return t('notifications.hoursAgo', { count: hours });
  return t('notifications.daysAgo', { count: days });
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
