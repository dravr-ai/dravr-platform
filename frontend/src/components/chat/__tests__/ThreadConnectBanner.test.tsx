// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The connect-a-provider nudge over an open thread, driven through the thread's own hook
// ABOUTME: Pins that it asks the caller to connect over their own thread, never over a coach's plan thread for an athlete (carnet#1191)

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ThreadConnectBanner from '../ThreadConnectBanner';
import { useChatThread } from '../useChatThread';
import { ToastProvider } from '../../ui';

const OWN_THREAD = 'conv-own';
const PLAN_THREAD = 'conv-plan';

const getConversations = vi.fn();
const getProvidersStatus = vi.fn();

vi.mock('../../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: vi.fn().mockResolvedValue({ messages: [] }),
    getConversationVerdicts: vi.fn().mockResolvedValue({ verdicts: [] }),
    sendTurn: vi.fn(),
    stopTurn: vi.fn(),
    markConversationRead: vi.fn().mockResolvedValue(undefined),
  },
  coachesApi: { list: vi.fn().mockResolvedValue({ agents: [] }) },
  providersApi: { getProvidersStatus: (...a: unknown[]) => getProvidersStatus(...a) },
  groupsApi: { getGroup: vi.fn(), readRoom: vi.fn() },
}));

vi.mock('../../../services/analytics', () => ({ track: vi.fn() }));
vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { id: 'coach-1', role: 'user' } }),
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

function Host({ conversationId }: { conversationId: string }) {
  const thread = useChatThread({
    conversationId,
    onSelectConversation: vi.fn(),
    openThread: vi.fn(),
    pendingComposerAction: null,
    onPendingComposerActionConsumed: vi.fn(),
  });
  return (
    <>
      <p data-testid="thread-title">{thread.title}</p>
      <ThreadConnectBanner thread={thread} onNavigate={vi.fn()} />
    </>
  );
}

function renderThread(conversationId: string) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <Host conversationId={conversationId} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe('ThreadConnectBanner', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // The coach has connected nothing of their own.
    getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: false }] });
    getConversations.mockResolvedValue({
      conversations: [
        { id: OWN_THREAD, title: 'My training', agent_id: null },
        {
          id: PLAN_THREAD,
          title: 'Plan · Alex',
          agent_id: null,
          subject_user_id: 'athlete-alex',
          subject_display_name: 'Alex',
        },
      ],
      total: 2,
    });
  });

  it('asks a caller with nothing connected to connect, over their own thread', async () => {
    renderThread(OWN_THREAD);

    expect(await screen.findByTestId('connect-provider-banner')).toBeInTheDocument();
  });

  it("never asks the coach to connect over an athlete's plan thread", async () => {
    renderThread(PLAN_THREAD);

    // The thread's row and the provider status have both answered.
    expect(await screen.findByText('Plan · Alex')).toBeInTheDocument();
    await waitFor(() => expect(getProvidersStatus).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByTestId('connect-provider-banner')).toBeNull();
  });
});
