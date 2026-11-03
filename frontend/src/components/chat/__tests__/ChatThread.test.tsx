// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The thread every web chat surface shares, driven through its own hook with no host chrome around it
// ABOUTME: Pins a typed send, a stop reaching the running turn, a send with no thread opening one the host's way, the composer's focus rule, and that only an operator reads which model wrote a reply

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import type {
  CoachingGroup,
  DraftState,
  Message,
  SubjectPlanResponse,
  UserRole,
  WorkoutPlan,
} from '@pierre/shared-types';
import ChatThread from '../ChatThread';
import { useChatThread, type OpenThread } from '../useChatThread';
import type { PendingComposerAction } from '../types';
import { ToastProvider } from '../../ui';

const CONVERSATION_ID = 'conv-1';

const getConversations = vi.fn();
const getConversationMessages = vi.fn();
const sendTurn = vi.fn();
const stopTurn = vi.fn();
const getSubjectPlan = vi.fn();
const listParticipants = vi.fn();
const getGroup = vi.fn();
const readRoom = vi.fn();

vi.mock('../../../services/api', () => ({
  chatApi: {
    getConversations: (...a: unknown[]) => getConversations(...a),
    getConversationMessages: (...a: unknown[]) => getConversationMessages(...a),
    getConversationVerdicts: vi.fn().mockResolvedValue({ verdicts: [] }),
    sendTurn: (...a: unknown[]) => sendTurn(...a),
    stopTurn: (...a: unknown[]) => stopTurn(...a),
    markConversationRead: vi.fn().mockResolvedValue(undefined),
    getSubjectPlan: (...a: unknown[]) => getSubjectPlan(...a),
    listParticipants: (...a: unknown[]) => listParticipants(...a),
  },
  coachesApi: { list: vi.fn().mockResolvedValue({ agents: [] }) },
  providersApi: {
    getProvidersStatus: vi.fn().mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] }),
  },
  groupsApi: {
    getGroup: (...a: unknown[]) => getGroup(...a),
    readRoom: (...a: unknown[]) => readRoom(...a),
  },
}));

vi.mock('../../../services/analytics', () => ({ track: vi.fn() }));
// The viewer's role decides whether a reply names its model; each test sets it.
const viewer = vi.hoisted(() => ({ role: 'user' as UserRole }));
vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { id: 'viewer-1', role: viewer.role } }),
}));
vi.mock('../../../hooks/useUsageStatus', () => ({
  useUsageStatus: () => ({
    level: 'none',
    sendDisabled: false,
    text: null,
    invalidate: vi.fn(),
    applyNotice: vi.fn(),
  }),
}));

const EARLIER: Message[] = [
  { id: 'm1', role: 'user', content: 'Hi', created_at: '2026-09-30T08:00:00Z' },
  { id: 'm2', role: 'assistant', content: 'Hi, ready when you are.', created_at: '2026-09-30T08:00:04Z' },
];

type TurnOptions = { onDone?: (turn: unknown) => void };

/** A turn that stays in flight until the test ends it. */
function heldTurn() {
  let finish: (turn: unknown) => void = () => undefined;
  sendTurn.mockImplementationOnce(
    (_id: string, _content: string, options: TurnOptions) =>
      new Promise<void>(resolve => {
        finish = turn => {
          options.onDone?.(turn);
          resolve();
        };
      }),
  );
  return { finish: (turn: unknown) => finish(turn) };
}

function turnEnding(content: string) {
  return {
    assistant: {
      message: { id: 'm4', role: 'assistant', content, created_at: '2026-09-30T08:01:00Z' },
      blocks: [{ type: 'prose', text: content }],
      finish_reason: 'stop',
    },
    telemetry: { model: 'test-model', execution_time_ms: 10 },
  };
}

/** The smallest host: it owns the thread and draws nothing but it. */
function Host(props: {
  initial: string | null;
  openThread: OpenThread;
  focusComposerOnOpen?: boolean;
  action?: PendingComposerAction;
}) {
  const [conversationId, setConversationId] = useState<string | null>(props.initial);
  const [action, setAction] = useState<PendingComposerAction | null>(props.action ?? null);
  const thread = useChatThread({
    conversationId,
    onSelectConversation: setConversationId,
    openThread: props.openThread,
    pendingComposerAction: action,
    onPendingComposerActionConsumed: () => setAction(null),
  });
  return (
    <div className="flex h-full flex-col">
      <p data-testid="host-conversation">{conversationId ?? 'none'}</p>
      <p data-testid="host-subtitle">{thread.subtitle ?? ''}</p>
      <ChatThread
        thread={thread}
        conversationTitle={thread.title}
        followOnOpen
        focusComposerOnOpen={props.focusComposerOnOpen ?? false}
      />
    </div>
  );
}

