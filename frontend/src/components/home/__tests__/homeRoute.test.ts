// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Home conversation route — `home/chat/<id>` round-trips and never reads as an activity's subview
// ABOUTME: A malformed escape or another subview names no conversation

import { describe, it, expect } from 'vitest';
import { homeConversationRoute, parseHomeConversation } from '../homeRoute';
import { parseActivitySubview } from '../../activity/activityRoute';

describe('Home conversation route', () => {
  it('round-trips an id that needs escaping', () => {
    const route = homeConversationRoute('conv 1/2');
    expect(route).toBe('home/chat/conv%201%2F2');
    expect(parseHomeConversation(route.slice('home/'.length))).toBe('conv 1/2');
  });

  it('names no conversation for any other subview', () => {
    expect(parseHomeConversation('')).toBeNull();
    expect(parseHomeConversation('chat')).toBeNull();
    expect(parseHomeConversation('chat/')).toBeNull();
    expect(parseHomeConversation('activity/strava/123')).toBeNull();
    expect(parseHomeConversation('chat/%E0%A4%A')).toBeNull();
  });

  it('is never read as an activity, and an activity never as a conversation', () => {
    expect(parseActivitySubview('chat/conv-1')).toBeNull();
    expect(parseHomeConversation('activity/strava/123')).toBeNull();
  });
});
