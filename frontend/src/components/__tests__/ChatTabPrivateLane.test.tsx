// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's private lane in a one-athlete room on the Groups tab — the switch, Adjust plan from this, the private send (carnet#1249)
// ABOUTME: Pins who sees the switch, that it starts at Everyone and stays put, where a private turn goes and what a refused quote leaves

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { TurnRequestError } from '@pierre/api-client';
import type { CoachingGroup, GroupTranscriptEntry, TurnCallbacks, TurnEnvelope } from '@pierre/shared-types';
import ChatTab from '../ChatTab';
import { ToastProvider } from '../ui';

const ROOM_CONVERSATION_ID = 'conv-room';
const WORK_CONVERSATION_ID = 'conv-work';
const GROUP_ID = 'group-7';
const COACH_ID = 'user-coach';
const ATHLETE_ID = 'user-camille';
const AGENT_ID = 'agent-dravr';
const ENTRY_ID = 'entry-camille-1';
const ATHLETE_WORDS = "Legs felt heavy in this morning's swim. Is Thursday's threshold run still a good idea?";
const QUESTION = "Move Thursday's threshold run to Saturday and keep Thursday easy.";

const api = vi.hoisted(() => ({
  getConversations: vi.fn(),
  getConversationMessages: vi.fn(),
  getConversationVerdicts: vi.fn(),
  listParticipants: vi.fn(),
  sendTurn: vi.fn(),
  openAthleteThread: vi.fn(),
  stopTurn: vi.fn(),
  getGroup: vi.fn(),
  readRoom: vi.fn(),
}));

// Only the network is stubbed: ChatTab, the thread, the composer and the
// shared private-lane hook run for real over it. The server half — what a
// work-thread turn, a quote and a publish do to the room — is pinned against
// the real routes by `private_lane_test`; the browser flow over mocked routes
// by `e2e/coach-plan-thread.spec.ts`.
vi.mock('../../services/api', () => ({
  notificationsApi: { getUnreadCount: async () => ({ unread_count: 0 }) },
  chatApi: {
    getConversations: (...a: unknown[]) => api.getConversations(...a),
    getConversationMessages: (...a: unknown[]) => api.getConversationMessages(...a),
    getConversationVerdicts: (...a: unknown[]) => api.getConversationVerdicts(...a),
    listParticipants: (...a: unknown[]) => api.listParticipants(...a),
    sendTurn: (...a: unknown[]) => api.sendTurn(...a),
    openAthleteThread: (...a: unknown[]) => api.openAthleteThread(...a),
    stopTurn: (...a: unknown[]) => api.stopTurn(...a),
    markConversationRead: vi.fn().mockResolvedValue(undefined),
    createConversation: vi.fn(),
    updateConversation: vi.fn(),
    deleteConversation: vi.fn(),
    markConversationUnread: vi.fn(),
  },
  coachesApi: { list: vi.fn().mockResolvedValue({ agents: [] }) },
  providersApi: {
    getProvidersStatus: vi.fn().mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] }),
  },
  groupsApi: {
    getGroup: (...a: unknown[]) => api.getGroup(...a),
    readRoom: (...a: unknown[]) => api.readRoom(...a),
  },
}));

vi.mock('../groups/GroupInfoPanel', () => ({ default: () => <div data-testid="group-info-panel" /> }));
vi.mock('../../services/analytics', () => ({ track: vi.fn() }));

const viewer = vi.hoisted(() => ({ id: 'user-coach' }));
vi.mock('../../hooks/useAuth', () => ({ useAuth: () => ({ token: 'test-token', user: viewer }) }));
vi.mock('../../hooks/useUsageStatus', () => ({
  useUsageStatus: () => ({
    data: undefined,
    isLoading: false,
    error: null,
    level: 'none',
    sendDisabled: false,
    message: '',
    resetsAt: '',
    triggerCounter: null,
    invalidate: vi.fn(),
    applyNotice: vi.fn(),
  }),
}));

