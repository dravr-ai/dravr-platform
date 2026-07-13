// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the notification screen → expo-router target mapping
// ABOUTME: Regression coverage for the 2026-07-13 coach-Reply deep-link fix

import { resolveNotificationTarget } from '../navigation';

const CHAT_ROUTE = '/(app)/(tabs)/(chat)';
const INSIGHTS_ROUTE = '/(app)/(tabs)/(social)';
const SETTINGS_ROUTE = '/(app)/(tabs)/(settings)';

describe('resolveNotificationTarget', () => {
  it('deep-links a coach message to its conversation in the chat tab', () => {
    // dravr-commere trigger_coach_message payload shape.
    const data = { screen: 'coach', action: 'chat', id: 'conv-abc-123' };
    expect(resolveNotificationTarget(data)).toEqual({
      pathname: CHAT_ROUTE,
      params: { conversationId: 'conv-abc-123' },
    });
  });

  it('deep-links from the Reply action button the same way', () => {
    const data = { screen: 'coach', action: 'chat', id: 'conv-abc-123' };
    expect(resolveNotificationTarget(data, 'reply')).toEqual({
      pathname: CHAT_ROUTE,
      params: { conversationId: 'conv-abc-123' },
    });
  });

  it('falls back to the bare chat tab when a coach payload carries no id', () => {
    expect(resolveNotificationTarget({ screen: 'coach' })).toEqual({ pathname: CHAT_ROUTE });
    expect(resolveNotificationTarget({ screen: 'coach', id: 42 })).toEqual({ pathname: CHAT_ROUTE });
  });

  it('routes recovery / activity / stats / social to the Insights tab', () => {
    for (const screen of ['recovery', 'activity', 'activities', 'stats', 'social']) {
      expect(resolveNotificationTarget({ screen, id: 'ignored' })).toEqual({
        pathname: INSIGHTS_ROUTE,
      });
    }
  });

  it('routes settings deep links to the settings tab', () => {
    expect(resolveNotificationTarget({ screen: 'settings' })).toEqual({ pathname: SETTINGS_ROUTE });
  });

  it('resolves via the action id when the payload has no usable screen', () => {
    expect(resolveNotificationTarget({}, 'settings')).toEqual({ pathname: SETTINGS_ROUTE });
  });

  it('returns null when neither screen nor action id maps anywhere', () => {
    expect(resolveNotificationTarget(undefined)).toBeNull();
    expect(resolveNotificationTarget(null)).toBeNull();
    expect(resolveNotificationTarget({})).toBeNull();
    expect(resolveNotificationTarget({ screen: 'unknown' }, 'reply')).toBeNull();
    // The legacy `route` key is no longer honoured — only `screen` routes.
    expect(resolveNotificationTarget({ route: '/somewhere' })).toBeNull();
  });
});
