// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reconnecting from a reply opens the in-app auth session; an ordinary link still opens the browser
// ABOUTME: Safari taking the reconnect over is a hand-off the callback has no way back from

import React from 'react';
import * as Linking from 'expo-linking';
import * as WebBrowser from 'expo-web-browser';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, waitFor } from '@testing-library/react-native';

import type { ReplyBlock } from '@pierre/shared-types';
import { ChatScreen } from '../src/screens/chat/ChatScreen';

const LINK = 'https://app.dravr.ai/help/garmin';
// getFriendlyUrlName drops the scheme and keeps a path this short whole.
const FRIENDLY_LINK_TEXT = 'app.dravr.ai/help/garmin';
const REPLY = `Ta connexion Garmin est expirée. Détails ici : ${LINK}`;
const AUTHORIZATION_URL = 'https://connect.garmin.com/oauth2Confirm?client_id=dravr';
const RETURN_URL = 'dravr://oauth-callback';

const mockInitMobileOAuth = jest.fn();
/** What the turn told this surface to draw — a link in prose, and a reconnect. */
const mockBlocks: ReplyBlock[] = [
  { type: 'prose', text: REPLY },
  {
    type: 'reconnect',
    provider: 'garmin',
    display_name: 'Garmin',
    url: 'https://app.dravr.ai/providers/garmin/connect?token=one-time',
    text: 'Reconnecte Garmin pour continuer.',
  },
];

jest.mock('expo-web-browser', () => ({
  openAuthSessionAsync: jest.fn(() => Promise.resolve({ type: 'cancel' })),
  openBrowserAsync: jest.fn(() => Promise.resolve({ type: 'success' })),
}));

jest.mock('expo-linking', () => ({
  openURL: jest.fn(() => Promise.resolve(true)),
  parse: jest.fn(() => ({ queryParams: {} })),
  createURL: jest.fn((path: string) => `dravr://${path}`),
}));

jest.mock('../src/services/api', () => ({
  oauthApi: {
    getProvidersStatus: jest.fn(() => Promise.resolve({ providers: [] })),
    initMobileOAuth: (...args: unknown[]) => mockInitMobileOAuth(...args),
  },
  chatApi: {},
  coachesApi: {},
  groupsApi: {},
}));

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, isLoading: false, user: { id: 'user-1' } }),
}));

jest.mock('../src/services/analytics', () => ({ trackMobile: jest.fn() }));

jest.mock('../src/screens/chat/useMessages', () => ({
  useMessages: () => ({
    messages: [
      {
        id: 'msg-1',
        role: 'assistant',
        content: 'Ta connexion Garmin est expirée. Détails ici : https://app.dravr.ai/help/garmin',
        created_at: '2026-09-02T10:00:04Z',
      },
    ],
    isSending: false,
    error: null,
    messageFeedback: {},
    messageFeedbackComment: {},
    messageBlocks: { 'msg-1': mockBlocks },
    verdicts: [],
    verdictsLoading: false,
    quotaNotice: null,
    progressText: null,
    loadMessages: jest.fn(),
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

jest.mock('../src/screens/chat/useConversations', () => ({
  useConversations: () => ({
    conversations: [{ id: 'conv-1', title: 'Garmin' }],
    currentConversation: { id: 'conv-1', title: 'Garmin' },
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

jest.mock('../src/screens/chat/useUsageStatus', () => ({
  useUsageStatus: () => ({
    data: undefined,
    isLoading: false,
    level: 'none',
    message: '',
    sendDisabled: false,
    resetsAt: '',
    invalidate: jest.fn(),
    applyNotice: jest.fn(),
  }),
}));

jest.mock('../src/screens/chat/useChatVoiceInput', () => ({
  useChatVoiceInput: () => ({
    isListening: false,
    transcript: '',
    partialTranscript: '',
    error: null,
    isAvailable: false,
    handleVoicePress: jest.fn(),
    clearTranscript: jest.fn(),
  }),
}));

jest.mock('../src/screens/chat/useMarkConversationRead', () => ({
  useMarkConversationRead: jest.fn(),
}));

jest.mock('../src/screens/chat/useChatPlusActions', () => ({
  useChatPlusActions: () => ({
    actions: [],
    flows: {
      groupNamePromptVisible: false,
      participantsVisible: false,
      openParticipants: jest.fn(),
      closeParticipants: jest.fn(),
      submitGroupName: jest.fn(),
      cancelGroupName: jest.fn(),
    },
  }),
}));

/** The screen's chrome reads the notification count through React Query. */
function renderChatScreen() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ChatScreen />
    </QueryClientProvider>,
  );
}

describe('ChatScreen provider reconnect', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockInitMobileOAuth.mockResolvedValue({ authorization_url: AUTHORIZATION_URL });
  });

  it('reconnects inside the app’s auth session rather than handing off to the browser', async () => {
    const { getByText } = renderChatScreen();

    fireEvent.press(getByText('Reconnect Garmin'));

    await waitFor(() => expect(WebBrowser.openAuthSessionAsync).toHaveBeenCalledTimes(1));
    // A fresh authorization URL minted against the app's own return address:
    // the block's URL was minted for a browser callback and cannot come back.
    expect(mockInitMobileOAuth).toHaveBeenCalledWith('garmin', RETURN_URL);
    expect(WebBrowser.openAuthSessionAsync).toHaveBeenCalledWith(AUTHORIZATION_URL, RETURN_URL);
    // Safari never gets it, so there is nothing for the athlete to come back from.
    expect(Linking.openURL).not.toHaveBeenCalled();
  });

  it('still opens an ordinary link with the system browser', async () => {
    const { getByText } = renderChatScreen();

    fireEvent.press(getByText(FRIENDLY_LINK_TEXT));

    await waitFor(() => expect(Linking.openURL).toHaveBeenCalledWith(LINK));
    expect(WebBrowser.openAuthSessionAsync).not.toHaveBeenCalled();
  });
});
