// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the draft controls a coach's plan card carries — owner-only, Publish, a refused publish's conflict, Discard
// ABOUTME: Drives a workout_plan block through MessageItem, and a private row in a room through MessageList (carnet#1249)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { DraftState, ReplyBlock, SubjectPlanResponse, WorkoutPlan } from '@pierre/shared-types';
import MessageItem from '../MessageItem';
import MessageList from '../MessageList';
import { ToastProvider } from '../../ui';
import { QUERY_KEYS } from '../../../constants/queryKeys';
import type { Message } from '../types';

const CONVERSATION_ID = 'conv-plan-alice';
const COACH_ID = 'user-coach';
const ATHLETE_ID = 'user-alice';

const api = vi.hoisted(() => ({
  listParticipants: vi.fn(),
  getSubjectPlan: vi.fn(),
  publishSubjectPlan: vi.fn(),
  discardSubjectPlanDraft: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  chatApi: {
    listParticipants: (...a: unknown[]) => api.listParticipants(...a),
    getSubjectPlan: (...a: unknown[]) => api.getSubjectPlan(...a),
    publishSubjectPlan: (...a: unknown[]) => api.publishSubjectPlan(...a),
    discardSubjectPlanDraft: (...a: unknown[]) => api.discardSubjectPlanDraft(...a),
  },
}));

const viewer = vi.hoisted(() => ({ id: 'user-coach' }));
vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: viewer }),
}));

const PLAN: WorkoutPlan = {
  goal_race: { name: 'Harricana 65K', date: '2027-09-11', discipline: 'trail', priority: 'A' },
  phases: [],
  weeks: [],
  weeks_deferred: 0,
};

const DRAFT: DraftState = {
  draft_id: 'draft-7',
  updated_at: '2026-10-16T14:05:00Z',
  conflicts: [],
  season_relaid: false,
};

function subjectPlan(draftState: DraftState | null): SubjectPlanResponse {
  return {
    athlete: { user_id: ATHLETE_ID, display_name: 'Alice Runner' },
    today: '2026-10-16',
    live: PLAN,
    draft: draftState ? PLAN : null,
    draft_state: draftState,
  };
}

function reply(): Message {
  return { id: 'msg-1', role: 'assistant', content: 'Draft saved.', created_at: '2026-10-16T14:05:01Z' };
}

const DRAFT_BLOCKS: ReplyBlock[] = [
  { type: 'prose', text: 'Draft saved.' },
  {
    type: 'workout_plan',
    plan: PLAN,
    subject: { user_id: ATHLETE_ID, display_name: 'Alice Runner' },
    draft: DRAFT,
  },
];

