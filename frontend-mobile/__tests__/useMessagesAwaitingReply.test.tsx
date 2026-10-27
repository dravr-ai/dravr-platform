// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: useMessages restores a turn the server is still answering when the app reopens, and re-reads until its reply lands
// ABOUTME: Covers the poll's start and end, what stops it (another thread, a turn sent here, a stale flag), and the stop button

import React from 'react';
import { renderHook as rtlRenderHook, act } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { AWAITING_REPLY_POLL_MS } from '@pierre/chat-utils';

const mockGetConversationMessages = jest.fn();
const mockSendTurn = jest.fn();
const mockStopTurn = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversationMessages: (...args: unknown[]) => mockGetConversationMessages(...args),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    stopTurn: (...args: unknown[]) => mockStopTurn(...args),
    getConversationVerdicts: jest.fn().mockResolvedValue({ verdicts: [] }),
  },
}));

import { useMessages } from '../src/screens/chat/useMessages';

const THREAD = 'conv-awaited';
const OTHER_THREAD = 'conv-other';
const QUESTION_ID = 'q-1';
const ASKED_AT = '2026-10-26T08:00:00Z';
const NOW = Date.parse('2026-10-26T08:06:00Z');

const question = { id: QUESTION_ID, role: 'user', content: 'Build my plan', created_at: ASKED_AT };
const reply = { id: 'a-1', role: 'assistant', content: 'Your plan is ready.', created_at: '2026-10-26T08:08:00Z' };

/** The thread while the server still writes its reply. */
const awaited = () => ({
  messages: [question],
  awaiting_reply: { question_id: QUESTION_ID, asked_at: ASKED_AT },
});
/** The thread once the reply landed. */
const answered = () => ({ messages: [question, reply], awaiting_reply: null });

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

/** Let the poll's timer fire and its read settle. */
async function nextPoll() {
  await act(async () => {
    await jest.advanceTimersByTimeAsync(AWAITING_REPLY_POLL_MS);
  });
}

describe('useMessages — a turn still running when the app reopens', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.useFakeTimers({ now: NOW, doNotFake: ['nextTick', 'setImmediate'] });
  });

  afterEach(() => {
    jest.useRealTimers();
  });

  it('shows the turn from when it was asked and re-reads every few seconds until the reply lands', async () => {
    mockGetConversationMessages
      .mockResolvedValueOnce(awaited())
      .mockResolvedValueOnce(awaited())
      .mockResolvedValueOnce(answered());
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(THREAD);
    });
    expect(result.current.awaitingSince).toBe(Date.parse(ASKED_AT));
    expect(result.current.turnInFlight).toBe(true);
    expect(result.current.liveTrail).toBeNull();

    await nextPoll();
    expect(mockGetConversationMessages).toHaveBeenCalledTimes(2);
    expect(result.current.turnInFlight).toBe(true);

    await nextPoll();
    expect(mockGetConversationMessages).toHaveBeenCalledTimes(3);
    expect(result.current.turnInFlight).toBe(false);
    expect(result.current.messages.map(m => m.id)).toEqual([QUESTION_ID, 'a-1']);

    await nextPoll();
    await nextPoll();
    expect(mockGetConversationMessages).toHaveBeenCalledTimes(3);
  });

  it('ignores a stale flag when the reply is already in the transcript', async () => {
    mockGetConversationMessages.mockResolvedValue({
      ...answered(),
      awaiting_reply: { question_id: QUESTION_ID, asked_at: ASKED_AT },
    });
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(THREAD);
    });
    expect(result.current.awaitingSince).toBeNull();
    expect(result.current.turnInFlight).toBe(false);

    await nextPoll();
    expect(mockGetConversationMessages).toHaveBeenCalledTimes(1);
  });

  it('stops re-reading once the athlete opens another thread', async () => {
    mockGetConversationMessages.mockImplementation((id: string) =>
      Promise.resolve(id === THREAD ? awaited() : { messages: [] }),
    );
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(THREAD);
    });
    await act(async () => {
      await result.current.loadMessages(OTHER_THREAD);
    });
    expect(result.current.turnInFlight).toBe(false);

    await nextPoll();
    await nextPoll();
    expect(mockGetConversationMessages.mock.calls.map(call => call[0])).toEqual([THREAD, OTHER_THREAD]);
  });

  it('stops re-reading while a turn sent here brings its own reply', async () => {
    mockGetConversationMessages.mockResolvedValue(awaited());
    let finish: () => void = () => undefined;
    mockSendTurn.mockImplementationOnce(
      () =>
        new Promise<void>(resolve => {
          finish = resolve;
        }),
    );
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(THREAD);
    });
    let sending: Promise<string | null> = Promise.resolve(null);
    act(() => {
      sending = result.current.sendTurn(THREAD, 'And the week after?');
    });
    expect(result.current.awaitingSince).toBeNull();
    expect(result.current.liveTrail).not.toBeNull();
    expect(result.current.turnInFlight).toBe(true);

    await nextPoll();
    await nextPoll();
    expect(mockGetConversationMessages).toHaveBeenCalledTimes(1);

    await act(async () => {
      finish();
      await sending;
    });
  });

  it('keeps re-reading through a failed read', async () => {
    mockGetConversationMessages
      .mockResolvedValueOnce(awaited())
      .mockRejectedValueOnce(new Error('Network Error'))
      .mockResolvedValueOnce(answered());
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(THREAD);
    });
    await nextPoll();
    expect(result.current.turnInFlight).toBe(true);
    await nextPoll();
    expect(mockGetConversationMessages).toHaveBeenCalledTimes(3);
    expect(result.current.turnInFlight).toBe(false);
  });

  it('stops the restored turn on the open thread and re-reads it for the notice', async () => {
    const stopped = {
      messages: [question, { ...reply, id: 'a-stop', content: 'Reply stopped.' }],
      awaiting_reply: null,
    };
    mockGetConversationMessages.mockResolvedValueOnce(awaited()).mockResolvedValueOnce(stopped);
    mockStopTurn.mockResolvedValue(true);
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(THREAD);
    });
    await act(async () => {
      await result.current.stopTurn();
    });

    expect(mockStopTurn).toHaveBeenCalledWith(THREAD);
    expect(result.current.isStopping).toBe(false);
    expect(result.current.turnInFlight).toBe(false);
    expect(result.current.messages.map(m => m.content)).toEqual(['Build my plan', 'Reply stopped.']);
  });

  it('hands the stop button back when the athlete leaves a thread whose stop is pending', async () => {
    mockGetConversationMessages.mockImplementation((id: string) =>
      Promise.resolve(id === THREAD ? awaited() : { messages: [] }),
    );
    mockStopTurn.mockResolvedValue(true);
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(THREAD);
    });
    await act(async () => {
      await result.current.stopTurn();
    });
    expect(result.current.isStopping).toBe(true);

    await act(async () => {
      await result.current.loadMessages(OTHER_THREAD);
    });
    expect(result.current.isStopping).toBe(false);
  });

  it('keeps the stop pending while the server has not closed the question yet', async () => {
    mockGetConversationMessages
      .mockResolvedValueOnce(awaited())
      .mockResolvedValueOnce(awaited())
      .mockResolvedValueOnce(answered());
    mockStopTurn.mockResolvedValue(true);
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(THREAD);
    });
    await act(async () => {
      await result.current.stopTurn();
    });
    expect(result.current.isStopping).toBe(true);
    expect(result.current.turnInFlight).toBe(true);

    await nextPoll();
    expect(result.current.isStopping).toBe(false);
    expect(result.current.turnInFlight).toBe(false);
  });
});
