// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for a coach's private lane in a one-athlete room — who sees the switch, the work thread's rows, the quote, the send
// ABOUTME: Both clients bind this hook, so a divergence here would route a coach's private words differently on web and the phone

import { describe, it, expect, vi } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  TurnFailedError,
  TurnRequestError,
  type MessagesResponse,
  type MessageWindow,
  type SendTurnOptions,
} from '@pierre/api-client';
import { composeRoomThread, type Translate } from '@pierre/chat-utils';
import type {
  CoachingGroup,
  Conversation,
  GroupTranscriptEntry,
  Message,
  TurnEnvelope,
} from '@pierre/shared-types';
import en from '../../i18n/src/locales/en/translation.json';
import {
  audienceStorageKey,
  createPrivateLaneHooks,
  findWorkThread,
  privateSendFailureText,
  soloAthleteFor,
  type PrivateLaneApis,
  type UsePrivateLaneInput,
} from '../src/privateLaneHooks';
import { describeTurnFailure } from '../src/turnFailure';

const COACH_ID = 'user-coach';
const ATHLETE_ID = 'user-alice';
const GROUP_ID = 'group-alice';
const WORK_ID = 'conv-work-alice';
const ENTRY_ID = '8f0c6a2e-4c1b-4d7e-9a39-2b5f1f1d9e10';

/** The English catalogue, resolved the way a client's `t` resolves a key and its params. */
const t: Translate = (key, params = {}) => {
  const template = key
    .split('.')
    .reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], en);
  if (typeof template !== 'string') throw new Error(`en has no ${key}`);
  return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(params[name]));
};

type LaneGroup = UsePrivateLaneInput['group'];

/** The room as its coach reads it: the server names the one athlete. */
const SOLO_ROOM: NonNullable<LaneGroup> = {
  id: GROUP_ID,
  coach_user_id: COACH_ID,
  solo_athlete: { user_id: ATHLETE_ID, display_name: 'Alice Runner' },
};

function conversation(overrides: Partial<Conversation> & Pick<Conversation, 'id'>): Conversation {
  return {
    title: 'Alice Runner',
    message_count: 2,
    created_at: '2026-10-20T10:00:00Z',
    updated_at: '2026-10-28T10:00:00Z',
    ...overrides,
  };
}

const WORK_THREAD = conversation({
  id: WORK_ID,
  subject_user_id: ATHLETE_ID,
  subject_group_id: GROUP_ID,
  subject_display_name: 'Alice Runner',
  group_id: null,
});
const ROOM_THREAD = conversation({ id: 'conv-room', title: 'Alice · 70.3 build', group_id: GROUP_ID });

const WORK_ROWS: Message[] = [
  { id: 'w-1', role: 'user', content: 'Plan for the week?', created_at: '2026-10-27T20:00:00Z' },
  { id: 'w-2', role: 'tool_call', content: '<tool_call>get_plan</tool_call>', created_at: '2026-10-27T20:00:01Z' },
  { id: 'w-3', role: 'assistant', content: 'Here is the week.', created_at: '2026-10-27T20:00:04Z' },
];

/** The athlete's words in the room and the agent's reply to them, as the coach's room thread draws them. */
function roomRows() {
  const entry = (overrides: Partial<GroupTranscriptEntry> & Pick<GroupTranscriptEntry, 'id' | 'created_at'>) => ({
    speaker: 'member' as const,
    withheld: false,
    own: false,
    author_user_id: ATHLETE_ID,
    author_display_name: 'Alice Runner',
    content: 'Legs felt heavy this morning.',
    message_id: null,
    ...overrides,
  });
  const [athleteRow, replyRow] = composeRoomThread(
    [],
    [
      entry({ id: ENTRY_ID, created_at: '2026-10-28T07:12:00Z' }),
      entry({ id: 'e-reply', created_at: '2026-10-28T07:13:00Z', speaker: 'coach', content: "I'd keep Thursday easy." }),
    ],
  );
  return { athleteRow, replyRow };
}

