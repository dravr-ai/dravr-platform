// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's private lane in a one-athlete room on the phone — the switch, where a line goes, "Adjust plan from this", a refused quote
// ABOUTME: The real thread, list, composer, useMessages and the phone's lane binding together, with only the network and the menu stubbed

import React, { useEffect, useRef, useState } from 'react';
import type { TextInput } from 'react-native';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { TurnRequestError, type SendTurnOptions } from '@pierre/api-client';
import type { CoachingGroup, Conversation, GroupTranscriptEntry, TurnEnvelope } from '@pierre/shared-types';

const mockGetConversationMessages = jest.fn();
const mockSendTurn = jest.fn();
const mockReadRoom = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversationMessages: (...args: unknown[]) => mockGetConversationMessages(...args),
    getConversationVerdicts: jest.fn().mockResolvedValue({ verdicts: [] }),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    stopTurn: jest.fn().mockResolvedValue(true),
    openAthleteThread: jest.fn(),
    listCommands: jest.fn(() => Promise.resolve([])),
  },
  coachesApi: { list: jest.fn(() => Promise.resolve({ agents: [] })) },
  groupsApi: { readRoom: (...args: unknown[]) => mockReadRoom(...args) },
}));
jest.mock('expo-router', () => ({ useLocalSearchParams: () => ({ conversationId: 'conv-room' }) }));
jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn() }));
jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  const glyph = (props: Record<string, unknown>) =>
    require('react').createElement(View, { testID: `icon-${props.name}` });
  return { Ionicons: glyph, Feather: glyph, MaterialCommunityIcons: glyph };
});
jest.mock('react-native-safe-area-context', () => ({
  useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }),
}));
jest.mock('../src/screens/chat/useChatVoiceInput', () => ({
  useChatVoiceInput: () => ({ isListening: false, isAvailable: false, partialTranscript: '', handleVoicePress: jest.fn() }),
}));
// The long-press menu is its own unit (MessageMenu.test.tsx); here a stub
// records what it would offer, so a test can pick a row.
jest.mock('../src/screens/chat/presentMessageMenu', () => ({ presentMessageMenu: jest.fn() }));
jest.mock('../src/screens/chat/VerdictSheet', () => ({ VerdictSheet: () => null }));
jest.mock('../src/components/ProviderNotice', () => ({ ProviderNoticeSheet: () => null }));
jest.mock('../src/components/OAuthCredentialsSection', () => ({ OAuthCredentialsSection: () => null }));
jest.mock('../src/components/IntervalsIcuLinkModal', () => ({ IntervalsIcuLinkModal: () => null }));

import { ChatThread, type ChatThreadProps } from '../src/screens/chat/ChatThread';
import { useMessages } from '../src/screens/chat/useMessages';
import { presentMessageMenu } from '../src/screens/chat/presentMessageMenu';
import { usePrivateLane } from '../src/hooks/usePrivateLane';

const COACH_ID = 'user-coach';
const ATHLETE_ID = 'user-jd';
const GROUP_ID = 'group-jd';
const ROOM_ID = 'conv-room';
const WORK_ID = 'conv-work-jd';

type LaneGroup = Pick<CoachingGroup, 'id' | 'coach_user_id' | 'solo_athlete'>;

/** The room as its coach reads it: the server names the one athlete. */
const SOLO_ROOM: LaneGroup = {
  id: GROUP_ID,
  coach_user_id: COACH_ID,
  solo_athlete: { user_id: ATHLETE_ID, display_name: 'J-D' },
};
/** A room of two athletes: the server names none. */
const TWO_ATHLETE_ROOM: LaneGroup = { id: GROUP_ID, coach_user_id: COACH_ID };

const WORK_THREAD: Conversation = {
  id: WORK_ID,
  title: 'J-D',
  message_count: 0,
  created_at: '2026-09-10T09:00:00Z',
  updated_at: '2026-09-10T09:00:00Z',
  subject_user_id: ATHLETE_ID,
  subject_group_id: GROUP_ID,
  group_id: null,
};

/** J-D's line in the room, and the agent's reply to it. */
const ROOM: GroupTranscriptEntry[] = [
  {
    id: 't2',
    speaker: 'member',
    withheld: false,
    own: false,
    author_user_id: ATHLETE_ID,
    author_display_name: 'J-D',
    content: 'Legs felt heavy this morning.',
    message_id: 'jd-1',
    created_at: '2026-09-14T07:12:00Z',
  },
  {
    id: 't3',
    speaker: 'coach',
    withheld: false,
    own: false,
    author_user_id: ATHLETE_ID,
    author_display_name: 'J-D',
    content: "I'd keep Thursday easy.",
    message_id: 'jd-2',
    created_at: '2026-09-14T07:13:00Z',
  },
];

