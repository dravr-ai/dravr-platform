// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: useMessages.stopTurn asks the server to end the running turn and renders the notice the stream ends on (carnet#705)
// ABOUTME: Covers the stop request, the stopped notice landing as the reply, a stop with nothing left to end, and a stop that could not be sent

import React from 'react';
import { renderHook as rtlRenderHook, act, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockSendTurn = jest.fn();
const mockStopTurn = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversationMessages: jest.fn().mockResolvedValue({ messages: [] }),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    stopTurn: (...args: unknown[]) => mockStopTurn(...args),
    getConversationVerdicts: jest.fn().mockResolvedValue({ verdicts: [] }),
  },
}));

import { useMessages } from '../src/screens/chat/useMessages';

const CONVERSATION_ID = 'conv-1';
const QUESTION = 'How does my week look?';
const STOPPED_NOTICE = 'Reply stopped.';

type TurnOptions = { onDone?: (turn: unknown) => void };

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

/** The envelope the server ends a turn with. */
function turnEnding(content: string, finishReason: string, model: string) {
  return {
    user_message: { id: 'u1', role: 'user', content: QUESTION, created_at: '2026-09-30T08:00:00Z' },
    assistant: {
      message: { id: 'a1', role: 'assistant', content, created_at: '2026-09-30T08:00:05Z', finish_reason: finishReason },
      blocks: [{ type: 'prose', text: content }],
      finish_reason: finishReason,
    },
    telemetry: { model, execution_time_ms: 10 },
  };
}

/** A turn that stays in flight until the test ends it. */
function heldTurn() {
  let finish: (turn: unknown) => void = () => undefined;
  mockSendTurn.mockImplementationOnce(
    (_id: string, _content: string, options: TurnOptions) =>
      new Promise<void>(resolve => {
        finish = turn => {
          options.onDone?.(turn);
          resolve();
        };
      }),
  );
  return { finish: (turn: unknown) => finish(turn) };
}

describe('useMessages — stopping a running turn', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('does nothing while no turn is running', async () => {
    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.stopTurn();
    });
    expect(mockStopTurn).not.toHaveBeenCalled();
    expect(result.current.isStopping).toBe(false);
  });

  it('stops the conversation the turn was posted to and renders the notice the stream ends on', async () => {
    const turn = heldTurn();
    mockStopTurn.mockResolvedValue(true);
    const { result } = renderHook(() => useMessages());

    let sending: Promise<string | null> = Promise.resolve(null);
    act(() => {
      sending = result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });
    await waitFor(() => expect(result.current.isSending).toBe(true));

    await act(async () => {
      await result.current.stopTurn();
    });
    expect(mockStopTurn).toHaveBeenCalledTimes(1);
    expect(mockStopTurn).toHaveBeenCalledWith(CONVERSATION_ID);
    // The stop is on its way; the button waits for the stream to end.
    expect(result.current.isStopping).toBe(true);
    expect(result.current.isSending).toBe(true);

    await act(async () => {
      turn.finish(turnEnding(STOPPED_NOTICE, 'stopped', 'stopped'));
      await sending;
    });

    expect(result.current.isSending).toBe(false);
    expect(result.current.isStopping).toBe(false);
    expect(result.current.messages.map(m => [m.role, m.content])).toEqual([
      ['user', QUESTION],
      ['assistant', STOPPED_NOTICE],
    ]);
  });

  it('hands the button back when the reply had already landed', async () => {
    const turn = heldTurn();
    mockStopTurn.mockResolvedValue(false);
    const { result } = renderHook(() => useMessages());

    let sending: Promise<string | null> = Promise.resolve(null);
    act(() => {
      sending = result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });
    await waitFor(() => expect(result.current.isSending).toBe(true));

    await act(async () => {
      await result.current.stopTurn();
    });
    expect(result.current.isStopping).toBe(false);
    expect(result.current.isSending).toBe(true);

    await act(async () => {
      turn.finish(turnEnding('Your week: 42 km, all easy.', 'stop', 'mock-model'));
      await sending;
    });
    expect(result.current.messages[1].content).toBe('Your week: 42 km, all easy.');
  });

  it('hands the button back and reports it when the stop could not be sent', async () => {
    const turn = heldTurn();
    mockStopTurn.mockRejectedValue(new Error('Network Error'));
    const { result } = renderHook(() => useMessages());

    let sending: Promise<string | null> = Promise.resolve(null);
    act(() => {
      sending = result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });
    await waitFor(() => expect(result.current.isSending).toBe(true));

    await act(async () => {
      await result.current.stopTurn();
    });
    expect(result.current.isStopping).toBe(false);
    expect(result.current.isSending).toBe(true);
    expect(result.current.error).toEqual(expect.any(String));
    expect(result.current.error).not.toBe('');

    await act(async () => {
      turn.finish(turnEnding('Your week: 42 km, all easy.', 'stop', 'mock-model'));
      await sending;
    });
  });
});
