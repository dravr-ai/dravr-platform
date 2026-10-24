// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins what the chat screen hands the verdict sheet — the thread's own name and the reply the chip hangs under
// ABOUTME: And what its menu copies for support (verdict, message, conversation ids); a claim sent back for evidence is a draft

import React from 'react';
import { Alert } from 'react-native';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';

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

const mockSetStringAsync = jest.fn();
jest.mock('expo-clipboard', () => ({
  setStringAsync: (...args: unknown[]) => mockSetStringAsync(...args),
}));
const mockPresentMenu = jest.fn();
jest.mock('../src/utils/presentMenu', () => ({
  presentMenu: (...args: unknown[]) => mockPresentMenu(...args),
}));

const CONVERSATION = {
  id: 'conv-1',
  title: 'Easy Tuesday run',
  agent_id: 'coach-1',
  message_count: 2,
  created_at: '2026-10-05T13:50:00Z',
  updated_at: '2026-10-05T13:52:00Z',
};
// The thread on screen; a test that needs Home's personal thread swaps it in.
let mockConversation: typeof CONVERSATION & { personal?: boolean } = CONVERSATION;
jest.mock('../src/screens/chat/useConversations', () => ({
  useConversations: () => ({
    conversations: [mockConversation],
    currentConversation: mockConversation,
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

// The mock factories are hoisted above this module's other constants, so the
// rows they return spell the claim out rather than reading a constant that is
// not yet assigned when they are built.
const mockMessages = [
  {
    id: 'msg-user',
    conversation_id: 'conv-1',
    role: 'user',
    content: 'How was my run?',
    created_at: '2026-10-05T13:51:00Z',
  },
  {
    id: 'msg-reply',
    conversation_id: 'conv-1',
    role: 'assistant',
    content: 'No HR zones on file, so this is an estimate. If it felt smooth, count the session as a good one.',
    created_at: '2026-10-05T13:52:00Z',
  },
];
const mockVerdicts = [
  {
    id: 'verdict-9',
    conversation_id: 'conv-1',
    message_id: 'msg-reply',
    agent_id: 'coach-1',
    claim_text: 'If it felt smooth, count the session as a good one.',
    category: 'training_prescription',
    status: 'supported',
    evidence_strength: 'mixed',
    confidence: 0.8,
    layer_fired: 'evidence',
    explanation: 'Supported by Rønnestad and Mujika 2014',
    evidence_refs: 'doi:10.1111/sms.12104',
    created_at: '2026-10-05T13:52:01Z',
  },
];
const mockSendTurn = jest.fn();
jest.mock('../src/screens/chat/useMessages', () => ({
  useMessages: () => ({
    messages: mockMessages,
    messageFeedback: {},
    messageFeedbackComment: {},
    messageBlocks: {},
    verdicts: mockVerdicts,
    verdictsLoading: false,
    isSending: false,
    error: null,
    quotaNotice: null,
    progressText: null,
    roomUnavailable: false,
    loadMessages: jest.fn().mockResolvedValue(undefined),
    refreshVerdicts: jest.fn(),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
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

function renderScreen(seed?: (client: QueryClient) => void) {
  // `gcTime: Infinity`: a seeded or fetched query would otherwise arm a
  // five-minute garbage-collection timer that keeps jest from exiting.
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } });
  seed?.(client);
  return render(
    <QueryClientProvider client={client}>
      <ChatScreen />
    </QueryClientProvider>,
  );
}

/** Open the sheet from the reply's chip, then its card's actions menu, and run the menu's one row. */
function copyFromMenu(view: ReturnType<typeof renderScreen>) {
  fireEvent.press(view.getByTestId('verdict-chip'));
  fireEvent.press(view.getByTestId('verdict-actions'));
  const [rows] = mockPresentMenu.mock.calls[0] as [{ label: string; onPress: () => void }[]];
  rows[0].onPress();
}

describe('ChatScreen verdict sheet', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockConversation = CONVERSATION;
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('names the open thread beside the reply the chip opened, and expands that reply', () => {
    const view = renderScreen();

    fireEvent.press(view.getByTestId('verdict-chip'));
    expect(view.getByTestId('verdict-source')).toHaveTextContent(/Easy Tuesday run/);

    fireEvent.press(view.getByTestId('verdict-source'));
    expect(view.getByTestId('verdict-source-preview')).toHaveTextContent(/No HR zones on file/);
  });

  // carnet#1146: the server stamps a rebound personal thread with its agent's
  // canonical English title. Home's thread is named after its agent as the
  // athlete's own agent list spells it, so the sheet names it that way too.
  it("names Home's personal thread after its agent, in the athlete's language", async () => {
    mockConversation = { ...CONVERSATION, title: 'Fuelling Agent', personal: true };
    const view = renderScreen((client) =>
      client.setQueryData(QUERY_KEYS.coaches.list(), {
        agents: [{ id: 'coach-1', title: 'Agent Ravitaillement' }],
      }),
    );

    fireEvent.press(view.getByTestId('verdict-chip'));
    await waitFor(() => expect(view.getByTestId('verdict-source')).toHaveTextContent(/Agent Ravitaillement/));
    expect(view.getByTestId('verdict-source')).not.toHaveTextContent(/Fuelling Agent/);
  });

  it('copies the verdict, message and conversation ids for support, and says so', async () => {
    mockSetStringAsync.mockResolvedValue(true);
    const view = renderScreen();

    copyFromMenu(view);

    await waitFor(() =>
      expect(mockSetStringAsync).toHaveBeenCalledWith('verdict verdict-9\nmessage msg-reply\nconversation conv-1'),
    );
    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith('Copied'));
  });

  it('says the copy failed when the clipboard refuses it', async () => {
    mockSetStringAsync.mockRejectedValue(new Error('denied'));
    const view = renderScreen();

    copyFromMenu(view);

    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith('Copy failed'));
  });

  // carnet#828: asking to back a claim up pre-fills the composer, so what the
  // athlete sends from it is reported as a draft, as it is on web.
  it('sends the back-up request it pre-filled as a draft', async () => {
    mockSendTurn.mockResolvedValue(null);
    const view = renderScreen();

    fireEvent.press(view.getByTestId('verdict-chip'));
    fireEvent.press(view.getByTestId('verdict-ask'));
    fireEvent.press(view.getByTestId('send-button'));

    await waitFor(() => expect(mockSendTurn).toHaveBeenCalledTimes(1));
    expect(mockSendTurn.mock.calls[0][1]).toContain('If it felt smooth, count the session as a good one.');
    expect(mockSendTurn.mock.calls[0][2]).toEqual({ origin: 'draft' });
  });
});
