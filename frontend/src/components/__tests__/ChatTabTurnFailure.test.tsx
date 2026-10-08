// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A failed chat turn on web reads in the athlete's language, keeps their question, and offers a retry only where one can help
// ABOUTME: Covers the fail-fast 503 the server never stored, a mid-stream failed frame it did, and a spent quota (carnet#680)

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Message } from '@pierre/shared-types';
import { TurnFailedError, TurnRequestError } from '@pierre/api-client';
import { i18n } from '@pierre/i18n';
import ChatTab from '../ChatTab';
import { ToastProvider } from '../ui';

const CONVERSATION_ID = 'conv-1';

const getConversations = vi.fn();
const getConversationMessages = vi.fn();
const getConversationVerdicts = vi.fn();
const listParticipants = vi.fn();
const sendTurn = vi.fn();
const listCoaches = vi.fn();
const getProvidersStatus = vi.fn();

vi.mock('../../services/api', () => ({
  // The header's notifications bell reads the unread count.
  notificationsApi: { getUnreadCount: async () => ({ unread_count: 0 }) },
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

const QUESTION = 'Comment se présente ma semaine ?';
const REPLY = 'Ta semaine : 42 km, tout en endurance.';
/** What the server says, in English, when it refuses a turn fast. */
const SERVER_TEXT = 'The resource is temporarily unavailable';

const EARLIER: Message[] = [
  { id: 'm1', role: 'user', content: 'Salut', created_at: '2026-09-30T08:00:00Z' },
  { id: 'm2', role: 'assistant', content: 'Salut, je suis prêt.', created_at: '2026-09-30T08:00:04Z' },
];

type TurnOptions = {
  onDelta?: (delta: string) => void;
  onDone?: (turn: unknown) => void;
  onError?: (error: Error) => void;
};

/** A turn the server answered in full: the envelope the transport hands `onDone`. */
function answeredTurn(content: string) {
  return {
    assistant: { message: { id: 'm4', role: 'assistant', content, created_at: '2026-09-30T08:01:00Z' }, blocks: [] },
    telemetry: { model: 'mock-model', execution_time_ms: 10 },
  };
}

function renderChat() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <ChatTab selectedConversation={CONVERSATION_ID} onSelectConversation={vi.fn()} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

async function send(text: string) {
  const user = userEvent.setup();
  const input = await screen.findByPlaceholderText(i18n.t('chat.messageDravrPlaceholder'));
  await user.type(input, text);
  await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));
  return user;
}

describe('ChatTab — a failed turn, in French', () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    await act(async () => {
      await i18n.changeLanguage('fr');
    });
    getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] });
    getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    listParticipants.mockResolvedValue([]);
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Semi-marathon', agent_id: null }],
      total: 1,
    });
    listCoaches.mockResolvedValue({ agents: [] });
    // The server never stored the refused turn: every read is the thread as it was.
    getConversationMessages.mockResolvedValue({ messages: EARLIER });
  });

  afterEach(async () => {
    await act(async () => {
      await i18n.changeLanguage('en');
    });
  });

  it('words a fail-fast 503 in French, keeps the question and re-sends it on retry', async () => {
    sendTurn.mockImplementationOnce((_id: string, _content: string, options: TurnOptions) => {
      options.onError?.(
        new TurnRequestError(SERVER_TEXT, 503, { code: 'ResourceUnavailable', message: SERVER_TEXT }),
      );
      return Promise.resolve();
    });
    renderChat();
    const user = await send(QUESTION);

    const note = await screen.findByTestId('turn-error');
    expect(note).toHaveTextContent(i18n.t('errors.generic'));
    expect(i18n.t('errors.generic')).toContain('temporairement indisponible');
    expect(screen.queryByText(SERVER_TEXT)).toBeNull();

    // The re-read after the failure holds no copy of the question; the thread
    // still shows it, once.
    await waitFor(() => expect(getConversationMessages.mock.calls.length).toBeGreaterThanOrEqual(2));
    expect(screen.getAllByText(QUESTION)).toHaveLength(1);

    sendTurn.mockImplementationOnce((_id: string, _content: string, options: TurnOptions) => {
      options.onDone?.(answeredTurn(REPLY));
      return Promise.resolve();
    });
    await user.click(screen.getByRole('button', { name: i18n.t('chat.retry') }));

    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(2));
    expect(sendTurn.mock.calls[1][1]).toBe(QUESTION);
    expect(await screen.findByText(REPLY)).toBeInTheDocument();
    expect(screen.queryByTestId('turn-error')).toBeNull();
  });

  it('words a mid-stream failure from its code and offers no retry for the question the server stored', async () => {
    sendTurn.mockImplementationOnce((_id: string, _content: string, options: TurnOptions) => {
      options.onDelta?.('Je regarde ');
      // A mid-stream failure: the server stored the question before it ran.
      getConversationMessages.mockResolvedValue({
        messages: [...EARLIER, { id: 'm3', role: 'user', content: QUESTION, created_at: '2026-09-30T08:01:00Z' }],
      });
      options.onError?.(new TurnFailedError(SERVER_TEXT, 'ResourceUnavailable'));
      return Promise.resolve();
    });
    renderChat();
    await send(QUESTION);

    const note = await screen.findByTestId('turn-error');
    expect(note).toHaveTextContent(i18n.t('errors.generic'));
    expect(screen.queryByText(SERVER_TEXT)).toBeNull();
    await waitFor(() => expect(getConversationMessages.mock.calls.length).toBeGreaterThanOrEqual(2));
    await waitFor(() => expect(screen.getAllByText(QUESTION)).toHaveLength(1));
    // A re-send would store the same question a second time.
    expect(screen.queryByRole('button', { name: i18n.t('chat.retry') })).toBeNull();
  });

  it('offers no retry for a spent quota, which a re-send would only meet again', async () => {
    sendTurn.mockImplementationOnce((_id: string, _content: string, options: TurnOptions) => {
      options.onError?.(
        new TurnRequestError('Daily message limit reached', 429, {
          code: 'QuotaExceeded',
          message: 'Daily message limit reached',
          details: { limit_type: 'daily_messages', current: 50, limit: 50 },
        }),
      );
      return Promise.resolve();
    });
    renderChat();
    await send(QUESTION);

    const note = await screen.findByTestId('turn-error');
    expect(note).toHaveTextContent('Limite de messages quotidiens atteinte (50/50). Réinitialisation demain.');
    await waitFor(() => expect(getConversationMessages.mock.calls.length).toBeGreaterThanOrEqual(2));
    // The refused question still stands in the thread, without a retry.
    expect(screen.getAllByText(QUESTION)).toHaveLength(1);
    expect(screen.queryByRole('button', { name: i18n.t('chat.retry') })).toBeNull();
  });
});
