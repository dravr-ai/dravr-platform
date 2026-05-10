// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the notification screen → tab id mapping
// ABOUTME: Regression coverage for the 2026-05-09 web sweep where Recovery rows didn't navigate

import { describe, it, expect } from 'vitest';
import { mapScreenToTab } from '../navigation';

describe('mapScreenToTab', () => {
  it('routes recovery + activity + activities + stats to the Insights tab', () => {
    expect(mapScreenToTab('recovery')).toBe('insights');
    expect(mapScreenToTab('activity')).toBe('insights');
    expect(mapScreenToTab('activities')).toBe('insights');
    expect(mapScreenToTab('stats')).toBe('insights');
  });

  it('routes social to the Insights tab (friends sub-view is set by the caller)', () => {
    expect(mapScreenToTab('social')).toBe('insights');
  });

  it('routes coach back to the chat tab', () => {
    expect(mapScreenToTab('coach')).toBe('chat');
  });

  it('routes settings deep links to the settings tab', () => {
    expect(mapScreenToTab('settings')).toBe('settings');
  });

  it('returns null for unknown screen names so callers do not strand the user', () => {
    expect(mapScreenToTab('unknown_screen')).toBeNull();
    expect(mapScreenToTab('')).toBeNull();
    expect(mapScreenToTab(undefined)).toBeNull();
    expect(mapScreenToTab(null)).toBeNull();
  });
});
