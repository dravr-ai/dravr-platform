// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the Groups tab's hash subview: a thread, or a thread with its info open, and nothing else
// ABOUTME: The notification a group's link proposal sends opens `chat/<id>/info`; the parse must read it back

import { describe, expect, it } from 'vitest';
import { webNotificationRoute } from '@pierre/shared-constants';
import { parseChatSubview } from '../chatRoute';

describe('parseChatSubview', () => {
  it('reads a thread', () => {
    expect(parseChatSubview('conv-1')).toEqual({ conversationId: 'conv-1', openInfo: false });
  });

  it('reads a thread with its info open', () => {
    expect(parseChatSubview('conv-1/info')).toEqual({ conversationId: 'conv-1', openInfo: true });
  });

  it('decodes an escaped id, slash included', () => {
    const id = 'conv/with space';
    expect(parseChatSubview(`${encodeURIComponent(id)}/info`)).toEqual({
      conversationId: id,
      openInfo: true,
    });
  });

  it('reads back the route a group link proposal opens', () => {
    const route = webNotificationRoute({ screen: 'group_info', id: 'room-7' });
    expect(route).toBe('chat/room-7/info');
    expect(parseChatSubview(route!.slice('chat/'.length))).toEqual({
      conversationId: 'room-7',
      openInfo: true,
    });
  });

  it('names no thread for an empty, unknown or malformed subview', () => {
    expect(parseChatSubview('')).toBeNull();
    expect(parseChatSubview('/info')).toBeNull();
    expect(parseChatSubview('conv-1/members')).toBeNull();
    expect(parseChatSubview('conv-1/info/more')).toBeNull();
    expect(parseChatSubview('%E0%A4%A')).toBeNull();
  });
});
