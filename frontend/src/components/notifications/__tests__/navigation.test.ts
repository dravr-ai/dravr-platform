// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the web half of the shared notification screen → route resolution
// ABOUTME: Regression coverage for the 2026-05-09 web sweep where Recovery rows didn't navigate

import { describe, it, expect } from 'vitest';
import { webNotificationRoute } from '@pierre/shared-constants';

describe('webNotificationRoute', () => {
  // The Insights surface was retired by the Chat-First Cutover. Every training
  // deep-link now opens the chat, where the coach reads those numbers to the
  // athlete; asserted screen by screen so a regression names the screen.
  it('routes an activity sync to the chat', () => {
    expect(webNotificationRoute({ screen: 'activity', id: 'act-1' })).toBe('chat');
  });

  it('routes the activity list to the chat', () => {
    expect(webNotificationRoute({ screen: 'activities' })).toBe('chat');
  });

  it('routes a recovery alert to the chat', () => {
    expect(webNotificationRoute({ screen: 'recovery' })).toBe('chat');
  });

  it('routes a training-load alert to the chat', () => {
    expect(webNotificationRoute({ screen: 'stats' })).toBe('chat');
  });

  it('no longer routes the retired social screen anywhere', () => {
    // Nothing emits `social` since friends and the feed were deleted; a row
    // persisted before the cutover marks itself read and stays put.
    expect(webNotificationRoute({ screen: 'social' })).toBeNull();
    expect(webNotificationRoute({ screen: 'social', action: 'friend_request', id: 'req-1' })).toBeNull();
  });

  it('routes settings deep links to the settings tab', () => {
    expect(webNotificationRoute({ screen: 'settings' })).toBe('settings');
  });

  it('routes a provider-reauth notification to Data Providers', () => {
    // `connections` is what pierre-tool-runtime emits on provider_needs_reauth.
    // Neither client's hand-written map handled it, so the tap went nowhere.
    expect(webNotificationRoute({ screen: 'connections', provider: 'whoop' })).toBe(
      'data-providers',
    );
  });

  it('deep-links a coach message to its conversation thread', () => {
    // dravr-commere trigger_coach_message payload shape.
    const data = { screen: 'coach', action: 'chat', id: 'conv-abc-123' };
    expect(webNotificationRoute(data)).toBe('chat/conv-abc-123');
  });

  it('deep-links from the Reply action button the same way', () => {
    const data = { screen: 'coach', action: 'chat', id: 'conv-abc-123' };
    expect(webNotificationRoute(data, 'reply')).toBe('chat/conv-abc-123');
  });

  it('percent-encodes conversation ids that contain reserved characters', () => {
    const data = { screen: 'coach', id: 'conv/with space' };
    expect(webNotificationRoute(data)).toBe(`chat/${encodeURIComponent('conv/with space')}`);
  });

  it('falls back to the bare chat tab when a coach payload carries no id', () => {
    expect(webNotificationRoute({ screen: 'coach' })).toBe('chat');
    expect(webNotificationRoute({ screen: 'coach', id: 42 })).toBe('chat');
  });

  it('ignores a conversation id on a screen that is not the coach thread', () => {
    // A recovery alert lands on the chat surface but names no conversation:
    // its `id` is the alert's own subject, not a thread to reopen.
    expect(webNotificationRoute({ screen: 'recovery', id: 'ignored' })).toBe('chat');
  });

  it('resolves via the action id when the payload has no usable screen', () => {
    expect(webNotificationRoute({}, 'settings')).toBe('settings');
  });

  it('returns null when neither screen nor action id maps anywhere', () => {
    expect(webNotificationRoute(undefined)).toBeNull();
    expect(webNotificationRoute(null)).toBeNull();
    expect(webNotificationRoute({})).toBeNull();
    expect(webNotificationRoute({ screen: 'unknown_screen' }, 'reply')).toBeNull();
    // The legacy `route` key is not honoured — only `screen` routes.
    expect(webNotificationRoute({ route: '/somewhere' })).toBeNull();
  });
});
