// ABOUTME: Shared notification constants and utilities for web and mobile
// ABOUTME: Category metadata (colors, labels, icon names) and time formatting

import type { NotificationCategory } from '@pierre/shared-types';

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
  social: { label: 'Social', color: '#234e40', iconName: 'users' },            // on_primary_fixed_variant
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
  'social',
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
