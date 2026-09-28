// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that an open group thread is read again, room included, when the app comes back to the foreground
// ABOUTME: And that a room the thread could not read is said on screen rather than left as a silent gap

import React from 'react';
import { AppState, type AppStateStatus } from 'react-native';
import { act, render } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

// The native header's height feeds the keyboard-avoiding column; there is no
// navigator under a unit test, so the header is as tall as nothing.
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

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => mockRouter,
    useFocusEffect: () => {},
  }),
);
jest.mock('expo-linking', () => ({ openURL: jest.fn() }));
jest.mock('react-native-safe-area-context', () => ({
  useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }),
}));
jest.mock('../src/contexts/AuthContext', () => ({ useAuth: () => ({ isAuthenticated: true }) }));
jest.mock('../src/services/analytics', () => ({ trackMobile: jest.fn() }));

// A group thread is open: a Telegram room the athlete is a member of.
const GROUP_CONVERSATION = {
  id: 'conv-room',
  title: 'Sunday Riders',
  agent_id: 'coach-1',
  group_id: 'group-7',
  group_name: 'Sunday Riders',
  message_count: 4,
  created_at: '2026-09-14T09:00:00Z',
  updated_at: '2026-09-14T11:00:00Z',
};
jest.mock('../src/screens/chat/useConversations', () => ({
  useConversations: () => ({
    conversations: [GROUP_CONVERSATION],
    currentConversation: GROUP_CONVERSATION,
    isLoading: false,
    error: null,
    loadConversations: jest.fn(),
    setCurrentConversation: jest.fn(),
    createConversation: jest.fn(),
    switchToConversation: jest.fn(),
    deleteConversation: jest.fn(),
    renameConversation: jest.fn(),
    justCreatedConversationRef: { current: null },
  }),
}));

const mockLoadMessages = jest.fn();
let mockRoomUnavailable = false;
jest.mock('../src/screens/chat/useMessages', () => ({
  useMessages: () => ({
    messages: [],
    messageFeedback: {},
    messageFeedbackComment: {},
    messageBlocks: {},
    verdicts: [],
    verdictsLoading: false,
    isSending: false,
    error: null,
    quotaNotice: null,
    progressText: null,
    roomUnavailable: mockRoomUnavailable,
    loadMessages: (...args: unknown[]) => mockLoadMessages(...args),
    refreshVerdicts: jest.fn(),
    sendTurn: jest.fn(),
    retryMessage: jest.fn(),
    handleThumbsUp: jest.fn(),
    handleThumbsDown: jest.fn(),
    submitFeedbackReason: jest.fn(),
    clearMessages: jest.fn(),
    setMessages: jest.fn(),
    setMessageBlocks: jest.fn(),
    setIsSending: jest.fn(),
    scrollToBottom: jest.fn(),
    flatListRef: { current: null },
  }),
}));
jest.mock('../src/screens/chat/useProviderStatus', () => ({
  useProviderStatus: () => ({
    connectedProviders: [],
    selectedProvider: null,
    connectingProvider: null,
    needsCredentialsProvider: null,
    error: null,
    hasConnectedProvider: false,
    loadProviderStatus: jest.fn(),
    setSelectedProvider: jest.fn(),
    setNeedsCredentialsProvider: jest.fn(),
    handleConnectProvider: jest.fn(),
  }),
}));
jest.mock('../src/screens/chat/useUsageStatus', () => ({
  useUsageStatus: () => ({
    data: null,
    isLoading: false,
    level: null,
    message: null,
    sendDisabled: false,
    invalidate: jest.fn(),
    applyNotice: jest.fn(),
  }),
}));
jest.mock('../src/screens/chat/useChatVoiceInput', () => ({
  useChatVoiceInput: () => ({
    isListening: false,
    isAvailable: false,
    partialTranscript: '',
    handleVoicePress: jest.fn(),
  }),
}));
jest.mock('../src/screens/chat/useMarkConversationRead', () => ({
  useMarkConversationRead: () => {},
}));
jest.mock('../src/screens/chat/useChatPlusActions', () => ({
  useChatPlusActions: () => ({ actions: [], flows: { openParticipants: jest.fn() } }),
}));

import { ChatScreen } from '../src/screens/chat/ChatScreen';

/** Every AppState listener the screen tree subscribed, so a test can drive the foreground edge. */
function captureAppStateListeners(): ((status: AppStateStatus) => void)[] {
  const listeners: ((status: AppStateStatus) => void)[] = [];
  jest.spyOn(AppState, 'addEventListener').mockImplementation((type, listener) => {
    if (type === 'change') listeners.push(listener as (status: AppStateStatus) => void);
    return { remove: jest.fn() } as unknown as ReturnType<typeof AppState.addEventListener>;
  });
  return listeners;
}

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ChatScreen />
    </QueryClientProvider>,
  );
}

describe('ChatScreen group thread', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockLoadMessages.mockResolvedValue(undefined);
    mockRoomUnavailable = false;
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('reads the open group thread, room included, when the app returns to the foreground', () => {
    const listeners = captureAppStateListeners();
    renderScreen();

    // Opening the thread reads it with its group.
    expect(mockLoadMessages).toHaveBeenCalledWith('conv-room', 'group-7');
    mockLoadMessages.mockClear();

    act(() => {
      for (const listener of listeners) listener('background');
    });
    expect(mockLoadMessages).not.toHaveBeenCalled();

    act(() => {
      for (const listener of listeners) listener('active');
    });
    expect(mockLoadMessages).toHaveBeenCalledWith('conv-room', 'group-7');
  });

  it('says the room could not be read instead of showing one side of it silently', () => {
    mockRoomUnavailable = true;
    const { getByTestId } = renderScreen();

    expect(getByTestId('room-load-failed')).toHaveTextContent(
      'The room transcript could not be loaded.',
    );
  });
});