/** The room: the signed-in coach, Camille and the agent. The server names Camille to the coach alone. */
function groupRecord(overrides: Partial<CoachingGroup> = {}): CoachingGroup {
  return {
    id: GROUP_ID,
    tenant_id: 'tenant-a',
    name: 'Camille Roy · 70.3 build',
    description: null,
    agent_id: AGENT_ID,
    agent_title: 'Dravr',
    agent_handle: 'dravr',
    owner_id: COACH_ID,
    coach_user_id: COACH_ID,
    coach_display_name: 'Philippe',
    peer_data_sharing: false,
    respond_mode: 'all',
    digest_mode: 'off',
    max_members: 10,
    is_active: true,
    created_at: '2026-10-01T00:00:00Z',
    updated_at: '2026-10-01T00:00:00Z',
    solo_athlete: { user_id: ATHLETE_ID, display_name: 'Camille Roy' },
    ...overrides,
  };
}

const roomConversation = {
  id: ROOM_CONVERSATION_ID,
  title: 'Camille Roy · 70.3 build',
  agent_id: AGENT_ID,
  group_id: GROUP_ID,
  group_name: 'Camille Roy · 70.3 build',
};

/** The coach's work thread about Camille, bound to her through this room (carnet#1070). */
const workConversation = {
  id: WORK_CONVERSATION_ID,
  title: 'Camille Roy',
  agent_id: AGENT_ID,
  group_id: null,
  subject_user_id: ATHLETE_ID,
  subject_group_id: GROUP_ID,
  subject_display_name: 'Camille Roy',
};

/** The room's transcript: Camille's question, and the coach's own answer drawn from their conversation. */
const roomEntries: GroupTranscriptEntry[] = [
  {
    id: ENTRY_ID,
    speaker: 'member',
    withheld: false,
    own: false,
    author_user_id: ATHLETE_ID,
    author_display_name: 'Camille Roy',
    content: ATHLETE_WORDS,
    message_id: null,
    created_at: '2026-10-29T07:12:00Z',
  },
  {
    id: 'entry-coach-1',
    speaker: 'member',
    withheld: false,
    own: true,
    author_user_id: COACH_ID,
    author_display_name: 'Philippe',
    content: "Agreed. I'll rework the week tonight.",
    message_id: 'msg-room-1',
    created_at: '2026-10-29T07:30:00Z',
  },
];

const roomMessages = [
  { id: 'msg-room-1', role: 'user', content: "Agreed. I'll rework the week tonight.", created_at: '2026-10-29T07:30:00Z' },
];

/** The work thread's rows: a quoted private question and the agent's reply. */
const workMessages = [
  {
    id: 'msg-work-1',
    conversation_id: WORK_CONVERSATION_ID,
    role: 'user',
    content: QUESTION,
    created_at: '2026-10-29T20:40:00Z',
    quote: {
      entry_id: ENTRY_ID,
      author_user_id: ATHLETE_ID,
      author_display_name: 'Camille Roy',
      content: ATHLETE_WORDS,
      created_at: '2026-10-29T07:12:00Z',
    },
  },
  {
    id: 'msg-work-2',
    conversation_id: WORK_CONVERSATION_ID,
    role: 'assistant',
    content: 'Done. Two days change; the other five stay as they are.',
    created_at: '2026-10-29T20:41:00Z',
  },
];

/** The turn the work thread answers a private question with. */
function privateTurn(): TurnEnvelope {
  return {
    turn_id: 'turn-work-1',
    user_message: { ...workMessages[0] },
    assistant: {
      message: { ...workMessages[1] },
      blocks: [{ type: 'prose', text: workMessages[1].content }],
      finish_reason: 'stop',
    },
    conversation_updated_at: '2026-10-29T20:41:00Z',
    telemetry: { model: 'm', provider_name: 'p', tool_calls_count: 0, tools_called: [], execution_time_ms: 900 },
  } as unknown as TurnEnvelope;
}

function renderRoom() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <ChatTab selectedConversation={ROOM_CONVERSATION_ID} onSelectConversation={vi.fn()} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

/** The composer's field, whatever its audience names it. */
function composer(): HTMLTextAreaElement {
  return document.querySelector('textarea[data-composer="true"]') as HTMLTextAreaElement;
}

