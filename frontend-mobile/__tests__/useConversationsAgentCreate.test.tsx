// ABOUTME: A thread created with an agent is read on open — the agent's welcome is already in it
// ABOUTME: An agentless create keeps skipping the first read of a thread known to be empty
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { renderHook, act } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockCreateConversation = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    createConversation: (...args: unknown[]) => mockCreateConversation(...args),
  },
}));

import { useConversations } from '../src/screens/chat/useConversations';

function render() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return renderHook(() => useConversations(), {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}

describe('useConversations create', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  // carnet#735: the server posts the agent's welcome while creating the
  // thread, so skipping the first read would open it blank.
  it('reads an agent-bound thread on open', async () => {
    mockCreateConversation.mockResolvedValue({ id: 'conv-agent', title: 'Tempo Agent', agent_id: 'a1' });
    const { result } = render();

    await act(async () => {
      await result.current.createConversation({ agent_id: 'a1' });
    });

    expect(result.current.justCreatedConversationRef.current).toBeNull();
  });

  it('skips the first read of an agentless thread, which is empty', async () => {
    mockCreateConversation.mockResolvedValue({ id: 'conv-plain', title: 'New thread' });
    const { result } = render();

    await act(async () => {
      await result.current.createConversation({});
    });

    expect(result.current.justCreatedConversationRef.current).toBe('conv-plain');
  });
});
