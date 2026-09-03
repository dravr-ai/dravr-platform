// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Sends the first line of a new thread and pins the title the screen gives it
// ABOUTME: The thread is named for the moment it starts, not for whatever was typed into it

import React from 'react';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  const glyph = (props: Record<string, unknown>) =>
    require('react').createElement(View, { testID: `icon-${props.name}` });
  return { Ionicons: glyph, Feather: glyph, MaterialCommunityIcons: glyph };
});

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () => ({
  useRouter: () => mockRouter,
  useLocalSearchParams: () => ({}),
  useFocusEffect: () => {},
}));
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
    providerModalVisible: false,
    connectingProvider: null,
    needsCredentialsProvider: null,
    error: null,
    hasConnectedProvider: false,
    loadProviderStatus: jest.fn(),
    setSelectedProvider: jest.fn(),
    setProviderModalVisible: jest.fn(),
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
    mockCreateConversation.mockResolvedValue({ id: 'conv-1', title: 'Chat', coach_id: null });
    mockSendTurn.mockResolvedValue(null);
  });

  it('names a new thread for the moment it starts, not for the line that opened it', async () => {
    // The header's unread badge is a real query; give it a client rather than
    // mocking the bell away, so the screen renders the way it ships.
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const { getByTestId } = render(
      <QueryClientProvider client={client}>
        <ChatScreen />
      </QueryClientProvider>,
    );

    fireEvent.changeText(
      getByTestId('message-input'),
      'Est-ce que je peux faire du seuil demain matin avant le boulot ?',
    );
    fireEvent.press(getByTestId('send-button'));

    await waitFor(() => expect(mockCreateConversation).toHaveBeenCalledTimes(1));
    const { title } = mockCreateConversation.mock.calls[0][0] as { title: string };

    // `Chat Sep 2 16:18` — the prefix in the athlete's language, the day, and
    // the same 24-hour clock the conversation row shows.
    expect(title).toMatch(/^Chat .+ \d{2}:\d{2}$/);
    // The old title was the first line truncated to 50 characters, which is
    // what a thread named after its own question looks like.
    expect(title).not.toContain('seuil');

    // The line itself is still the turn, sent on the thread that was created.
    expect(mockSendTurn).toHaveBeenCalledWith(
      'conv-1',
      'Est-ce que je peux faire du seuil demain matin avant le boulot ?',
    );
  });
});
