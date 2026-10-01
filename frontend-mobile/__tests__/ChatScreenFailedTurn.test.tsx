// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The real chat screen opening a thread for its first question, whose turn fails at once
// ABOUTME: The read the new thread starts must not land after the failure and wipe the question and its note

import React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { TurnFailedError } from '@pierre/api-client';
import { i18n } from '@pierre/i18n';

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
jest.mock('expo-linking', () => ({ openURL: jest.fn() }));
jest.mock('react-native-safe-area-context', () => ({
  useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }),
}));
jest.mock('../src/contexts/AuthContext', () => ({ useAuth: () => ({ isAuthenticated: true }) }));
jest.mock('../src/services/analytics', () => ({ trackMobile: jest.fn() }));
jest.mock('../src/components/notifications/NotificationBellButton', () => ({
  NotificationBellButton: () => null,
}));

const mockGetConversationMessages = jest.fn();
const mockGetConversationVerdicts = jest.fn();
const mockSendTurn = jest.fn();
const mockCreateConversation = jest.fn();
const mockGetConversations = jest.fn();
jest.mock('../src/services/api', () => ({
  chatApi: {
    createConversation: (...args: unknown[]) => mockCreateConversation(...args),
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
    getConversationMessages: (...args: unknown[]) => mockGetConversationMessages(...args),
    getConversationVerdicts: (...args: unknown[]) => mockGetConversationVerdicts(...args),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    submitMessageFeedback: jest.fn(),
    deleteMessageFeedback: jest.fn(),
  },
  oauthApi: {},
  coachesApi: {},
  groupsApi: {},
  notificationsApi: {},
}));

// `useConversations` and `useMessages` are the real hooks: the new thread's
// creation and the read it starts are what this file is about. The other
// hooks hand back the same state on every render, as the real ones do.
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
jest.mock('../src/screens/chat/useMarkConversationRead', () => ({
  useMarkConversationRead: () => {},
}));
jest.mock('../src/screens/chat/useChatPlusActions', () => {
  const state = { actions: [], flows: { openParticipants: jest.fn() } };
  return { useChatPlusActions: () => state };
});

import { ChatScreen } from '../src/screens/chat/ChatScreen';
import type { Message } from '../src/types';

const QUESTION = 'Was I too fast on the first kilometre?';
/** The server's own words for the refusal — never what the athlete reads. */
const SERVER_TEXT = 'The resource is temporarily unavailable';
/** What the athlete reads instead: the catalogue's sentence for the code, regex-escaped. */
const FAILURE = i18n.t('errors.generic').replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const CREATED = { id: 'conv-new', title: 'Sep 30', created_at: '2026-09-30T10:00:00Z', updated_at: '2026-09-30T10:00:00Z' };

let landRead: () => void = () => undefined;

describe('ChatScreen first question whose turn fails at once', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetConversationVerdicts.mockResolvedValue({ verdicts: [] });
    mockGetConversations.mockResolvedValue({ conversations: [] });
    mockCreateConversation.mockResolvedValue(CREATED);
    // The new thread's read is still on the wire when the turn fails, and
    // lands after it with the empty transcript the server holds.
    let answerRead: (value: { messages: Message[] }) => void = () => undefined;
    mockGetConversationMessages.mockImplementation(
      () => new Promise((resolve) => {
        answerRead = resolve;
      }),
    );
    landRead = () => answerRead({ messages: [] });
    // The turn fails at once — the transport's error, before any reply.
    mockSendTurn.mockImplementation(
      async (_conversationId: string, _content: string, options: { onError?: (error: Error) => void }) => {
        options.onError?.(new TurnFailedError(SERVER_TEXT, 'ResourceUnavailable'));
      },
    );
  });

  it('keeps the question and its failure when the new thread\'s read lands after them', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <ChatScreen />
      </QueryClientProvider>,
    );
    await screen.findByTestId('chat-empty-state');

    fireEvent.changeText(screen.getByTestId('message-input'), QUESTION);
    await act(async () => {
      fireEvent.press(screen.getByTestId('send-button'));
    });
    await waitFor(() => expect(mockSendTurn).toHaveBeenCalledWith('conv-new', QUESTION, expect.anything()));
    expect(await screen.findByText(new RegExp(`^⚠️ ${FAILURE}`))).toBeTruthy();

    await act(async () => {
      landRead();
      await Promise.resolve();
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getAllByText(QUESTION)).toHaveLength(1);
    expect(screen.getByText(new RegExp(`^⚠️ ${FAILURE}`))).toBeTruthy();
    expect(screen.getByTestId('message-retry')).toBeTruthy();
    expect(screen.queryByText(new RegExp(SERVER_TEXT))).toBeNull();
  });
});
