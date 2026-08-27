// ABOUTME: e2e — the unified list, the chat tab badge and the read marker over one stubbed server
// ABOUTME: Rows badge what is unread, opening a thread POSTs /read, and the badge clears from the same cache

import React from 'react';
import { render, fireEvent, waitFor, act } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Conversation } from '@pierre/shared-types';

import { installHttpStub, type HttpStub } from './helpers/httpStub';

const mockPush = jest.fn();
let mockSegments: string[] = ['(app)', '(tabs)', '(chat)'];

jest.mock('expo-router', () => {
  const React = require('react');
  const { View } = require('react-native');
  return {
    useRouter: () => ({
      push: mockPush,
      replace: jest.fn(),
      back: jest.fn(),
      navigate: jest.fn(),
      canGoBack: () => true,
    }),
    useLocalSearchParams: () => ({}),
    useGlobalSearchParams: () => ({}),
    useSegments: () => mockSegments,
    useFocusEffect: (cb: () => void | (() => void)) => {
      React.useEffect(() => cb(), [cb]);
    },
    Tabs: Object.assign(
      ({ children }: { children: React.ReactNode }) => React.createElement(View, null, children),
      { Screen: () => null },
    ),
  };
});

import { ConversationsScreen } from '../../src/screens/conversations/ConversationsScreen';
import { ExpandableTabBar } from '../../src/components/ui/ExpandableTabBar';

function conversation(overrides: Partial<Conversation> & { id: string }): Conversation {
  return {
    title: 'Tempo Tuesday',
    coach_id: null,
    message_count: 4,
    unread_count: 0,
    created_at: '2026-08-20T10:00:00Z',
    updated_at: '2026-08-26T09:50:00Z',
    ...overrides,
  } as Conversation;
}

/** The list screen and the tab bar over one cache, the way the app mounts them. */
function renderShell() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ConversationsScreen />
      <ExpandableTabBar />
    </QueryClientProvider>,
  );
}

describe('the unified conversation list and its read marker', () => {
  let stub: HttpStub;
  let rows: Conversation[];

  beforeEach(() => {
    mockPush.mockClear();
    mockSegments = ['(app)', '(tabs)', '(chat)'];
    rows = [
      conversation({
        id: 'conv-telegram',
        title: 'Telegram DM',
        channel_type: 'telegram',
        unread_count: 2,
        message_count: 6,
        last_message: { preview: 'On garde le tempo demain', role: 'assistant', created_at: '2026-08-26T09:50:00Z' },
      }),
      conversation({
        id: 'conv-coach',
        title: 'Training plan',
        coach_id: 'coach-1',
        coach_handle: 'coach-tempo',
        coach_title: 'Coach Tempo',
        unread_count: 3,
        message_count: 9,
        last_message: { preview: 'Easy Thursday', role: 'assistant', created_at: '2026-08-26T08:00:00Z' },
      }),
      conversation({
        id: 'conv-group',
        title: 'Harricana',
        group_id: 'group-1',
        group_name: 'Harricana',
        coach_title: 'Coach Tempo',
        unread_count: 0,
        message_count: 12,
        last_message: { preview: 'Bloc 3 starts Monday', role: 'assistant', created_at: '2026-08-25T18:00:00Z' },
      }),
    ];

    stub = installHttpStub({
      'GET /api/chat/conversations?limit=50&offset=0': () => ({
        data: { conversations: rows, total: rows.length, limit: 50, offset: 0 },
      }),
      'POST /api/chat/conversations/conv-coach/read': () => {
        rows = rows.map((row) => (row.id === 'conv-coach' ? { ...row, unread_count: 0 } : row));
        return { data: { success: true } };
      },
      'GET /api/notifications/unread-count': { data: { unread_count: 0 } },
    });
  });

  afterEach(() => {
    stub.restore();
  });

  // Turns red if a messaging-origin thread stops appearing in the app's own
  // list, or if the kinds stop being told apart on the row.
  it('draws every conversation as one row whatever created it', async () => {
    const { findByTestId, getByTestId } = renderShell();

    expect(await findByTestId('conversation-row-conv-telegram')).toBeTruthy();
    expect(getByTestId('conversation-channel-badge-conv-telegram')).toHaveTextContent('Telegram');
    expect(getByTestId('conversation-handle-conv-coach')).toHaveTextContent('@coach-tempo');
    expect(getByTestId('conversation-kind-conv-group').props.accessibilityLabel).toBe('Group chat');
    expect(getByTestId('conversation-preview-conv-group')).toHaveTextContent('Coach Tempo: Bloc 3 starts Monday');
  });

  it('badges the chat tab with the unread total of the same rows', async () => {
    const { findByTestId } = renderShell();
    expect(await findByTestId('tab-chat-badge')).toHaveTextContent('5');
  });

  // The whole point of the marker: opening a thread posts it, the row's badge
  // goes, and the tab pill drops by exactly that row's count.
  it('opening an unread thread posts the marker and clears its share of the badge', async () => {
    const { findByTestId, queryByTestId } = renderShell();

    await act(async () => {
      fireEvent.press(await findByTestId('conversation-row-conv-coach'));
    });

    expect(mockPush).toHaveBeenCalledWith({
      pathname: '/(app)/(tabs)/(chat)/[conversationId]',
      params: { conversationId: 'conv-coach' },
    });
    await waitFor(() => {
      expect(stub.requestsFor('POST').map((request) => request.url)).toEqual([
        '/api/chat/conversations/conv-coach/read',
      ]);
    });

    await waitFor(() => expect(queryByTestId('conversation-unread-conv-coach')).toBeNull());
    expect(await findByTestId('tab-chat-badge')).toHaveTextContent('2');
  });

  it('leaves a read thread alone when it is opened', async () => {
    const { findByTestId } = renderShell();

    await act(async () => {
      fireEvent.press(await findByTestId('conversation-row-conv-group'));
    });

    expect(stub.requestsFor('POST')).toEqual([]);
  });
});
