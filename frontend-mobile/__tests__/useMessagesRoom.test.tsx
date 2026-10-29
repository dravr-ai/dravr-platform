// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins how the mobile thread reads a group conversation — the room woven through the caller's own rows, and a coach's private turns
// ABOUTME: Covers the room read, the unreadable-room flag, a retry that skips room rows, and a private turn sent to the work thread with its quote

import React from 'react';
import { renderHook as rtlRenderHook, act, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { TurnRequestError, type SendTurnOptions } from '@pierre/api-client';
import type { Conversation, GroupTranscriptEntry, TurnEnvelope } from '@pierre/shared-types';

const mockGetConversationMessages = jest.fn();
const mockGetConversationVerdicts = jest.fn().mockResolvedValue({ verdicts: [] });
const mockSendTurn = jest.fn();
const mockStopTurn = jest.fn();
const mockOpenAthleteThread = jest.fn();
const mockReadRoom = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversationMessages: (...args: unknown[]) => mockGetConversationMessages(...args),
    getConversationVerdicts: (...args: unknown[]) => mockGetConversationVerdicts(...args),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    stopTurn: (...args: unknown[]) => mockStopTurn(...args),
    openAthleteThread: (...args: unknown[]) => mockOpenAthleteThread(...args),
  },
  groupsApi: {
    readRoom: (...args: unknown[]) => mockReadRoom(...args),
  },
}));

import { useMessages } from '../src/screens/chat/useMessages';
import { usePrivateLane } from '../src/hooks/usePrivateLane';
import type { Message } from '../src/types';

function renderHook<T>(hook: () => T) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return rtlRenderHook(hook, {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}

const CONVERSATION_ID = 'conv-room';
const GROUP_ID = 'group-7';

const OWN = [
  { id: 'm1', role: 'user', content: 'How was my week?', created_at: '2026-09-14T10:00:00Z' },
  { id: 'm2', role: 'assistant', content: 'Solid week.', created_at: '2026-09-14T10:00:05Z' },
];

function entry(overrides: Partial<GroupTranscriptEntry> & Pick<GroupTranscriptEntry, 'id' | 'created_at'>): GroupTranscriptEntry {
  return {
    speaker: 'member',
    withheld: false,
    own: false,
    author_user_id: 'user-jd',
    author_display_name: 'J-D',
    content: null,
    message_id: null,
    ...overrides,
  };
}

const ROOM: GroupTranscriptEntry[] = [
  entry({ id: 't1', created_at: '2026-09-14T10:00:00.2Z', own: true, content: 'How was my week?', message_id: 'm1' }),
  entry({ id: 't2', created_at: '2026-09-14T11:00:00Z', content: 'Tempo on Saturday?', message_id: 'jd-1' }),
  entry({ id: 't3', created_at: '2026-09-14T11:00:04Z', speaker: 'coach', content: 'Easy 90 minutes, J-D.', message_id: 'jd-2' }),
  entry({ id: 't4', created_at: '2026-09-14T11:05:00Z', withheld: true, author_user_id: null, author_display_name: null }),
];

describe('useMessages in a group thread', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetConversationMessages.mockResolvedValue({ messages: OWN, feedback: [] });
  });

  it('weaves the room through the caller\'s own rows, reading it back to their oldest row', async () => {
    mockReadRoom.mockResolvedValue(ROOM);
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID, GROUP_ID);
    });

    expect(mockReadRoom).toHaveBeenCalledWith(GROUP_ID, '2026-09-14T10:00:00Z');
    expect(result.current.messages.map((m) => m.id)).toEqual(['m1', 'm2', 'room-t2', 'room-t3', 'room-t4']);
    expect(result.current.messages[2].room?.author_name).toBe('J-D');
    expect(result.current.messages[3].content).toBe('Easy 90 minutes, J-D.');
    expect(result.current.messages[4].room?.withheld).toBe(true);
    expect(result.current.roomUnavailable).toBe(false);
  });

  it('reads no room for a thread that is not a group\'s', async () => {
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID, null);
    });

    expect(mockReadRoom).not.toHaveBeenCalled();
    expect(result.current.messages.map((m) => m.id)).toEqual(['m1', 'm2']);
  });

  it('keeps the caller\'s own rows and flags the room when it cannot be read', async () => {
    mockReadRoom.mockRejectedValue(new Error('network down'));
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID, GROUP_ID);
    });

    expect(result.current.messages.map((m) => m.id)).toEqual(['m1', 'm2']);
    expect(result.current.roomUnavailable).toBe(true);
  });

  it('retries the caller\'s own question, never another member\'s line that sits before the failed reply', async () => {
    // A failed reply the client kept, with another member's line landing
    // between the caller's question and it.
    const failed: Message = {
      id: 'error-1',
      role: 'assistant',
      content: '⚠️ failed',
      created_at: '2026-09-14T12:00:10Z',
      isError: true,
    };
    mockGetConversationMessages.mockResolvedValue({
      messages: [{ id: 'q1', role: 'user', content: 'Plan my Sunday?', created_at: '2026-09-14T12:00:00Z' }],
      feedback: [],
    });
    mockReadRoom.mockResolvedValue([
      entry({ id: 't9', created_at: '2026-09-14T12:00:05Z', content: 'I am in for Sunday' }),
    ]);
    mockSendTurn.mockImplementation(async () => {});
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID, GROUP_ID);
    });
    act(() => {
      result.current.setMessages((prev) => [...prev, failed]);
    });
    expect(result.current.messages.map((m) => m.id)).toEqual(['q1', 'room-t9', 'error-1']);

    await act(async () => {
      await result.current.retryMessage('error-1', CONVERSATION_ID);
    });

    expect(mockSendTurn).toHaveBeenCalledTimes(1);
    expect(mockSendTurn.mock.calls[0][0]).toBe(CONVERSATION_ID);
    expect(mockSendTurn.mock.calls[0][1]).toBe('Plan my Sunday?');
  });
});