describe("a coach's private lane in a one-athlete room", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    viewer.id = COACH_ID;
    api.getConversationVerdicts.mockResolvedValue({ verdicts: [] });
    api.listParticipants.mockResolvedValue([]);
    api.getGroup.mockResolvedValue(groupRecord());
    api.readRoom.mockResolvedValue(roomEntries);
    api.getConversations.mockResolvedValue({ conversations: [roomConversation], total: 1 });
    api.getConversationMessages.mockImplementation((id: string) =>
      Promise.resolve({ messages: id === WORK_CONVERSATION_ID ? workMessages : roomMessages }),
    );
    api.openAthleteThread.mockResolvedValue(workConversation);
  });

  it('gives the coach the switch, at Everyone, with the room placeholder', async () => {
    renderRoom();

    // Named by the label it shows, then what a press does; no pressed state on top.
    const toggle = await screen.findByRole('button', { name: 'Everyone. Switch to only Dravr' });
    expect(toggle).toHaveTextContent('Everyone');
    expect(toggle).not.toHaveAttribute('aria-pressed');
    expect(composer()).toHaveAttribute('placeholder', 'Message Camille Roy and Dravr. Type / for commands');
  });

  it('gives the athlete no switch and nothing to adjust from, even were a solo athlete named', async () => {
    viewer.id = ATHLETE_ID;
    renderRoom();

    expect(await screen.findByTestId('room-entry-member')).toBeInTheDocument();
    expect(screen.queryByTestId('composer-audience-switch')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Adjust plan from this' })).toBeNull();
    expect(composer()).toHaveAttribute('placeholder', 'Message Dravr...');
  });

  it('gives the coach of a room of two athletes no switch', async () => {
    api.getGroup.mockResolvedValue(groupRecord({ solo_athlete: undefined }));
    renderRoom();

    expect(await screen.findByTestId('room-entry-member')).toBeInTheDocument();
    expect(screen.queryByTestId('composer-audience-switch')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Adjust plan from this' })).toBeNull();
  });

  it('keeps the switch where the coach left it in this room', async () => {
    const user = userEvent.setup();
    const { unmount } = renderRoom();

    await user.click(await screen.findByTestId('composer-audience-switch'));
    expect(screen.getByTestId('composer-audience-switch')).toHaveAccessibleName('Only Dravr. Switch to everyone');
    expect(window.localStorage.getItem(`dravr.privateLane.audience:${COACH_ID}:${GROUP_ID}`)).toBe('private');
    unmount();

    renderRoom();
    await waitFor(() =>
      expect(screen.getByTestId('composer-audience-switch')).toHaveAccessibleName('Only Dravr. Switch to everyone'),
    );
    expect(composer()).toHaveAttribute('placeholder', "Ask Dravr to change Camille Roy's plan. Camille Roy won't see this.");
  });

  it('sends a room message to the room at Everyone', async () => {
    api.sendTurn.mockImplementation(async (_id: string, _content: string, options: TurnCallbacks) => {
      options.onError?.(new Error('offline'));
    });
    const user = userEvent.setup();
    renderRoom();

    await screen.findByTestId('composer-audience-switch');
    await user.type(composer(), 'See you Saturday');
    await user.click(screen.getByRole('button', { name: 'Send message' }));

    await waitFor(() => expect(api.sendTurn).toHaveBeenCalledTimes(1));
    expect(api.sendTurn.mock.calls[0][0]).toBe(ROOM_CONVERSATION_ID);
    expect(api.sendTurn.mock.calls[0][2]).not.toHaveProperty('quoteEntryId');
    expect(api.openAthleteThread).not.toHaveBeenCalled();
  });

  it("Adjust plan from this quotes Camille's message, turns the composer private, and the send reaches the work thread", async () => {
    api.sendTurn.mockImplementation(async (_id: string, _content: string, options: TurnCallbacks) => {
      options.onDone?.(privateTurn());
    });
    const user = userEvent.setup();
    renderRoom();

    const athleteRow = (await screen.findByTestId('room-entry-member')).closest('[data-testid="message-row"]') as HTMLElement;
    await user.click(within(athleteRow).getByRole('button', { name: 'Adjust plan from this' }));

    expect(screen.getByTestId('composer-quote')).toHaveTextContent("From Camille Roy's message");
    expect(screen.getByTestId('composer-audience-switch')).toHaveAccessibleName('Only Dravr. Switch to everyone');
    expect(composer()).toHaveFocus();

    await user.type(composer(), QUESTION);
    await user.click(screen.getByRole('button', { name: 'Send message' }));

    // No work thread yet: the first private send opens it through this room.
    await waitFor(() => expect(api.sendTurn).toHaveBeenCalledTimes(1));
    expect(api.openAthleteThread).toHaveBeenCalledWith(ATHLETE_ID, GROUP_ID);
    const [conversationId, content, options] = api.sendTurn.mock.calls[0];
    expect(conversationId).toBe(WORK_CONVERSATION_ID);
    expect(content).toBe(QUESTION);
    expect(options).toMatchObject({ quoteEntryId: ENTRY_ID });

    // The turn lands in the room as private rows, under the line, the reply tagged.
    expect(await screen.findByTestId('private-lane-divider')).toHaveTextContent('Only you and Dravr see what follows');
    expect(await screen.findByTestId('private-reply-tag')).toHaveTextContent('Only you');
    expect(screen.queryByTestId('composer-quote')).toBeNull();
  });

  it("draws the work thread's rows in the room: dashed, after the room's own, under one divider", async () => {
    api.getConversations.mockResolvedValue({ conversations: [roomConversation, workConversation], total: 2 });
    renderRoom();

    const reply = await screen.findByText('Done. Two days change; the other five stay as they are.');
    const rows = screen.getAllByTestId('message-row');
    const question = rows.find((row) => row.textContent?.includes(QUESTION)) as HTMLElement;
    expect(question).toHaveAttribute('data-audience', 'private');
    expect(question.querySelector('.chat-bubble-private')).not.toBeNull();
    expect(within(question).getByTestId('message-quote')).toHaveTextContent("From Camille Roy's message");
    const replyRow = reply.closest('[data-testid="message-row"]') as HTMLElement;
    expect(replyRow).toHaveAttribute('aria-label', 'Only you and Dravr can see this message');
    expect(within(replyRow).getByTestId('private-reply-tag')).toHaveTextContent('Only you');

    // One divider, between the room's rows and the private stretch.
    const dividers = screen.getAllByTestId('private-lane-divider');
    expect(dividers).toHaveLength(1);
    const roomRow = rows.find((row) => row.textContent?.includes("I'll rework the week tonight")) as HTMLElement;
    expect(roomRow).toHaveAttribute('data-audience', 'room');
    expect(roomRow.compareDocumentPosition(dividers[0]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(dividers[0].compareDocumentPosition(question) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // A private reply is copied, never rated against the room's conversation.
    expect(within(replyRow).queryByTitle('Good response')).toBeNull();
  });

  it('puts the words back and says why when the server refuses the quote', async () => {
    api.sendTurn.mockImplementation(async (_id: string, _content: string, options: TurnCallbacks) => {
      options.onError?.(
        new TurnRequestError('quote withheld', 403, {
          code: 'PermissionDenied',
          details: { reason: 'quote_withheld' },
        }),
      );
    });
    // The work thread the send opened holds nothing: the server stored no turn.
    api.getConversationMessages.mockImplementation((id: string) =>
      Promise.resolve({ messages: id === WORK_CONVERSATION_ID ? [] : roomMessages }),
    );
    const user = userEvent.setup();
    renderRoom();

    await user.click(await screen.findByRole('button', { name: 'Adjust plan from this' }));
    await user.type(composer(), QUESTION);
    await user.click(screen.getByRole('button', { name: 'Send message' }));

    expect(await screen.findByTestId('turn-error')).toHaveTextContent(
      'This message can no longer be used for the plan.',
    );
    await waitFor(() => expect(composer().value).toBe(QUESTION));
    // The refused quote is let go; the composer stays private for the retry.
    expect(screen.queryByTestId('composer-quote')).toBeNull();
    expect(screen.getByTestId('composer-audience-switch')).toHaveAccessibleName('Only Dravr. Switch to everyone');
    // Nothing reached the room.
    expect(api.sendTurn.mock.calls.every(([id]) => id === WORK_CONVERSATION_ID)).toBe(true);
    expect(screen.queryByTestId('private-lane-divider')).toBeNull();
  });
});
