// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the unified conversation list hook — paging, de-duplication, unread total, mutations
// ABOUTME: Proves the rows the screen and the tab badge both read accumulate across pages and stop at the total

import React from 'react';
import { renderHook, waitFor, act } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockGetConversations = jest.fn();
const mockMarkConversationRead = jest.fn();
const mockMarkConversationUnread = jest.fn();
const mockDeleteConversation = jest.fn();
const mockUpdateConversation = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
    markConversationRead: (...args: unknown[]) => mockMarkConversationRead(...args),
    markConversationUnread: (...args: unknown[]) => mockMarkConversationUnread(...args),
    deleteConversation: (...args: unknown[]) => mockDeleteConversation(...args),
    updateConversation: (...args: unknown[]) => mockUpdateConversation(...args),
  },
}));

import { useConversationList, useConversationRows } from '../src/screens/conversations/useConversationList';

type Conv = {
  id: string;
  title: string;
  coach_id: string | null;
  message_count: number;
  unread_count: number;
  created_at: string;
  updated_at: string;
};

function makeConv(id: string, overrides: Partial<Conv> = {}): Conv {
  return {
    id,
    title: `Chat ${id}`,
    coach_id: null,
    message_count: 3,
    unread_count: 0,
    created_at: '2026-04-10T10:00:00Z',
    updated_at: `2026-04-10T10:00:0${id.length}Z`,
    ...overrides,
  };
}

function wrapper({ children }: { children: React.ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe('useConversationRows', () => {
  beforeEach(() => jest.clearAllMocks());

  it('accumulates pages and stops asking once the total is loaded', async () => {
    const first = Array.from({ length: 50 }, (_, i) => makeConv(`c${i}`));
    mockGetConversations
      .mockResolvedValueOnce({ conversations: first, total: 51, limit: 50, offset: 0 })
      .mockResolvedValueOnce({ conversations: [makeConv('c50')], total: 51, limit: 50, offset: 50 });

    const { result } = renderHook(() => useConversationRows(), { wrapper });
    await waitFor(() => expect(result.current.rows).toHaveLength(50));
    expect(result.current.total).toBe(51);
    expect(result.current.hasMore).toBe(true);

    await act(async () => {
      result.current.loadMore();
    });
    await waitFor(() => expect(result.current.rows).toHaveLength(51));
    expect(mockGetConversations).toHaveBeenNthCalledWith(2, 50, 50);
    expect(result.current.rows.map((row) => row.id)).toContain('c50');

    await act(async () => {
      result.current.loadMore();
    });
    expect(mockGetConversations).toHaveBeenCalledTimes(2);
    expect(result.current.hasMore).toBe(false);
  });

  it('draws one row per conversation when a page repeats one that moved', async () => {
    mockGetConversations
      .mockResolvedValueOnce({ conversations: [makeConv('a'), makeConv('b')], total: 3, limit: 50, offset: 0 })
      .mockResolvedValueOnce({ conversations: [makeConv('b'), makeConv('c')], total: 3, limit: 50, offset: 2 });

    const { result } = renderHook(() => useConversationRows(), { wrapper });
    await waitFor(() => expect(result.current.rows).toHaveLength(2));
    await act(async () => {
      result.current.loadMore();
    });
    await waitFor(() => expect(result.current.rows).toHaveLength(3));
    expect(result.current.rows.map((row) => row.id).sort()).toEqual(['a', 'b', 'c']);
  });

  it('sums the unread counts the tab badge wears', async () => {
    mockGetConversations.mockResolvedValue({
      conversations: [makeConv('a', { unread_count: 3 }), makeConv('b', { unread_count: 4 }), makeConv('c')],
      total: 3,
      limit: 50,
      offset: 0,
    });

    const { result } = renderHook(() => useConversationRows(), { wrapper });
    await waitFor(() => expect(result.current.unreadTotal).toBe(7));
  });
});

describe('useConversationList', () => {
  beforeEach(() => jest.clearAllMocks());

  it('marks a row unread against its own message count before the server answers', async () => {
    mockGetConversations.mockResolvedValue({
      conversations: [makeConv('a', { message_count: 6, unread_count: 0 })],
      total: 1,
      limit: 50,
      offset: 0,
    });
    let resolveUnread: () => void = () => undefined;
    mockMarkConversationUnread.mockReturnValue(
      new Promise<void>((resolve) => {
        resolveUnread = resolve;
      }),
    );

    const { result } = renderHook(() => useConversationList(), { wrapper });
    await waitFor(() => expect(result.current.rows).toHaveLength(1));

    act(() => {
      void result.current.markUnread('a');
    });
    await waitFor(() => expect(result.current.rows[0].unreadCount).toBe(6));
    expect(mockMarkConversationUnread).toHaveBeenCalledWith('a');

    await act(async () => {
      resolveUnread();
    });
  });

  it('drops a deleted row from the list immediately', async () => {
    mockGetConversations.mockResolvedValue({
      conversations: [makeConv('a'), makeConv('b')],
      total: 2,
      limit: 50,
      offset: 0,
    });
    let resolveDelete: () => void = () => undefined;
    mockDeleteConversation.mockReturnValue(
      new Promise<void>((resolve) => {
        resolveDelete = resolve;
      }),
    );

    const { result } = renderHook(() => useConversationList(), { wrapper });
    await waitFor(() => expect(result.current.rows).toHaveLength(2));

    act(() => {
      void result.current.remove('a');
    });
    await waitFor(() => expect(result.current.rows.map((row) => row.id)).toEqual(['b']));

    await act(async () => {
      resolveDelete();
    });
  });
});
