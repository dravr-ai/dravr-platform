// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the unified conversation list — rows, search, empty state, Load more, row actions
// ABOUTME: Mocks chatApi and asserts what the list draws and which calls its actions make

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Conversation } from '@pierre/shared-types';
import ConversationList from '../ConversationList';
import { CONVERSATION_PAGE_SIZE } from '../../../hooks/useConversationList';

const getConversations = vi.fn();
const updateConversation = vi.fn();
const deleteConversation = vi.fn();
const markConversationUnread = vi.fn();

vi.mock('../../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    updateConversation: (...a: unknown[]) => updateConversation(...a),
    deleteConversation: (...a: unknown[]) => deleteConversation(...a),
    markConversationUnread: (...a: unknown[]) => markConversationUnread(...a),
  },
}));

function conversation(overrides: Partial<Conversation> = {}): Conversation {
  return {
    id: 'conv-1',
    title: 'Marathon plan',
    coach_id: 'coach-running',
    coach_title: 'Running Coach',
    coach_handle: 'running-coach',
    message_count: 3,
    unread_count: 0,
    created_at: '2026-04-13T18:00:00Z',
    updated_at: '2026-04-13T18:00:00Z',
    last_message: { preview: 'Easy 10k tomorrow', role: 'assistant', created_at: '2026-04-13T18:00:00Z' },
    ...overrides,
  };
}

function page(conversations: Conversation[], total = conversations.length) {
  return { conversations, total, limit: CONVERSATION_PAGE_SIZE, offset: 0 };
}

function renderList(
  props: Partial<{ selectedConversation: string | null; onSelectConversation: (id: string | null) => void }> = {},
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ConversationList
        selectedConversation={props.selectedConversation ?? null}
        onSelectConversation={props.onSelectConversation ?? vi.fn()}
      />
    </QueryClientProvider>,
  );
}

