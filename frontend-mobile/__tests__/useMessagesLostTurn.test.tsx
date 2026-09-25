// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: A turn lost while the app was backgrounded, and the athlete's return to it
// ABOUTME: The re-read on return shows the reply the server finished exactly once, in place of the note

import React from 'react';
import { renderHook as rtlRenderHook, act, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  IDLE_STOP_AFTER_MS,
  IdleWatch,
  idleAbort,
  registerIdleWatch,
  resetIdleAbort,
} from '@pierre/shared-constants';

const mockGetConversationMessages = jest.fn();
const mockGetConversationVerdicts = jest.fn();
const mockSendTurn = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversationMessages: (...args: unknown[]) => mockGetConversationMessages(...args),
    getConversationVerdicts: (...args: unknown[]) => mockGetConversationVerdicts(...args),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    submitMessageFeedback: jest.fn(),
    deleteMessageFeedback: jest.fn(),
  },
}));

import { useMessages } from '../src/screens/chat/useMessages';
import type { Message } from '../src/types';
import { TurnIdleAbortedError } from '@pierre/api-client';
import { i18n } from '@pierre/i18n';

/** The note the chat shows for a turn the idle stop dropped: the catalogue's own words. */
const LOST_NOTE = i18n.t('chat.turnIdleAborted');
const QUESTION = 'How was my week?';
const REPLY = 'Your week: 42 km, all of it easy.';

const EARLIER: Message[] = [
  { id: 'm1', role: 'user', content: 'Hello', created_at: '2026-09-21T23:40:00Z' },
  { id: 'm2', role: 'assistant', content: 'Hi, ready when you are.', created_at: '2026-09-21T23:40:04Z' },
];
/** The server persisted the question at dispatch, before any reply. */
const QUESTION_ONLY: Message[] = [
  ...EARLIER,
  { id: 'm3', role: 'user', content: QUESTION, created_at: '2026-09-21T23:46:22Z' },
];
/** ...and went on to persist the reply after the app's stream was gone. */
const ANSWERED: Message[] = [
  ...QUESTION_ONLY,
  { id: 'm4', role: 'assistant', content: REPLY, created_at: '2026-09-21T23:47:19Z' },
];

const OLD_REPLY = 'Your week: 40 km, mostly easy.';
/** A thread whose question already has a stored answer the athlete regenerates. */
const ANSWERED_BEFORE: Message[] = [
  ...QUESTION_ONLY,
  { id: 'm4', role: 'assistant', content: OLD_REPLY, created_at: '2026-09-21T23:46:30Z' },
];
/** Regenerate deletes nothing: the old reply stays, and the re-sent question is new. */
const REGENERATING: Message[] = [
  ...ANSWERED_BEFORE,
  { id: 'm5', role: 'user', content: QUESTION, created_at: '2026-09-21T23:48:00Z' },
];
const REGENERATED: Message[] = [
  ...REGENERATING,
  { id: 'm6', role: 'assistant', content: REPLY, created_at: '2026-09-21T23:48:41Z' },
];

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

/** The turn's transport, held open so the test decides how it ends. */
interface OpenTurn {
  signal?: AbortSignal;
  /** Drop the connection the way a suspended app's socket goes. */
  dropConnection: () => void;
}

function holdTurnOpen(): OpenTurn {
  const turn: OpenTurn = { dropConnection: () => undefined };
  mockSendTurn.mockImplementation(
    (
      _conversationId: string,
      _content: string,
      options: { signal?: AbortSignal; onError?: (error: Error) => void },
    ) =>
      new Promise<void>(resolve => {
        turn.signal = options.signal;
        // Faithful to `sendTurn`: an aborted signal ends the turn with the
        // idle note, a transport failure with the runtime's own text.
        options.signal?.addEventListener('abort', () => {
          options.onError?.(new TurnIdleAbortedError());
          resolve();
        });
        turn.dropConnection = () => {
          options.onError?.(new Error('Network request failed'));
          resolve();
        };
      }),
  );
  return turn;
}

