// ABOUTME: Tests for a thread read in windows — joining the windows read back to the newest, and the per-row channel tag
// ABOUTME: Guards that a row two windows hold is drawn once and that only an athlete's messaging-app row is tagged

import { describe, it, expect } from 'vitest';
import type { Message } from '@pierre/shared-types';
import {
  hasEarlierMessages,
  joinMessageWindows,
  messageChannelOrigin,
  prependWindow,
} from '../src/message-window';

function row(id: string, extra: Partial<Message> = {}): Message {
  return { id, role: 'user', content: id, created_at: '2026-10-20T00:00:00Z', ...extra };
}

describe('joinMessageWindows', () => {
  it('is the newest window alone before anything was read back', () => {
    expect(joinMessageWindows(undefined, [row('c')]).map((m) => m.id)).toEqual(['c']);
    expect(joinMessageWindows([], [row('c')]).map((m) => m.id)).toEqual(['c']);
  });

  it('puts the windows read back before the newest one', () => {
    const joined = joinMessageWindows([row('a'), row('b')], [row('c'), row('d')]);
    expect(joined.map((m) => m.id)).toEqual(['a', 'b', 'c', 'd']);
  });

  it('draws a row both hold once, from the newest window', () => {
    const joined = joinMessageWindows(
      [row('a'), row('b', { content: 'old' })],
      [row('b', { content: 'new' }), row('c')],
    );
    expect(joined.map((m) => m.id)).toEqual(['a', 'b', 'c']);
    expect(joined[1].content).toBe('new');
  });
});

describe('hasEarlierMessages', () => {
  it('reads the newest window until something was read back', () => {
    expect(hasEarlierMessages(undefined, true)).toBe(true);
    expect(hasEarlierMessages(undefined, false)).toBe(false);
    expect(hasEarlierMessages(undefined, undefined)).toBe(false);
  });

  it('reads the oldest window once one was read back', () => {
    expect(hasEarlierMessages({ messages: [row('a')], hasMore: false }, true)).toBe(false);
    expect(hasEarlierMessages({ messages: [row('a')], hasMore: true }, false)).toBe(true);
  });
});

describe('prependWindow', () => {
  it('puts an older window before every row held and keeps its has_more', () => {
    const first = prependWindow([row('c'), row('d')], { messages: [row('a'), row('b')], has_more: false });
    expect(first.messages.map((m) => m.id)).toEqual(['a', 'b', 'c', 'd']);
    expect(first.hasMore).toBe(false);
  });

  it('never holds a row twice', () => {
    const first = prependWindow([row('b'), row('c')], { messages: [row('a'), row('b')], has_more: true });
    expect(first.messages.map((m) => m.id)).toEqual(['a', 'b', 'c']);
  });

  it('keeps the rows between the windows read back and a newest window that moved on', () => {
    const earlier = prependWindow([row('c'), row('d')], { messages: [row('a'), row('b')] });
    // Rows landed: the newest window now starts after `c`.
    const joined = joinMessageWindows(earlier.messages, [row('d'), row('e')]);
    expect(joined.map((m) => m.id)).toEqual(['a', 'b', 'c', 'd', 'e']);
  });
});

describe('messageChannelOrigin', () => {
  it('tags an athlete row that came over a messaging app', () => {
    expect(messageChannelOrigin(row('a', { channel_type: 'telegram' }))).toEqual({
      channel: 'telegram',
      label: 'Telegram',
    });
    expect(messageChannelOrigin(row('a', { channel_type: 'whatsapp' }))?.label).toBe('WhatsApp');
  });

  it('leaves app rows, untagged rows and the agent rows alone', () => {
    expect(messageChannelOrigin(row('a', { channel_type: 'web' }))).toBeNull();
    expect(messageChannelOrigin(row('a', { channel_type: 'mobile' }))).toBeNull();
    expect(messageChannelOrigin(row('a'))).toBeNull();
    expect(
      messageChannelOrigin(row('a', { role: 'assistant', channel_type: 'telegram' })),
    ).toBeNull();
  });
});
