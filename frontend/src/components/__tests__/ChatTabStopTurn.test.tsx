// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: While a web chat turn runs the send button is a stop button that asks the server to end the turn (carnet#705)
// ABOUTME: Covers the stop request, the stopped notice arriving as the turn's reply, and a stop the server had nothing left to end

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Message } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import ChatTab from '../ChatTab';
import { ToastProvider } from '../ui';

const CONVERSATION_ID = 'conv-1';

const getConversations = vi.fn();
const getConversationMessages = vi.fn();
const getConversationVerdicts = vi.fn();
const listParticipants = vi.fn();
const sendTurn = vi.fn();
const stopTurn = vi.fn();
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
    stopTurn: (...a: unknown[]) => stopTurn(...a),
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

const QUESTION = 'How does my week look?';
const STOPPED_NOTICE = 'Reply stopped.';
const REPLY = 'Your week: 42 km, all easy.';

const EARLIER: Message[] = [
  { id: 'm1', role: 'user', content: 'Hi', created_at: '2026-09-30T08:00:00Z' },
  { id: 'm2', role: 'assistant', content: 'Hi, ready when you are.', created_at: '2026-09-30T08:00:04Z' },
];

type TurnOptions = { onDone?: (turn: unknown) => void };

/** The envelope the transport hands `onDone`, as the server ends a turn with. */
function turnEnding(content: string, finishReason: string, model: string) {
  return {
    assistant: {
      message: { id: 'm4', role: 'assistant', content, created_at: '2026-09-30T08:01:00Z' },
      blocks: [{ type: 'prose', text: content }],
      finish_reason: finishReason,
    },
    telemetry: { model, execution_time_ms: 10 },
  };
}

/** A turn that stays in flight until the test ends it. */
function heldTurn() {
  let finish: (turn: unknown) => void = () => undefined;
  sendTurn.mockImplementationOnce(
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

describe('ChatTab — stopping a running turn', () => {
  beforeEach(() => {
    vi.clearAllMocks();
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

  it('turns send into stop while the turn runs, asks the server to stop it, and shows the notice it ends on', async () => {
    const turn = heldTurn();
    stopTurn.mockResolvedValue(true);
    renderChat();
    const user = await send(QUESTION);

    const stop = await screen.findByRole('button', { name: i18n.t('chat.stopTurnAria') });
    expect(screen.queryByRole('button', { name: i18n.t('chat.sendMessageAria') })).toBeNull();

    await user.click(stop);
    expect(stopTurn).toHaveBeenCalledTimes(1);
    expect(stopTurn).toHaveBeenCalledWith(CONVERSATION_ID);
    // The stop is sent; the button waits for the stream to end rather than
    // letting a second press through.
    await waitFor(() => expect(screen.getByTestId('stop-turn-button')).toBeDisabled());

    await act(async () => {
      turn.finish(turnEnding(STOPPED_NOTICE, 'stopped', 'stopped'));
    });

    expect(await screen.findByText(STOPPED_NOTICE)).toBeInTheDocument();
    expect(screen.getByText(QUESTION)).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: i18n.t('chat.sendMessageAria') })).toBeInTheDocument();
    expect(screen.queryByTestId('stop-turn-button')).toBeNull();
  });

  // The athlete can open another thread while a turn streams. The stop went
  // to whichever thread was on screen, so the running turn carried on and the
  // other thread was asked to stop a turn it never had.
  it('stops the conversation the turn is streaming in, not the one opened since', async () => {
    const OTHER_CONVERSATION_ID = 'conv-2';
    getConversations.mockResolvedValue({
      conversations: [
        { id: CONVERSATION_ID, title: 'Half marathon', agent_id: null },
        { id: OTHER_CONVERSATION_ID, title: 'Recovery week', agent_id: null },
      ],
      total: 2,
    });
    const turn = heldTurn();
    stopTurn.mockResolvedValue(true);
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const tree = (selected: string) => (
      <QueryClientProvider client={queryClient}>
        <ToastProvider>
          <ChatTab selectedConversation={selected} onSelectConversation={vi.fn()} />
        </ToastProvider>
      </QueryClientProvider>
    );
    const view = render(tree(CONVERSATION_ID));
    const user = await send(QUESTION);
    await screen.findByRole('button', { name: i18n.t('chat.stopTurnAria') });

    view.rerender(tree(OTHER_CONVERSATION_ID));
    await waitFor(() =>
      expect(getConversationMessages).toHaveBeenCalledWith(OTHER_CONVERSATION_ID),
    );

    await user.click(screen.getByRole('button', { name: i18n.t('chat.stopTurnAria') }));
    await waitFor(() => expect(stopTurn).toHaveBeenCalledTimes(1));
    expect(stopTurn).toHaveBeenCalledWith(CONVERSATION_ID);

    await act(async () => {
      turn.finish(turnEnding(STOPPED_NOTICE, 'stopped', 'stopped'));
    });
  });

  it('hands the stop button back when the reply had already landed, and delivers that reply', async () => {
    const turn = heldTurn();
    stopTurn.mockResolvedValue(false);
    renderChat();
    const user = await send(QUESTION);

    await user.click(await screen.findByRole('button', { name: i18n.t('chat.stopTurnAria') }));
    await waitFor(() => expect(stopTurn).toHaveBeenCalledWith(CONVERSATION_ID));
    await waitFor(() => expect(screen.getByTestId('stop-turn-button')).toBeEnabled());

    await act(async () => {
      turn.finish(turnEnding(REPLY, 'stop', 'mock-model'));
    });
    expect(await screen.findByText(REPLY)).toBeInTheDocument();
  });

  it('hands the stop button back and says so when the stop could not be sent', async () => {
    const turn = heldTurn();
    stopTurn.mockRejectedValue(new Error('Network Error'));
    renderChat();
    const user = await send(QUESTION);

    await user.click(await screen.findByRole('button', { name: i18n.t('chat.stopTurnAria') }));
    await waitFor(() => expect(screen.getByTestId('stop-turn-button')).toBeEnabled());
    expect(await screen.findByText(i18n.t('errors.network'))).toBeInTheDocument();

    await act(async () => {
      turn.finish(turnEnding(REPLY, 'stop', 'mock-model'));
    });
    expect(await screen.findByText(REPLY)).toBeInTheDocument();
  });
});
