// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: useMessages reads a retract frame: the live trail leaves "writing" for "thinking" until the next words arrive
// ABOUTME: Words before a tool call, a stream that broke off or a refusal are taken back; the phone's trail follows them

import React from 'react';
import { renderHook as rtlRenderHook, act, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockSendTurn = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversationMessages: jest.fn().mockResolvedValue({ messages: [] }),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    stopTurn: jest.fn(),
    getConversationVerdicts: jest.fn().mockResolvedValue({ verdicts: [] }),
  },
}));

import { useMessages } from '../src/screens/chat/useMessages';

const CONVERSATION_ID = 'conv-1';
const QUESTION = 'How does my week look?';
const REPLY = 'Three easy runs this week.';

type TurnOptions = {
  onDone?: (turn: unknown) => void;
  onDelta?: (delta: string) => void;
  onRetract?: () => void;
};

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

/** A turn that stays in flight until the test ends it, its live callbacks exposed. */
function heldTurn() {
  let options: TurnOptions = {};
  let resolveTurn: () => void = () => undefined;
  mockSendTurn.mockImplementationOnce(
    (_id: string, _content: string, turnOptions: TurnOptions) =>
      new Promise<void>(resolve => {
        options = turnOptions;
        resolveTurn = resolve;
      }),
  );
  return {
    delta: (text: string) => options.onDelta?.(text),
    retract: () => options.onRetract?.(),
    finish: (turn: unknown) => {
      options.onDone?.(turn);
      resolveTurn();
    },
  };
}

describe('useMessages — streamed words taken back', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('returns the live trail to thinking, then to writing when the reply starts', async () => {
    const turn = heldTurn();
    const { result } = renderHook(() => useMessages());

    let sending: Promise<string | null> = Promise.resolve(null);
    act(() => {
      sending = result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });
    await waitFor(() => expect(result.current.liveTrail).not.toBeNull());

    act(() => turn.delta('Let me look at your runs.'));
    expect(result.current.liveTrail?.phase).toBe('writing');

    act(() => turn.retract());
    expect(result.current.liveTrail?.phase).toBe('thinking');

    act(() => turn.delta(REPLY));
    expect(result.current.liveTrail?.phase).toBe('writing');

    await act(async () => {
      turn.finish({
        user_message: { id: 'u1', role: 'user', content: QUESTION, created_at: '2026-10-26T08:00:00Z' },
        assistant: {
          message: { id: 'a1', role: 'assistant', content: REPLY, created_at: '2026-10-26T08:00:05Z' },
          blocks: [{ type: 'prose', text: REPLY }],
          finish_reason: 'stop',
        },
        telemetry: { model: 'claude-opus-5-5', execution_time_ms: 10 },
      });
      await sending;
    });
    expect(result.current.messages.map(m => [m.role, m.content])).toEqual([
      ['user', QUESTION],
      ['assistant', REPLY],
    ]);
  });
});
