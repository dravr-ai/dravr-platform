// ABOUTME: Tests for mobile ConversationsScreen coach-session grouping (Sprint C19 parity with web ConversationsPanel)
// ABOUTME: Covers empty state, grouping by coach, collapse/expand, and "Without a coach" bucket
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';

const mockGetConversations = jest.fn();
const mockListCoaches = jest.fn();
const mockAsyncGetItem = jest.fn();
const mockAsyncSetItem = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
    updateConversation: jest.fn(),
    deleteConversation: jest.fn(),
  },
  coachesApi: {
    list: (...args: unknown[]) => mockListCoaches(...args),
  },
}));

jest.mock('@react-native-async-storage/async-storage', () => ({
  __esModule: true,
  default: {
    getItem: (...args: unknown[]) => mockAsyncGetItem(...args),
    setItem: (...args: unknown[]) => mockAsyncSetItem(...args),
  },
}));

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true }),
}));

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn() }),
  useFocusEffect: (cb: () => void) => {
    // Run the effect body once synchronously so loadConversations fires.
    const React = require('react');
    React.useEffect(() => {
      const cleanup = cb();
      return cleanup;
    }, [cb]);
  },
}));

import { ConversationsScreen } from '../src/screens/conversations/ConversationsScreen';

type Conv = {
  id: string;
  title: string | null;
  user_id?: string;
  tenant_id?: string;
  coach_id?: string | null;
  created_at: string;
  updated_at: string;
};

type Coach = {
  id: string;
  title: string;
  description: string | null;
  system_prompt: string;
  category: string;
  tags: string[];
  token_count: number;
  is_favorite: boolean;
  use_count: number;
  last_used_at: string | null;
  created_at: string;
  updated_at: string;
  is_system: boolean;
};

function makeConv(overrides: Partial<Conv> = {}): Conv {
  return {
    id: 'c1',
    title: 'Hello',
    coach_id: null,
    created_at: '2026-04-10T10:00:00Z',
    updated_at: '2026-04-13T10:00:00Z',
    ...overrides,
  };
}

function makeCoach(overrides: Partial<Coach> = {}): Coach {
  return {
    id: 'coach-1',
    title: 'Marathon Coach',
    description: null,
    system_prompt: '',
    category: 'running',
    tags: [],
    token_count: 0,
    is_favorite: false,
    use_count: 0,
    last_used_at: null,
    created_at: '2026-04-01T00:00:00Z',
    updated_at: '2026-04-01T00:00:00Z',
    is_system: false,
    ...overrides,
  };
}

describe('ConversationsScreen — coach-session grouping', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockAsyncGetItem.mockResolvedValue(null);
    mockAsyncSetItem.mockResolvedValue(undefined);
  });

  it('renders empty state when there are no conversations', async () => {
    mockGetConversations.mockResolvedValueOnce({ conversations: [] });
    mockListCoaches.mockResolvedValueOnce({ coaches: [] });

    const { findByText } = render(<ConversationsScreen />);

    expect(await findByText('No Conversations')).toBeTruthy();
  });

  it('groups conversations by coach title', async () => {
    mockGetConversations.mockResolvedValueOnce({
      conversations: [
        makeConv({ id: 'c1', title: 'Training plan', coach_id: 'coach-1' }),
        makeConv({ id: 'c2', title: 'Race strategy', coach_id: 'coach-1' }),
        makeConv({ id: 'c3', title: 'Nutrition question', coach_id: 'coach-2' }),
      ],
    });
    mockListCoaches.mockResolvedValueOnce({
      coaches: [
        makeCoach({ id: 'coach-1', title: 'Marathon Coach' }),
        makeCoach({ id: 'coach-2', title: 'Nutrition Coach' }),
      ],
    });

    const { findByText, findByTestId } = render(<ConversationsScreen />);

    expect(await findByText('Marathon Coach')).toBeTruthy();
    expect(await findByText('Nutrition Coach')).toBeTruthy();
    // Group headers must be present as toggle buttons.
    expect(await findByTestId('session-group-header-coach-1')).toBeTruthy();
    expect(await findByTestId('session-group-header-coach-2')).toBeTruthy();
    // Children render under their headers.
    expect(await findByText('Training plan')).toBeTruthy();
    expect(await findByText('Race strategy')).toBeTruthy();
    expect(await findByText('Nutrition question')).toBeTruthy();
  });

  it('collapses and expands a group on header press', async () => {
    mockGetConversations.mockResolvedValueOnce({
      conversations: [
        makeConv({ id: 'c1', title: 'Training plan', coach_id: 'coach-1' }),
      ],
    });
    mockListCoaches.mockResolvedValueOnce({
      coaches: [makeCoach({ id: 'coach-1', title: 'Marathon Coach' })],
    });

    const { findByTestId, queryByText, getByText } = render(<ConversationsScreen />);

    const header = await findByTestId('session-group-header-coach-1');
    expect(getByText('Training plan')).toBeTruthy();

    await act(async () => {
      fireEvent.press(header);
    });

    await waitFor(() => {
      expect(queryByText('Training plan')).toBeNull();
    });
    // Collapsed set persisted to AsyncStorage
    expect(mockAsyncSetItem).toHaveBeenCalledWith(
      'dravr.conversations-panel.collapsed',
      JSON.stringify(['coach-1']),
    );

    await act(async () => {
      fireEvent.press(header);
    });

    await waitFor(() => {
      expect(queryByText('Training plan')).toBeTruthy();
    });
  });

  it('shows a "Without a coach" bucket for conversations with null coach_id', async () => {
    mockGetConversations.mockResolvedValueOnce({
      conversations: [
        makeConv({ id: 'c1', title: 'Orphan chat', coach_id: null }),
        makeConv({ id: 'c2', title: 'With coach', coach_id: 'coach-1' }),
      ],
    });
    mockListCoaches.mockResolvedValueOnce({
      coaches: [makeCoach({ id: 'coach-1', title: 'Marathon Coach' })],
    });

    const { findByText, findByTestId } = render(<ConversationsScreen />);

    expect(await findByText('Marathon Coach')).toBeTruthy();
    expect(await findByText('Without a coach')).toBeTruthy();
    expect(await findByTestId('session-group-header-__no_coach__')).toBeTruthy();
    expect(await findByText('Orphan chat')).toBeTruthy();
  });
});
