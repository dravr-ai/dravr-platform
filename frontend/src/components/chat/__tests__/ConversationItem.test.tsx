// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the unified list row — unread weight and count, kind glyph, channel badge, preview
// ABOUTME: Asserts the row menu fires its own handlers and the avatar palette covers every slot

import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { AVATAR_SLOTS, buildConversationRow } from '@pierre/chat-utils';

/** The words the row cannot spell itself, as the English client resolves them. */
const LABELS = { locale: 'en-US', you: 'You', coach: 'Coach', untitled: 'Untitled chat' };
import type { Conversation } from '@pierre/shared-types';
import ConversationItem from '../ConversationItem';
import { AVATAR_SLOT_CLASSES, avatarSlotClass } from '../avatarSlots';

const NOW = new Date('2026-08-27T12:00:00');

function conversation(overrides: Partial<Conversation> = {}): Conversation {
  return {
    id: 'conv-1',
    title: 'Sunday long run',
    message_count: 4,
    unread_count: 0,
    created_at: '2026-08-27T09:00:00',
    updated_at: '2026-08-27T09:50:00',
    last_message: { preview: 'Keep the cadence high', role: 'assistant', created_at: '2026-08-27T09:50:00' },
    ...overrides,
  };
}

const noop = vi.fn();

function renderRow(
  conv: Conversation,
  handlers: Partial<{
    onSelect: () => void;
    onMarkUnread: () => void;
    onDelete: () => void;
    onStartRename: () => void;
  }> = {},
  state: { isSelected?: boolean; isEditing?: boolean; editedTitleValue?: string } = {},
) {
  return render(
    <ul>
      <ConversationItem
        row={buildConversationRow(conv, LABELS, NOW)}
        isSelected={state.isSelected ?? false}
        isEditing={state.isEditing ?? false}
        editedTitleValue={state.editedTitleValue ?? ''}
        onSelect={handlers.onSelect ?? noop}
        onStartRename={handlers.onStartRename ?? noop}
        onMarkUnread={handlers.onMarkUnread ?? noop}
        onDelete={handlers.onDelete ?? noop}
        onTitleChange={noop}
        onSaveRename={noop}
        onCancelRename={noop}
      />
    </ul>,
  );
}

describe('ConversationItem anatomy', () => {
  it('draws the initials avatar, the title, the preview and the time of the last message', () => {
    renderRow(conversation());

    expect(screen.getByTestId('conversation-avatar')).toHaveTextContent('SL');
    expect(screen.getByTestId('conversation-title')).toHaveTextContent('Sunday long run');
    expect(screen.getByTestId('conversation-preview')).toHaveTextContent('Keep the cadence high');
    expect(screen.getByTestId('conversation-timestamp')).toHaveTextContent('09:50');
  });

  it('bolds the title and shows the count while the thread has unread rows', () => {
    renderRow(conversation({ unread_count: 3 }));

    expect(screen.getByTestId('conversation-title')).toHaveClass('font-semibold');
    expect(screen.getByTestId('conversation-unread-count')).toHaveTextContent('3');
    expect(screen.getByTestId('conversation-row')).toHaveAttribute('data-unread', 'true');
  });

  it('keeps the title at normal weight and shows no count once everything is read', () => {
    renderRow(conversation({ unread_count: 0 }));

    expect(screen.getByTestId('conversation-title')).toHaveClass('font-normal');
    expect(screen.queryByTestId('conversation-unread-count')).toBeNull();
  });

  it('caps a large unread count at 99+', () => {
    renderRow(conversation({ unread_count: 140 }));

    expect(screen.getByTestId('conversation-unread-count')).toHaveTextContent('99+');
  });

  it('prefixes the athlete own last message with You:', () => {
    renderRow(
      conversation({
        last_message: { preview: 'How was my week?', role: 'user', created_at: '2026-08-27T09:50:00' },
      }),
    );

    expect(screen.getByTestId('conversation-preview')).toHaveTextContent('You: How was my week?');
  });

  it('marks a group thread with the group glyph and names the coach in the preview', () => {
    renderRow(
      conversation({
        group_id: 'group-1',
        group_name: 'Sunday Riders',
        coach_title: 'Tempo Coach',
      }),
    );

    const glyph = screen.getByTestId('conversation-kind-glyph');
    expect(glyph).toHaveAttribute('data-kind', 'group');
    expect(glyph).toHaveAccessibleName('Group chat');
    expect(screen.getByTestId('conversation-preview')).toHaveTextContent('Tempo Coach: Keep the cadence high');
  });

  it('shows the channel badge for a messaging-origin thread', () => {
    renderRow(conversation({ channel_type: 'telegram' }));

    const badge = screen.getByTestId('conversation-channel-badge');
    expect(badge).toHaveTextContent('Telegram');
    expect(badge).toHaveAccessibleName('From Telegram');
  });

  it('derives the channel badge from a Messaging: title when the column is not populated', () => {
    renderRow(conversation({ title: 'Messaging: whatsapp', channel_type: 'web' }));

    expect(screen.getByTestId('conversation-channel-badge')).toHaveTextContent('WhatsApp');
  });

  it('shows neither glyph nor badge for a plain in-app thread', () => {
    renderRow(conversation({ channel_type: 'web' }));

    expect(screen.queryByTestId('conversation-kind-glyph')).toBeNull();
    expect(screen.queryByTestId('conversation-channel-badge')).toBeNull();
  });

  it('falls back to the coach handle under a thread with no message yet', () => {
    renderRow(conversation({ coach_id: 'coach-1', coach_handle: 'recovery-coach', last_message: null }));

    expect(screen.getByTestId('conversation-preview')).toHaveTextContent('@recovery-coach');
  });
});

