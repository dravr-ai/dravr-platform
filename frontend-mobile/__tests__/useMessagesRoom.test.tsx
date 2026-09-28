// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins how the mobile thread reads a group conversation — the room woven through the caller's own rows
// ABOUTME: Covers the room read back to the caller's oldest row, the unreadable-room flag, and a retry that skips room rows

import React from 'react';
import { renderHook as rtlRenderHook, act } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { GroupTranscriptEntry } from '@pierre/shared-types';

const mockGetConversationMessages = jest.fn();
const mockGetConversationVerdicts = jest.fn().mockResolvedValue({ verdicts: [] });
const mockSendTurn = jest.fn();
const mockReadRoom = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversationMessages: (...args: unknown[]) => mockGetConversationMessages(...args),
    getConversationVerdicts: (...args: unknown[]) => mockGetConversationVerdicts(...args),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
  },
  groupsApi: {
    readRoom: (...args: unknown[]) => mockReadRoom(...args),
  },
}));

import { useMessages } from '../src/screens/chat/useMessages';
import type { Message } from '../src/types';

function renderHook<T>(hook: () => T) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return rtlRenderHook(hook, {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}

const CONVERSATION_ID = 'conv-room';
const GROUP_ID = 'group-7';

const OWN = [
  { id: 'm1', role: 'user', content: 'How was my week?', created_at: '2026-09-14T10:00:00Z' },
  { id: 'm2', role: 'assistant', content: 'Solid week.', created_at: '2026-09-14T10:00:05Z' },
];

function entry(overrides: Partial<GroupTranscriptEntry> & Pick<GroupTranscriptEntry, 'id' | 'created_at'>): GroupTranscriptEntry {
  return {
    speaker: 'member',
    withheld: false,
    own: false,
    author_user_id: 'user-jd',
    author_display_name: 'J-D',
    content: null,
    message_id: null,
    ...overrides,
  };
}

const ROOM: GroupTranscriptEntry[] = [
  entry({ id: 't1', created_at: '2026-09-14T10:00:00.2Z', own: true, content: 'How was my week?', message_id: 'm1' }),
  entry({ id: 't2', created_at: '2026-09-14T11:00:00Z', content: 'Tempo on Saturday?', message_id: 'jd-1' }),
  entry({ id: 't3', created_at: '2026-09-14T11:00:04Z', speaker: 'coach', content: 'Easy 90 minutes, J-D.', message_id: 'jd-2' }),
  entry({ id: 't4', created_at: '2026-09-14T11:05:00Z', withheld: true, author_user_id: null, author_display_name: null }),
];

describe('useMessages in a group thread', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetConversationMessages.mockResolvedValue({ messages: OWN, feedback: [] });
  });

  it('weaves the room through the caller\'s own rows, reading it back to their oldest row', async () => {
    mockReadRoom.mockResolvedValue(ROOM);
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID, GROUP_ID);
    });

    expect(mockReadRoom).toHaveBeenCalledWith(GROUP_ID, '2026-09-14T10:00:00Z');
    expect(result.current.messages.map((m) => m.id)).toEqual(['m1', 'm2', 'room-t2', 'room-t3', 'room-t4']);
    expect(result.current.messages[2].room?.author_name).toBe('J-D');
    expect(result.current.messages[3].content).toBe('Easy 90 minutes, J-D.');
    expect(result.current.messages[4].room?.withheld).toBe(true);
    expect(result.current.roomUnavailable).toBe(false);
  });

  it('reads no room for a thread that is not a group\'s', async () => {
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID, null);
    });

    expect(mockReadRoom).not.toHaveBeenCalled();
    expect(result.current.messages.map((m) => m.id)).toEqual(['m1', 'm2']);
  });

  it('keeps the caller\'s own rows and flags the room when it cannot be read', async () => {
    mockReadRoom.mockRejectedValue(new Error('network down'));
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID, GROUP_ID);
    });

    expect(result.current.messages.map((m) => m.id)).toEqual(['m1', 'm2']);
    expect(result.current.roomUnavailable).toBe(true);
  });

  it('retries the caller\'s own question, never another member\'s line that sits before the failed reply', async () => {
    // A failed reply the client kept, with another member's line landing
    // between the caller's question and it.
    const failed: Message = {
      id: 'error-1',
      role: 'assistant',
      content: '⚠️ failed',
      created_at: '2026-09-14T12:00:10Z',
      isError: true,
    };
    mockGetConversationMessages.mockResolvedValue({
      messages: [{ id: 'q1', role: 'user', content: 'Plan my Sunday?', created_at: '2026-09-14T12:00:00Z' }],
      feedback: [],
    });
    mockReadRoom.mockResolvedValue([
      entry({ id: 't9', created_at: '2026-09-14T12:00:05Z', content: 'I am in for Sunday' }),
    ]);
    mockSendTurn.mockImplementation(async () => {});
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID, GROUP_ID);
    });
    act(() => {
      result.current.setMessages((prev) => [...prev, failed]);
    });
    expect(result.current.messages.map((m) => m.id)).toEqual(['q1', 'room-t9', 'error-1']);

    await act(async () => {
      await result.current.retryMessage('error-1', CONVERSATION_ID);
    });

    expect(mockSendTurn).toHaveBeenCalledTimes(1);
    expect(mockSendTurn.mock.calls[0][0]).toBe(CONVERSATION_ID);
    expect(mockSendTurn.mock.calls[0][1]).toBe('Plan my Sunday?');
  });
});
