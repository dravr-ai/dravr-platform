// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins when the phone asks for the empty thread's starters: on the new thread, never on an open one loading
// ABOUTME: The server counts each read as one showing, so opening a thread whose messages are on their way counts none

import React from 'react';
import { render } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

// The native header's height feeds the keyboard-avoiding column; there is no
// navigator under a unit test, so the header is given a height to hand on.
const mockHeaderHeight = 91;
jest.mock('expo-router/react-navigation', () => ({
  ...jest.requireActual('expo-router/react-navigation'),
  useHeaderHeight: () => mockHeaderHeight,
}));
// The library's own jest mock renders its KeyboardAvoidingView as a bare View,
// which a test cannot tell from any other. This one keeps the rest of that
// mock and gives the view a name, so the column can be found and its props read.
jest.mock('react-native-keyboard-controller', () => {
  const React = require('react');
  const { View } = require('react-native');
  return {
    ...require('react-native-keyboard-controller/jest'),
    KeyboardAvoidingView: ({ children, ...props }: { children?: React.ReactNode }) =>
      React.createElement(View, { testID: 'keyboard-controller-avoiding-view', ...props }, children),
  };
});
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

// The thread on screen: none (the new thread) or an open one.
let mockCurrentConversation: { id: string; title: string; agent_id: string | null } | null = null;
const mockCreateConversation = jest.fn();
jest.mock('../src/screens/chat/useConversations', () => ({
  useConversations: () => ({
    conversations: [],
    currentConversation: mockCurrentConversation,
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

// What the screen asks the starters hook for: whether the empty thread is
// the one on screen, render by render.
const mockStartersShown: boolean[] = [];
jest.mock('../src/hooks/useEmptyThreadStarters', () => ({
  useEmptyThreadStarters: (shown: boolean) => {
    mockStartersShown.push(shown);
    return [];
  },
}));

import { ChatScreen } from '../src/screens/chat/ChatScreen';

function renderScreen() {
  // `gcTime: Infinity` keeps React Query from arming a garbage-collection
  // timer that would hold jest open after the last test.
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity }, mutations: { gcTime: Infinity } } });
  return render(
    <QueryClientProvider client={client}>
      <ChatScreen />
    </QueryClientProvider>,
  );
}

describe('ChatScreen — when the empty thread asks for its starters (carnet#829)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockStartersShown.length = 0;
  });

  it('asks on the new thread, where nothing has been sent yet', () => {
    mockCurrentConversation = null;
    renderScreen();
    expect(mockStartersShown.length).toBeGreaterThan(0);
    expect(mockStartersShown.at(-1)).toBe(true);
  });

  it('never asks on an open thread whose messages have not landed yet', () => {
    mockCurrentConversation = { id: 'conv-open', title: 'Tempo Agent', agent_id: 'agent-1' };
    renderScreen();
    expect(mockStartersShown.length).toBeGreaterThan(0);
    expect(mockStartersShown.every((shown) => !shown)).toBe(true);
  });
});
