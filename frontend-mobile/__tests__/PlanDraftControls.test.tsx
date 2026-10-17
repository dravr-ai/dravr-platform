// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the draft controls a coach's plan card carries on the phone — owner-only, Publish, a refused publish, Discard
// ABOUTME: Reads the card back from a persisted transcript row through MessageList, so the block, the card and the shared hook run together

import React from 'react';
import { Alert } from 'react-native';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { DraftState, SubjectPlanResponse, WorkoutPlan } from '@pierre/shared-types';

const CONVERSATION_ID = 'conv-plan-alice';
const COACH_ID = 'user-coach';
const ATHLETE_ID = 'user-alice';

const mockListParticipants = jest.fn();
const mockGetSubjectPlan = jest.fn();
const mockPublish = jest.fn();
const mockDiscard = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    listParticipants: (...a: unknown[]) => mockListParticipants(...a),
    getSubjectPlan: (...a: unknown[]) => mockGetSubjectPlan(...a),
    publishSubjectPlan: (...a: unknown[]) => mockPublish(...a),
    discardSubjectPlanDraft: (...a: unknown[]) => mockDiscard(...a),
  },
}));

let mockViewerId = COACH_ID;
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: mockViewerId }, isAuthenticated: true }),
}));

jest.mock('../src/screens/chat/presentMessageMenu', () => ({
  presentMessageMenu: jest.fn(),
}));

import { MessageList } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

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

/** The agent's reply that saved the draft, as the transcript stores it. */
const draftReply: Message = {
  id: 'msg-plan-2',
  role: 'assistant',
  content: 'Draft saved.',
  created_at: '2026-10-16T14:05:01Z',
  scene_blocks: JSON.stringify([
    {
      kind: 'workout_plan',
      source_tool: 'save_training_plan',
      plan: PLAN,
      subject: { user_id: ATHLETE_ID, display_name: 'Alice Runner' },
      draft: DRAFT,
    },
  ]),
} as Message;

function renderThread(conversationId: string | null = CONVERSATION_ID) {
  // No garbage-collection timers: a cache entry's five-minute timer would
  // otherwise hold an in-band run open long after the last assertion.
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity },
      mutations: { retry: false, gcTime: Infinity },
    },
  });
  return render(
    <QueryClientProvider client={client}>
      <MessageList
        conversationId={conversationId}
        messages={[draftReply]}
        isLoading={false}
        isSending={false}
        messageFeedback={{}}
        messageFeedbackComment={{}}
        messageBlocks={{}}
        flatListRef={React.createRef()}
        onScrollToBottom={jest.fn()}
        onThumbsUp={jest.fn()}
        onThumbsDown={jest.fn()}
        onSubmitFeedbackReason={jest.fn()}
        onRetryMessage={jest.fn()}
        onOpenUrl={jest.fn()}
        onReconnectProvider={jest.fn()}
      />
    </QueryClientProvider>,
  );
}

describe("a coach's draft on the phone's plan card", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockViewerId = COACH_ID;
    mockListParticipants.mockResolvedValue([
      { user_id: COACH_ID, role: 'owner', added_by: COACH_ID, added_at: '2026-10-16T12:00:00Z' },
      { user_id: ATHLETE_ID, role: 'member', added_by: COACH_ID, added_at: '2026-10-16T12:01:00Z' },
    ]);
    mockGetSubjectPlan.mockResolvedValue(subjectPlan(DRAFT));
  });

  it('shows the owner the badge, Publish and Discard', async () => {
    const screen = renderThread();

    expect(await screen.findByTestId('plan-draft-badge')).toHaveTextContent("Draft · Alice Runner can't see it yet");
    expect(screen.getByTestId('plan-draft-publish')).toHaveTextContent('Publish to Alice Runner');
    expect(screen.getByTestId('plan-draft-discard')).toHaveTextContent('Discard draft');
  });

  it('shows the athlete the plan and none of the controls, and never reads the draft for them', async () => {
    mockViewerId = ATHLETE_ID;
    const screen = renderThread();

    expect(screen.getByText('Harricana 65K')).toBeTruthy();
    await waitFor(() => expect(mockListParticipants).toHaveBeenCalledWith(CONVERSATION_ID));
    expect(screen.queryByTestId('plan-draft-controls')).toBeNull();
    expect(mockGetSubjectPlan).not.toHaveBeenCalled();
  });

  it('publishes and says the athlete was told', async () => {
    mockPublish.mockResolvedValue({
      status: 'published',
      published: { plan_id: 'plan-2', superseded_plan_id: null, athlete_notified: true },
    });
    const screen = renderThread();

    fireEvent.press(await screen.findByTestId('plan-draft-publish'));

    expect(await screen.findByTestId('plan-draft-published')).toHaveTextContent(
      'Published. Alice Runner has been notified.',
    );
    expect(mockPublish).toHaveBeenCalledWith(CONVERSATION_ID, { overwrite_athlete_changes: false });
    expect(screen.queryByTestId('plan-draft-discard')).toBeNull();
  });

  it('shows what a refused publish collided with, and publishes anyway on request', async () => {
    mockPublish.mockResolvedValueOnce({
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
    mockPublish.mockResolvedValueOnce({
      status: 'published',
      published: { plan_id: 'plan-2', superseded_plan_id: 'plan-1', athlete_notified: true },
    });
    const screen = renderThread();

    fireEvent.press(await screen.findByTestId('plan-draft-publish'));

    const conflict = await screen.findByTestId('plan-draft-conflict');
    expect(conflict).toHaveTextContent(/Alice Runner's plan changed since this draft began/);
    // Both weeks are named; how they are joined is the engine's list grammar
    // where it has one (Node here), a comma on Hermes.
    expect(conflict).toHaveTextContent(/Weeks changed in both the draft and the live plan: 2026-10-19/);
    expect(conflict).toHaveTextContent(/2026-10-26/);
    expect(conflict).toHaveTextContent(/season was rebuilt after this draft began/);

    fireEvent.press(screen.getByTestId('plan-draft-overwrite'));

    expect(await screen.findByTestId('plan-draft-published')).toBeTruthy();
    expect(mockPublish).toHaveBeenLastCalledWith(CONVERSATION_ID, { overwrite_athlete_changes: true });
  });

  it('asks before discarding, then discards and takes the controls away', async () => {
    const alert = jest.spyOn(Alert, 'alert');
    mockDiscard.mockResolvedValue(undefined);
    const screen = renderThread();

    fireEvent.press(await screen.findByTestId('plan-draft-discard'));

    expect(alert).toHaveBeenCalledTimes(1);
    const [title, message, buttons] = alert.mock.calls[0];
    expect(title).toBe('Discard draft');
    expect(message).toBe("Discard this draft? Alice Runner's live plan stays as it is.");
    expect(mockDiscard).not.toHaveBeenCalled();

    mockGetSubjectPlan.mockResolvedValue(subjectPlan(null));
    const confirm = buttons?.find((button) => button.style === 'destructive');
    confirm?.onPress?.();

    await waitFor(() => expect(mockDiscard).toHaveBeenCalledWith(CONVERSATION_ID));
    await waitFor(() => expect(screen.queryByTestId('plan-draft-controls')).toBeNull());
    alert.mockRestore();
  });

  it('draws no controls on a card outside a thread', () => {
    const screen = renderThread(null);

    expect(screen.getByText('Harricana 65K')).toBeTruthy();
    expect(mockListParticipants).not.toHaveBeenCalled();
    expect(screen.queryByTestId('plan-draft-controls')).toBeNull();
  });
});