describe('useMessages turn lost while the app was backgrounded', () => {
  let watch: IdleWatch;

  beforeEach(() => {
    jest.clearAllMocks();
    jest.useFakeTimers();
    resetIdleAbort();
    // The app root's binding, minus React Query: the idle stop drops the
    // stream, the return starts a fresh stretch.
    watch = new IdleWatch({
      onIdle: idleAbort,
      onSuspend: () => undefined,
      onActive: resetIdleAbort,
    });
    registerIdleWatch(watch);
    mockGetConversationVerdicts.mockResolvedValue({ verdicts: [] });
    mockGetConversationMessages.mockResolvedValue({ messages: EARLIER });
  });

  afterEach(() => {
    registerIdleWatch(null);
    watch.stop();
    jest.useRealTimers();
  });

  /** Open the thread, send the question, and leave for another app. */
  async function sendThenBackground(result: { current: ReturnType<typeof useMessages> }) {
    await act(async () => {
      await result.current.loadMessages('conv-1');
    });
    let sending: Promise<string | null> = Promise.resolve(null);
    await act(async () => {
      sending = result.current.sendTurn('conv-1', QUESTION);
      await Promise.resolve();
    });
    await act(async () => {
      watch.suspend();
    });
    return () => sending;
  }

  it('shows the reply the server finished while the app was away, once, in place of the note', async () => {
    const turn = holdTurnOpen();
    const { result } = renderHook(() => useMessages());
    const sending = await sendThenBackground(result);

    // Backgrounding is not what drops the turn; the whole threshold is.
    expect(turn.signal?.aborted).toBe(false);
    mockGetConversationMessages.mockResolvedValue({ messages: QUESTION_ONLY });
    await act(async () => {
      jest.advanceTimersByTime(IDLE_STOP_AFTER_MS);
      await sending();
    });
    expect(turn.signal?.aborted).toBe(true);

    const note = result.current.messages.find(m => m.isError);
    // The idle note carries its own guidance and is not told to "try again".
    expect(note?.content).toBe(`⚠️ ${LOST_NOTE}`);

    // The server finishes while the athlete is still away; they come back.
    mockGetConversationMessages.mockResolvedValue({ messages: ANSWERED });
    await act(async () => {
      watch.resume();
    });

    await waitFor(() => expect(result.current.messages.map(m => m.id)).toEqual(['m1', 'm2', 'm3', 'm4']));
    expect(result.current.messages.filter(m => m.content === REPLY)).toHaveLength(1);
    expect(result.current.messages.filter(m => m.content === QUESTION)).toHaveLength(1);
    expect(result.current.messages.some(m => m.isError)).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it('recovers a turn whose connection the platform dropped while the app was backgrounded', async () => {
    const turn = holdTurnOpen();
    const { result } = renderHook(() => useMessages());
    const sending = await sendThenBackground(result);

    // Not the idle stop: the OS let the socket go during a minute on Strava.
    await act(async () => {
      jest.advanceTimersByTime(60_000);
      turn.dropConnection();
      await sending();
    });
    expect(turn.signal?.aborted).toBe(false);
    expect(result.current.messages.some(m => m.isError)).toBe(true);

    mockGetConversationMessages.mockResolvedValue({ messages: ANSWERED });
    await act(async () => {
      watch.resume();
    });

    await waitFor(() => expect(result.current.messages.map(m => m.id)).toEqual(['m1', 'm2', 'm3', 'm4']));
    expect(result.current.messages.filter(m => m.content === REPLY)).toHaveLength(1);
  });

  it('keeps the note when the reply has not been written by the time the athlete is back', async () => {
    holdTurnOpen();
    const { result } = renderHook(() => useMessages());
    const sending = await sendThenBackground(result);

    mockGetConversationMessages.mockResolvedValue({ messages: QUESTION_ONLY });
    await act(async () => {
      jest.advanceTimersByTime(IDLE_STOP_AFTER_MS);
      await sending();
    });
    const reads = mockGetConversationMessages.mock.calls.length;

    await act(async () => {
      watch.resume();
    });

    await waitFor(() => expect(mockGetConversationMessages.mock.calls.length).toBe(reads + 1));
    expect(result.current.messages.some(m => m.isError && m.content === `⚠️ ${LOST_NOTE}`)).toBe(true);
    expect(result.current.messages.some(m => m.content === REPLY)).toBe(false);
  });

  it('leaves the screen alone when the athlete has moved off the thread', async () => {
    holdTurnOpen();
    const { result } = renderHook(() => useMessages());
    const sending = await sendThenBackground(result);

    await act(async () => {
      jest.advanceTimersByTime(IDLE_STOP_AFTER_MS);
      await sending();
    });
    // The screen opened a fresh chat before the return reached the re-read.
    act(() => {
      result.current.clearMessages();
    });
    const reads = mockGetConversationMessages.mock.calls.length;
    mockGetConversationMessages.mockResolvedValue({ messages: ANSWERED });

    await act(async () => {
      watch.resume();
    });

    expect(mockGetConversationMessages.mock.calls.length).toBe(reads);
    expect(result.current.messages).toEqual([]);
  });
  it('puts the question back under a reload when the server never received the turn', async () => {
    const turn = holdTurnOpen();
    const { result } = renderHook(() => useMessages());
    const sending = await sendThenBackground(result);
    await act(async () => {
      jest.advanceTimersByTime(60_000);
      turn.dropConnection();
      await sending();
    });

    // The screen reloads the thread; the server holds nothing of this turn.
    mockGetConversationMessages.mockResolvedValue({ messages: EARLIER });
    await act(async () => {
      await result.current.loadMessages('conv-1');
    });

    const rows = result.current.messages;
    expect(rows.map(m => m.id).slice(0, 2)).toEqual(['m1', 'm2']);
    expect(rows).toHaveLength(4);
    // The athlete's line, then the note whose Retry re-sends it.
    expect(rows[2]).toMatchObject({ role: 'user', content: QUESTION });
    expect(rows[3]).toMatchObject({
      isError: true,
      content: `⚠️ Network request failed\n\n${i18n.t('chat.turnTryAgain')}`,
    });
    expect(result.current.error).toBe('Network request failed');
  });

  it('words the idle note in the athlete\'s language', async () => {
    await act(async () => {
      await i18n.changeLanguage('fr');
    });
    try {
      holdTurnOpen();
      const { result } = renderHook(() => useMessages());
      const sending = await sendThenBackground(result);
      mockGetConversationMessages.mockResolvedValue({ messages: QUESTION_ONLY });
      await act(async () => {
        jest.advanceTimersByTime(IDLE_STOP_AFTER_MS);
        await sending();
      });
      const note = result.current.messages.find(m => m.isError);
      expect(note?.content).toBe(`⚠️ ${i18n.t('chat.turnIdleAborted')}`);
      expect(note?.content).toContain('Rouvre cette conversation');
      expect(result.current.error).toBe(i18n.t('chat.turnIdleAborted'));
    } finally {
      await act(async () => {
        await i18n.changeLanguage('en');
      });
    }
  });

  it('words the try-again suffix of a dropped turn in the athlete\'s language', async () => {
    await act(async () => {
      await i18n.changeLanguage('fr');
    });
    try {
      const turn = holdTurnOpen();
      const { result } = renderHook(() => useMessages());
      const sending = await sendThenBackground(result);
      await act(async () => {
        jest.advanceTimersByTime(60_000);
        turn.dropConnection();
        await sending();
      });
      const note = result.current.messages.find(m => m.isError);
      expect(note?.content).toBe('⚠️ Network request failed\n\nRéessaie.');
    } finally {
      await act(async () => {
        await i18n.changeLanguage('en');
      });
    }
  });

  it('holds the re-read back when the athlete sends a new turn before it lands', async () => {
    const turn = holdTurnOpen();
    const { result } = renderHook(() => useMessages());
    const sending = await sendThenBackground(result);
    mockGetConversationMessages.mockResolvedValue({ messages: QUESTION_ONLY });
    await act(async () => {
      jest.advanceTimersByTime(IDLE_STOP_AFTER_MS);
      await sending();
    });

    // Back in the app; the re-read is on the wire when the athlete asks again.
    let landRead: (response: { messages: Message[] }) => void = () => undefined;
    mockGetConversationMessages.mockImplementation(
      () => new Promise(resolve => {
        landRead = resolve;
      }),
    );
    const reads = mockGetConversationMessages.mock.calls.length;
    await act(async () => {
      watch.resume();
    });
    expect(mockGetConversationMessages.mock.calls.length).toBe(reads + 1);

    const SECOND = 'And next week?';
    await act(async () => {
      void result.current.sendTurn('conv-1', SECOND);
      await Promise.resolve();
    });
    await act(async () => {
      landRead({ messages: ANSWERED });
      await Promise.resolve();
    });

    // The new turn's question is still on screen: the late transcript, which
    // has never heard of it, did not paint over it.
    expect(result.current.messages.some(m => m.content === SECOND)).toBe(true);
    expect(result.current.isSending).toBe(true);

    await act(async () => {
      turn.dropConnection();
      await Promise.resolve();
    });
  });

  it('keeps a regenerated turn\'s note until its own reply lands, though the old one comes back', async () => {
    holdTurnOpen();
    mockGetConversationMessages.mockResolvedValue({ messages: ANSWERED_BEFORE });
    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.loadMessages('conv-1');
    });

    // Long-press, Retry on the stored answer, then off to another app.
    let retrying: Promise<void> = Promise.resolve();
    await act(async () => {
      retrying = result.current.retryMessage('m4', 'conv-1');
      await Promise.resolve();
    });
    await act(async () => {
      watch.suspend();
    });
    mockGetConversationMessages.mockResolvedValue({ messages: REGENERATING });
    await act(async () => {
      jest.advanceTimersByTime(IDLE_STOP_AFTER_MS);
      await retrying;
    });
    expect(result.current.messages.some(m => m.isError)).toBe(true);

    // Back: the server still stores the old reply and has the re-sent
    // question, and nothing answers it yet.
    const reads = mockGetConversationMessages.mock.calls.length;
    await act(async () => {
      watch.resume();
    });
    await waitFor(() => expect(mockGetConversationMessages.mock.calls.length).toBe(reads + 1));
    await waitFor(() =>
      expect(result.current.messages.map(m => m.id).slice(0, 5)).toEqual(['m1', 'm2', 'm3', 'm4', 'm5']),
    );
    const note = result.current.messages[5];
    expect(note?.isError).toBe(true);
    expect(note?.content).toBe(`⚠️ ${LOST_NOTE}`);
    expect(result.current.messages.some(m => m.content === REPLY)).toBe(false);

    // The regenerated reply lands, and the next read of the thread shows it
    // once and takes the note down.
    mockGetConversationMessages.mockResolvedValue({ messages: REGENERATED });
    await act(async () => {
      await result.current.loadMessages('conv-1');
    });
    expect(result.current.messages.map(m => m.id)).toEqual(['m1', 'm2', 'm3', 'm4', 'm5', 'm6']);
    expect(result.current.messages.filter(m => m.content === REPLY)).toHaveLength(1);
    expect(result.current.error).toBeNull();
  });
});
