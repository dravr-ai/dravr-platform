// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The chat screen opens the thread its route names out of the one conversation list, by id
// ABOUTME: Red if the screen keeps its own list, re-reads the thread each time the list re-reads its row, or shows one thread's unsent line in another

import React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';

// No navigator under a unit test, so the header the column offsets by is 0 tall.
jest.mock('expo-router/react-navigation', () => ({
  ...jest.requireActual('expo-router/react-navigation'),
  useHeaderHeight: () => 0,
}));
jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  const glyph = (props: Record<string, unknown>) =>
    require('react').createElement(View, { testID: `icon-${props.name}` });
  return { Ionicons: glyph, Feather: glyph, MaterialCommunityIcons: glyph };
});

const mockParams: Record<string, string> = { conversationId: 'conv-1' };
const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => mockRouter,
    useLocalSearchParams: () => mockParams,
    useFocusEffect: () => {},
  }),
);
jest.mock('expo-linking', () => ({ openURL: jest.fn() }));
jest.mock('react-native-safe-area-context', () => ({
  useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }),
}));
jest.mock('../src/contexts/AuthContext', () => ({ useAuth: () => ({ isAuthenticated: true }) }));
jest.mock('../src/services/analytics', () => ({ trackMobile: jest.fn() }));
jest.mock('../src/components/notifications/NotificationBellButton', () => ({
  NotificationBellButton: () => null,
}));

// The header's title view is not under test here.
jest.mock('../src/screens/chat/ChatHeaderTitle', () => ({
  ...jest.requireActual('../src/screens/chat/ChatHeaderTitle'),
  ChatHeaderTitle: () => null,
}));
// The info sheet shows which row the screen holds open.
jest.mock('../src/screens/chat/ConversationInfoSheet', () => ({
  ConversationInfoSheet: ({ conversation }: { conversation: { title?: string } | null }) => {
    const { Text: SheetText } = require('react-native');
    return <SheetText testID="open-thread-title">{conversation?.title ?? 'none'}</SheetText>;
  },
}));

const mockGetConversations = jest.fn();
jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
  },
  oauthApi: {},
  coachesApi: {},
  groupsApi: {},
  notificationsApi: {},
}));

// `useOpenConversation` is the real hook, over the real list. The other hooks
// hand back the same state on every render, as the real ones do.
const mockLoadMessages = jest.fn().mockResolvedValue(undefined);
const mockClearMessages = jest.fn();
const mockSendTurn = jest.fn().mockResolvedValue(null);
jest.mock('../src/screens/chat/useMessages', () => {
  const state = {
    messages: [],
    messageFeedback: {},
    messageFeedbackComment: {},
    messageBlocks: {},
    verdicts: [],
    verdictsLoading: false,
    isSending: false,
    error: null,
    quotaNotice: null,
    liveTrail: null,
    turnTrails: new Map(),
    awaitingSince: null,
    turnInFlight: false,
    roomUnavailable: false,
    loadMessages: (...args: unknown[]) => mockLoadMessages(...args),
    refreshVerdicts: jest.fn(),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    retryMessage: jest.fn(),
    handleThumbsUp: jest.fn(),
    handleThumbsDown: jest.fn(),
    submitFeedbackReason: jest.fn(),
    clearMessages: (...args: unknown[]) => mockClearMessages(...args),
    setMessages: jest.fn(),
    setMessageBlocks: jest.fn(),
    setIsSending: jest.fn(),
    scrollToBottom: jest.fn(),
    flatListRef: { current: null },
  };
  return { useMessages: () => state };
});
jest.mock('../src/screens/chat/useProviderStatus', () => {
  const state = {
    connectedProviders: ['strava'],
    providersLoaded: true,
    selectedProvider: null,
    connectingProvider: null,
    needsCredentialsProvider: null,
    error: null,
    hasConnectedProvider: true,
    loadProviderStatus: jest.fn(),
    setSelectedProvider: jest.fn(),
    setNeedsCredentialsProvider: jest.fn(),
    handleConnectProvider: jest.fn(),
  };
  return { useProviderStatus: () => state };
});
jest.mock('../src/screens/chat/useUsageStatus', () => {
  const state = {
    data: null,
    isLoading: false,
    level: null,
    message: null,
    sendDisabled: false,
    invalidate: jest.fn(),
    applyNotice: jest.fn(),
  };
  return { useUsageStatus: () => state };
});
jest.mock('../src/screens/chat/useChatVoiceInput', () => {
  const state = {
    isListening: false,
    isAvailable: false,
    partialTranscript: '',
    handleVoicePress: jest.fn(),
  };
  return { useChatVoiceInput: () => state };
});
jest.mock('../src/hooks/useGroups', () => ({
  ...jest.requireActual('../src/hooks/useGroups'),
  useGroup: () => ({ group: null, isLoading: false, isError: false, error: null, refetch: jest.fn() }),
}));
jest.mock('../src/screens/chat/useMarkConversationRead', () => ({
  useMarkConversationRead: () => {},
}));
jest.mock('../src/screens/chat/useChatPlusActions', () => {
  const state = { actions: [], flows: { openParticipants: jest.fn() } };
  return { useChatPlusActions: () => state };
});