function renderCard(conversationId: string | null = CONVERSATION_ID, blocks: ReplyBlock[] = DRAFT_BLOCKS) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <MessageItem conversationId={conversationId} message={reply()} blocks={blocks} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe("a coach's draft on the plan card", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    viewer.id = COACH_ID;
    api.listParticipants.mockResolvedValue([
      { user_id: COACH_ID, role: 'owner', added_by: COACH_ID, added_at: '2026-10-16T12:00:00Z' },
      { user_id: ATHLETE_ID, role: 'member', added_by: COACH_ID, added_at: '2026-10-16T12:01:00Z' },
    ]);
    api.getSubjectPlan.mockResolvedValue(subjectPlan(DRAFT));
  });

  it('shows the owner the badge, Publish and Discard', async () => {
    renderCard();

    expect(await screen.findByTestId('plan-draft-badge')).toHaveTextContent("Draft · Alice Runner can't see it yet");
    expect(screen.getByRole('button', { name: 'Publish to Alice Runner' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Discard draft' })).toBeEnabled();
    expect(screen.getByText('Harricana 65K')).toBeInTheDocument();
  });

  it('shows the athlete the plan and none of the controls, and never reads the draft for them', async () => {
    viewer.id = ATHLETE_ID;
    renderCard();

    expect(screen.getByText('Harricana 65K')).toBeInTheDocument();
    await waitFor(() => expect(api.listParticipants).toHaveBeenCalledWith(CONVERSATION_ID));
    expect(screen.queryByTestId('plan-draft-controls')).toBeNull();
    expect(screen.queryByRole('button', { name: /Publish/ })).toBeNull();
    expect(api.getSubjectPlan).not.toHaveBeenCalled();
  });

  it('draws no controls for a plan card outside a thread', () => {
    renderCard(null);

    expect(screen.getByText('Harricana 65K')).toBeInTheDocument();
    expect(api.listParticipants).not.toHaveBeenCalled();
    expect(screen.queryByTestId('plan-draft-controls')).toBeNull();
  });

  it('publishes and says the athlete was told', async () => {
    api.publishSubjectPlan.mockResolvedValue({
      status: 'published',
      published: { plan_id: 'plan-2', superseded_plan_id: 'plan-1', athlete_notified: true },
    });
    const user = userEvent.setup();
    renderCard();

    await user.click(await screen.findByRole('button', { name: 'Publish to Alice Runner' }));

    expect(await screen.findByTestId('plan-draft-published')).toHaveTextContent(
      'Published. Alice Runner has been notified.',
    );
    expect(api.publishSubjectPlan).toHaveBeenCalledWith(CONVERSATION_ID, { overwrite_athlete_changes: false });
    expect(screen.queryByRole('button', { name: 'Discard draft' })).toBeNull();
  });

  it('shows what a refused publish collided with, and publishes anyway on request', async () => {
    api.publishSubjectPlan.mockResolvedValueOnce({
      status: 'conflict',
      conflict: {
        error_code: 'plan_publish_conflict',
        conflicts: [
          { week_start: '2026-10-19', reason: 'athlete_changed_week' },
          { week_start: '2026-10-26', reason: 'athlete_changed_week' },
        ],
        season_relaid: true,
      },
    });
    api.publishSubjectPlan.mockResolvedValueOnce({
      status: 'published',
      published: { plan_id: 'plan-2', superseded_plan_id: 'plan-1', athlete_notified: true },
    });
    const user = userEvent.setup();
    renderCard();

    await user.click(await screen.findByRole('button', { name: 'Publish to Alice Runner' }));

    const conflict = await screen.findByTestId('plan-draft-conflict');
    expect(conflict).toHaveTextContent("Alice Runner's plan changed since this draft began");
    expect(conflict).toHaveTextContent(
      'Weeks changed in both the draft and the live plan: 2026-10-19 and 2026-10-26',
    );
    expect(conflict).toHaveTextContent("Alice Runner's season was rebuilt after this draft began");

    await user.click(screen.getByRole('button', { name: 'Publish anyway' }));

    expect(await screen.findByTestId('plan-draft-published')).toBeInTheDocument();
    expect(api.publishSubjectPlan).toHaveBeenLastCalledWith(CONVERSATION_ID, { overwrite_athlete_changes: true });
  });

  it('says the athlete stopped sharing when the publish is refused for it', async () => {
    api.publishSubjectPlan.mockRejectedValue(
      Object.assign(new Error('Request failed with status code 403'), {
        response: { status: 403, data: { error_code: 'coach_sharing_revoked' } },
      }),
    );
    const user = userEvent.setup();
    renderCard();

    await user.click(await screen.findByRole('button', { name: 'Publish to Alice Runner' }));

    expect(await screen.findByTestId('plan-draft-consent-revoked')).toHaveTextContent(
      "Alice Runner stopped sharing with you, so their plan can't be changed.",
    );
  });

  it('asks before discarding, then discards and takes the controls away', async () => {
    api.discardSubjectPlanDraft.mockResolvedValue(undefined);
    const user = userEvent.setup();
    renderCard();

    await user.click(await screen.findByRole('button', { name: 'Discard draft' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent("Discard this draft? Alice Runner's live plan stays as it is.");
    expect(api.discardSubjectPlanDraft).not.toHaveBeenCalled();

    api.getSubjectPlan.mockResolvedValue(subjectPlan(null));
    await user.click(within(dialog).getByRole('button', { name: 'Discard draft' }));

    await waitFor(() => expect(api.discardSubjectPlanDraft).toHaveBeenCalledWith(CONVERSATION_ID));
    await waitFor(() => expect(screen.queryByTestId('plan-draft-controls')).toBeNull());
  });
});

// carnet#1249: in a one-athlete room the coach's private rows are drawn
// inline, and a draft card on one acts on the coach's work thread about the
// athlete — never on the room's conversation the transcript belongs to.
describe('a draft card on a private row in the room', () => {
  const ROOM_CONVERSATION_ID = 'conv-room';
  const GROUP_ID = 'group-1';

  /** The private reply that saved the draft, as the work thread stores it. */
  const privateDraftReply: Message = {
    id: 'msg-work-2',
    role: 'assistant',
    content: 'Done. Two days change; the other five stay as they are.',
    created_at: '2026-10-29T20:41:00Z',
    audience: 'private',
    scene_blocks: JSON.stringify([
      {
        kind: 'workout_plan',
        source_tool: 'save_training_plan',
        plan: PLAN,
        subject: { user_id: ATHLETE_ID, display_name: 'Alice Runner' },
        draft: DRAFT,
      },
    ]),
  };

  function renderRoom(queryClient: QueryClient) {
    return render(
      <QueryClientProvider client={queryClient}>
        <ToastProvider>
          <MessageList
            conversationId={ROOM_CONVERSATION_ID}
            messages={[privateDraftReply]}
            messageMetadata={new Map()}
            messageFeedback={new Map()}
            messageFeedbackComment={new Map()}
            assistantLabel="Dravr"
            isLoading={false}
            isStreaming={false}
            streamingContent=""
            errorMessage={null}
            oauthNotification={null}
            onDismissError={vi.fn()}
            onDismissOAuthNotification={vi.fn()}
            onCopyMessage={vi.fn()}
            onShareMessage={vi.fn()}
            onThumbsUp={vi.fn()}
            onThumbsDown={vi.fn()}
            onSubmitFeedbackReason={vi.fn()}
            onRetryMessage={vi.fn()}
            privateLane={{ conversationId: CONVERSATION_ID, canAdjustFrom: () => false, onAdjustFrom: vi.fn() }}
          />
        </ToastProvider>
      </QueryClientProvider>,
    );
  }

  beforeEach(() => {
    vi.clearAllMocks();
    viewer.id = COACH_ID;
    api.listParticipants.mockResolvedValue([
      { user_id: COACH_ID, role: 'owner', added_by: COACH_ID, added_at: '2026-10-16T12:00:00Z' },
    ]);
    api.getSubjectPlan.mockResolvedValue(subjectPlan(DRAFT));
    api.publishSubjectPlan.mockResolvedValue({
      status: 'published',
      published: { plan_id: 'plan-2', superseded_plan_id: 'plan-1', athlete_notified: true, posted_in_room: true },
    });
  });

  it('acts on the work thread, publishes there, and reads the room again for the post', async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    // The work thread, bound to the room it was opened from, and a room read on screen.
    queryClient.setQueryData(QUERY_KEYS.chat.conversations(), {
      pages: [
        {
          conversations: [
            { id: CONVERSATION_ID, title: 'Alice Runner', subject_user_id: ATHLETE_ID, subject_group_id: GROUP_ID },
          ],
          total: 1,
        },
      ],
      pageParams: [0],
    });
    const roomKey = [...QUERY_KEYS.groups.room(GROUP_ID), null];
    queryClient.setQueryData(roomKey, []);
    const user = userEvent.setup();
    renderRoom(queryClient);

    // The private row is under the line saying who sees it, tagged for the coach.
    expect(screen.getByTestId('private-lane-divider')).toHaveTextContent('Only you and Dravr see what follows');
    expect(screen.getByTestId('private-reply-tag')).toHaveTextContent('Only you');
    // The draft badge is the plan thread's own.
    expect(await screen.findByTestId('plan-draft-badge')).toHaveTextContent("Draft · Alice Runner can't see it yet");
    expect(api.listParticipants).toHaveBeenCalledWith(CONVERSATION_ID);
    expect(api.listParticipants).not.toHaveBeenCalledWith(ROOM_CONVERSATION_ID);

    await user.click(screen.getByRole('button', { name: 'Publish to Alice Runner' }));

    expect(await screen.findByTestId('plan-draft-published')).toHaveTextContent(
      'Published. Alice Runner has been notified.',
    );
    expect(api.publishSubjectPlan).toHaveBeenCalledWith(CONVERSATION_ID, { overwrite_athlete_changes: false });
    await waitFor(() => expect(queryClient.getQueryState(roomKey)?.isInvalidated).toBe(true));
  });
});
