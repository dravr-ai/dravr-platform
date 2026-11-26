// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A trifecta athlete's room or plan thread opened by a link or a notification carries « Open the plan » in its header, to the athlete's workspace (carnet#1818)
// ABOUTME: Pins it for a roster coach on a conversation the server names the athlete on, and its absence for every other reader and thread

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

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
    useLocalSearchParams: () => ({ conversationId: 'conv-room-lea' }),
    useFocusEffect: () => {},
  }),
);
jest.mock('expo-linking', () => ({ openURL: jest.fn() }));
jest.mock('react-native-safe-area-context', () => ({
  useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }),
}));
let mockManagesRoster = true;
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, user: { id: 'user-coach', manages_roster: mockManagesRoster } }),
}));
jest.mock('../src/services/analytics', () => ({ trackMobile: jest.fn() }));
jest.mock('../src/screens/chat/ConversationInfoSheet', () => ({ ConversationInfoSheet: () => null }));

const LEA = { user_id: 'u-lea', display_name: 'Léa Gagnon' };
/** The coach's room with Léa alone, as the list names it to her coach. */
const ROOM = {
  id: 'conv-room-lea',
  title: 'TRAKA',
  agent_id: 'coach-1',
  group_id: 'g-lea',
  group_name: 'TRAKA',
  message_count: 4,
  created_at: '2026-10-01T09:00:00Z',
  updated_at: '2026-11-25T11:00:00Z',
};
let mockConversation: Record<string, unknown> = { ...ROOM, solo_athlete: LEA };
jest.mock('../src/screens/chat/useOpenConversation', () => ({
  useOpenConversation: () => ({
    conversations: [mockConversation],
    openId: mockConversation.id,
    currentConversation: mockConversation,
    isLoading: false,
    refresh: jest.fn(),
    openConversation: jest.fn(),
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
// The room's group, read here and never from the network: the header action
// reads the athlete off the open conversation, not off the group.
jest.mock('../src/hooks/useGroups', () => ({
  ...jest.requireActual('../src/hooks/useGroups'),
  useGroup: () => ({ group: null, isLoading: false, isError: false, error: null, refetch: jest.fn() }),
}));
jest.mock('../src/screens/chat/useMarkConversationRead', () => ({
  useMarkConversationRead: () => {},
}));
jest.mock('../src/screens/chat/useChatPlusActions', () => ({
  useChatPlusActions: () => ({ actions: [], flows: { openParticipants: jest.fn() } }),
}));

import { ChatScreen } from '../src/screens/chat/ChatScreen';
import { coachWorkspaceHref } from '../src/navigation/routes';

function renderScreen() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <ChatScreen />
    </QueryClientProvider>,
  );
}

describe("ChatScreen — a trifecta athlete's thread", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockManagesRoster = true;
    mockConversation = { ...ROOM, solo_athlete: LEA };
  });

  it("carries « Open the plan » in the header, opening the athlete's workspace", () => {
    const { getByTestId } = renderScreen();

    const open = getByTestId('thread-open-workspace');
    expect(open).toHaveTextContent('Open the plan');
    expect(getByTestId('stack-header-right')).toContainElement(open);
    fireEvent.press(open);
    expect(mockRouter.push).toHaveBeenCalledWith(coachWorkspaceHref('u-lea'));
  });

  it('carries it on the plan thread too, which the server names the athlete on as well', () => {
    mockConversation = {
      id: 'conv-room-lea',
      title: 'Léa Gagnon',
      subject_user_id: 'u-lea',
      subject_group_id: 'g-lea',
      subject_display_name: 'Léa Gagnon',
      message_count: 2,
      solo_athlete: LEA,
      created_at: '2026-11-14T00:00:00Z',
      updated_at: '2026-11-25T10:05:00Z',
    };
    const { getByTestId } = renderScreen();

    fireEvent.press(getByTestId('thread-open-workspace'));
    expect(mockRouter.push).toHaveBeenCalledWith(coachWorkspaceHref('u-lea'));
  });

  it('carries nothing for a coach without a roster, who has no workspace to open', () => {
    mockManagesRoster = false;
    const { queryByTestId, getByTestId } = renderScreen();

    expect(getByTestId('stack-header-right')).toBeTruthy();
    expect(queryByTestId('thread-open-workspace')).toBeNull();
  });

  it('carries nothing on a room the server names no single athlete on', () => {
    mockConversation = { ...ROOM };
    const { queryByTestId, getByTestId } = renderScreen();

    expect(getByTestId('stack-header-right')).toBeTruthy();
    expect(queryByTestId('thread-open-workspace')).toBeNull();
  });
});