/** The turn a private question comes back as: the server's copy of the question carries the quote it verified. */
function envelope(question: string, quoted: boolean): TurnEnvelope {
  return {
    turn_id: 'turn-private-1',
    user_message: {
      id: 'w-10',
      conversation_id: WORK_ID,
      role: 'user',
      content: question,
      created_at: '2026-10-28T20:40:00Z',
      ...(quoted
        ? {
            quote: {
              entry_id: ENTRY_ID,
              author_user_id: ATHLETE_ID,
              author_display_name: 'Alice Runner',
              content: 'Legs felt heavy this morning.',
              created_at: '2026-10-28T07:12:00+00:00',
            },
          }
        : {}),
    },
    assistant: {
      message: {
        id: 'w-11',
        conversation_id: WORK_ID,
        role: 'assistant',
        content: 'Done. Two days change.',
        created_at: '2026-10-28T20:40:05Z',
      },
      blocks: [
        { type: 'prose', text: 'Done. Two days change.' },
        { type: 'scene', scene_blocks: '[{"kind":"plan"}]' },
      ],
      finish_reason: 'stop',
    },
    conversation_updated_at: '2026-10-28T20:40:05Z',
    telemetry: { model: 'gpt-5-codex', provider_name: 'copilot', tool_calls_count: 1, tools_called: ['save_training_plan'], execution_time_ms: 900 },
  };
}

/** A refusal as the turn transport hands it to `onError`. */
function turnRefusal(status: number, reason: string, code = 'PermissionDenied') {
  return new TurnRequestError('refused', status, { code, message: 'refused', details: { reason } });
}

/** A refusal shaped as the shared axios instance rejects it. */
function axiosRefusal(status: number, reason: string) {
  return Object.assign(new Error(`Request failed with status code ${status}`), {
    response: { status, data: { code: 'PermissionDenied', details: { reason } } },
  });
}

function fakeChatApi() {
  return {
    openAthleteThread: vi.fn<(athleteUserId: string, groupId: string) => Promise<Conversation>>(),
    getConversationMessages: vi.fn<(conversationId: string, window?: MessageWindow) => Promise<MessagesResponse>>(),
    sendTurn: vi.fn<(conversationId: string, content: string, options?: SendTurnOptions) => Promise<void>>(),
  };
}

function memoryStore(initial: Record<string, string> = {}) {
  const items = new Map(Object.entries(initial));
  return {
    items,
    getItem: vi.fn(async (key: string) => items.get(key) ?? null),
    setItem: vi.fn(async (key: string, value: string) => {
      items.set(key, value);
    }),
  };
}

function setup(store: PrivateLaneApis['audienceStore'] = memoryStore()) {
  const api = fakeChatApi();
  api.getConversationMessages.mockResolvedValue({ messages: WORK_ROWS, has_more: false, awaiting_reply: null });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => createElement(QueryClientProvider, { client }, children);
  const hooks = createPrivateLaneHooks({
    chatApi: api as unknown as PrivateLaneApis['chatApi'],
    audienceStore: store,
  });
  return { api, client, wrapper, hooks, store };
}

function renderLane(
  ctx: ReturnType<typeof setup>,
  input: Partial<UsePrivateLaneInput> = {},
) {
  return renderHook((props: UsePrivateLaneInput) => ctx.hooks.usePrivateLane(props), {
    wrapper: ctx.wrapper,
    initialProps: { group: SOLO_ROOM, viewerId: COACH_ID, conversations: [ROOM_THREAD, WORK_THREAD], ...input },
  });
}

/** A turn transport that answers every send with `turn`. */
function answers(api: ReturnType<typeof fakeChatApi>, turn: (content: string) => TurnEnvelope) {
  api.sendTurn.mockImplementation(async (_id, content, options) => {
    options?.onDone?.(turn(content));
  });
}

describe('soloAthleteFor', () => {
  it("names the room's one athlete to its coach alone", () => {
    expect(soloAthleteFor(SOLO_ROOM, COACH_ID)).toEqual({ user_id: ATHLETE_ID, display_name: 'Alice Runner' });
    expect(soloAthleteFor(SOLO_ROOM, ATHLETE_ID)).toBeNull();
    expect(soloAthleteFor(SOLO_ROOM, null)).toBeNull();
  });

  it('is null for a room of several athletes, which the server names no athlete for, and while the group loads', () => {
    expect(soloAthleteFor({ coach_user_id: COACH_ID }, COACH_ID)).toBeNull();
    expect(soloAthleteFor(null, COACH_ID)).toBeNull();
  });
});

