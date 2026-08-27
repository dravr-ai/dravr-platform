// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the unified list query — activity order, search, paging, unread total, row mutations
// ABOUTME: Mocks chatApi and asserts the offsets requested and the invalidations each mutation triggers

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import type { Conversation } from '@pierre/shared-types';
import {
  CONVERSATION_PAGE_SIZE,
  cachedConversations,
  patchCachedConversation,
  useConversationList,
  useConversationMutations,
  useUnreadConversationTotal,
} from '../useConversationList';
import { QUERY_KEYS } from '../../constants/queryKeys';

const getConversations = vi.fn();
const updateConversation = vi.fn();
const deleteConversation = vi.fn();
const markConversationUnread = vi.fn();

vi.mock('../../services/api', () => ({
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
    title: 'Sunday long run',
    message_count: 2,
    unread_count: 0,
    created_at: '2026-08-20T10:00:00Z',
    updated_at: '2026-08-20T10:00:00Z',
    ...overrides,
  };
}

function page(conversations: Conversation[], total = conversations.length) {
  return { conversations, total, limit: CONVERSATION_PAGE_SIZE, offset: 0 };
}

function wrapperFor(client: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}

function newClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
}

describe('useConversationList', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('asks for the first page and orders the rows by last activity, newest first', async () => {
    getConversations.mockResolvedValue(
      page([
        conversation({ id: 'old', title: 'Old', updated_at: '2026-08-01T10:00:00Z' }),
        conversation({
          id: 'fresh',
          title: 'Fresh',
          updated_at: '2026-08-01T10:00:00Z',
          last_message: { preview: 'Just now', role: 'assistant', created_at: '2026-08-27T09:00:00Z' },
        }),
        conversation({ id: 'mid', title: 'Mid', updated_at: '2026-08-15T10:00:00Z' }),
      ]),
    );

    const { result } = renderHook(() => useConversationList(), { wrapper: wrapperFor(newClient()) });

    await waitFor(() => expect(result.current.rows).toHaveLength(3));
    expect(getConversations).toHaveBeenCalledWith(CONVERSATION_PAGE_SIZE, 0);
    expect(result.current.rows.map((row) => row.id)).toEqual(['fresh', 'mid', 'old']);
    expect(result.current.total).toBe(3);
    expect(result.current.hasMore).toBe(false);
  });

  it('narrows the rows to the search query without refetching', async () => {
    getConversations.mockResolvedValue(
      page([
        conversation({ id: 'c1', title: 'Marathon plan' }),
        conversation({ id: 'c2', title: 'Deadlift form', coach_handle: 'strength-coach' }),
      ]),
    );

    const { result, rerender } = renderHook(({ query }) => useConversationList(query), {
      wrapper: wrapperFor(newClient()),
      initialProps: { query: '' },
    });
    await waitFor(() => expect(result.current.rows).toHaveLength(2));

    rerender({ query: '@strength' });
    expect(result.current.rows.map((row) => row.id)).toEqual(['c2']);
    expect(getConversations).toHaveBeenCalledTimes(1);
  });

  it('offers Load more while the total exceeds the loaded rows and fetches the next offset', async () => {
    const firstPage = Array.from({ length: CONVERSATION_PAGE_SIZE }, (_, i) =>
      conversation({ id: `c${i}`, title: `Chat ${i}` }),
    );
    getConversations.mockResolvedValueOnce(page(firstPage, CONVERSATION_PAGE_SIZE + 1));
    getConversations.mockResolvedValueOnce({
      conversations: [conversation({ id: 'c-last', title: 'The last one' })],
      total: CONVERSATION_PAGE_SIZE + 1,
      limit: CONVERSATION_PAGE_SIZE,
      offset: CONVERSATION_PAGE_SIZE,
    });

    const { result } = renderHook(() => useConversationList(), { wrapper: wrapperFor(newClient()) });
    await waitFor(() => expect(result.current.rows).toHaveLength(CONVERSATION_PAGE_SIZE));
    expect(result.current.hasMore).toBe(true);
    expect(result.current.total).toBe(CONVERSATION_PAGE_SIZE + 1);

    act(() => result.current.loadMore());

    await waitFor(() => expect(result.current.rows).toHaveLength(CONVERSATION_PAGE_SIZE + 1));
    expect(getConversations).toHaveBeenLastCalledWith(CONVERSATION_PAGE_SIZE, CONVERSATION_PAGE_SIZE);
    expect(result.current.hasMore).toBe(false);
  });

  it('sums the unread rows across conversations for the nav badge', async () => {
    getConversations.mockResolvedValue(
      page([
        conversation({ id: 'c1', unread_count: 2 }),
        conversation({ id: 'c2', unread_count: 0 }),
        conversation({ id: 'c3', unread_count: 5 }),
      ]),
    );

    const { result } = renderHook(() => useUnreadConversationTotal(), { wrapper: wrapperFor(newClient()) });

    await waitFor(() => expect(result.current).toBe(7));
  });

  it('never fetches for a caller that disabled the badge', async () => {
    getConversations.mockResolvedValue(page([conversation({ unread_count: 4 })]));

    const { result } = renderHook(() => useUnreadConversationTotal(false), {
      wrapper: wrapperFor(newClient()),
    });

    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(result.current).toBe(0);
    expect(getConversations).not.toHaveBeenCalled();
  });

  it('patches one cached row in place and reads the rows back across pages', async () => {
    getConversations.mockResolvedValue(page([conversation({ id: 'c1', unread_count: 3 })]));
    const client = newClient();
    const { result } = renderHook(() => useConversationList(), { wrapper: wrapperFor(client) });
    await waitFor(() => expect(result.current.rows).toHaveLength(1));

    act(() => patchCachedConversation(client, 'c1', { unread_count: 0 }));

    expect(cachedConversations(client)[0].unread_count).toBe(0);
    await waitFor(() => expect(result.current.rows[0].unreadCount).toBe(0));
  });
});

describe('useConversationMutations', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renames, deletes and marks unread through the api and refetches the list after each', async () => {
    getConversations.mockResolvedValue(page([conversation({ id: 'c1' })]));
    updateConversation.mockResolvedValue(conversation({ id: 'c1', title: 'Renamed' }));
    deleteConversation.mockResolvedValue(undefined);
    markConversationUnread.mockResolvedValue(undefined);
    const client = newClient();
    const invalidate = vi.spyOn(client, 'invalidateQueries');

    const { result } = renderHook(() => useConversationMutations(), { wrapper: wrapperFor(client) });

    await act(async () => {
      await result.current.rename('c1', 'Renamed');
    });
    expect(updateConversation).toHaveBeenCalledWith('c1', { title: 'Renamed' });

    await act(async () => {
      await result.current.markUnread('c1');
    });
    expect(markConversationUnread).toHaveBeenCalledWith('c1');

    await act(async () => {
      await result.current.remove('c1');
    });
    expect(deleteConversation).toHaveBeenCalledWith('c1');

    expect(invalidate).toHaveBeenCalledTimes(3);
    for (const call of invalidate.mock.calls) {
      expect(call[0]).toEqual({ queryKey: QUERY_KEYS.chat.conversations() });
    }
  });
});
