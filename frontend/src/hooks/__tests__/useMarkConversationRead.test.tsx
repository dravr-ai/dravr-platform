// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the read-marker hook — marks on open when unread, on every newest-message change, only while visible
// ABOUTME: Mocks chatApi and reads the optimistic zeroing straight out of the conversations cache

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import type { Conversation } from '@pierre/shared-types';
import { useMarkConversationRead } from '../useMarkConversationRead';
import { cachedConversations, useConversationList } from '../useConversationList';
import { QUERY_KEYS } from '../../constants/queryKeys';

const getConversations = vi.fn();
const markConversationRead = vi.fn();

vi.mock('../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    markConversationRead: (...a: unknown[]) => markConversationRead(...a),
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

function newClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
}

function wrapperFor(client: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}

/** Load the list into the cache so the hook can read the row's unread count. */
async function primeList(client: QueryClient, conversations: Conversation[]) {
  getConversations.mockResolvedValue({ conversations, total: conversations.length, limit: 50, offset: 0 });
  const list = renderHook(() => useConversationList(), { wrapper: wrapperFor(client) });
  await waitFor(() => expect(list.result.current.rows).toHaveLength(conversations.length));
  return list;
}

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
  document.dispatchEvent(new Event('visibilitychange'));
}

describe('useMarkConversationRead', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    markConversationRead.mockResolvedValue(undefined);
    setVisibility('visible');
  });

  afterEach(() => {
    setVisibility('visible');
  });

  it('marks a thread read on open once its messages resolve, and zeroes the row optimistically', async () => {
    const client = newClient();
    await primeList(client, [conversation({ id: 'conv-1', unread_count: 3 })]);
    // Hold the request open: the zeroed row must be visible before the
    // server has answered, not after the refetch that follows it.
    let settle: () => void = () => {};
    markConversationRead.mockImplementation(
      () => new Promise<void>((resolve) => { settle = resolve; }),
    );

    const { rerender } = renderHook(
      ({ latest }: { latest: string | null }) => useMarkConversationRead('conv-1', latest),
      { wrapper: wrapperFor(client), initialProps: { latest: null } },
    );
    // Nothing to mark until the thread has a message on screen.
    expect(markConversationRead).not.toHaveBeenCalled();

    rerender({ latest: 'm2' });

    await waitFor(() => expect(markConversationRead).toHaveBeenCalledWith('conv-1'));
    expect(cachedConversations(client)[0].unread_count).toBe(0);
    await act(async () => settle());
  });

  it('leaves the marker alone when the opened thread has nothing unread', async () => {
    const client = newClient();
    await primeList(client, [conversation({ id: 'conv-1', unread_count: 0 })]);

    renderHook(() => useMarkConversationRead('conv-1', 'm2'), { wrapper: wrapperFor(client) });

    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(markConversationRead).not.toHaveBeenCalled();
  });

  it('marks again when the newest message changes, even with nothing unread cached', async () => {
    const client = newClient();
    await primeList(client, [conversation({ id: 'conv-1', unread_count: 0 })]);

    const { rerender } = renderHook(
      ({ latest }: { latest: string | null }) => useMarkConversationRead('conv-1', latest),
      { wrapper: wrapperFor(client), initialProps: { latest: 'm2' as string | null } },
    );
    expect(markConversationRead).not.toHaveBeenCalled();

    // A reply landed at the end of a turn.
    rerender({ latest: 'm4' });

    await waitFor(() => expect(markConversationRead).toHaveBeenCalledTimes(1));
    expect(markConversationRead).toHaveBeenCalledWith('conv-1');

    // The same message never marks twice.
    rerender({ latest: 'm4' });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(markConversationRead).toHaveBeenCalledTimes(1);
  });

  it('waits for the tab to become visible before marking a message that arrived while hidden', async () => {
    const client = newClient();
    await primeList(client, [conversation({ id: 'conv-1', unread_count: 0 })]);
    const { rerender } = renderHook(
      ({ latest }: { latest: string | null }) => useMarkConversationRead('conv-1', latest),
      { wrapper: wrapperFor(client), initialProps: { latest: 'm2' as string | null } },
    );

    act(() => setVisibility('hidden'));
    rerender({ latest: 'm5' });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(markConversationRead).not.toHaveBeenCalled();

    act(() => setVisibility('visible'));

    await waitFor(() => expect(markConversationRead).toHaveBeenCalledWith('conv-1'));
  });

  it('refetches the list once the marker settles', async () => {
    const client = newClient();
    await primeList(client, [conversation({ id: 'conv-1', unread_count: 2 })]);
    const invalidate = vi.spyOn(client, 'invalidateQueries');

    renderHook(() => useMarkConversationRead('conv-1', 'm2'), { wrapper: wrapperFor(client) });

    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith({ queryKey: QUERY_KEYS.chat.conversations() }),
    );
  });

  it('treats switching to another thread as opening it', async () => {
    const client = newClient();
    await primeList(client, [
      conversation({ id: 'conv-1', unread_count: 0 }),
      conversation({ id: 'conv-2', unread_count: 1 }),
    ]);
    const { rerender } = renderHook(
      ({ id, latest }: { id: string; latest: string }) => useMarkConversationRead(id, latest),
      { wrapper: wrapperFor(client), initialProps: { id: 'conv-1', latest: 'm2' } },
    );
    expect(markConversationRead).not.toHaveBeenCalled();

    rerender({ id: 'conv-2', latest: 'm9' });

    await waitFor(() => expect(markConversationRead).toHaveBeenCalledWith('conv-2'));
    expect(markConversationRead).toHaveBeenCalledTimes(1);
  });
});
