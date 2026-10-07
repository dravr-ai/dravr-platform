// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reconnecting from a reply opens the in-app auth session; an ordinary link still opens the browser
// ABOUTME: Safari taking the reconnect over is a hand-off the callback has no way back from; WHOOP states its notice first; a flag shows the banner

import React from 'react';
import { Alert, Linking as DeviceLinking } from 'react-native';
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
const mockGetProvidersStatus = jest.fn();
/** What the turn told this surface to draw — a link in prose, and a reconnect. */
const GARMIN_BLOCKS: ReplyBlock[] = [
  { type: 'prose', text: REPLY },
  {
    type: 'reconnect',
    provider: 'garmin',
    display_name: 'Garmin',
    url: 'https://app.dravr.ai/providers/garmin/connect?token=one-time',
    text: 'Reconnecte Garmin pour continuer.',
  },
];
/** A WHOOP reconnect, as the turn draws one. */
const WHOOP_BLOCKS: ReplyBlock[] = [
  { type: 'prose', text: 'Ta connexion WHOOP est expirée.' },
  {
    type: 'reconnect',
    provider: 'whoop',
    display_name: 'WHOOP',
    url: 'https://app.dravr.ai/r/whoop-picker',
    text: 'Reconnecte WHOOP pour continuer.',
  },
];
let mockBlocks: ReplyBlock[] = GARMIN_BLOCKS;

// No navigator under a unit test, so the header the column offsets by is 0 tall.
jest.mock('expo-router/react-navigation', () => ({
  ...jest.requireActual('expo-router/react-navigation'),
  useHeaderHeight: () => 0,
}));

jest.mock('expo-web-browser', () => ({
  openAuthSessionAsync: jest.fn(() => Promise.resolve({ type: 'cancel' })),
  openBrowserAsync: jest.fn(() => Promise.resolve({ type: 'success' })),
}));

jest.mock('expo-linking', () => ({
  parse: jest.fn((url: string) => ({ queryParams: url.includes('success=true') ? { success: 'true' } : {} })),
  createURL: jest.fn((path: string) => `dravr://${path}`),
}));

