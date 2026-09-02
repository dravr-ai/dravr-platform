// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for one conversation-list row — the anatomy the shared row model is drawn with
// ABOUTME: Avatar slot, kind glyph, bold-when-unread title, preview, time, count badge, mention badge, swipe actions

import React from 'react';
import { render, fireEvent } from '@testing-library/react-native';
import { buildConversationRow, type ConversationRowModel } from '@pierre/chat-utils';

/** The words the row cannot spell itself, as the English client resolves them. */
const LABELS = { locale: 'en-US', you: 'You', coach: 'Coach', untitled: 'Untitled chat' };
import type { Conversation } from '@pierre/shared-types';
import { ConversationRow, previewMentionsSomeone } from '../src/screens/conversations/ConversationRow';

const NOW = new Date('2026-08-26T18:00:00');

function conversation(overrides: Partial<Conversation> = {}): Conversation {
  return {
    id: 'conv-1',
    title: 'Tempo Tuesday',
    coach_id: null,
    message_count: 5,
    unread_count: 0,
    created_at: '2026-08-20T10:00:00Z',
    updated_at: '2026-08-26T09:50:00Z',
    ...overrides,
  };
}

function row(overrides: Partial<Conversation> = {}): ConversationRowModel {
  return buildConversationRow(conversation(overrides), LABELS, NOW);
}

function renderRow(model: ConversationRowModel) {
  const handlers = {
    onPress: jest.fn(),
    onLongPress: jest.fn(),
    onMarkUnread: jest.fn(),
    onDelete: jest.fn(),
  };
  const view = render(<ConversationRow row={model} {...handlers} />);
  return { ...view, handlers };
}

describe('ConversationRow', () => {
  it('draws the initials avatar, the title, the preview and the relative time', () => {
    const model = row({
      last_message: { preview: 'Easy Thursday, then the long run', role: 'assistant', created_at: '2026-08-26T09:50:00' },
    });
    const { getByTestId, queryByTestId } = renderRow(model);

    // The avatar is decorative, so it is hidden from the accessibility tree —
    // the row's own label already names the thread. Queried explicitly here.
    expect(getByTestId('conversation-avatar-conv-1', { includeHiddenElements: true })).toHaveTextContent('TT');
    expect(getByTestId('conversation-title-conv-1')).toHaveTextContent('Tempo Tuesday');
    expect(getByTestId('conversation-preview-conv-1')).toHaveTextContent('Easy Thursday, then the long run');
    expect(getByTestId('conversation-time-conv-1')).toHaveTextContent('09:50');
    // A 1:1 thread with nothing unread: no glyph, no badges.
    expect(queryByTestId('conversation-kind-conv-1')).toBeNull();
    expect(queryByTestId('conversation-unread-conv-1')).toBeNull();
    expect(queryByTestId('conversation-mention-conv-1')).toBeNull();
  });

  it('prefixes the athlete own last line with You:', () => {
    const model = row({ last_message: { preview: 'How was the run?', role: 'user', created_at: '2026-08-26T09:50:00' } });
    expect(renderRow(model).getByTestId('conversation-preview-conv-1')).toHaveTextContent('You: How was the run?');
  });

  it('shows the group glyph and the coach voice in a group row', () => {
    const model = row({
      group_id: 'group-1',
      group_name: 'Harricana',
      coach_title: 'Coach Tempo',
      last_message: { preview: 'Bloc 3 starts Monday', role: 'assistant', created_at: '2026-08-26T09:50:00' },
    });
    const { getByTestId } = renderRow(model);
    expect(getByTestId('conversation-kind-conv-1').props.accessibilityLabel).toBe('Group chat');
    expect(getByTestId('conversation-preview-conv-1')).toHaveTextContent('Coach Tempo: Bloc 3 starts Monday');
  });

  it('shows the channel badge and glyph for a messaging-origin thread', () => {
    const model = row({ channel_type: 'telegram' });
    const { getByTestId } = renderRow(model);
    expect(getByTestId('conversation-kind-conv-1').props.accessibilityLabel).toBe('Messaging chat');
    expect(getByTestId('conversation-channel-badge-conv-1')).toHaveTextContent('Telegram');
  });

  it('shows the coach handle beside the title of a coach thread', () => {
    const model = row({ coach_id: 'coach-1', coach_handle: 'coach-tempo', coach_title: 'Coach Tempo' });
    expect(renderRow(model).getByTestId('conversation-handle-conv-1')).toHaveTextContent('@coach-tempo');
  });

  it('badges the unread count and caps it at 99+', () => {
    expect(renderRow(row({ unread_count: 4 })).getByTestId('conversation-unread-conv-1')).toHaveTextContent('4');
    expect(renderRow(row({ unread_count: 250 })).getByTestId('conversation-unread-conv-1')).toHaveTextContent('99+');
  });

  it('adds the @ badge only when the unread preview mentions someone', () => {
    const mentioned = row({
      unread_count: 1,
      last_message: { preview: '@coach-tempo what about Sunday?', role: 'user', created_at: '2026-08-26T09:50:00' },
    });
    expect(renderRow(mentioned).getByTestId('conversation-mention-conv-1')).toBeTruthy();

    const readMention = row({
      unread_count: 0,
      last_message: { preview: '@coach-tempo what about Sunday?', role: 'user', created_at: '2026-08-26T09:50:00' },
    });
    expect(renderRow(readMention).queryByTestId('conversation-mention-conv-1')).toBeNull();

    const address = row({
      unread_count: 1,
      last_message: { preview: 'write to jf@dravr.ai', role: 'assistant', created_at: '2026-08-26T09:50:00' },
    });
    expect(renderRow(address).queryByTestId('conversation-mention-conv-1')).toBeNull();
  });

  it('reveals Mark unread on the left and Delete on the right, each calling its handler', () => {
    const { getByTestId, handlers } = renderRow(row());

    fireEvent.press(getByTestId('swipeable-conversation-conv-1-action-mark-unread'));
    expect(handlers.onMarkUnread).toHaveBeenCalledWith(expect.objectContaining({ id: 'conv-1' }));

    fireEvent.press(getByTestId('swipeable-conversation-conv-1-action-delete'));
    expect(handlers.onDelete).toHaveBeenCalledWith(expect.objectContaining({ id: 'conv-1' }));
  });

  it('hands press and long-press to the host', () => {
    const { getByTestId, handlers } = renderRow(row());
    fireEvent.press(getByTestId('conversation-row-conv-1'));
    fireEvent(getByTestId('conversation-row-conv-1'), 'longPress');
    expect(handlers.onPress).toHaveBeenCalledTimes(1);
    expect(handlers.onLongPress).toHaveBeenCalledTimes(1);
  });
});

describe('previewMentionsSomeone', () => {
  it('recognises the mention grammar and nothing else', () => {
    expect(previewMentionsSomeone('@coach-tempo how is my week')).toBe(true);
    expect(previewMentionsSomeone('You: hey @recovery_guru')).toBe(true);
    expect(previewMentionsSomeone('mail jf@dravr.ai')).toBe(false);
    expect(previewMentionsSomeone('no mention here')).toBe(false);
  });
});
