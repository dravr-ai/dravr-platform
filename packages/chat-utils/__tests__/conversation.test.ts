// ABOUTME: Tests for cross-surface conversation helpers — tool-row filtering + the channel label
// ABOUTME: Guards that tool_call/tool_result plumbing never reaches the user-facing thread

import { describe, it, expect } from 'vitest';
import type { Message } from '@pierre/shared-types';
import {
  isToolPlumbingMessage,
  filterDisplayMessages,
  stripToolScaffolding,
  resolveChannelOrigin,
} from '../src/conversation';

function msg(role: Message['role'], content = ''): Message {
  return { id: `${role}-${content.length}`, role, content, created_at: '2026-06-19T00:00:00Z' };
}

describe('isToolPlumbingMessage', () => {
  it('flags tool_call and tool_result rows', () => {
    expect(isToolPlumbingMessage(msg('tool_call'))).toBe(true);
    expect(isToolPlumbingMessage(msg('tool_result'))).toBe(true);
  });

  it('does not flag conversational roles', () => {
    expect(isToolPlumbingMessage(msg('user'))).toBe(false);
    expect(isToolPlumbingMessage(msg('assistant'))).toBe(false);
    expect(isToolPlumbingMessage(msg('system'))).toBe(false);
  });
});

describe('filterDisplayMessages', () => {
  it('drops tool_call / tool_result rows but keeps user + assistant', () => {
    const messages: Message[] = [
      msg('user', 'how was my run?'),
      msg('tool_call', '<tool_call>get_activities</tool_call>'),
      msg('tool_result', '<tool_result>{"distance":5000}</tool_result>'),
      msg('assistant', 'Your run was 5km.'),
    ];

    const visible = filterDisplayMessages(messages);

    expect(visible.map((m) => m.role)).toEqual(['user', 'assistant']);
    // No raw scaffolding survives into the displayed list.
    expect(visible.some((m) => m.content.includes('<tool_'))).toBe(false);
  });

  it('preserves order and is a no-op when there is no plumbing', () => {
    const messages: Message[] = [msg('user', 'hi'), msg('assistant', 'hello')];
    expect(filterDisplayMessages(messages)).toEqual(messages);
  });

  it('keeps system messages (not plumbing)', () => {
    const messages: Message[] = [msg('system', 'You are a coach.'), msg('user', 'hi')];
    expect(filterDisplayMessages(messages).map((m) => m.role)).toEqual(['system', 'user']);
  });
});

describe('stripToolScaffolding', () => {
  it('removes embedded tool_call / tool_result blocks from displayed content', () => {
    const content =
      'Here is your summary. <tool_call name="get_activities">{}</tool_call> Final answer.';
    const stripped = stripToolScaffolding(content);
    expect(stripped).not.toMatch(/tool_call/);
    expect(stripped).toContain('Here is your summary.');
    expect(stripped).toContain('Final answer.');
  });

  it('removes a multi-line tool_result block', () => {
    const content = 'Intro.\n<tool_result>\n{\n  "ok": true\n}\n</tool_result>\nOutro.';
    const stripped = stripToolScaffolding(content);
    expect(stripped).not.toMatch(/tool_result/);
    expect(stripped).not.toMatch(/ok/);
    expect(stripped).toContain('Intro.');
    expect(stripped).toContain('Outro.');
  });

  it('removes a stray unclosed opening tag', () => {
    expect(stripToolScaffolding('Partial answer <tool_call>')).toBe('Partial answer');
  });

  it('returns clean content unchanged', () => {
    expect(stripToolScaffolding('Just a normal reply.')).toBe('Just a normal reply.');
  });
});

describe('resolveChannelOrigin', () => {
  it('reads the channel from the durable column, whatever the title says', () => {
    expect(resolveChannelOrigin({ channel_type: 'telegram' })).toEqual({
      channel: 'telegram',
      label: 'Telegram',
    });
    expect(resolveChannelOrigin({ channel_type: ' WhatsApp ' })).toEqual({
      channel: 'whatsapp',
      label: 'WhatsApp',
    });
  });

  it('applies casing overrides for known channels', () => {
    expect(resolveChannelOrigin({ channel_type: 'imessage' })?.label).toBe('iMessage');
    expect(resolveChannelOrigin({ channel_type: 'sms' })?.label).toBe('SMS');
    expect(resolveChannelOrigin({ channel_type: 'discord' })?.label).toBe('Discord');
  });

  it('returns null for in-app chats (web/mobile) and an absent column', () => {
    expect(resolveChannelOrigin({ channel_type: 'web' })).toBeNull();
    expect(resolveChannelOrigin({ channel_type: 'mobile' })).toBeNull();
    expect(resolveChannelOrigin({ channel_type: '' })).toBeNull();
    expect(resolveChannelOrigin({ channel_type: null })).toBeNull();
    expect(resolveChannelOrigin({})).toBeNull();
  });
});
