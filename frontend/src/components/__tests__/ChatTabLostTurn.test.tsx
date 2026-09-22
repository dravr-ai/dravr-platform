// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A turn whose stream the idle stop dropped while the tab was hidden, and the athlete's return
// ABOUTME: The re-read on return renders the reply the server finished exactly once and takes the note down

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { QueryClient, QueryClientProvider, focusManager } from '@tanstack/react-query';
import type { Message } from '@pierre/shared-types';
import { IDLE_STOP_AFTER_MS } from '@pierre/shared-constants';
import ChatTab from '../ChatTab';
import { ToastProvider } from '../ui';
import { useIdleWatch } from '../../hooks/useIdleWatch';
import { resetIdleAbort } from '../../services/api/idleSignal';

const CONVERSATION_ID = 'conv-1';

const getConversations = vi.fn();
const getConversationMessages = vi.fn();
const getConversationVerdicts = vi.fn();
const listParticipants = vi.fn();
const sendTurn = vi.fn();
const listCoaches = vi.fn();
const getProvidersStatus = vi.fn();

vi.mock('../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: (...a: unknown[]) => getConversationMessages(...a),
    getConversationVerdicts: (...a: unknown[]) => getConversationVerdicts(...a),
    listParticipants: (...a: unknown[]) => listParticipants(...a),
    sendTurn: (...a: unknown[]) => sendTurn(...a),
    markConversationRead: vi.fn().mockResolvedValue(undefined),
    createConversation: vi.fn(),
    updateConversation: vi.fn(),
    deleteConversation: vi.fn(),
    markConversationUnread: vi.fn(),
  },
  coachesApi: { list: (...a: unknown[]) => listCoaches(...a) },
  providersApi: { getProvidersStatus: (...a: unknown[]) => getProvidersStatus(...a) },
  groupsApi: {},
}));

vi.mock('../../services/analytics', () => ({ track: vi.fn() }));
vi.mock('../../hooks/useAuth', () => ({ useAuth: () => ({ token: 'test-token' }) }));
vi.mock('../../hooks/useUsageStatus', () => ({
  useUsageStatus: () => ({
    data: undefined,
    isLoading: false,
    error: null,
    level: 'none',
    sendDisabled: false,
    message: '',
    resetsAt: '',
    triggerCounter: null,
    invalidate: vi.fn(),
    applyNotice: vi.fn(),
  }),
}));

/** What the mocked transport reports for an aborted turn — `sendTurn`'s own path. */
const LOST_NOTE = 'The app went idle before this reply arrived, so it may still have been written.';
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
/** ...and went on to persist the reply after the client's stream was gone. */
const ANSWERED: Message[] = [
  ...QUESTION_ONLY,
  { id: 'm4', role: 'assistant', content: REPLY, created_at: '2026-09-21T23:47:19Z' },
];

/** The chat surface under the idle watch, as `App` mounts them. */
function IdleWatched({ children }: { children: ReactNode }) {
  useIdleWatch();
  return <>{children}</>;
}

function renderWatchedChat() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <IdleWatched>
          <ChatTab selectedConversation={CONVERSATION_ID} onSelectConversation={vi.fn()} />
        </IdleWatched>
      </ToastProvider>
    </QueryClientProvider>,
  );
}

/**
 * Answer the next send the way the real transport answers an aborted one: the
 * stream stays open until the signal fires, then `onError` carries the note
 * and the call resolves.
 */
function streamUntilAborted() {
  let signal: AbortSignal | undefined;
  sendTurn.mockImplementation(
    (
      _conversationId: string,
      _content: string,
      options: { signal?: AbortSignal; onError?: (error: Error) => void },
    ) =>
      new Promise<void>(resolve => {
        signal = options.signal;
        options.signal?.addEventListener('abort', () => {
          options.onError?.(new Error(LOST_NOTE));
          resolve();
        });
      }),
  );
  return () => signal;
}

describe('ChatTab turn lost while the tab was hidden', () => {
  let visibility: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers({ shouldAdvanceTime: true });
    // Earlier files in the same worker may have left the shared controller tripped.
    resetIdleAbort();
    visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible');
    getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] });
    getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    listParticipants.mockResolvedValue([]);
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Half marathon', agent_id: null }],
      total: 1,
    });
    listCoaches.mockResolvedValue({ agents: [] });
    getConversationMessages.mockResolvedValue({ messages: EARLIER });
  });

  afterEach(() => {
    visibility.mockRestore();
    focusManager.setFocused(undefined);
    vi.useRealTimers();
  });

  async function sendThenHide() {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const input = await screen.findByPlaceholderText('Message Dravr...');
    await user.type(input, QUESTION);
    await user.click(screen.getByRole('button', { name: 'Send message' }));
    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));

    // The athlete switches to the Strava tab to authorize.
    visibility.mockReturnValue('hidden');
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
  }

  async function showTab() {
    visibility.mockReturnValue('visible');
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
  }

  it('re-reads the thread on return and renders the finished reply once, without the note', async () => {
    const turnSignal = streamUntilAborted();
    renderWatchedChat();
    await sendThenHide();

    // Hiding the tab is not what drops the turn.
    expect(turnSignal()?.aborted).toBe(false);

    // Only the whole threshold hidden does. The server has the question and
    // is still writing when the stream goes.
    getConversationMessages.mockResolvedValue({ messages: QUESTION_ONLY });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(IDLE_STOP_AFTER_MS);
    });
    expect(turnSignal()?.aborted).toBe(true);
    expect(await screen.findByText(LOST_NOTE)).toBeInTheDocument();

    // It finishes while the athlete is still away; they come back.
    getConversationMessages.mockResolvedValue({ messages: ANSWERED });
    const readsBeforeReturn = getConversationMessages.mock.calls.length;
    await showTab();

    expect(await screen.findByText(REPLY)).toBeInTheDocument();
    expect(getConversationMessages.mock.calls.length).toBeGreaterThan(readsBeforeReturn);
    // Once each: the optimistic question was replaced by the persisted one,
    // and the reply is the transcript's row, not a second copy beside it.
    expect(screen.getAllByText(REPLY)).toHaveLength(1);
    expect(screen.getAllByText(QUESTION)).toHaveLength(1);
    expect(screen.queryByText(LOST_NOTE)).toBeNull();
  });

  it('keeps the note when the reply has not landed by the time the athlete is back', async () => {
    streamUntilAborted();
    renderWatchedChat();
    await sendThenHide();

    getConversationMessages.mockResolvedValue({ messages: QUESTION_ONLY });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(IDLE_STOP_AFTER_MS);
    });
    await screen.findByText(LOST_NOTE);

    const readsBeforeReturn = getConversationMessages.mock.calls.length;
    await showTab();
    await waitFor(() =>
      expect(getConversationMessages.mock.calls.length).toBeGreaterThan(readsBeforeReturn),
    );

    // Still being written: the note stays and says what to do.
    expect(screen.getByText(LOST_NOTE)).toBeInTheDocument();
    expect(screen.queryByText(REPLY)).toBeNull();
  });
});