function envelope(question: string): TurnEnvelope {
  return {
    turn_id: 'turn-private-1',
    user_message: { id: 'w-10', conversation_id: WORK_ID, role: 'user', content: question, created_at: '2026-09-14T20:40:00Z' },
    assistant: {
      message: {
        id: 'w-11',
        conversation_id: WORK_ID,
        role: 'assistant',
        content: 'Done. Two days change.',
        created_at: '2026-09-14T20:40:05Z',
      },
      blocks: [{ type: 'prose', text: 'Done. Two days change.' }],
      finish_reason: 'stop',
    },
    conversation_updated_at: '2026-09-14T20:40:05Z',
    telemetry: {
      model: 'gpt-5-codex',
      provider_name: 'copilot',
      tool_calls_count: 1,
      tools_called: ['save_training_plan'],
      execution_time_ms: 900,
    },
  };
}

const USAGE = { applyNotice: jest.fn(), level: null, message: null, sendDisabled: false } as unknown as ChatThreadProps['usageStatus'];
const PROVIDERS = {
  connectedProviders: [],
  needsCredentialsProvider: null,
  setNeedsCredentialsProvider: jest.fn(),
  loadProviderStatus: jest.fn(),
  handleConnectProvider: jest.fn(),
  refreshAfterLink: jest.fn(),
} as unknown as ChatThreadProps['providerStatus'];

/** The room screen as ChatScreen wires it: one lane, the thread drawing it. */
function Room({ group, viewerId, sendText }: { group: LaneGroup; viewerId: string; sendText: ChatThreadProps['sendText'] }) {
  const thread = useMessages();
  const lane = usePrivateLane({ group, viewerId, conversations: [WORK_THREAD] });
  const [text, setText] = useState('');
  const inputRef = useRef<TextInput>(null);
  const { loadMessages } = thread;
  useEffect(() => {
    void loadMessages(ROOM_ID, GROUP_ID);
  }, [loadMessages]);
  return (
    <ChatThread
      conversationId={ROOM_ID}
      messagesHook={thread}
      usageStatus={USAGE}
      providerStatus={PROVIDERS}
      isLoading={false}
      inputText={text}
      onChangeInputText={setText}
      inputRef={inputRef}
      sendText={sendText}
      privateLane={{ lane, agentName: 'Dravr' }}
    />
  );
}

/** A fresh cache each time, so a remembered switch can only come back from storage. */
function renderRoom(group: LaneGroup = SOLO_ROOM, viewerId = COACH_ID, sendText = jest.fn().mockResolvedValue(undefined)) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity }, mutations: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={client}>
      <Room group={group} viewerId={viewerId} sendText={sendText} />
    </QueryClientProvider>,
  );
  return { ...view, sendText };
}

/** Type a line and press send. */
function sendLine(line: string) {
  fireEvent.changeText(screen.getByTestId('message-input'), line);
  fireEvent.press(screen.getByTestId('send-button'));
}

/** "Adjust plan from this" on J-D's room line, picked from the long-press menu. */
async function adjustFromAthleteLine() {
  const row = await screen.findByTestId('message-turn-room-t2');
  fireEvent(row, 'longPress');
  const menu = presentMessageMenu as jest.Mock;
  const [options] = menu.mock.calls[menu.mock.calls.length - 1];
  act(() => {
    options.onAdjustPlan();
  });
}