describe('findWorkThread', () => {
  it("finds the coach's thread about the athlete through this group, and no other thread", () => {
    const otherGroup = conversation({ id: 'conv-work-other', subject_user_id: ATHLETE_ID, subject_group_id: 'group-other' });
    const otherAthlete = conversation({ id: 'conv-work-bob', subject_user_id: 'user-bob', subject_group_id: GROUP_ID });

    expect(findWorkThread([ROOM_THREAD, otherGroup, otherAthlete, WORK_THREAD], ATHLETE_ID, GROUP_ID)?.id).toBe(WORK_ID);
    expect(findWorkThread([ROOM_THREAD, otherGroup, otherAthlete], ATHLETE_ID, GROUP_ID)).toBeNull();
  });
});

describe('privateSendFailureText', () => {
  it('says a refused quote can no longer be used, whichever reason refused it', () => {
    for (const reason of ['quote_not_allowed', 'quote_not_found', 'quote_withheld', 'quote_not_athlete']) {
      expect(privateSendFailureText(turnRefusal(403, reason), { t, athleteName: 'Alice Runner' })).toBe(
        'This message can no longer be used for the plan.',
      );
    }
  });

  it('names an athlete who stopped sharing, from the turn or from opening the thread', () => {
    const stopped = "Alice Runner stopped sharing with you, so their plan can't be changed.";
    expect(privateSendFailureText(turnRefusal(403, 'coach_sharing_revoked'), { t, athleteName: 'Alice Runner' })).toBe(stopped);
    expect(privateSendFailureText(axiosRefusal(403, 'coach_sharing_revoked'), { t, athleteName: 'Alice Runner' })).toBe(stopped);
  });

  it('words every other failure as any failed turn is worded', () => {
    const gate = turnRefusal(403, 'not_group_coach');
    const broke = new TurnFailedError('stream broke', 'ResourceUnavailable');
    expect(privateSendFailureText(gate, { t, athleteName: 'Alice Runner' })).toBe(describeTurnFailure(gate, { t }));
    expect(privateSendFailureText(broke, { t, athleteName: 'Alice Runner' })).toBe(describeTurnFailure(broke, { t }));
  });
});

describe('usePrivateLane — who has one', () => {
  it('gives the athlete no switch and reads nothing: the private rows are never fetched outside the coach', async () => {
    const ctx = setup();
    const { result } = renderLane(ctx, { viewerId: ATHLETE_ID });

    expect(result.current.available).toBe(false);
    expect(result.current.audience).toBe('room');
    expect(result.current.conversationId).toBeNull();
    expect(result.current.rows).toEqual([]);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(ctx.api.getConversationMessages).not.toHaveBeenCalled();
    expect(ctx.store.getItem).not.toHaveBeenCalled();
    await expect(result.current.send('hello')).resolves.toEqual({ kind: 'skipped' });
    expect(ctx.api.sendTurn).not.toHaveBeenCalled();
  });

  it('gives the coach of a room of several athletes no switch', () => {
    const ctx = setup();
    const { result } = renderLane(ctx, { group: { id: GROUP_ID, coach_user_id: COACH_ID } });

    expect(result.current.available).toBe(false);
    expect(result.current.athlete).toBeNull();
  });

  it('opens no work thread just to look at the room, and draws nothing until one exists', async () => {
    const ctx = setup();
    const { result } = renderLane(ctx, { conversations: [ROOM_THREAD] });

    expect(result.current.available).toBe(true);
    expect(result.current.athlete).toEqual({ user_id: ATHLETE_ID, display_name: 'Alice Runner' });
    expect(result.current.conversationId).toBeNull();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(ctx.api.openAthleteThread).not.toHaveBeenCalled();
    expect(ctx.api.getConversationMessages).not.toHaveBeenCalled();
    expect(result.current.rows).toEqual([]);
  });

  it("reads the existing work thread's rows for the room to draw, tool plumbing dropped", async () => {
    const ctx = setup();
    const { result } = renderLane(ctx);

    await waitFor(() => expect(result.current.rows.map((row) => row.id)).toEqual(['w-1', 'w-3']));
    expect(result.current.conversationId).toBe(WORK_ID);
    expect(ctx.api.getConversationMessages).toHaveBeenCalledWith(WORK_ID);
    // The rows sit under the thread's own key, so a publish that refreshes the
    // plan card's thread refreshes the lane too.
    expect(ctx.client.getQueryData(['chat-messages', WORK_ID])).toMatchObject({ messages: WORK_ROWS });
  });
});

