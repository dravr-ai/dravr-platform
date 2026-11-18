// ABOUTME: The chat surface embedded in another view — the host's way in until a thread exists, then that thread alone
// ABOUTME: Pins that a `send` the host hands over opens a conversation and draws no list or header; drafts and chips keep their origin
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import EmbeddedChat from '../EmbeddedChat';
import type { PendingComposerAction } from '../types';
import { ToastProvider } from '../../ui';

const QUESTION = 'What recovery do you advise after my activity “Morning Trail Run” from Tuesday 29 September?';

const createConversation = vi.fn();
const sendTurn = vi.fn();
const getConversations = vi.fn();
const getConversationMessages = vi.fn();

vi.mock('../../../services/api', () => ({
  // The header's notifications bell reads the unread count.
  notificationsApi: { getUnreadCount: async () => ({ unread_count: 0 }) },
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: (...a: unknown[]) => getConversationMessages(...a),
    getConversationVerdicts: vi.fn().mockResolvedValue({ verdicts: [] }),
    listParticipants: vi.fn().mockResolvedValue([]),
    sendTurn: (...a: unknown[]) => sendTurn(...a),
    markConversationRead: vi.fn().mockResolvedValue(undefined),
    createConversation: (...a: unknown[]) => createConversation(...a),
    updateConversation: vi.fn(),
    deleteConversation: vi.fn(),
    markConversationUnread: vi.fn(),
  },
  coachesApi: { list: vi.fn().mockResolvedValue({ agents: [] }) },
  providersApi: {
    getProvidersStatus: vi.fn().mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] }),
  },
  groupsApi: { getGroup: vi.fn(), readRoom: vi.fn() },
}));

vi.mock('../../../services/analytics', () => ({ track: vi.fn() }));
vi.mock('../../../hooks/useAuth', () => ({ useAuth: () => ({ token: 'test-token' }) }));
vi.mock('../../../hooks/useUsageStatus', () => ({
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

/** A host that owns the thread and hands the surface a `send` from its own button, as the activity view does. */
function Host() {
  const [conversation, setConversation] = useState<string | null>(null);
  const [action, setAction] = useState<PendingComposerAction | null>(null);
  return (
    <div>
      <p data-testid="host-conversation">{conversation ?? 'none'}</p>
      <EmbeddedChat
        conversationId={conversation}
        onSelectConversation={setConversation}
        pendingComposerAction={action}
        onPendingComposerActionConsumed={() => setAction(null)}
        emptyState={
          <button type="button" onClick={() => setAction({ kind: 'send', text: QUESTION })}>
            host question
          </button>
        }
      />
    </div>
  );
}

function renderHost() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <Host />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe('EmbeddedChat', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getConversations.mockResolvedValue({ conversations: [], total: 0 });
    getConversationMessages.mockResolvedValue({ messages: [] });
    createConversation.mockResolvedValue({ id: 'conv-activity' });
    sendTurn.mockResolvedValue(undefined);
  });

  it('shows the host way in, then opens a thread and sends the question into it', async () => {
    renderHost();

    // No thread yet: the host's own way in, and none of the chat tab's chrome.
    const ask = await screen.findByRole('button', { name: 'host question' });
    expect(screen.getByTestId('embedded-chat')).toBeInTheDocument();
    expect(screen.queryByTestId('thread-header')).toBeNull();
    expect(screen.queryByTestId('chat-empty-provider-status')).toBeNull();

    await userEvent.click(ask);

    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    expect(createConversation).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('host-conversation')).toHaveTextContent('conv-activity');
    expect(sendTurn.mock.calls[0][0]).toBe('conv-activity');
    expect(sendTurn.mock.calls[0][1]).toBe(QUESTION);
    // The question stays in the transcript: the fresh thread's first read
    // answering empty after it was sent does not wipe it.
    expect(await screen.findByText(QUESTION)).toBeInTheDocument();
    // The thread's composer takes over from the host's way in.
    expect(screen.queryByRole('button', { name: 'host question' })).toBeNull();
    // The view owns the way in: the thread's composer does not take focus on open.
    expect(screen.getByPlaceholderText('Message Dravr...')).toBeInTheDocument();
    expect(screen.getByPlaceholderText('Message Dravr...')).not.toHaveFocus();
    expect(screen.queryByTestId('thread-header')).toBeNull();
  });
});

/** A host whose thread is open, handing the surface one composer action, as the activity view's chips do. */
function OpenThreadHost({ initial }: { initial: PendingComposerAction }) {
  const [conversation, setConversation] = useState<string | null>('conv-activity');
  const [action, setAction] = useState<PendingComposerAction | null>(initial);
  return (
    <EmbeddedChat
      conversationId={conversation}
      onSelectConversation={setConversation}
      pendingComposerAction={action}
      onPendingComposerActionConsumed={() => setAction(null)}
      emptyState={null}
    />
  );
}

function renderOpenThread(initial: PendingComposerAction) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <OpenThreadHost initial={initial} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

// carnet#828: analytics tells a typed question from a pre-filled one.
describe('EmbeddedChat message origin', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getConversations.mockResolvedValue({
      conversations: [{ id: 'conv-activity', title: 'Morning Trail Run', agent_id: null }],
      total: 1,
    });
    getConversationMessages.mockResolvedValue({ messages: [] });
    sendTurn.mockResolvedValue(undefined);
  });

  it('reports a draft the athlete sends as a draft', async () => {
    renderOpenThread({ kind: 'draft', text: 'Suggest a route for today' });
    const user = userEvent.setup();
    const input = await screen.findByPlaceholderText('Message Dravr...');
    await waitFor(() => expect(input).toHaveValue('Suggest a route for today'));
    await user.type(input, ' near the river');
    await user.click(screen.getByRole('button', { name: 'Send message' }));

    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    expect(sendTurn.mock.calls[0][1]).toBe('Suggest a route for today near the river');
    expect(sendTurn.mock.calls[0][2].origin).toBe('draft');
  });

  it('forgets a draft the athlete erased and retyped', async () => {
    renderOpenThread({ kind: 'draft', text: 'Draft' });
    const user = userEvent.setup();
    const input = await screen.findByPlaceholderText('Message Dravr...');
    await waitFor(() => expect(input).toHaveValue('Draft'));
    await user.clear(input);
    await user.type(input, 'My own words');
    await user.click(screen.getByRole('button', { name: 'Send message' }));

    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    expect(sendTurn.mock.calls[0][2].origin).toBeUndefined();
  });

  it('passes a chip send on as a chip', async () => {
    renderOpenThread({ kind: 'send', text: 'How was my pacing?', origin: 'chip' });

    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    expect(sendTurn.mock.calls[0][1]).toBe('How was my pacing?');
    expect(sendTurn.mock.calls[0][2].origin).toBe('chip');
  });
});
