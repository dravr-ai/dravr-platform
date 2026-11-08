// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the shared notification category list to the set the server can store
// ABOUTME: Turns red if a toggle appears without its metadata, or if a retired category returns

import { NOTIFICATION_CATEGORIES, NOTIFICATION_CATEGORY_META } from '@pierre/shared-constants';

describe('notification categories', () => {
  // In display order. Social left with commere 0.2.0 when the Chat-First Cutover
  // retired Insights and Friends; AI Insights and Reminders left on carnet#1436,
  // the first having no writer and the second only a route no client called.
  it('offers exactly the live categories, each with metadata', () => {
    expect(NOTIFICATION_CATEGORIES).toEqual([
      'training',
      'recovery',
      'coach',
      'achievement',
      'system',
    ]);
    expect(Object.keys(NOTIFICATION_CATEGORY_META).sort()).toEqual(
      [...NOTIFICATION_CATEGORIES].sort(),
    );
  });
});