function renderThread(props: Parameters<typeof Host>[0]) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <Host {...props} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

function composer(): HTMLTextAreaElement {
  const field = document.querySelector<HTMLTextAreaElement>('textarea[data-composer="true"]');
  if (!field) throw new Error('no composer on screen');
  return field;
}

describe('ChatThread', () => {
  const openThread = vi.fn<OpenThread>();

  beforeEach(() => {
    vi.clearAllMocks();
    viewer.role = 'user';
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Half marathon', agent_id: null }],
      total: 1,
    });
    getConversationMessages.mockResolvedValue({ messages: EARLIER });
    openThread.mockResolvedValue({ id: 'conv-new' });
  });

  it('sends the typed line into the open thread, clears the composer and draws the reply', async () => {
    const turn = heldTurn();
    const user = userEvent.setup();
    renderThread({ initial: CONVERSATION_ID, openThread });

    await screen.findByText('Hi, ready when you are.');
    await user.type(composer(), 'How does my week look?');
    await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));

    expect(sendTurn).toHaveBeenCalledTimes(1);
    expect(sendTurn.mock.calls[0][0]).toBe(CONVERSATION_ID);
    expect(sendTurn.mock.calls[0][1]).toBe('How does my week look?');
    expect(composer()).toHaveValue('');
    expect(await screen.findByText('How does my week look?')).toBeInTheDocument();

    await act(async () => {
      turn.finish(turnEnding('Your week: 42 km, all easy.'));
    });
    expect(await screen.findByText('Your week: 42 km, all easy.')).toBeInTheDocument();
    expect(openThread).not.toHaveBeenCalled();
  });

  it('never shows an athlete which model wrote a reply, live or reloaded', async () => {
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Half marathon', agent_id: null, model: 'stored-model' }],
      total: 1,
    });
    const turn = heldTurn();
    const user = userEvent.setup();
    renderThread({ initial: CONVERSATION_ID, openThread });

    await screen.findByText('Hi, ready when you are.');
    await user.type(composer(), 'How does my week look?');
    await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));
    await act(async () => {
      turn.finish(turnEnding('Your week: 42 km, all easy.'));
    });
    await screen.findByText('Your week: 42 km, all easy.');

    expect(screen.queryByText(/test-model/), 'the live reply names no model').not.toBeInTheDocument();
    expect(screen.queryByText(/stored-model/), 'the reloaded reply names no model').not.toBeInTheDocument();
  });

  it('shows an operator the model under each reply, live and reloaded', async () => {
    viewer.role = 'admin';
    getConversations.mockResolvedValue({
      conversations: [{ id: CONVERSATION_ID, title: 'Half marathon', agent_id: null, model: 'stored-model' }],
      total: 1,
    });
    const turn = heldTurn();
    const user = userEvent.setup();
    renderThread({ initial: CONVERSATION_ID, openThread });

    expect(await screen.findByText(/stored-model/), 'the reloaded reply names its model').toBeInTheDocument();
    await user.type(composer(), 'How does my week look?');
    await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));
    await act(async () => {
      turn.finish(turnEnding('Your week: 42 km, all easy.'));
    });
    await screen.findByText('Your week: 42 km, all easy.');

    expect(screen.getByText(/test-model/), 'the live reply names its model').toBeInTheDocument();
  });

  it('asks the server to stop the running turn from the composer’s stop button', async () => {
    const turn = heldTurn();
    stopTurn.mockResolvedValue(true);
    const user = userEvent.setup();
    renderThread({ initial: CONVERSATION_ID, openThread });

    await screen.findByText('Hi, ready when you are.');
    await user.type(composer(), 'How does my week look?');
    await user.click(screen.getByRole('button', { name: i18n.t('chat.sendMessageAria') }));

    await user.click(await screen.findByRole('button', { name: i18n.t('chat.stopTurnAria') }));
    expect(stopTurn).toHaveBeenCalledWith(CONVERSATION_ID);
    await waitFor(() => expect(screen.getByTestId('stop-turn-button')).toBeDisabled());

    await act(async () => {
      turn.finish(turnEnding('Reply stopped.'));
    });
    expect(await screen.findByRole('button', { name: i18n.t('chat.sendMessageAria') })).toBeInTheDocument();
  });

  it('opens a thread the host’s way for a send with none open, then sends into it', async () => {
    sendTurn.mockResolvedValue(undefined);
    renderThread({
      initial: null,
      openThread,
      action: { kind: 'send', text: 'uc:1:about_me_more', display: 'Finish my profile' },
    });

    await waitFor(() => expect(sendTurn).toHaveBeenCalledTimes(1));
    // The athlete speaks first, so the thread's agent posts no welcome over the answer.
    expect(openThread).toHaveBeenCalledWith({ athlete_speaks_first: true });
    expect(screen.getByTestId('host-conversation')).toHaveTextContent('conv-new');
    expect(sendTurn.mock.calls[0][0]).toBe('conv-new');
    expect(sendTurn.mock.calls[0][1]).toBe('uc:1:about_me_more');
    expect(await screen.findByText('Finish my profile')).toBeInTheDocument();
  });

  it('leaves focus where it was unless the host asks the composer to take it', async () => {
    const first = renderThread({ initial: CONVERSATION_ID, openThread, focusComposerOnOpen: false });
    await screen.findByText('Hi, ready when you are.');
    expect(composer()).not.toHaveFocus();
    first.unmount();

    renderThread({ initial: CONVERSATION_ID, openThread, focusComposerOnOpen: true });
    await screen.findByText('Hi, ready when you are.');
    expect(composer()).toHaveFocus();
  });
});