describe('usePrivateLane — the switch', () => {
  it('starts at Everyone, and remembers a move per viewer and room', async () => {
    const store = memoryStore();
    const ctx = setup(store);
    const { result } = renderLane(ctx);
    await waitFor(() => expect(store.getItem).toHaveBeenCalledWith(audienceStorageKey(COACH_ID, GROUP_ID)));
    expect(result.current.audience).toBe('room');

    act(() => result.current.toggleAudience());

    await waitFor(() => expect(result.current.audience).toBe('private'));
    expect(store.items.get('dravr.privateLane.audience:user-coach:group-alice')).toBe('private');

    act(() => result.current.setAudience('room'));
    await waitFor(() => expect(result.current.audience).toBe('room'));
    expect(store.items.get('dravr.privateLane.audience:user-coach:group-alice')).toBe('room');
  });

  it('comes back where the coach left it in that room, and nowhere else', async () => {
    const store = memoryStore({ [audienceStorageKey(COACH_ID, GROUP_ID)]: 'private' });
    const ctx = setup(store);
    const { result, rerender } = renderLane(ctx);

    await waitFor(() => expect(result.current.audience).toBe('private'));

    const otherRoom = { ...SOLO_ROOM, id: 'group-other' };
    rerender({ group: otherRoom, viewerId: COACH_ID, conversations: [] });
    await waitFor(() => expect(store.getItem).toHaveBeenCalledWith(audienceStorageKey(COACH_ID, 'group-other')));
    expect(result.current.audience).toBe('room');
  });

  it('starts at Everyone over a store that fails to read, and still moves over one that fails to write', async () => {
    const broken = {
      getItem: vi.fn(async () => {
        throw new Error('storage disabled');
      }),
      setItem: vi.fn(() => {
        throw new Error('quota exceeded');
      }),
    };
    const ctx = setup(broken);
    const { result } = renderLane(ctx);
    await waitFor(() => expect(broken.getItem).toHaveBeenCalled());
    expect(result.current.audience).toBe('room');

    act(() => result.current.toggleAudience());

    await waitFor(() => expect(result.current.audience).toBe('private'));
    expect(broken.setItem).toHaveBeenCalled();
  });

  it('keeps a move made before the stored position was read', async () => {
    let answer: (value: string | null) => void = () => undefined;
    const slow = {
      getItem: vi.fn(
        () =>
          new Promise<string | null>((resolve) => {
            answer = resolve;
          }),
      ),
      setItem: vi.fn(async () => undefined),
    };
    const ctx = setup(slow);
    const { result } = renderLane(ctx);
    await waitFor(() => expect(slow.getItem).toHaveBeenCalled());

    act(() => result.current.setAudience('private'));
    await waitFor(() => expect(result.current.audience).toBe('private'));
    // The store answers with what it held before the move.
    await act(async () => answer('room'));

    await waitFor(() => expect(ctx.client.isFetching()).toBe(0));
    expect(ctx.client.getQueryData(['chat', 'private-lane-audience', COACH_ID, GROUP_ID])).toBe('private');
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(result.current.audience).toBe('private');
  });
});

describe('usePrivateLane — Adjust plan from this', () => {
  it("quotes the athlete's room words and switches to the agent alone", async () => {
    const ctx = setup();
    const { result } = renderLane(ctx);
    const { athleteRow } = roomRows();
    expect(result.current.canAdjustFrom(athleteRow)).toBe(true);

    act(() => {
      expect(result.current.adjustFrom(athleteRow)).toBe(true);
    });

    await waitFor(() => expect(result.current.audience).toBe('private'));
    expect(result.current.quote).toEqual({
      entry_id: ENTRY_ID,
      author_user_id: ATHLETE_ID,
      author_display_name: 'Alice Runner',
      content: 'Legs felt heavy this morning.',
      created_at: '2026-10-28T07:12:00Z',
    });

    act(() => result.current.removeQuote());
    expect(result.current.quote).toBeNull();
  });

  it("refuses the agent's reply to the athlete, which is not their words", () => {
    const ctx = setup();
    const { result } = renderLane(ctx);
    const { replyRow } = roomRows();

    expect(result.current.canAdjustFrom(replyRow)).toBe(false);
    act(() => {
      expect(result.current.adjustFrom(replyRow)).toBe(false);
    });
    expect(result.current.quote).toBeNull();
    expect(result.current.audience).toBe('room');
  });

  it('offers nothing to adjust from to a viewer with no lane', () => {
    const ctx = setup();
    const { result } = renderLane(ctx, { viewerId: ATHLETE_ID });

    expect(result.current.canAdjustFrom(roomRows().athleteRow)).toBe(false);
  });
});