describe('ConversationItem actions', () => {
  it('selects the thread from the row and never from its actions menu', () => {
    const onSelect = vi.fn();
    const onMarkUnread = vi.fn();
    const onDelete = vi.fn();
    const onStartRename = vi.fn();
    renderRow(conversation(), { onSelect, onMarkUnread, onDelete, onStartRename });

    fireEvent.click(screen.getByRole('button', { name: /Sunday long run/ }));
    expect(onSelect).toHaveBeenCalledTimes(1);

    // The three actions live behind one trigger, so the row's own click
    // target is never covered by them.
    expect(screen.queryByRole('menu', { name: 'Conversation actions' })).toBeNull();
    for (const action of [
      'Mark conversation unread',
      'Delete conversation',
      'Rename conversation',
    ]) {
      fireEvent.click(screen.getByTestId('conversation-actions-trigger'));
      fireEvent.click(screen.getByRole('menuitem', { name: action }));
    }

    expect(onMarkUnread).toHaveBeenCalledTimes(1);
    expect(onDelete).toHaveBeenCalledTimes(1);
    expect(onStartRename).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it('marks the selected row as current', () => {
    renderRow(conversation(), {}, { isSelected: true });

    expect(screen.getByRole('button', { name: /Sunday long run/ })).toHaveAttribute('aria-current', 'true');
  });

  it('renames through a design-system input, not a raw field', () => {
    renderRow(conversation(), {}, { isEditing: true, editedTitleValue: 'Sunday tempo' });

    const input = screen.getByLabelText('Conversation title');
    expect(input).toHaveValue('Sunday tempo');
    expect(input).toHaveClass('boreal-underline-input');
  });
});

describe('avatar palette', () => {
  it('provides one token colour per shared avatar slot', () => {
    expect(AVATAR_SLOT_CLASSES).toHaveLength(AVATAR_SLOTS);
    expect(new Set(AVATAR_SLOT_CLASSES).size).toBe(AVATAR_SLOTS);
  });

  it('gives the same thread the same colour on every render', () => {
    const first = buildConversationRow(conversation({ id: 'conv-stable' }), LABELS, NOW).avatarSlot;
    const second = buildConversationRow(conversation({ id: 'conv-stable' }), LABELS, NOW).avatarSlot;
    expect(avatarSlotClass(first)).toBe(avatarSlotClass(second));
    expect(avatarSlotClass(AVATAR_SLOTS)).toBe(AVATAR_SLOT_CLASSES[0]);
  });
});
