// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the notification screen → tab id mapping
// ABOUTME: Regression coverage for the 2026-05-09 web sweep where Recovery rows didn't navigate

import { describe, it, expect } from 'vitest';
import { mapScreenToTab, resolveNotificationRoute } from '../navigation';

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

describe('resolveNotificationRoute', () => {
  it('deep-links a coach message to its conversation thread', () => {
    // dravr-commere trigger_coach_message payload shape.
    const data = { screen: 'coach', action: 'chat', id: 'conv-abc-123' };
    expect(resolveNotificationRoute(data)).toBe('chat/conv-abc-123');
  });

  it('deep-links from the Reply action button the same way', () => {
    const data = { screen: 'coach', action: 'chat', id: 'conv-abc-123' };
    expect(resolveNotificationRoute(data, 'reply')).toBe('chat/conv-abc-123');
  });

  it('percent-encodes conversation ids that contain reserved characters', () => {
    const data = { screen: 'coach', id: 'conv/with space' };
    expect(resolveNotificationRoute(data)).toBe(`chat/${encodeURIComponent('conv/with space')}`);
  });

  it('falls back to the bare chat tab when a coach payload carries no id', () => {
    expect(resolveNotificationRoute({ screen: 'coach' })).toBe('chat');
    expect(resolveNotificationRoute({ screen: 'coach', id: 42 })).toBe('chat');
  });

  it('routes non-coach screens to their tab without a sub-view', () => {
    expect(resolveNotificationRoute({ screen: 'recovery', id: 'ignored' })).toBe('insights');
    expect(resolveNotificationRoute({ screen: 'settings' })).toBe('settings');
  });

  it('resolves via the action id when the payload has no usable screen', () => {
    expect(resolveNotificationRoute({}, 'settings')).toBe('settings');
  });

  it('returns null when neither screen nor action id maps anywhere', () => {
    expect(resolveNotificationRoute(undefined)).toBeNull();
    expect(resolveNotificationRoute({})).toBeNull();
    expect(resolveNotificationRoute({ screen: 'unknown' }, 'reply')).toBeNull();
  });
});
