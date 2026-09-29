// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the mobile half of the shared notification screen → route resolution
// ABOUTME: A notification opens its thread, Home, or a settings screen — or nothing, never the empty chat list

import { mobileNotificationTarget } from '@pierre/shared-constants';

// A conversation opens the thread route, which sits beside the tabs.
const THREAD_ROUTE = '/(app)/chat/[conversationId]';
const HOME_ROUTE = '/(app)/(tabs)/(home)';
const CONNECTIONS_ROUTE = '/(app)/(tabs)/(settings)/connections';

describe('mobileNotificationTarget', () => {
  it('deep-links an agent message to its thread', () => {
    const data = { screen: 'coach', action: 'chat', id: 'conv-abc-123' };
    expect(mobileNotificationTarget(data)).toEqual({
      pathname: THREAD_ROUTE,
      params: { conversationId: 'conv-abc-123' },
    });
  });

  it('opens the thread an insight was computed in, like an agent message', () => {
    const data = { screen: 'coach', action: 'chat', id: 'conv-fitness-1', params: {} };
    expect(mobileNotificationTarget(data)).toEqual({
      pathname: THREAD_ROUTE,
      params: { conversationId: 'conv-fitness-1' },
    });
  });

  it('opens nothing for a coach payload that names no thread', () => {
    // The chat tab is a list of threads: without one there is nothing to show.
    expect(mobileNotificationTarget({ screen: 'coach' })).toBeNull();
    expect(mobileNotificationTarget({ screen: 'coach', action: 'plan' })).toBeNull();
    expect(mobileNotificationTarget({ screen: 'coach', id: 42 })).toBeNull();
  });

  it('opens Home for a personal record and a weekly summary', () => {
    // The `id` on a personal record is the activity, so no param rides along.
    expect(mobileNotificationTarget({ screen: 'activity', id: 'act-1' })).toEqual({
      pathname: HOME_ROUTE,
    });
    expect(mobileNotificationTarget({ screen: 'activities' })).toEqual({ pathname: HOME_ROUTE });
  });

  it('routes a provider notification to the connections screen', () => {
    // `connections` is what a provider reauth and a sync failure emit.
    expect(mobileNotificationTarget({ screen: 'connections', provider: 'whoop' })).toEqual({
      pathname: CONNECTIONS_ROUTE,
    });
  });

  it('opens nothing for a row stored under a retired screen', () => {
    expect(mobileNotificationTarget({ screen: 'stats' })).toBeNull();
    expect(mobileNotificationTarget({ screen: 'recovery' })).toBeNull();
    expect(mobileNotificationTarget({ screen: 'settings', action: 'reconnect' })).toBeNull();
    // A plan update claimed a change nobody made; the token went with it.
    expect(mobileNotificationTarget({ screen: 'plan' })).toBeNull();
    expect(
      mobileNotificationTarget({ screen: 'social', action: 'friend_request', id: 'req-1' }),
    ).toBeNull();
  });

  it('returns null when the payload names no screen', () => {
    // An insight computed outside any conversation carries no destination.
    expect(mobileNotificationTarget({ params: { score: '32' } })).toBeNull();
    expect(mobileNotificationTarget(undefined)).toBeNull();
    expect(mobileNotificationTarget(null)).toBeNull();
    expect(mobileNotificationTarget({})).toBeNull();
    // The legacy `route` key is not honoured — only `screen` routes.
    expect(mobileNotificationTarget({ route: '/somewhere' })).toBeNull();
  });
});
