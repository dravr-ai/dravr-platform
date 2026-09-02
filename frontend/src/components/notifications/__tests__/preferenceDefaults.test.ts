// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the shared merge that fills a category's default when the server stored no override
// ABOUTME: An account with nothing stored rendered an empty preferences screen on both clients

import { describe, it, expect } from 'vitest';
import {
  NOTIFICATION_CATEGORIES,
  defaultNotificationPreference,
  mergeNotificationPreferences,
} from '@pierre/shared-constants';
import type { NotificationPreferenceItem } from '@pierre/shared-types';

describe('notification preference defaults', () => {
  // The endpoint returns overrides, so a new account gets `{"preferences": []}`.
  // Dropping the categories it did not mention left the screen with no rows, no
  // quiet hours and no daily cap, which reads as a broken page.
  it('returns every category at its default when the server stored nothing', () => {
    const rows = mergeNotificationPreferences([]);

    expect(rows.map((r) => r.category)).toEqual([...NOTIFICATION_CATEGORIES]);
    for (const row of rows) {
      // Absent means delivered: `check_suppression` finds no row and lets the
      // notification through, so an unset category shows as on, uncapped and
      // with no quiet hours rather than as off.
      expect(row.enabled).toBe(true);
      expect(row.quiet_hours_start).toBeNull();
      expect(row.quiet_hours_end).toBeNull();
      expect(row.timezone).toBeNull();
      expect(row.max_per_day).toBeNull();
      expect(row.sub_preferences).toBeNull();
    }
  });

  it('lets a stored override win over the default it replaces', () => {
    const stored: NotificationPreferenceItem = {
      category: 'coach',
      enabled: false,
      sub_preferences: null,
      quiet_hours_start: '22:00',
      quiet_hours_end: '07:00',
      timezone: 'America/Toronto',
      max_per_day: 3,
    };

    const rows = mergeNotificationPreferences([stored]);

    expect(rows).toHaveLength(NOTIFICATION_CATEGORIES.length);
    expect(rows.find((r) => r.category === 'coach')).toEqual(stored);
    // Everything else still falls back rather than disappearing behind the one
    // row the athlete happens to have changed.
    expect(rows.find((r) => r.category === 'training')).toEqual(
      defaultNotificationPreference('training'),
    );
  });

  it('appends a category the server knows about and this build does not', () => {
    const unknown = {
      ...defaultNotificationPreference('training'),
      category: 'weather' as NotificationPreferenceItem['category'],
    };

    const rows = mergeNotificationPreferences([unknown]);

    expect(rows).toHaveLength(NOTIFICATION_CATEGORIES.length + 1);
    expect(rows[rows.length - 1].category).toBe('weather');
  });
});
