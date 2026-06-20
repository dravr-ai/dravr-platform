// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for ConversationItem component
// ABOUTME: Verifies the messaging-channel origin badge derives from the title

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import ConversationItem from '../ConversationItem';
import type { Conversation } from '../types';

function makeConversation(overrides: Partial<Conversation> = {}): Conversation {
  return {
    id: 'conv-1',
    title: 'Chat Jun 7 11:15 AM',
    message_count: 4,
    created_at: '2026-06-19T00:00:00Z',
    updated_at: '2026-06-19T00:00:00Z',
    ...overrides,
  };
}

const noop = vi.fn();

function renderItem(conversation: Conversation, displayTitle?: string) {
  return render(
    <ConversationItem
      conversation={conversation}
      displayTitle={displayTitle}
      isSelected={false}
      isEditing={false}
      editedTitleValue=""
      onSelect={noop}
      onStartRename={noop}
      onDelete={noop}
      onTitleChange={noop}
      onSaveRename={noop}
      onCancelRename={noop}
    />,
  );
}

describe('ConversationItem channel badge', () => {
  it('shows a Telegram badge for a messaging-origin conversation', () => {
    renderItem(makeConversation({ title: 'Messaging: telegram' }));

    const badge = screen.getByTestId('conversation-channel-badge');
    expect(badge).toBeInTheDocument();
    expect(badge).toHaveTextContent('Telegram');
  });

  it('derives the badge from the title even when a displayTitle overrides the label', () => {
    // ConversationsPanel passes a coach-aware displayTitle; the badge must still
    // come from the conversation's own "Messaging: <channel>" title.
    renderItem(makeConversation({ title: 'Messaging: whatsapp' }), 'Coach Phil · Jun 7');

    const badge = screen.getByTestId('conversation-channel-badge');
    expect(badge).toHaveTextContent('WhatsApp');
  });

  it('shows no badge for an ordinary web conversation', () => {
    renderItem(makeConversation({ title: 'Chat Jun 7 11:15 AM' }));

    expect(screen.queryByTestId('conversation-channel-badge')).not.toBeInTheDocument();
  });
});
