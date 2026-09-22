// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The real chat screen reloading a thread whose turn was lost while the app was backgrounded
// ABOUTME: The screen's own reload after the send must keep the note until a read holds the reply

import React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { IdleWatch } from '@pierre/shared-constants';

// No navigator under a unit test, so the header the column offsets by is 0 tall.
jest.mock('@react-navigation/elements', () => ({ useHeaderHeight: () => 0 }));
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
jest.mock('../src/services/api', () => ({
  chatApi: {
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

// The open thread is conv-1. `useMessages` is the real hook: the screen's
// focus effect reloads through it, which is what this file is about. The
// other hooks hand back the same state on every render, as the real ones do —
// the screen reloads the thread whenever the open conversation changes
// identity, so a fresh object per render would reload it forever.
jest.mock('../src/screens/chat/useConversations', () => {
  const conversation = { id: 'conv-1', title: 'Half marathon' };
  const state = {
    conversations: [conversation],
    currentConversation: conversation,
    isLoading: false,
    error: null,
    loadConversations: jest.fn(),
    setCurrentConversation: jest.fn(),
    createConversation: jest.fn(),
    switchToConversation: jest.fn(),
    deleteConversation: jest.fn(),
    renameConversation: jest.fn(),
    justCreatedConversationRef: { current: null },
  };
  return { useConversations: () => state };
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
jest.mock('../src/screens/chat/useMarkConversationRead', () => ({
  useMarkConversationRead: () => {},
}));
jest.mock('../src/screens/chat/useChatPlusActions', () => {
  const state = { actions: [], flows: { openParticipants: jest.fn() } };
  return { useChatPlusActions: () => state };
});

import { ChatScreen } from '../src/screens/chat/ChatScreen';
import { idleAbort, registerIdleWatch, resetIdleAbort } from '../src/services/idleSignal';
import type { Message } from '../src/types';

/** What the transport reports for an aborted turn — `sendTurn`'s own text, pinned by its unit test. */
const LOST_NOTE =
  'The app went idle before this reply arrived, so it may still have been written. ' +
  'Reopen this conversation to check, or send your message again.';
const QUESTION = 'How was my week?';
const REPLY = 'Your week: 42 km, all of it easy.';

const EARLIER: Message[] = [
  { id: 'm1', role: 'user', content: 'Hello', created_at: '2026-09-21T23:40:00Z' },
  { id: 'm2', role: 'assistant', content: 'Hi, ready when you are.', created_at: '2026-09-21T23:40:04Z' },
];
/** The server persisted the question at dispatch, before any reply. */
const QUESTION_ONLY: Message[] = [
  ...EARLIER,
  { id: 'm3', role: 'user', content: QUESTION, created_at: '2026-09-21T23:46:22Z' },
];
/** ...and went on to persist the reply after the app's stream was gone. */
const ANSWERED: Message[] = [
  ...QUESTION_ONLY,
  { id: 'm4', role: 'assistant', content: REPLY, created_at: '2026-09-21T23:47:19Z' },
];

/** Answer the next send the way the transport answers an aborted one. */
function streamUntilAborted() {
  mockSendTurn.mockImplementation(
    (
      _conversationId: string,
      _content: string,
      options: { signal?: AbortSignal; onError?: (error: Error) => void },
    ) =>
      new Promise<void>(resolve => {
        options.signal?.addEventListener('abort', () => {
          options.onError?.(new Error(LOST_NOTE));
          resolve();
        });
      }),
  );
}

describe('ChatScreen turn lost while the app was backgrounded', () => {
  let watch: IdleWatch;
  // The watch's one armed deadline, fired by hand: the screen renders on real
  // timers, and only the idle threshold is skipped ahead.
  let deadline: (() => void) | null = null;
  function passIdleThreshold() {
    const fire = deadline;
    deadline = null;
    fire?.();
  }

  beforeEach(() => {
    jest.clearAllMocks();
    resetIdleAbort();
    // The app root's binding, minus React Query: the idle stop drops the
    // stream, the return starts a fresh stretch.
    watch = new IdleWatch({
      onIdle: idleAbort,
      onSuspend: () => undefined,
      onActive: resetIdleAbort,
      setTimer: fn => {
        deadline = fn;
        return fn;
      },
      clearTimer: handle => {
        if (deadline === handle) deadline = null;
      },
    });
    registerIdleWatch(watch);
    mockGetConversationVerdicts.mockResolvedValue({ verdicts: [] });
    mockGetConversationMessages.mockResolvedValue({ messages: EARLIER });
  });

  afterEach(() => {
    registerIdleWatch(null);
    watch.stop();
  });

  it("keeps the note through the screen's own reload, and shows the reply once the athlete is back", async () => {
    streamUntilAborted();
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <ChatScreen />
      </QueryClientProvider>,
    );
    await screen.findByText('Hi, ready when you are.');

    fireEvent.changeText(screen.getByTestId('message-input'), QUESTION);
    await act(async () => {
      fireEvent.press(screen.getByTestId('send-button'));
    });
    await waitFor(() => expect(mockSendTurn).toHaveBeenCalledTimes(1));

    // Off to the Strava app; the whole threshold passes and the stream goes.
    // The server has the question and is still writing.
    await act(async () => {
      watch.suspend();
    });
    mockGetConversationMessages.mockResolvedValue({ messages: QUESTION_ONLY });
    const readsBeforeAbort = mockGetConversationMessages.mock.calls.length;
    await act(async () => {
      passIdleThreshold();
    });

    // The send settling re-runs the screen's focus effect, which reloads the
    // open thread. That read does not hold the reply, so the note and its
    // Retry stay — under the question the server now holds.
    await waitFor(() =>
      expect(mockGetConversationMessages.mock.calls.length).toBeGreaterThan(readsBeforeAbort),
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText(`⚠️ ${LOST_NOTE}`)).toBeTruthy();
    expect(screen.getByTestId('message-retry')).toBeTruthy();
    expect(screen.getAllByText(QUESTION)).toHaveLength(1);
    expect(screen.queryByText(REPLY)).toBeNull();

    // It finishes while the athlete is still away; they come back.
    mockGetConversationMessages.mockResolvedValue({ messages: ANSWERED });
    await act(async () => {
      watch.resume();
    });

    expect(await screen.findByText(REPLY)).toBeTruthy();
    expect(screen.getAllByText(REPLY)).toHaveLength(1);
    expect(screen.getAllByText(QUESTION)).toHaveLength(1);
    expect(screen.queryByText(`⚠️ ${LOST_NOTE}`)).toBeNull();
    expect(screen.queryByTestId('message-retry')).toBeNull();
  });
});