describe("a coach's private lane in a one-athlete room, on the phone", () => {
  let workRows: unknown[] = [];

  beforeEach(async () => {
    jest.clearAllMocks();
    await AsyncStorage.clear();
    workRows = [];
    mockGetConversationMessages.mockImplementation(async (id: string) =>
      id === WORK_ID
        ? { messages: workRows, feedback: [] }
        : {
            messages: [{ id: 'm1', role: 'user', content: 'Morning all', created_at: '2026-09-14T07:00:00Z' }],
            feedback: [],
          },
    );
    mockReadRoom.mockResolvedValue(ROOM);
  });

  it('shows the athlete no switch', async () => {
    renderRoom(SOLO_ROOM, ATHLETE_ID);
    await screen.findByText('Legs felt heavy this morning.');
    expect(screen.queryByTestId('composer-audience-switch')).toBeNull();
    expect(screen.queryByTestId('message-turn-room-t2')).toBeNull();
  });

  it('shows the coach no switch in a room of two athletes', async () => {
    renderRoom(TWO_ATHLETE_ROOM);
    await screen.findByText('Legs felt heavy this morning.');
    expect(screen.queryByTestId('composer-audience-switch')).toBeNull();
  });

  it('starts at Everyone and sends to the room through the host, never the work thread', async () => {
    const { sendText } = renderRoom();
    const toggle = await screen.findByTestId('composer-audience-switch');
    expect(toggle).toHaveTextContent('Everyone');

    sendLine('See you Saturday');

    await waitFor(() => expect(sendText).toHaveBeenCalledWith('See you Saturday', { origin: undefined }));
    expect(mockSendTurn).not.toHaveBeenCalled();
  });

  it('remembers Only Dravr for this room on this device', async () => {
    const first = renderRoom();
    fireEvent.press(await screen.findByTestId('composer-audience-switch'));
    await waitFor(() => expect(screen.getByTestId('composer-audience-switch')).toHaveTextContent('Only Dravr'));
    // The phone keeps it in AsyncStorage, per viewer and room.
    await waitFor(async () =>
      expect(await AsyncStorage.getItem(`dravr.privateLane.audience:${COACH_ID}:${GROUP_ID}`)).toBe('private'),
    );
    first.unmount();

    renderRoom();
    await waitFor(() => expect(screen.getByTestId('composer-audience-switch')).toHaveTextContent('Only Dravr'));
  });

  it('sends an Only Dravr line to the work thread and draws it, dashed, in the room', async () => {
    mockSendTurn.mockImplementation(async (_id: string, content: string, options: SendTurnOptions) => {
      const turn = envelope(content);
      workRows = [turn.user_message, turn.assistant.message];
      options.onDone?.(turn);
    });
    const { sendText } = renderRoom();
    fireEvent.press(await screen.findByTestId('composer-audience-switch'));
    await waitFor(() => expect(screen.getByTestId('composer-audience-switch')).toHaveTextContent('Only Dravr'));

    sendLine('Move Thursday to Saturday.');

    await waitFor(() => expect(mockSendTurn).toHaveBeenCalledTimes(1));
    expect(mockSendTurn.mock.calls[0][0]).toBe(WORK_ID);
    expect(mockSendTurn.mock.calls[0][1]).toBe('Move Thursday to Saturday.');
    expect(sendText).not.toHaveBeenCalled();
    expect(await screen.findByText('Move Thursday to Saturday.')).toBeTruthy();
    expect(screen.getByTestId('private-lane-divider')).toHaveTextContent('Only you and Dravr see what follows');
    expect(screen.getByTestId('private-reply-tag')).toHaveTextContent('Only you');
  });

  it('quotes the athlete\'s line and flips the switch on "Adjust plan from this", and sends the quote along', async () => {
    mockSendTurn.mockImplementation(async (_id: string, content: string, options: SendTurnOptions) => {
      options.onDone?.(envelope(content));
    });
    renderRoom();
    expect(await screen.findByTestId('composer-audience-switch')).toHaveTextContent('Everyone');

    await adjustFromAthleteLine();

    await waitFor(() => expect(screen.getByTestId('composer-audience-switch')).toHaveTextContent('Only Dravr'));
    expect(screen.getByTestId('composer-quote-chip')).toHaveTextContent("From J-D's message");
    expect(screen.getByTestId('message-input').props.placeholder).toBe(
      "Ask Dravr to change J-D's plan. J-D won't see this.",
    );

    sendLine('Keep Thursday easy.');

    await waitFor(() => expect(mockSendTurn).toHaveBeenCalledTimes(1));
    expect(mockSendTurn.mock.calls[0][0]).toBe(WORK_ID);
    expect(mockSendTurn.mock.calls[0][2]).toMatchObject({ quoteEntryId: 't2' });
    await waitFor(() => expect(screen.queryByTestId('composer-quote-chip')).toBeNull());
  });

  it('removes the quote from its chip', async () => {
    renderRoom();
    await adjustFromAthleteLine();
    fireEvent.press(await screen.findByTestId('composer-quote-remove'));
    await waitFor(() => expect(screen.queryByTestId('composer-quote-chip')).toBeNull());
    // The switch stays where "Adjust plan from this" put it.
    expect(screen.getByTestId('composer-audience-switch')).toHaveTextContent('Only Dravr');
  });

  it('puts the words back and says why when the server refuses the quote', async () => {
    mockSendTurn.mockImplementation(async (_id: string, _content: string, options: SendTurnOptions) => {
      options.onError?.(
        new TurnRequestError('refused', 403, {
          code: 'PermissionDenied',
          message: 'refused',
          details: { reason: 'quote_withheld' },
        }),
      );
    });
    renderRoom();
    await adjustFromAthleteLine();

    sendLine('Keep Thursday easy.');

    expect(await screen.findByTestId('private-send-failed')).toHaveTextContent(
      'This message can no longer be used for the plan.',
    );
    expect(screen.getByTestId('message-input').props.value).toBe('Keep Thursday easy.');
    expect(screen.queryByTestId('composer-quote-chip')).toBeNull();
    // Nothing was stored: the question the lane showed while it ran is gone.
    expect(screen.queryByTestId('private-bubble')).toBeNull();
  });
});