describe('usePrivateLane — the private send', () => {
  it('opens the work thread on the first private send, then sends the quoted question there and draws the turn', async () => {
    const ctx = setup();
    ctx.api.openAthleteThread.mockResolvedValue(WORK_THREAD);
    // What the thread holds once the turn is stored, for the re-read after it.
    const stored = envelope('Move Thursday to Saturday', true);
    ctx.api.getConversationMessages.mockResolvedValue({
      messages: [stored.user_message, stored.assistant.message],
      has_more: false,
      awaiting_reply: null,
    });
    const invalidate = vi.spyOn(ctx.client, 'invalidateQueries');
    // The turn is held open, so the row drawn while it runs can be read.
    let finish: () => void = () => undefined;
    ctx.api.sendTurn.mockImplementation(
      (_id, content, options) =>
        new Promise<void>((resolve) => {
          finish = () => {
            options?.onDone?.(envelope(content, true));
            resolve();
          };
        }),
    );
    const { result } = renderLane(ctx, { conversations: [ROOM_THREAD] });
    act(() => {
      result.current.adjustFrom(roomRows().athleteRow);
    });
    // The client's own onDone runs in the tick the lane wrote the turn's rows.
    let written: MessagesResponse | undefined;
    const onDone = vi.fn(() => {
      written = ctx.client.getQueryData<MessagesResponse>(['chat-messages', WORK_ID]);
    });

    let sent: ReturnType<typeof result.current.send> = Promise.resolve({ kind: 'skipped' });
    act(() => {
      sent = result.current.send('Move Thursday to Saturday', { onDone });
    });

    await waitFor(() => expect(ctx.api.sendTurn).toHaveBeenCalled());
    expect(ctx.api.openAthleteThread).toHaveBeenCalledWith(ATHLETE_ID, GROUP_ID);
    const [threadId, content, options] = ctx.api.sendTurn.mock.calls[0];
    expect(threadId).toBe(WORK_ID);
    expect(content).toBe('Move Thursday to Saturday');
    expect(options?.quoteEntryId).toBe(ENTRY_ID);
    await waitFor(() => expect(result.current.rows.map((row) => row.content)).toEqual(['Move Thursday to Saturday']));
    expect(result.current.isSending).toBe(true);
    expect(result.current.rows[0].quote?.entry_id).toBe(ENTRY_ID);
    expect(result.current.conversationId).toBe(WORK_ID);

    await act(async () => {
      finish();
      await sent;
    });

    await expect(sent).resolves.toMatchObject({ kind: 'sent', conversationId: WORK_ID });
    expect(onDone).toHaveBeenCalledTimes(1);
    // The pending question gave way to the server's, quote and all, and the reply carries its scenes.
    expect(written?.messages.map((row) => row.id)).toEqual(['w-10', 'w-11']);
    expect(written?.messages[0].quote?.content).toBe('Legs felt heavy this morning.');
    expect(written?.messages[1].scene_blocks).toBe('[{"kind":"plan"}]');
    expect(written?.awaiting_reply).toBeNull();
    expect(result.current.quote).toBeNull();
    expect(result.current.isSending).toBe(false);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['chat-conversations'] });
    // Once the send lets it, the lane reads back what the server kept.
    await waitFor(() => expect(ctx.api.getConversationMessages).toHaveBeenCalledWith(WORK_ID));
    expect(result.current.rows.map((row) => row.id)).toEqual(['w-10', 'w-11']);
  });

  it('sends to the existing work thread without opening it, and carries no quote when none is held', async () => {
    const ctx = setup();
    answers(ctx.api, (content) => envelope(content, false));
    const { result } = renderLane(ctx);
    await waitFor(() => expect(result.current.rows).toHaveLength(2));

    let outcome: Awaited<ReturnType<typeof result.current.send>> = { kind: 'skipped' };
    await act(async () => {
      outcome = await result.current.send('Ease Sunday off');
    });

    expect(outcome.kind).toBe('sent');
    expect(ctx.api.openAthleteThread).not.toHaveBeenCalled();
    expect(ctx.api.sendTurn.mock.calls[0][0]).toBe(WORK_ID);
    expect(ctx.api.sendTurn.mock.calls[0][2]).not.toHaveProperty('quoteEntryId');
  });

  it('brings an archived work thread back by opening it before the send', async () => {
    const ctx = setup();
    ctx.api.openAthleteThread.mockResolvedValue(WORK_THREAD);
    answers(ctx.api, (content) => envelope(content, false));
    const archived = { ...WORK_THREAD, archived_at: '2026-10-25T10:00:00Z' };
    const { result } = renderLane(ctx, { conversations: [ROOM_THREAD, archived] });

    await act(async () => {
      await result.current.send('Back to it');
    });

    expect(ctx.api.openAthleteThread).toHaveBeenCalledWith(ATHLETE_ID, GROUP_ID);
    expect(ctx.api.sendTurn.mock.calls[0][0]).toBe(WORK_ID);
  });

  it('takes the question back and drops the quote when the server refuses the quote', async () => {
    const ctx = setup();
    const onError = vi.fn();
    ctx.api.sendTurn.mockImplementation(async (_id, _content, options) => {
      options?.onError?.(turnRefusal(403, 'quote_withheld'));
    });
    const { result } = renderLane(ctx);
    await waitFor(() => expect(result.current.rows).toHaveLength(2));
    act(() => {
      result.current.adjustFrom(roomRows().athleteRow);
    });

    let outcome: Awaited<ReturnType<typeof result.current.send>> = { kind: 'skipped' };
    await act(async () => {
      outcome = await result.current.send('Move Thursday', { onError });
    });

    expect(outcome).toMatchObject({ kind: 'failed', conversationId: WORK_ID, refused: true });
    expect(onError).toHaveBeenCalledTimes(1);
    expect(result.current.quote).toBeNull();
    const cached = ctx.client.getQueryData<MessagesResponse>(['chat-messages', WORK_ID]);
    expect(cached?.messages.map((row) => row.id)).toEqual(['w-1', 'w-2', 'w-3']);
    expect(outcome.kind === 'failed' && privateSendFailureText(outcome.error, { t, athleteName: 'Alice Runner' })).toBe(
      'This message can no longer be used for the plan.',
    );
  });

  it('keeps the quote for the retry when the turn broke for another reason', async () => {
    const ctx = setup();
    ctx.api.sendTurn.mockImplementation(async (_id, _content, options) => {
      options?.onError?.(new TurnFailedError('stream broke', 'ResourceUnavailable'));
    });
    const { result } = renderLane(ctx);
    act(() => {
      result.current.adjustFrom(roomRows().athleteRow);
    });

    let outcome: Awaited<ReturnType<typeof result.current.send>> = { kind: 'skipped' };
    await act(async () => {
      outcome = await result.current.send('Move Thursday');
    });

    expect(outcome).toMatchObject({ kind: 'failed', refused: false });
    expect(result.current.quote?.entry_id).toBe(ENTRY_ID);
  });

  it('sends nothing when opening the work thread is refused, and says why through onError', async () => {
    const ctx = setup();
    const refusal = axiosRefusal(403, 'coach_sharing_revoked');
    ctx.api.openAthleteThread.mockRejectedValue(refusal);
    const onError = vi.fn();
    const { result } = renderLane(ctx, { conversations: [ROOM_THREAD] });

    let outcome: Awaited<ReturnType<typeof result.current.send>> = { kind: 'skipped' };
    await act(async () => {
      outcome = await result.current.send('Move Thursday', { onError });
    });

    expect(outcome).toEqual({ kind: 'failed', conversationId: null, error: refusal, refused: true });
    expect(onError).toHaveBeenCalledWith(refusal);
    expect(ctx.api.sendTurn).not.toHaveBeenCalled();
    expect(result.current.conversationId).toBeNull();
  });

  it('skips blank text, and a second send while one runs', async () => {
    const ctx = setup();
    let finish: () => void = () => undefined;
    ctx.api.sendTurn.mockImplementation(
      (_id, content, options) =>
        new Promise<void>((resolve) => {
          finish = () => {
            options?.onDone?.(envelope(content, false));
            resolve();
          };
        }),
    );
    const { result } = renderLane(ctx);

    await expect(result.current.send('   ')).resolves.toEqual({ kind: 'skipped' });

    let first: ReturnType<typeof result.current.send> = Promise.resolve({ kind: 'skipped' });
    act(() => {
      first = result.current.send('One');
    });
    await waitFor(() => expect(ctx.api.sendTurn).toHaveBeenCalledTimes(1));
    await expect(result.current.send('Two')).resolves.toEqual({ kind: 'skipped' });

    await act(async () => {
      finish();
      await first;
    });
    expect(ctx.api.sendTurn).toHaveBeenCalledTimes(1);
  });
});