describe('ConversationList', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('names the "+" beside the chat when there are no conversations', async () => {
    getConversations.mockResolvedValue(page([]));
    renderList();

    // The list has no "+" of its own: the chat surface owns the one compose
    // menu, and it is on screen beside this pane whenever the list is empty.
    expect(await screen.findByTestId('conversation-list-empty')).toHaveTextContent(
      'No chats yet — start one from the "+" beside the chat',
    );
    expect(screen.queryByRole('button', { name: 'New' })).toBeNull();
  });

  it('draws one flat row per conversation, newest activity first, with preview and unread count', async () => {
    getConversations.mockResolvedValue(
      page([
        conversation({ id: 'c-old', title: 'Track workout', updated_at: '2026-04-01T10:00:00Z', last_message: null }),
        conversation({ id: 'c-new', title: 'Marathon plan', unread_count: 2 }),
        conversation({
          id: 'c-group',
          title: 'Sunday Riders',
          group_id: 'group-1',
          group_name: 'Sunday Riders',
          coach_title: 'Tempo Coach',
          updated_at: '2026-04-10T10:00:00Z',
          last_message: { preview: 'Ride at 8', role: 'assistant', created_at: '2026-04-10T10:00:00Z' },
        }),
      ]),
    );
    renderList();

    const list = await screen.findByRole('list', { name: 'Conversations' });
    const rows = within(list).getAllByTestId('conversation-row');
    expect(rows.map((row) => row.getAttribute('data-conversation-id'))).toEqual(['c-new', 'c-group', 'c-old']);
    // No coach grouping headers, no "Without a coach" bucket — one flat list.
    expect(screen.queryByText(/Without a coach/)).toBeNull();
    expect(screen.queryByText('Running Coach')).toBeNull();
    expect(within(rows[0]).getByTestId('conversation-preview')).toHaveTextContent('Easy 10k tomorrow');
    expect(within(rows[0]).getByTestId('conversation-unread-count')).toHaveTextContent('2');
    expect(within(rows[1]).getByTestId('conversation-kind-glyph')).toHaveAttribute('data-kind', 'group');
    expect(within(rows[1]).getByTestId('conversation-preview')).toHaveTextContent('Tempo Coach: Ride at 8');
  });

  it('opens a row without touching the read marker', async () => {
    getConversations.mockResolvedValue(page([conversation({ id: 'c1', unread_count: 2 })]));
    const onSelectConversation = vi.fn();
    renderList({ onSelectConversation });

    fireEvent.click(await screen.findByRole('button', { name: /Marathon plan/ }));

    expect(onSelectConversation).toHaveBeenCalledWith('c1');
    expect(markConversationUnread).not.toHaveBeenCalled();
  });

  it('filters the rows by title, handle or preview and reports when nothing matches', async () => {
    getConversations.mockResolvedValue(
      page([
        conversation({ id: 'c1', title: 'Marathon plan' }),
        conversation({ id: 'c2', title: 'Deadlift form', coach_handle: 'strength-coach', last_message: null }),
      ]),
    );
    renderList();
    await screen.findByText('Deadlift form');

    const search = screen.getByLabelText('Search conversations');
    fireEvent.change(search, { target: { value: 'strength' } });
    await waitFor(() => expect(screen.queryByText('Marathon plan')).toBeNull());
    expect(screen.getByText('Deadlift form')).toBeInTheDocument();

    fireEvent.change(search, { target: { value: '10k' } });
    await waitFor(() => expect(screen.getByText('Marathon plan')).toBeInTheDocument());
    expect(screen.queryByText('Deadlift form')).toBeNull();

    fireEvent.change(search, { target: { value: 'kettlebell' } });
    expect(await screen.findByText('No chats match')).toBeInTheDocument();
  });

  it('offers Load more while the total exceeds the rows and asks for the next page', async () => {
    const first = Array.from({ length: CONVERSATION_PAGE_SIZE }, (_, i) =>
      conversation({ id: `c${i}`, title: `Chat ${i}` }),
    );
    getConversations.mockResolvedValueOnce(page(first, CONVERSATION_PAGE_SIZE + 1));
    getConversations.mockResolvedValueOnce({
      conversations: [conversation({ id: 'c-last', title: 'The last one' })],
      total: CONVERSATION_PAGE_SIZE + 1,
      limit: CONVERSATION_PAGE_SIZE,
      offset: CONVERSATION_PAGE_SIZE,
    });
    renderList();

    fireEvent.click(await screen.findByTestId('conversation-list-load-more'));

    expect(await screen.findByText('The last one')).toBeInTheDocument();
    expect(getConversations).toHaveBeenLastCalledWith(CONVERSATION_PAGE_SIZE, CONVERSATION_PAGE_SIZE);
    await waitFor(() => expect(screen.queryByTestId('conversation-list-load-more')).toBeNull());
  });

  it('marks a conversation unread from the row action without selecting it', async () => {
    getConversations.mockResolvedValue(page([conversation({ id: 'c1' })]));
    markConversationUnread.mockResolvedValue(undefined);
    const onSelectConversation = vi.fn();
    renderList({ onSelectConversation });
    await screen.findByText('Marathon plan');

    fireEvent.click(screen.getByTestId('conversation-actions-trigger'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Mark conversation unread' }));

    await waitFor(() => expect(markConversationUnread).toHaveBeenCalledWith('c1'));
    expect(onSelectConversation).not.toHaveBeenCalled();
  });

  it('renames inline and saves the trimmed title on Enter', async () => {
    getConversations.mockResolvedValue(page([conversation({ id: 'c1' })]));
    updateConversation.mockResolvedValue(conversation({ id: 'c1', title: 'Fall marathon' }));
    renderList();
    await screen.findByText('Marathon plan');

    fireEvent.click(screen.getByTestId('conversation-actions-trigger'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Rename conversation' }));
    const input = screen.getByLabelText('Conversation title');
    expect(input).toHaveValue('Marathon plan');
    fireEvent.change(input, { target: { value: '  Fall marathon  ' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() => expect(updateConversation).toHaveBeenCalledWith('c1', { title: 'Fall marathon' }));
  });

  it('deletes after confirmation and deselects the open thread', async () => {
    getConversations.mockResolvedValue(page([conversation({ id: 'c1' })]));
    deleteConversation.mockResolvedValue(undefined);
    const onSelectConversation = vi.fn();
    renderList({ selectedConversation: 'c1', onSelectConversation });
    await screen.findByText('Marathon plan');

    fireEvent.click(screen.getByTestId('conversation-actions-trigger'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete conversation' }));
    expect(await screen.findByText('Delete Conversation')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

    await waitFor(() => expect(deleteConversation).toHaveBeenCalledWith('c1'));
    await waitFor(() => expect(onSelectConversation).toHaveBeenCalledWith(null));
  });
});