jest.mock('../src/services/api', () => ({
  oauthApi: {
    getProvidersStatus: () => mockGetProvidersStatus(),
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
    mockBlocks = GARMIN_BLOCKS;
    mockGetProvidersStatus.mockResolvedValue({ providers: [] });
    mockInitMobileOAuth.mockResolvedValue({ authorization_url: AUTHORIZATION_URL });
    // A link leaves the app through openExternal, the one opener (carnet#803).
    jest.spyOn(DeviceLinking, 'openURL').mockResolvedValue(true);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('reconnects inside the app’s auth session rather than handing off to the browser', async () => {
    const { getByText } = renderChatScreen();

    fireEvent.press(getByText('Reconnect Garmin'));

    await waitFor(() => expect(WebBrowser.openAuthSessionAsync).toHaveBeenCalledTimes(1));
    // A fresh authorization URL minted against the app's own return address:
    // the block's URL was minted for a browser callback and cannot come back.
    expect(mockInitMobileOAuth).toHaveBeenCalledWith('garmin', RETURN_URL, { tosConsent: false });
    expect(WebBrowser.openAuthSessionAsync).toHaveBeenCalledWith(AUTHORIZATION_URL, RETURN_URL);
    // Safari never gets it, so there is nothing for the athlete to come back from.
    expect(DeviceLinking.openURL).not.toHaveBeenCalled();
  });

  it("states WHOOP's owner authorization before reconnecting, and carries the acceptance", async () => {
    mockBlocks = WHOOP_BLOCKS;
    mockGetProvidersStatus.mockResolvedValue({
      providers: [
        {
          provider: 'whoop',
          display_name: 'WHOOP',
          requires_oauth: true,
          connected: true,
          needs_reauth: true,
          capabilities: ['sleep', 'recovery'],
          consent_required: true,
        },
      ],
    });
    const { getByText, findByText, getByTestId } = renderChatScreen();
    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalled());

    fireEvent.press(getByText('Reconnect WHOOP'));

    expect(await findByText('Before you connect WHOOP')).toBeTruthy();
    expect(mockInitMobileOAuth).not.toHaveBeenCalled();
    expect(getByTestId('provider-notice-continue')).toBeDisabled();

    fireEvent.press(getByTestId('provider-notice-consent'));
    fireEvent.press(getByTestId('provider-notice-continue'));

    await waitFor(() =>
      expect(mockInitMobileOAuth).toHaveBeenCalledWith('whoop', RETURN_URL, { tosConsent: true }),
    );
    expect(WebBrowser.openAuthSessionAsync).toHaveBeenCalledWith(AUTHORIZATION_URL, RETURN_URL);
  });

  // carnet#647: the thread is pushed over the tab shell and hides its
  // banner, so the thread carries the one the athlete sees.
  it('shows the reconnect banner in the thread for a connected provider flagged needs_reauth', async () => {
    mockGetProvidersStatus.mockResolvedValue({
      providers: [
        {
          provider: 'garmin',
          display_name: 'Garmin',
          requires_oauth: false,
          connected: true,
          needs_reauth: true,
          capabilities: ['activities'],
        },
      ],
    });
    const { findByTestId, getAllByTestId } = renderChatScreen();

    expect(await findByTestId('reconnect-banner-providers')).toHaveTextContent(
      'Reconnect Garmin to see your new activities.',
    );
    expect(getAllByTestId('reconnect-banner')).toHaveLength(1);
  });

  it('shows no reconnect banner in the thread while every connection is healthy', async () => {
    mockGetProvidersStatus.mockResolvedValue({
      providers: [
        {
          provider: 'garmin',
          display_name: 'Garmin',
          requires_oauth: false,
          connected: true,
          needs_reauth: false,
          capabilities: ['activities'],
        },
      ],
    });
    const { queryByTestId, getByTestId } = renderChatScreen();

    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalled());
    await mockGetProvidersStatus.mock.results[0].value;
    expect(getByTestId('chat-screen')).toBeTruthy();
    expect(queryByTestId('reconnect-banner')).toBeNull();
  });

  // A reconnect made from a reply reloads the thread's status; the shared
  // one the banner and the header read takes the same answer.
  it('drops the banner and the reconnect wording once a reconnect from the reply succeeds', async () => {
    const flagged = {
      providers: [
        {
          provider: 'garmin',
          display_name: 'Garmin',
          requires_oauth: true,
          connected: true,
          needs_reauth: true,
          capabilities: ['activities'],
        },
      ],
    };
    const healthy = { providers: [{ ...flagged.providers[0], needs_reauth: false }] };
    mockGetProvidersStatus.mockResolvedValue(flagged);
    (WebBrowser.openAuthSessionAsync as jest.Mock).mockResolvedValueOnce({
      type: 'success',
      url: `${RETURN_URL}?success=true`,
    });
    // Keyed on the URL, not queued once: the screen parses other links too,
    // and a queued answer could be spent on one of them first.
    (Linking.parse as jest.Mock).mockImplementation((url: string) => ({
      queryParams: url.includes('success=true') ? { success: 'true' } : {},
    }));
    const { findByTestId, getByText, queryByTestId } = renderChatScreen();

    expect(await findByTestId('chat-header-provider-status')).toHaveTextContent('Reconnect needed');
    expect(await findByTestId('reconnect-banner')).toBeTruthy();

    mockGetProvidersStatus.mockResolvedValue(healthy);
    fireEvent.press(getByText('Reconnect Garmin'));

    await waitFor(() => expect(WebBrowser.openAuthSessionAsync).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(queryByTestId('reconnect-banner')).toBeNull(), { timeout: 5000 });
    expect(await findByTestId('chat-header-provider-status')).toHaveTextContent('Garmin connected');
  });

  it('still opens an ordinary link with the system browser', async () => {
    const { getByText } = renderChatScreen();

    fireEvent.press(getByText(FRIENDLY_LINK_TEXT));

    await waitFor(() => expect(DeviceLinking.openURL).toHaveBeenCalledWith(LINK));
    expect(WebBrowser.openAuthSessionAsync).not.toHaveBeenCalled();
  });

  it('says so when the device cannot open a link from a reply', async () => {
    (DeviceLinking.openURL as jest.Mock).mockRejectedValue(new Error('no handler'));
    const alert = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    const { getByText } = renderChatScreen();

    fireEvent.press(getByText(FRIENDLY_LINK_TEXT));

    await waitFor(() =>
      expect(alert).toHaveBeenCalledWith('Could not open link', `Open ${LINK} in your browser instead.`),
    );
  });
});
