// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The thread every web chat surface shares, driven through its own hook with no host chrome around it
// ABOUTME: Pins a typed send, a stop reaching the running turn, a send with no thread opening one the host's way, the composer's focus rule, and that only an operator reads which model wrote a reply

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import type { Message, UserRole } from '@pierre/shared-types';
import ChatThread from '../ChatThread';
import { useChatThread, type OpenThread } from '../useChatThread';
import type { PendingComposerAction } from '../types';
import { ToastProvider } from '../../ui';

const CONVERSATION_ID = 'conv-1';

const getConversations = vi.fn();
const getConversationMessages = vi.fn();
const sendTurn = vi.fn();
const stopTurn = vi.fn();

vi.mock('../../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: (...a: unknown[]) => getConversationMessages(...a),
    getConversationVerdicts: vi.fn().mockResolvedValue({ verdicts: [] }),
    sendTurn: (...a: unknown[]) => sendTurn(...a),
    stopTurn: (...a: unknown[]) => stopTurn(...a),
    markConversationRead: vi.fn().mockResolvedValue(undefined),
  },
  coachesApi: { list: vi.fn().mockResolvedValue({ agents: [] }) },
  providersApi: {
    getProvidersStatus: vi.fn().mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] }),
  },
  groupsApi: { getGroup: vi.fn(), readRoom: vi.fn() },
}));

vi.mock('../../../services/analytics', () => ({ track: vi.fn() }));
// The viewer's role decides whether a reply names its model; each test sets it.
const viewer = vi.hoisted(() => ({ role: 'user' as UserRole }));
vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { id: 'viewer-1', role: viewer.role } }),
}));
vi.mock('../../../hooks/useUsageStatus', () => ({
  useUsageStatus: () => ({
    level: 'none',
    sendDisabled: false,
    text: null,
    invalidate: vi.fn(),
    applyNotice: vi.fn(),
  }),
}));

const EARLIER: Message[] = [
  { id: 'm1', role: 'user', content: 'Hi', created_at: '2026-09-30T08:00:00Z' },
  { id: 'm2', role: 'assistant', content: 'Hi, ready when you are.', created_at: '2026-09-30T08:00:04Z' },
];

type TurnOptions = { onDone?: (turn: unknown) => void };

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

function turnEnding(content: string) {
  return {
    assistant: {
      message: { id: 'm4', role: 'assistant', content, created_at: '2026-09-30T08:01:00Z' },
      blocks: [{ type: 'prose', text: content }],
      finish_reason: 'stop',
    },
    telemetry: { model: 'test-model', execution_time_ms: 10 },
  };
}

/** The smallest host: it owns the thread and draws nothing but it. */
function Host(props: {
  initial: string | null;
  openThread: OpenThread;
  focusComposerOnOpen?: boolean;
  action?: PendingComposerAction;
}) {
  const [conversationId, setConversationId] = useState<string | null>(props.initial);
  const [action, setAction] = useState<PendingComposerAction | null>(props.action ?? null);
  const thread = useChatThread({
    conversationId,
    onSelectConversation: setConversationId,
    openThread: props.openThread,
    pendingComposerAction: action,
    onPendingComposerActionConsumed: () => setAction(null),
  });
  return (
    <div className="flex h-full flex-col">
      <p data-testid="host-conversation">{conversationId ?? 'none'}</p>
      <ChatThread
        thread={thread}
        conversationTitle={thread.title}
        followOnOpen
        focusComposerOnOpen={props.focusComposerOnOpen ?? false}
      />
    </div>
  );
}

function renderThread(props: Parameters<typeof Host>[0]) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <Host {...props} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

function composer(): HTMLTextAreaElement {
  const field = document.querySelector<HTMLTextAreaElement>('textarea[data-composer="true"]');
  if (!field) throw new Error('no composer on screen');
  return field;
}

