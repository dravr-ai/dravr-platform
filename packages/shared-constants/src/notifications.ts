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
 * Colors are aligned with the Pierre design system pillar colors
 * where applicable (activity=training, recovery=recovery).
 */
export const NOTIFICATION_CATEGORY_META: Record<NotificationCategory, NotificationCategoryMeta> = {
  training: { label: 'Training', color: '#4ADE80', iconName: 'dumbbell' },
  recovery: { label: 'Recovery', color: '#818CF8', iconName: 'heart' },
  social: { label: 'Social', color: '#818CF8', iconName: 'users' },
  coach: { label: 'Coach', color: '#38BDF8', iconName: 'message-circle' },
  achievement: { label: 'Achievements', color: '#FBBF24', iconName: 'trophy' },
  system: { label: 'System', color: '#94A3B8', iconName: 'settings' },
  ai: { label: 'AI Insights', color: '#22D3EE', iconName: 'brain' },
  reminders: { label: 'Reminders', color: '#F472B6', iconName: 'clock' },
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
