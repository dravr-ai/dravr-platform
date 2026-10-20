// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the personal thread's topics at the head of Home's history — titled, and opening where each starts
// ABOUTME: A thread with one topic lists nothing; an untitled topic reads as a new chat (carnet#840)

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';

const getPersonalConversation = vi.fn();
const getConversationTopics = vi.fn();
const getConversations = vi.fn();

vi.mock('../../../services/api', () => ({
  chatApi: {
    getPersonalConversation: (...args: unknown[]) => getPersonalConversation(...args),
    getConversations: (...args: unknown[]) => getConversations(...args),
  },
  topicsApi: {
    getConversationTopics: (...args: unknown[]) => getConversationTopics(...args),
  },
}));

import PersonalTopics from '../PersonalTopics';

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe('PersonalTopics', () => {
  beforeEach(() => {
    getPersonalConversation.mockResolvedValue({ id: 'thread-1' });
    getConversations.mockResolvedValue({ conversations: [], total: 0 });
  });

  it('lists the topics, newest first, and opens the one picked', async () => {
    getConversationTopics.mockResolvedValue([
      { divider_id: 'd2', started_at: '2026-10-20T14:00:00Z', title: 'Plan my long run' },
      { divider_id: 'd1', started_at: '2026-10-18T14:00:00Z' },
      { started_at: '2026-10-10T14:00:00Z', title: 'How was my week?' },
    ]);
    const onOpenTopic = vi.fn();
    render(<PersonalTopics onOpenTopic={onOpenTopic} />, { wrapper });

    const topics = await screen.findAllByTestId('personal-topic');
    expect(topics.map(t => t.textContent)).toEqual([
      expect.stringContaining('Plan my long run'),
      expect.stringContaining('New chat'),
      expect.stringContaining('How was my week?'),
    ]);
    await userEvent.click(topics[0]);
    expect(onOpenTopic).toHaveBeenCalledWith('thread-1', 'd2');
    await userEvent.click(topics[2]);
    expect(onOpenTopic).toHaveBeenCalledWith('thread-1', null);
  });

  it('lists nothing for a thread with one topic', async () => {
    getConversationTopics.mockResolvedValue([{ started_at: '2026-10-10T14:00:00Z', title: 'Only' }]);
    render(<PersonalTopics onOpenTopic={vi.fn()} />, { wrapper });
    await vi.waitFor(() => expect(getConversationTopics).toHaveBeenCalled());
    expect(screen.queryByTestId('personal-topics')).not.toBeInTheDocument();
  });
});
