// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the web half of the shared notification screen → route resolution
// ABOUTME: A notification opens its thread, Home, or a settings pane — or nothing, never the empty chat

import { describe, it, expect } from 'vitest';
import { webNotificationRoute } from '@pierre/shared-constants';

describe('webNotificationRoute', () => {
  it('deep-links an agent message to its conversation thread', () => {
    const data = { screen: 'coach', action: 'chat', id: 'conv-abc-123' };
    expect(webNotificationRoute(data)).toBe('chat/conv-abc-123');
  });

  it('opens the thread an insight was computed in, like an agent message', () => {
    // A fitness improvement fired from the agent's own tool call names the
    // conversation it was answering in, where the agent explained the score.
    const data = { screen: 'coach', action: 'chat', id: 'conv-fitness-1', params: {} };
    expect(webNotificationRoute(data)).toBe('chat/conv-fitness-1');
  });

  it('percent-encodes conversation ids that contain reserved characters', () => {
    const data = { screen: 'coach', id: 'conv/with space' };
    expect(webNotificationRoute(data)).toBe(`chat/${encodeURIComponent('conv/with space')}`);
  });

  it('opens nothing for a coach payload that names no thread', () => {
    // The chat is a list of threads: without one there is nothing to show, and
    // landing on the empty chat was the dead end.
    expect(webNotificationRoute({ screen: 'coach' })).toBeNull();
    expect(webNotificationRoute({ screen: 'coach', action: 'plan' })).toBeNull();
    expect(webNotificationRoute({ screen: 'coach', id: 42 })).toBeNull();
  });

  it('opens Home for a personal record, a weekly summary and a plan update', () => {
    // Home carries the latest activities with their routes and the plan's week.
    expect(webNotificationRoute({ screen: 'activity', id: 'act-1' })).toBe('home');
    expect(webNotificationRoute({ screen: 'activities' })).toBe('home');
    expect(webNotificationRoute({ screen: 'plan' })).toBe('home');
  });

  it('never reads an activity id as a conversation', () => {
    expect(webNotificationRoute({ screen: 'activity', id: 'conv-lookalike' })).toBe('home');
  });

  it('routes a provider notification to the connections pane', () => {
    // `connections` is what a provider reauth and a sync failure emit. The
    // server maps it to the `connections` settings pane, whose web route is the
    // settings section.
    expect(webNotificationRoute({ screen: 'connections', provider: 'whoop' })).toBe(
      'settings/connections',
    );
  });

  it('opens nothing for a row stored under a retired screen', () => {
    // Rows written before the retired Insights tokens left the vocabulary, and
    // the social screen before it, render as information rather than as links.
    expect(webNotificationRoute({ screen: 'stats' })).toBeNull();
    expect(webNotificationRoute({ screen: 'recovery' })).toBeNull();
    expect(webNotificationRoute({ screen: 'settings', action: 'reconnect' })).toBeNull();
    expect(webNotificationRoute({ screen: 'social', id: 'req-1' })).toBeNull();
  });

  it('returns null when the payload names no screen', () => {
    // An insight computed outside any conversation carries no destination.
    expect(webNotificationRoute({ params: { score: '32' } })).toBeNull();
    expect(webNotificationRoute(undefined)).toBeNull();
    expect(webNotificationRoute(null)).toBeNull();
    expect(webNotificationRoute({})).toBeNull();
    // The legacy `route` key is not honoured — only `screen` routes.
    expect(webNotificationRoute({ route: '/somewhere' })).toBeNull();
  });
});