describe('ChatThread', () => {
  const openThread = vi.fn<OpenThread>();

  beforeEach(() => {
    vi.clearAllMocks();
    viewer.role = 'user';
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Half marathon', agent_id: null }],
      total: 1,
    });
    getConversationMessages.mockResolvedValue({ messages: EARLIER });
    openThread.mockResolvedValue({ id: 'conv-new' });
  });

  it('sends the typed line into the open thread, clears the composer and draws the reply', async () => {
    const turn = heldTurn();
    const user = userEvent.setup();
    renderThread({ initial: CONVERSATION_ID, openThread });

    await screen.findByText('Hi, ready when you are.');
    await user.type(composer(), 'How does my week look?');
    await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));

    expect(sendTurn).toHaveBeenCalledTimes(1);
    expect(sendTurn.mock.calls[0][0]).toBe(CONVERSATION_ID);
    expect(sendTurn.mock.calls[0][1]).toBe('How does my week look?');
    expect(composer()).toHaveValue('');
    expect(await screen.findByText('How does my week look?')).toBeInTheDocument();

    await act(async () => {
      turn.finish(turnEnding('Your week: 42 km, all easy.'));
    });
    expect(await screen.findByText('Your week: 42 km, all easy.')).toBeInTheDocument();
    expect(openThread).not.toHaveBeenCalled();
  });

  it('never shows an athlete which model wrote a reply, live or reloaded', async () => {
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Half marathon', agent_id: null, model: 'stored-model' }],
      total: 1,
    });
    const turn = heldTurn();
    const user = userEvent.setup();
    renderThread({ initial: CONVERSATION_ID, openThread });

    await screen.findByText('Hi, ready when you are.');
    await user.type(composer(), 'How does my week look?');
    await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));
    await act(async () => {
      turn.finish(turnEnding('Your week: 42 km, all easy.'));
    });
    await screen.findByText('Your week: 42 km, all easy.');

    expect(screen.queryByText(/test-model/), 'the live reply names no model').not.toBeInTheDocument();
    expect(screen.queryByText(/stored-model/), 'the reloaded reply names no model').not.toBeInTheDocument();
  });

  it('shows an operator the model under each reply, live and reloaded', async () => {
    viewer.role = 'admin';
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Half marathon', agent_id: null, model: 'stored-model' }],
      total: 1,
    });
    const turn = heldTurn();
    const user = userEvent.setup();
    renderThread({ initial: CONVERSATION_ID, openThread });

    expect(await screen.findByText(/stored-model/), 'the reloaded reply names its model').toBeInTheDocument();
    await user.type(composer(), 'How does my week look?');
    await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));
    await act(async () => {
      turn.finish(turnEnding('Your week: 42 km, all easy.'));
    });
    await screen.findByText('Your week: 42 km, all easy.');

    expect(screen.getByText(/test-model/), 'the live reply names its model').toBeInTheDocument();
  });

  it('asks the server to stop the running turn from the composer’s stop button', async () => {
    const turn = heldTurn();
    stopTurn.mockResolvedValue(true);
    const user = userEvent.setup();
    renderThread({ initial: CONVERSATION_ID, openThread });

    await screen.findByText('Hi, ready when you are.');
    await user.type(composer(), 'How does my week look?');
    await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));

    await user.click(await screen.findByRole('button', { name: i18n.t('chat.stopTurnAria') }));
    expect(stopTurn).toHaveBeenCalledWith(CONVERSATION_ID);
    await waitFor(() => expect(screen.getByTestId('stop-turn-button')).toBeDisabled());

    await act(async () => {
      turn.finish(turnEnding('Reply stopped.'));
    });
    expect(await screen.findByRole('button', { name: i18n.t('chat.sendMessageAria') })).toBeInTheDocument();
  });

  it('opens a thread the host’s way for a send with none open, then sends into it', async () => {
    sendTurn.mockResolvedValue(undefined);
    renderThread({
      initial: null,
      openThread,
      action: { kind: 'send', text: 'uc:1:about_me_more', display: 'Finish my profile' },
    });

    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    // The athlete speaks first, so the thread's agent posts no welcome over the answer.
    expect(openThread).toHaveBeenCalledWith({ athlete_speaks_first: true });
    expect(screen.getByTestId('host-conversation')).toHaveTextContent('conv-new');
    expect(sendTurn.mock.calls[0][0]).toBe('conv-new');
    expect(sendTurn.mock.calls[0][1]).toBe('uc:1:about_me_more');
    expect(await screen.findByText('Finish my profile')).toBeInTheDocument();
  });

  it('leaves focus where it was unless the host asks the composer to take it', async () => {
    const first = renderThread({ initial: CONVERSATION_ID, openThread, focusComposerOnOpen: false });
    await screen.findByText('Hi, ready when you are.');
    expect(composer()).not.toHaveFocus();
    first.unmount();

    renderThread({ initial: CONVERSATION_ID, openThread, focusComposerOnOpen: true });
    await screen.findByText('Hi, ready when you are.');
    expect(composer()).toHaveFocus();
  });
});
