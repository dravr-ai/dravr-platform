// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the open thread's read marker — it moves only while the screen is focused and the app awake
// ABOUTME: Proves the cached list row drops to zero unread before the request and that one transcript posts once

import React from 'react';
import { AppState } from 'react-native';
import { renderHook, act, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';

const mockMarkConversationRead = jest.fn();
let mockIsFocused = true;

jest.mock('../src/services/api', () => ({
  chatApi: {
    markConversationRead: (...args: unknown[]) => mockMarkConversationRead(...args),
  },
}));

jest.mock('expo-router', () => ({
  useIsFocused: () => mockIsFocused,
}));

import { useMarkConversationRead } from '../src/screens/chat/useMarkConversationRead';

type Listener = (status: string) => void;

function seedList(client: QueryClient, unread: number) {
  client.setQueryData(QUERY_KEYS.chat.conversations(), {
    pageParams: [0],
    pages: [
      {
        conversations: [
          {
            id: 'c1',
            title: 'Training plan',
            coach_id: null,
            message_count: 6,
            unread_count: unread,
            created_at: '2026-08-20T10:00:00Z',
            updated_at: '2026-08-26T10:00:00Z',
          },
        ],
        total: 1,
        limit: 50,
        offset: 0,
      },
    ],
  });
}

function cachedUnread(client: QueryClient): number | undefined {
  const data = client.getQueryData(QUERY_KEYS.chat.conversations()) as
    | { pages: Array<{ conversations: Array<{ id: string; unread_count: number }> }> }
    | undefined;
  return data?.pages[0].conversations.find((row) => row.id === 'c1')?.unread_count;
}

function renderMarker(client: QueryClient, initial: { conversationId: string | null; lastMessageId: string | null }) {
  return renderHook((props: { conversationId: string | null; lastMessageId: string | null }) => useMarkConversationRead(props), {
    initialProps: initial,
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}

describe('useMarkConversationRead', () => {
  let listeners: Listener[] = [];

  beforeEach(() => {
    jest.clearAllMocks();
    mockIsFocused = true;
    listeners = [];
    mockMarkConversationRead.mockResolvedValue(undefined);
    jest.spyOn(AppState, 'addEventListener').mockImplementation((_event, handler) => {
      listeners.push(handler as Listener);
      return { remove: jest.fn() } as never;
    });
    Object.defineProperty(AppState, 'currentState', { value: 'active', configurable: true });
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('marks the thread read once per transcript and clears the row badge first', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    seedList(client, 4);

    const { rerender } = renderMarker(client, { conversationId: 'c1', lastMessageId: 'm9' });

    await waitFor(() => expect(mockMarkConversationRead).toHaveBeenCalledWith('c1'));
    expect(cachedUnread(client)).toBe(0);

    // The same transcript re-rendering must not re-post the marker.
    rerender({ conversationId: 'c1', lastMessageId: 'm9' });
    expect(mockMarkConversationRead).toHaveBeenCalledTimes(1);
  });

  it('moves the marker again when a new message lands', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    seedList(client, 0);

    const { rerender } = renderMarker(client, { conversationId: 'c1', lastMessageId: 'm9' });
    await waitFor(() => expect(mockMarkConversationRead).toHaveBeenCalledTimes(1));

    rerender({ conversationId: 'c1', lastMessageId: 'm10' });
    await waitFor(() => expect(mockMarkConversationRead).toHaveBeenCalledTimes(2));
  });

  it('does not mark a thread read while the screen is not focused', () => {
    mockIsFocused = false;
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    seedList(client, 3);

    renderMarker(client, { conversationId: 'c1', lastMessageId: 'm9' });

    expect(mockMarkConversationRead).not.toHaveBeenCalled();
    expect(cachedUnread(client)).toBe(3);
  });

  it('does not mark a thread read while the app is in the background', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    seedList(client, 3);

    const { rerender } = renderMarker(client, { conversationId: 'c1', lastMessageId: null });
    act(() => {
      for (const listener of listeners) listener('background');
    });

    rerender({ conversationId: 'c1', lastMessageId: 'm9' });
    expect(mockMarkConversationRead).not.toHaveBeenCalled();

    // Coming back to the foreground with the thread still open marks it read.
    act(() => {
      for (const listener of listeners) listener('active');
    });
    await waitFor(() => expect(mockMarkConversationRead).toHaveBeenCalledWith('c1'));
  });

  it('does nothing for a composer with no conversation yet', () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    renderMarker(client, { conversationId: null, lastMessageId: null });
    expect(mockMarkConversationRead).not.toHaveBeenCalled();
  });
});