import { ChatScreen } from '../src/screens/chat/ChatScreen';
import { NEW_CONVERSATION_ID } from '../src/navigation/routes';

function row(id: string, overrides: Record<string, unknown> = {}) {
  return {
    id,
    title: `Chat ${id}`,
    agent_id: null,
    message_count: 2,
    unread_count: 0,
    created_at: '2026-10-01T10:00:00Z',
    updated_at: '2026-10-01T10:00:00Z',
    ...overrides,
  };
}

describe('ChatScreen opening its thread out of the conversation list', () => {
  let client: QueryClient;

  beforeEach(() => {
    jest.clearAllMocks();
    mockParams.conversationId = 'conv-1';
    delete mockParams.draft;
    client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    mockGetConversations.mockResolvedValue({ conversations: [row('conv-0'), row('conv-1')], total: 2 });
  });
  afterEach(() => {
    client.clear();
  });

  function mount() {
    return render(
      <QueryClientProvider client={client}>
        <ChatScreen />
      </QueryClientProvider>,
    );
  }

  it('resolves the routed thread from the list and reads it once', async () => {
    mount();

    await waitFor(() => expect(screen.getByTestId('open-thread-title').props.children).toBe('Chat conv-1'));
    expect(mockLoadMessages).toHaveBeenCalledTimes(1);
    expect(mockLoadMessages).toHaveBeenCalledWith('conv-1', undefined);
    // One read of the list fed the screen: it keeps no list of its own.
    expect(mockGetConversations).toHaveBeenCalledTimes(1);
  });

  it('follows the list when the row changes, without reading the thread again', async () => {
    mount();
    await waitFor(() => expect(mockLoadMessages).toHaveBeenCalledTimes(1));
    // The screen mounts with nothing open and says so once; from here the
    // thread on screen must not be emptied.
    mockClearMessages.mockClear();

    // What every turn does to the open row: the list re-reads it with a new
    // count and a new instant. A rename made on the list screen arrives the same way.
    mockGetConversations.mockResolvedValue({
      conversations: [
        row('conv-1', { title: 'Renamed elsewhere', message_count: 4, updated_at: '2026-10-01T11:00:00Z' }),
        row('conv-0'),
      ],
      total: 2,
    });
    await act(async () => {
      await client.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
    });

    await waitFor(() =>
      expect(screen.getByTestId('open-thread-title').props.children).toBe('Renamed elsewhere'),
    );
    expect(mockLoadMessages).toHaveBeenCalledTimes(1);
    expect(mockClearMessages).not.toHaveBeenCalled();
  });

  it("keeps each thread's unsent line to that thread when the route opens another", async () => {
    const view = mount();
    await waitFor(() => expect(screen.getByTestId('open-thread-title').props.children).toBe('Chat conv-1'));
    fireEvent.changeText(screen.getByTestId('message-input'), 'je suis pret');

    mockParams.conversationId = 'conv-0';
    view.rerender(
      <QueryClientProvider client={client}>
        <ChatScreen />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(screen.getByTestId('open-thread-title').props.children).toBe('Chat conv-0'));
    expect(screen.getByTestId('message-input').props.value).toBe('');
    fireEvent.changeText(screen.getByTestId('message-input'), 'Saturday works');

    mockParams.conversationId = 'conv-1';
    view.rerender(
      <QueryClientProvider client={client}>
        <ChatScreen />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(screen.getByTestId('message-input').props.value).toBe('je suis pret'));
  });

  // Ids of their own in the tests below: a composer line outlives the
  // screen that typed it, and the test above leaves lines in conv-0 and conv-1.
  it("fills the routed thread's composer with the route's draft and sends it as a draft", async () => {
    mockGetConversations.mockResolvedValue({ conversations: [row('conv-drafted')], total: 1 });
    mockParams.conversationId = 'conv-drafted';
    mockParams.draft = 'How did my week go?';
    mount();

    await waitFor(() => expect(screen.getByTestId('open-thread-title').props.children).toBe('Chat conv-drafted'));
    expect(screen.getByTestId('message-input').props.value).toBe('How did my week go?');
    fireEvent.press(screen.getByTestId('send-button'));

    await waitFor(() => expect(mockSendTurn).toHaveBeenCalledTimes(1));
    expect(mockSendTurn).toHaveBeenCalledWith('conv-drafted', 'How did my week go?', { origin: 'draft' });
  });

  it('leaves a line typed in a new chat, then left, out of the next thread opened', async () => {
    mockGetConversations.mockResolvedValue({ conversations: [row('conv-next')], total: 1 });
    mockParams.conversationId = NEW_CONVERSATION_ID;
    const newChat = mount();
    fireEvent.changeText(screen.getByTestId('message-input'), 'left in a new chat');
    newChat.unmount();

    mockParams.conversationId = 'conv-next';
    mount();
    await waitFor(() => expect(screen.getByTestId('open-thread-title').props.children).toBe('Chat conv-next'));
    expect(screen.getByTestId('message-input').props.value).toBe('');
  });

  it('keeps a line typed in a new chat there when the same screen opens an existing thread', async () => {
    mockGetConversations.mockResolvedValue({ conversations: [row('conv-picked')], total: 1 });
    mockParams.conversationId = NEW_CONVERSATION_ID;
    const view = mount();
    fireEvent.changeText(screen.getByTestId('message-input'), 'typed in a new chat');

    mockParams.conversationId = 'conv-picked';
    view.rerender(
      <QueryClientProvider client={client}>
        <ChatScreen />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(screen.getByTestId('open-thread-title').props.children).toBe('Chat conv-picked'));
    expect(screen.getByTestId('message-input').props.value).toBe('');

    mockParams.conversationId = NEW_CONVERSATION_ID;
    view.rerender(
      <QueryClientProvider client={client}>
        <ChatScreen />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(screen.getByTestId('message-input').props.value).toBe('typed in a new chat'));
  });

  it('finds a routed thread that sits below the first page of the list', async () => {
    mockGetConversations.mockImplementation((_limit: number, offset: number) =>
      Promise.resolve(
        offset === 0
          ? { conversations: [row('conv-0')], total: 2 }
          : { conversations: [row('conv-1', { group_id: 'group-9' })], total: 2 },
      ),
    );
    mount();

    await waitFor(() => expect(mockLoadMessages).toHaveBeenCalledWith('conv-1', 'group-9'));
    expect(mockGetConversations).toHaveBeenCalledWith(50, 1);
  });
});
