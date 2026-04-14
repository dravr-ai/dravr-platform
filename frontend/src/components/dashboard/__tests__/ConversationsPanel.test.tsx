// ABOUTME: Sprint C15 tests — ConversationsPanel groups conversations by coach
// ABOUTME: Mocks chatApi.getConversations + coachesApi.list, asserts session hierarchy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ConversationsPanel from '../ConversationsPanel';
import type { Conversation, Coach } from '../../chat/types';

vi.mock('../../../services/api', async () => ({
  chatApi: {
    getConversations: vi.fn(),
    updateConversation: vi.fn(),
    deleteConversation: vi.fn(),
  },
  coachesApi: {
    list: vi.fn(),
  },
}));

const { chatApi, coachesApi } = await import('../../../services/api');

function sampleConversation(overrides: Partial<Conversation> = {}): Conversation {
  return {
    id: 'conv-1',
    title: 'Marathon plan',
    coach_id: 'coach-running',
    model: 'gemini-pro',
    total_tokens: 100,
    message_count: 3,
    created_at: '2026-04-13T18:00:00Z',
    updated_at: '2026-04-13T18:00:00Z',
    last_message_at: '2026-04-13T18:00:00Z',
    ...overrides,
  };
}

function sampleCoach(overrides: Partial<Coach> = {}): Coach {
  return {
    id: 'coach-running',
    user_id: 'user-1',
    tenant_id: 'tenant-a',
    title: 'Running Coach',
    system_prompt: 'You are a running coach.',
    category: 'training',
    created_at: '2026-04-01T00:00:00Z',
    updated_at: '2026-04-01T00:00:00Z',
    ...overrides,
  } as Coach;
}

function renderPanel() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ConversationsPanel selectedConversation={null} onSelectConversation={() => {}} />
    </QueryClientProvider>,
  );
}

describe('ConversationsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it('renders the empty state when there are no conversations', async () => {
    vi.mocked(chatApi.getConversations).mockResolvedValueOnce({ conversations: [] });
    vi.mocked(coachesApi.list).mockResolvedValueOnce({ coaches: [] });
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText(/No conversations yet/i)).toBeInTheDocument();
    });
  });

  it('groups conversations under their coach title', async () => {
    vi.mocked(chatApi.getConversations).mockResolvedValueOnce({
      conversations: [
        sampleConversation({ id: 'c1', coach_id: 'coach-running', title: 'Marathon plan' }),
        sampleConversation({
          id: 'c2',
          coach_id: 'coach-running',
          title: 'Track workout',
        }),
        sampleConversation({ id: 'c3', coach_id: 'coach-strength', title: 'Deadlift form' }),
      ],
    });
    vi.mocked(coachesApi.list).mockResolvedValueOnce({
      coaches: [
        sampleCoach({ id: 'coach-running', title: 'Running Coach' }),
        sampleCoach({ id: 'coach-strength', title: 'Strength Coach' }),
      ],
    });
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText('Running Coach')).toBeInTheDocument();
    });
    expect(screen.getByText('Strength Coach')).toBeInTheDocument();
    expect(screen.getByText('Marathon plan')).toBeInTheDocument();
    expect(screen.getByText('Track workout')).toBeInTheDocument();
    expect(screen.getByText('Deadlift form')).toBeInTheDocument();
  });

  it('collapses and restores a group when the header is clicked', async () => {
    vi.mocked(chatApi.getConversations).mockResolvedValueOnce({
      conversations: [
        sampleConversation({ id: 'c1', coach_id: 'coach-running', title: 'Marathon plan' }),
      ],
    });
    vi.mocked(coachesApi.list).mockResolvedValueOnce({
      coaches: [sampleCoach({ id: 'coach-running', title: 'Running Coach' })],
    });
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText('Marathon plan')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /Toggle Running Coach session/i }));
    await waitFor(() => {
      expect(screen.queryByText('Marathon plan')).not.toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /Toggle Running Coach session/i }));
    await waitFor(() => {
      expect(screen.getByText('Marathon plan')).toBeInTheDocument();
    });
  });

  it('renders a "Without a coach" group for unattached conversations', async () => {
    vi.mocked(chatApi.getConversations).mockResolvedValueOnce({
      conversations: [
        sampleConversation({ id: 'c1', coach_id: null, title: 'General chat' }),
      ],
    });
    vi.mocked(coachesApi.list).mockResolvedValueOnce({ coaches: [] });
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText(/Without a coach/i)).toBeInTheDocument();
    });
    expect(screen.getByText('General chat')).toBeInTheDocument();
  });
});