// carnet#1249: the coach of a one-athlete room sends "Only <agent>" turns to
// their work thread about the athlete, through the phone's lane binding.
describe('useMessages with a coach\'s private lane', () => {
  const COACH_ID = 'user-coach';
  const WORK_ID = 'conv-work-jd';
  const SOLO_ROOM = {
    id: GROUP_ID,
    coach_user_id: COACH_ID,
    solo_athlete: { user_id: 'user-jd', display_name: 'J-D' },
  };
  const WORK_THREAD: Conversation = {
    id: WORK_ID,
    title: 'J-D',
    message_count: 0,
    created_at: '2026-09-10T09:00:00Z',
    updated_at: '2026-09-10T09:00:00Z',
    subject_user_id: 'user-jd',
    subject_group_id: GROUP_ID,
    group_id: null,
  };

  /** The private turn as the server answers it: the question it stored, then the reply. */
  const ENVELOPE: TurnEnvelope = {
    turn_id: 'turn-private-1',
    user_message: {
      id: 'w-10',
      conversation_id: WORK_ID,
      role: 'user',
      content: 'Make Saturday easy.',
      created_at: '2026-09-14T11:10:00Z',
    },
    assistant: {
      message: {
        id: 'w-11',
        conversation_id: WORK_ID,
        role: 'assistant',
        content: 'Done. Saturday is easy now.',
        created_at: '2026-09-14T11:10:05Z',
      },
      blocks: [{ type: 'prose', text: 'Done. Saturday is easy now.' }],
      finish_reason: 'stop',
    },
    conversation_updated_at: '2026-09-14T11:10:05Z',
    telemetry: {
      model: 'gpt-5-codex',
      provider_name: 'copilot',
      tool_calls_count: 1,
      tools_called: ['save_training_plan'],
      execution_time_ms: 900,
    },
  };

  /** What the work thread holds, read on every fetch of it. */
  let workRows: Message[] = [];

  beforeEach(() => {
    jest.clearAllMocks();
    workRows = [];
    mockGetConversationMessages.mockImplementation(async (id: string) =>
      id === WORK_ID ? { messages: workRows, feedback: [] } : { messages: OWN, feedback: [] },
    );
    mockReadRoom.mockResolvedValue(ROOM);
  });

  function renderRoom() {
    return renderHook(() => ({
      thread: useMessages(),
      lane: usePrivateLane({ group: SOLO_ROOM, viewerId: COACH_ID, conversations: [WORK_THREAD] }),
    }));
  }

  it('sends to the work thread with the quoted room entry, and never puts a private line in the room\'s rows', async () => {
    mockSendTurn.mockImplementation(async (_id: string, content: string, options: SendTurnOptions) => {
      options.onBlock?.({ type: 'prose', text: 'Done. Saturday is easy now.' });
      workRows = [{ ...ENVELOPE.user_message!, content }, ENVELOPE.assistant.message];
      options.onDone?.(ENVELOPE);
    });
    const { result } = renderRoom();
    await act(async () => {
      await result.current.thread.loadMessages(CONVERSATION_ID, GROUP_ID);
    });

    // "Adjust plan from this" on J-D's room line: the quote, and the switch to private.
    const athleteLine = result.current.thread.messages.find((m) => m.id === 'room-t2');
    expect(athleteLine).toBeDefined();
    act(() => {
      expect(result.current.lane.adjustFrom(athleteLine!)).toBe(true);
    });
    expect(result.current.lane.quote?.entry_id).toBe('t2');
    await waitFor(() => expect(result.current.lane.audience).toBe('private'));

    let outcome: Awaited<ReturnType<typeof result.current.thread.sendPrivateTurn>> | null = null;
    await act(async () => {
      outcome = await result.current.thread.sendPrivateTurn(result.current.lane.send, 'Make Saturday easy.');
    });

    expect(outcome).toMatchObject({ kind: 'sent', conversationId: WORK_ID });
    expect(mockOpenAthleteThread).not.toHaveBeenCalled();
    expect(mockSendTurn).toHaveBeenCalledTimes(1);
    expect(mockSendTurn).toHaveBeenCalledWith(
      WORK_ID,
      'Make Saturday easy.',
      expect.objectContaining({ quoteEntryId: 't2' }),
    );
    // The room conversation's rows are untouched: a private line is the lane's alone.
    expect(result.current.thread.messages.map((m) => m.id)).toEqual(['m1', 'm2', 'room-t2', 'room-t3', 'room-t4']);
    await waitFor(() => expect(result.current.lane.rows.map((m) => m.id)).toEqual(['w-10', 'w-11']));
    // The sent quote is let go; the reply's blocks are keyed for its card.
    expect(result.current.lane.quote).toBeNull();
    expect(result.current.thread.messageBlocks['w-11']).toEqual([{ type: 'prose', text: 'Done. Saturday is easy now.' }]);
    expect(result.current.thread.isSending).toBe(false);
    expect(result.current.thread.privateTurnInFlight).toBe(false);
  });

  it('draws the private turn live, and stops it in the work thread rather than the room', async () => {
    let finish: () => void = () => undefined;
    mockSendTurn.mockImplementation(
      (_id: string, _content: string, options: SendTurnOptions) =>
        new Promise<void>((resolve) => {
          options.onDelta?.('Moving Saturday');
          finish = () => {
            options.onDone?.(ENVELOPE);
            resolve();
          };
        }),
    );
    mockStopTurn.mockResolvedValue(true);
    const { result } = renderRoom();
    await act(async () => {
      await result.current.thread.loadMessages(CONVERSATION_ID, GROUP_ID);
    });

    let sending: Promise<unknown> = Promise.resolve();
    act(() => {
      sending = result.current.thread.sendPrivateTurn(result.current.lane.send, 'Make Saturday easy.');
    });
    await waitFor(() => expect(result.current.thread.streamingContent).toBe('Moving Saturday'));
    expect(result.current.thread.isSending).toBe(true);
    expect(result.current.thread.privateTurnInFlight).toBe(true);

    // The room's own stop has nothing to stop: the turn is not the room's.
    await act(async () => {
      await result.current.thread.stopTurn();
    });
    expect(mockStopTurn).not.toHaveBeenCalled();

    await act(async () => {
      await result.current.thread.stopPrivateTurn(WORK_ID);
    });
    expect(mockStopTurn).toHaveBeenCalledWith(WORK_ID);
    expect(result.current.thread.isStopping).toBe(true);

    await act(async () => {
      finish();
      await sending;
    });
    expect(result.current.thread.isStopping).toBe(false);
    expect(result.current.thread.streamingContent).toBe('');
    expect(result.current.thread.privateTurnInFlight).toBe(false);
  });

  it('hands a refused quote back as refused, and lets the quote go', async () => {
    mockSendTurn.mockImplementation(async (_id: string, _content: string, options: SendTurnOptions) => {
      options.onError?.(
        new TurnRequestError('refused', 403, {
          code: 'PermissionDenied',
          message: 'refused',
          details: { reason: 'quote_withheld' },
        }),
      );
    });
    const { result } = renderRoom();
    await act(async () => {
      await result.current.thread.loadMessages(CONVERSATION_ID, GROUP_ID);
    });
    act(() => {
      result.current.lane.adjustFrom(result.current.thread.messages.find((m) => m.id === 'room-t2')!);
    });

    let outcome: Awaited<ReturnType<typeof result.current.thread.sendPrivateTurn>> | null = null;
    await act(async () => {
      outcome = await result.current.thread.sendPrivateTurn(result.current.lane.send, 'Make Saturday easy.');
    });

    expect(outcome).toMatchObject({ kind: 'failed', refused: true });
    expect(result.current.lane.quote).toBeNull();
    expect(result.current.thread.isSending).toBe(false);
    expect(result.current.thread.messages.map((m) => m.id)).toEqual(['m1', 'm2', 'room-t2', 'room-t3', 'room-t4']);
  });
});
