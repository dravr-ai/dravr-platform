// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that a thread routed with `info=group` opens with the group's info over it, and only that thread
// ABOUTME: A coach's link proposal notifies the member there, where the link waits for their answer

import React from 'react';
import { render } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { mobileNotificationTarget } from '@pierre/shared-constants';

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

let mockParams: Record<string, string> = {};
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

// The sheet's own content is covered by its tests; here only whether it is up.
jest.mock('../src/screens/chat/ConversationInfoSheet', () => ({
  ConversationInfoSheet: ({ visible }: { visible: boolean }) => {
    const { Text: SheetText } = require('react-native');
    return <SheetText testID="info-sheet">{visible ? 'open' : 'closed'}</SheetText>;
  },
}));

// The member's own copy of the group's chat is the thread on screen.
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
    liveTrail: null,
    turnTrails: new Map(),
    awaitingSince: null,
    turnInFlight: false,
    roomUnavailable: false,
    loadMessages: jest.fn().mockResolvedValue(undefined),
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

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ChatScreen />
    </QueryClientProvider>,
  );
}

describe('ChatScreen routed to a group link proposal', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockParams = {};
  });

  it("opens with the group's info over the thread the proposal's notice names", () => {
    const target = mobileNotificationTarget({ screen: 'group_info', id: 'conv-room' });
    mockParams = target?.params ?? {};

    const { getByTestId } = renderScreen();

    expect(getByTestId('info-sheet')).toHaveTextContent('open');
  });

  it('opens the thread alone when the route asks for no info', () => {
    mockParams = { conversationId: 'conv-room' };

    const { getByTestId } = renderScreen();

    expect(getByTestId('info-sheet')).toHaveTextContent('closed');
  });

  it('opens no info over another thread than the one the route names', () => {
    mockParams = { conversationId: 'conv-elsewhere', info: 'group' };

    const { getByTestId } = renderScreen();

    expect(getByTestId('info-sheet')).toHaveTextContent('closed');
  });
});
