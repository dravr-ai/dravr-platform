// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the mobile half of the shared notification screen → route resolution
// ABOUTME: Regression coverage for the 2026-07-13 coach-Reply deep-link fix

import { mobileNotificationTarget } from '@pierre/shared-constants';

const CHAT_ROUTE = '/(app)/(tabs)/(chat)';
const INSIGHTS_ROUTE = '/(app)/(tabs)/(social)';
const PROFILE_ROUTE = '/(app)/(tabs)/(settings)/profile';
const CONNECTIONS_ROUTE = '/(app)/(tabs)/(settings)/connections';

describe('mobileNotificationTarget', () => {
  it('deep-links a coach message to its conversation in the chat tab', () => {
    // dravr-commere trigger_coach_message payload shape.
    const data = { screen: 'coach', action: 'chat', id: 'conv-abc-123' };
    expect(mobileNotificationTarget(data)).toEqual({
      pathname: CHAT_ROUTE,
      params: { conversationId: 'conv-abc-123' },
    });
  });

  it('deep-links from the Reply action button the same way', () => {
    const data = { screen: 'coach', action: 'chat', id: 'conv-abc-123' };
    expect(mobileNotificationTarget(data, 'reply')).toEqual({
      pathname: CHAT_ROUTE,
      params: { conversationId: 'conv-abc-123' },
    });
  });

  it('falls back to the bare chat tab when a coach payload carries no id', () => {
    expect(mobileNotificationTarget({ screen: 'coach' })).toEqual({ pathname: CHAT_ROUTE });
    expect(mobileNotificationTarget({ screen: 'coach', id: 42 })).toEqual({ pathname: CHAT_ROUTE });
  });

  it('routes recovery / activity / activities / stats / social to the Insights tab', () => {
    for (const screen of ['recovery', 'activity', 'activities', 'stats', 'social']) {
      expect(mobileNotificationTarget({ screen, id: 'ignored' })).toEqual({
        pathname: INSIGHTS_ROUTE,
      });
    }
  });

  it('routes settings deep links to the settings surface', () => {
    expect(mobileNotificationTarget({ screen: 'settings' })).toEqual({ pathname: PROFILE_ROUTE });
  });

  it('routes a provider-reauth notification to the connections screen', () => {
    // `connections` is what pierre-tool-runtime emits on provider_needs_reauth.
    // Neither client's hand-written map handled it, so the tap went nowhere.
    expect(mobileNotificationTarget({ screen: 'connections', provider: 'whoop' })).toEqual({
      pathname: CONNECTIONS_ROUTE,
    });
  });

  it('resolves via the action id when the payload has no usable screen', () => {
    expect(mobileNotificationTarget({}, 'settings')).toEqual({ pathname: PROFILE_ROUTE });
  });

  it('returns null when neither screen nor action id maps anywhere', () => {
    expect(mobileNotificationTarget(undefined)).toBeNull();
    expect(mobileNotificationTarget(null)).toBeNull();
    expect(mobileNotificationTarget({})).toBeNull();
    expect(mobileNotificationTarget({ screen: 'unknown_screen' }, 'reply')).toBeNull();
    // The legacy `route` key is not honoured — only `screen` routes.
    expect(mobileNotificationTarget({ route: '/somewhere' })).toBeNull();
  });
});
