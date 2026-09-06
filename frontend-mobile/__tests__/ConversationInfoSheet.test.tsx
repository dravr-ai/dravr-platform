// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the chat header's info sheet — the three shapes a thread can have and the rows each shows
// ABOUTME: A group thread gets Group info, an agent thread gets Agent info, a plain thread gets rename/participants/delete

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Conversation } from '@pierre/shared-types';

const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn(), navigate: jest.fn() }),
  useLocalSearchParams: () => ({}),
}));

jest.mock('../src/services/api', () => ({
  coachesApi: { list: jest.fn().mockResolvedValue({ coaches: [] }) },
  groupsApi: {
    getGroup: jest.fn().mockResolvedValue(null),
    listMembers: jest.fn().mockResolvedValue({ members: [] }),
    getStats: jest.fn().mockResolvedValue({ stats: null }),
    listInvites: jest.fn().mockResolvedValue({ invites: [] }),
    getPermissions: jest.fn().mockResolvedValue({ can_create: true, policy: 'everyone', weekly_digest: false }),
    getTranscript: jest.fn().mockResolvedValue({ group_id: 'group-1', entries: [] }),
    getWeeklyReport: jest.fn(),
    getHealthFlags: jest.fn(),
  },
}));

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'user-1' }, isAuthenticated: true }),
}));

import { ConversationInfoSheet } from '../src/screens/chat/ConversationInfoSheet';

function conversation(overrides: Partial<Conversation> & { id: string }): Conversation {
  return {
    title: 'Tempo Tuesday',
    coach_id: null,
    message_count: 3,
    unread_count: 0,
    created_at: '2026-08-20T10:00:00Z',
    updated_at: '2026-08-26T09:50:00Z',
    ...overrides,
  } as Conversation;
}

function renderSheet(conv: Conversation | null) {
  const handlers = {
    onClose: jest.fn(),
    onSendCommand: jest.fn(),
    onRename: jest.fn(),
    onParticipants: jest.fn(),
    onDelete: jest.fn(),
    onLeaveThread: jest.fn(),
  };
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const view = render(
    <QueryClientProvider client={client}>
      <ConversationInfoSheet visible conversation={conv} {...handlers} />
    </QueryClientProvider>,
  );
  return { ...view, handlers };
}

describe('ConversationInfoSheet', () => {
  beforeEach(() => jest.clearAllMocks());

  it('shows rename, participants and delete for a plain thread', () => {
    const { getByTestId, queryByTestId, handlers } = renderSheet(conversation({ id: 'c1' }));

    expect(getByTestId('conversation-info-title')).toHaveTextContent('Tempo Tuesday');
    fireEvent.press(getByTestId('conversation-info-rename'));
    fireEvent.press(getByTestId('conversation-info-participants'));
    fireEvent.press(getByTestId('conversation-info-delete'));
    expect(handlers.onRename).toHaveBeenCalledTimes(1);
    expect(handlers.onParticipants).toHaveBeenCalledTimes(1);
    expect(handlers.onDelete).toHaveBeenCalledTimes(1);
    // A thread with no agent and no group has neither of the other shapes.
    expect(queryByTestId('coach-info-sheet')).toBeNull();
    expect(queryByTestId('group-info-sheet')).toBeNull();
  });

  it('shows Agent info for an agent-bound thread', () => {
    const { getByTestId, queryByTestId } = renderSheet(
      conversation({ id: 'c2', coach_id: 'coach-1', coach_title: 'Coach Tempo' }),
    );

    expect(getByTestId('coach-info-sheet')).toBeTruthy();
    expect(getByTestId('coach-info-title')).toHaveTextContent('Coach Tempo');
    expect(queryByTestId('conversation-info-plain')).toBeNull();
  });

  // A group thread bound to an agent is still a group row and still gets Group
  // info: the group is what the thread is about.
  it('shows Group info for a group thread even when an agent is attached', async () => {
    const { findByTestId, queryByTestId } = renderSheet(
      conversation({ id: 'c3', group_id: 'group-1', group_name: 'Harricana', coach_id: 'coach-1' }),
    );

    expect(await findByTestId('group-info-name')).toHaveTextContent('Harricana');
    await waitFor(() => expect(queryByTestId('coach-info-sheet')).toBeNull());
    expect(queryByTestId('conversation-info-plain')).toBeNull();
  });

  it('renders nothing without a thread', () => {
    const { queryByTestId } = renderSheet(null);
    expect(queryByTestId('conversation-info-sheet')).toBeNull();
  });
});
