// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the shared notification category list to the set the server can store
// ABOUTME: Turns red if a toggle appears without its metadata, or if a retired category returns

import { NOTIFICATION_CATEGORIES, NOTIFICATION_CATEGORY_META } from '@pierre/shared-constants';

describe('notification categories', () => {
  // dravr-commere `NotificationCategory::all()`, in display order. Social left
  // with commere 0.2.0 when the Chat-First Cutover retired Insights and
  // Friends; a toggle for it would mute a category the server cannot store.
  it('offers exactly the categories the server stores, each with metadata', () => {
    expect(NOTIFICATION_CATEGORIES).toEqual([
      'training',
      'recovery',
      'coach',
      'achievement',
      'system',
      'ai',
      'reminders',
    ]);
    expect(Object.keys(NOTIFICATION_CATEGORY_META).sort()).toEqual(
      [...NOTIFICATION_CATEGORIES].sort(),
    );
  });
});
