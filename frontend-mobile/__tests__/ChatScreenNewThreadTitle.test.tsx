// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Sends the first line of a new thread and pins that the screen leaves the naming to the server
// ABOUTME: The thread is named after its agent or the moment it starts, never for whatever was typed into it

import React from 'react';
import { KeyboardAvoidingView } from 'react-native';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

// The native header's height feeds the keyboard-avoiding column; there is no
// navigator under a unit test, so the header is as tall as nothing.
jest.mock('@react-navigation/elements', () => ({ useHeaderHeight: () => 0 }));
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

// An athlete with no thread open — the state the "+" and a cold start land in,
// and the only one where the screen has to name a conversation itself.
const mockCreateConversation = jest.fn();
jest.mock('../src/screens/chat/useConversations', () => ({
  useConversations: () => ({
    conversations: [],
    currentConversation: null,
    isLoading: false,
    error: null,
    loadConversations: jest.fn(),
    setCurrentConversation: jest.fn(),
    createConversation: (...args: unknown[]) => mockCreateConversation(...args),
    switchToConversation: jest.fn(),
    deleteConversation: jest.fn(),
    renameConversation: jest.fn(),
    justCreatedConversationRef: { current: null },
  }),
}));

const mockSendTurn = jest.fn();
jest.mock('../src/screens/chat/useMessages', () => ({
  useMessages: () => ({
    messages: [],
    messageFeedback: {},
    messageFeedbackComment: {},
    messageBlocks: {},
    verdicts: [],
    verdictsLoading: false,
    isSending: false,
    isLoading: false,
    error: null,
    progress: null,
    quotaNotice: null,
    loadMessages: jest.fn(),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    retryMessage: jest.fn(),
    handleThumbsUp: jest.fn(),
    handleThumbsDown: jest.fn(),
    submitFeedbackReason: jest.fn(),
    loadVerdicts: jest.fn(),
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

describe('ChatScreen new-thread title', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockCreateConversation.mockResolvedValue({ id: 'conv-1', title: 'Chat', agent_id: null });
    mockSendTurn.mockResolvedValue(null);
  });

  it('leaves the naming of a new thread to the server, never to the line that opened it', async () => {
    // The header's unread badge is a real query; give it a client rather than
    // mocking the bell away, so the screen renders the way it ships.
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const { getByTestId, UNSAFE_getByType } = render(
      <QueryClientProvider client={client}>
        <ChatScreen />
      </QueryClientProvider>,
    );

    // The composer is in the layout, so the keyboard shortens the column
    // through this view rather than lifting an overlay (Boreal v2.2 P3.4).
    expect(UNSAFE_getByType(KeyboardAvoidingView)).toBeTruthy();

    fireEvent.changeText(
      getByTestId('message-input'),
      'Est-ce que je peux faire du seuil demain matin avant le boulot ?',
    );
    fireEvent.press(getByTestId('send-button'));

    await waitFor(() => expect(mockCreateConversation).toHaveBeenCalledTimes(1));
    const params = mockCreateConversation.mock.calls[0][0] as { title?: string };

    // No title at all: the server names the thread after its agent, else for
    // the moment it starts in the athlete's language, and both clients print
    // that stored title. The old title was the first line truncated to 50
    // characters, which is what a thread named after its own question looks
    // like; the one after that was a dated stamp each client spelled itself.
    expect(params).toEqual({});
    expect(params.title).toBeUndefined();

    // The line itself is still the turn, sent on the thread that was created.
    expect(mockSendTurn).toHaveBeenCalledWith(
      'conv-1',
      'Est-ce que je peux faire du seuil demain matin avant le boulot ?',
    );
  });
});