describe("ChatThread in a coach's own threads", () => {
  const openThread = vi.fn<OpenThread>();
  const PLAN_ID = 'conv-plan-alice';
  const ROOM_ID = 'conv-room';
  const GROUP_ID = 'group-tri';
  const ALICE = { user_id: 'user-alice', display_name: 'Alice Runner' };
  const PLAN: WorkoutPlan = {
    goal_race: { name: 'Harricana 65K', date: '2027-09-11', discipline: 'trail', priority: 'A' },
    phases: [],
    weeks: [],
    weeks_deferred: 0,
  };
  const DRAFT: DraftState = { draft_id: 'draft-7', updated_at: '2026-10-16T14:05:00Z', conflicts: [], season_relaid: false };

  function subjectPlan(draft: DraftState | null): SubjectPlanResponse {
    return { athlete: ALICE, today: '2026-10-16', live: null, draft: draft ? PLAN : null, draft_state: draft };
  }

  function group(coachId: string): CoachingGroup {
    return {
      id: GROUP_ID,
      tenant_id: 'tenant-a',
      name: 'Tri — longue distance',
      description: null,
      agent_id: null,
      agent_title: 'Dravr',
      agent_handle: 'dravr',
      owner_id: coachId,
      coach_user_id: coachId,
      coach_display_name: 'Philippe',
      peer_data_sharing: false,
      respond_mode: 'all',
      digest_mode: 'off',
      max_members: 10,
      is_active: true,
      created_at: '2026-10-01T00:00:00Z',
      updated_at: '2026-10-01T00:00:00Z',
      solo_athlete: null,
    };
  }

  beforeEach(() => {
    vi.clearAllMocks();
    viewer.role = 'user';
    getConversations.mockResolvedValue({
      conversations: [
        {
          id: PLAN_ID,
          title: "Alice Runner's plan",
          agent_id: null,
          subject_user_id: ALICE.user_id,
          subject_display_name: ALICE.display_name,
        },
        { id: ROOM_ID, title: 'Tri — longue distance', agent_id: null, group_id: GROUP_ID, group_name: 'Tri — longue distance' },
      ],
      total: 2,
    });
    getConversationMessages.mockResolvedValue({ messages: [] });
    listParticipants.mockResolvedValue([
      { user_id: 'viewer-1', role: 'owner', added_by: 'viewer-1', added_at: '2026-10-16T12:00:00Z' },
    ]);
    getSubjectPlan.mockResolvedValue(subjectPlan(null));
    getGroup.mockResolvedValue(group('viewer-1'));
    readRoom.mockResolvedValue([]);
  });

  it("says an empty plan thread is the coach's, names the agent in the composer, and starts the plan in one tap", async () => {
    heldTurn();
    const user = userEvent.setup();
    renderThread({ initial: PLAN_ID, openThread });

    expect(await screen.findByTestId('chat-plan-thread-empty')).toHaveTextContent(
      i18n.t('humanCoach.planThread.empty', { name: 'Alice Runner' }),
    );
    expect(composer()).toHaveAttribute('placeholder', 'Write to the agent about Alice Runner’s plan…');
    expect(composer()).toHaveAccessibleName('Write to the agent about Alice Runner’s plan…');

    await user.click(await screen.findByRole('button', { name: 'Draft Alice Runner’s plan' }));
    expect(sendTurn).toHaveBeenCalledTimes(1);
    expect(sendTurn.mock.calls[0][0]).toBe(PLAN_ID);
    expect(sendTurn.mock.calls[0][1]).toBe(i18n.t('humanCoach.planThread.starterMessage', { name: 'Alice Runner' }));
    expect(screen.queryByTestId('chat-plan-thread-starter')).not.toBeInTheDocument();
  });

  it('draws the open draft with its controls in an empty plan thread, with no start beside it', async () => {
    getSubjectPlan.mockResolvedValue(subjectPlan(DRAFT));
    renderThread({ initial: PLAN_ID, openThread });

    const card = await screen.findByTestId('chat-plan-thread-open-draft');
    expect(card).toHaveTextContent('Harricana 65K');
    expect(await screen.findByRole('button', { name: 'Publish to Alice Runner' })).toBeEnabled();
    expect(screen.getByTestId('chat-plan-thread-empty')).toBeInTheDocument();
    expect(screen.queryByTestId('chat-plan-thread-starter')).not.toBeInTheDocument();
  });

  it('draws the open draft under a thread whose rows never carried it', async () => {
    getSubjectPlan.mockResolvedValue(subjectPlan(DRAFT));
    getConversationMessages.mockResolvedValue({ messages: EARLIER });
    renderThread({ initial: PLAN_ID, openThread });

    await screen.findByText('Hi, ready when you are.');
    expect(await screen.findByTestId('chat-plan-thread-open-draft')).toHaveTextContent('Harricana 65K');
    expect(screen.queryByTestId('chat-plan-thread-empty')).not.toBeInTheDocument();
  });

  it('draws no second card when a row read back already shows the open draft', async () => {
    getSubjectPlan.mockResolvedValue(subjectPlan(DRAFT));
    getConversationMessages.mockResolvedValue({
      messages: [
        {
          id: 'm-draft',
          role: 'assistant',
          content: 'Draft saved.',
          created_at: '2026-10-16T14:05:01Z',
          scene_blocks: JSON.stringify([{ kind: 'workout_plan', plan: PLAN, subject: ALICE, draft: DRAFT }]),
        },
      ],
    });
    renderThread({ initial: PLAN_ID, openThread });

    await screen.findByText('Draft saved.');
    await waitFor(() => expect(getSubjectPlan).toHaveBeenCalledWith(PLAN_ID));
    expect(await screen.findByRole('button', { name: 'Publish to Alice Runner' })).toBeEnabled();
    expect(screen.getAllByText('Harricana 65K')).toHaveLength(1);
    expect(screen.queryByTestId('chat-plan-thread-open-draft')).not.toBeInTheDocument();
  });

  it("says who reads the coach's empty room, and the composer writes to the group", async () => {
    renderThread({ initial: ROOM_ID, openThread });

    expect(await screen.findByTestId('chat-coach-room-empty')).toHaveTextContent(
      i18n.t('humanCoach.groupRoom.empty', { group: 'Tri — longue distance' }),
    );
    await waitFor(() => expect(composer()).toHaveAttribute('placeholder', 'Write to Tri — longue distance…'));
    // The room is called after its group: the line under it says what it is.
    expect(screen.getByTestId('host-subtitle')).toHaveTextContent('Group room');
    expect(getSubjectPlan).not.toHaveBeenCalled();
  });

  it("keeps a member's own words in the room they did not coach", async () => {
    getGroup.mockResolvedValue(group('coach-other'));
    renderThread({ initial: ROOM_ID, openThread });

    await waitFor(() => expect(getGroup).toHaveBeenCalledWith(GROUP_ID));
    await waitFor(() => expect(readRoom).toHaveBeenCalled());
    expect(screen.queryByTestId('chat-coach-room-empty')).not.toBeInTheDocument();
    expect(composer()).toHaveAttribute('placeholder', i18n.t('chat.messageDravrPlaceholder'));
  });
});
