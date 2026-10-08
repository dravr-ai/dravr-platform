// ABOUTME: The chat surface embedded in another view — the host's way in until a thread exists, then that thread alone
// ABOUTME: Pins that a `send` the host hands over opens a conversation, sends the text into it, and draws no list or header
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ChatTab, { type PendingComposerAction } from '../ChatTab';
import { ToastProvider } from '../ui';

const QUESTION = 'What recovery do you advise after my activity “Morning Trail Run” from Tuesday 29 September?';

const createConversation = vi.fn();
const sendTurn = vi.fn();
const getConversations = vi.fn();
const getConversationMessages = vi.fn();

vi.mock('../../services/api', () => ({
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

/** A host that owns the thread and hands the surface a `send` from its own button, as the activity view does. */
function Host() {
  const [conversation, setConversation] = useState<string | null>(null);
  const [action, setAction] = useState<PendingComposerAction | null>(null);
  return (
    <div>
      <p data-testid="host-conversation">{conversation ?? 'none'}</p>
      <ChatTab
        layout="embedded"
        selectedConversation={conversation}
        onSelectConversation={setConversation}
        pendingComposerAction={action}
        onPendingComposerActionConsumed={() => setAction(null)}
        embeddedEmptyState={
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

describe('ChatTab embedded', () => {
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
    expect(screen.getByPlaceholderText('Message Dravr...')).toBeInTheDocument();
    expect(screen.queryByTestId('thread-header')).toBeNull();
  });
});
